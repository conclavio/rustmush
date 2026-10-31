//! Explicit shipped hardware host composition. These are configuration values,
//! never certified stock, money, readiness, a title or a fitting capability.
use super::{HardwareProcess, PublishedHardware};
use crate::{
    TacsimRuleset,
    config::LoadedPack,
    definitions::core_replacement::{GovernedLineCores, SurfaceRigCores},
};
use mush_core::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, ModError, ObjId,
    OwnerId, Registrar,
};
use mush_economy::{
    AccountId, CatalogRef, EconomyModule, InstrumentId, VaultId,
    core_integration::{CoreDeliveryPromise, CorePaymentPolicy, ScopedCoreAccount},
    core_refit::{CorelessBase, RefitPaymentPolicy, UngovernTerms},
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
    /// **The ungoverning refit's site** (doc 17h cut M8b C2), when composed: a
    /// facility holding `core_ungovern` and not `core_refit`, and the provider
    /// it pays. The shipped launch composes none (who runs one is the Tether's
    /// back room, doc 17h §4); a test composes one.
    pub ungovern: Option<UngoverningSite>,
    /// **A refit bay** (doc 31 cut UD.4b), when composed: a facility holding `bay_refit`,
    /// its stores and the provider it pays. The bay refits a hull within its family of
    /// fielded units, a part from its stores.
    pub bay: Option<RefitBay>,
}

/// **A refit bay** (doc 31 cut UD.4b): who is paid, where, and from which stores.
#[derive(Clone, Debug)]
pub struct RefitBay {
    pub provider: ScopedCoreAccount,
    pub facility: ObjId,
    pub stores: VaultId,
    /// The highest fee a quote may charge, centicredits; `None` takes [`BAY_MAX_FEE_MINOR`].
    pub max_fee_minor: Option<i64>,
}
/// The default ceiling on a bay's quoted fee, centicredits (provisional, per the owner's
/// economy rule): ten core fittings.
pub const BAY_MAX_FEE_MINOR: i64 = 100_000;
/// The IC hours a bay refit works (provisional).
pub const BAY_WORK_UNITS: u64 = 4;

/// **An ungoverning shop** (doc 17h cut M8b C2): who is paid and where. The same shop is the
/// ungoverning integration site (doc 17h cut M8b-2, H2), where a heirloom or grey die is
/// integrated with no certificate.
#[derive(Clone, Debug)]
pub struct UngoverningSite {
    pub provider: ScopedCoreAccount,
    pub factory: ObjId,
    /// The shop's own completed-work container (doc 17h cut M8b-2 fix round 1, L2): a
    /// back-room core is born here, never in the chartered factory's.
    pub output_container: ObjId,
    /// The shop's fee, centicredits; `None` takes the catalogue's `ungovern_minor`
    /// (provisional: the fitting fee).
    pub fee_minor: Option<i64>,
}

#[derive(Clone)]
pub(crate) struct Registration {
    pub(crate) family: Box<GovernedLineCores>,
    /// Doc 17h cut M8b C1: the rig's family, built over the line family's
    /// catalog, so its catalog is the whole fitted set.
    pub(crate) rig: Box<SurfaceRigCores>,
    content: DefinitionDescriptor,
    /// Doc 31 UD.4b: the bay recipe, registered beside the shipped hardware
    /// recipes when a bay is composed.
    pub(crate) bay_recipe: Option<DefinitionDescriptor>,
}
impl Registration {
    /// The whole fitted catalog: the fielded units, the line roots, the rig roots.
    pub(crate) fn catalog(&self) -> &crate::definitions::Catalog {
        self.rig.catalog()
    }
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
    /// Doc 31 UD.4b: the refit bay, when the deployment names one.
    bay: Option<Box<mush_industry::bay_refit::BayRefitConfig>>,
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
    bay_economy: Option<mush_economy::bay_refit::BayRefitConfig>,
    bay_parts: Option<mush_inventory::bay_part::BayPartBinding>,
}
impl RosterPolicy {
    pub(crate) fn inventory(&self, module: InventoryModule) -> InventoryModule {
        let module = module.with_core_inputs(self.state.input.clone());
        match &self.state.bay_parts {
            Some(binding) => module.with_bay_parts(binding.clone()),
            None => module,
        }
    }
    pub(crate) fn industry(&self, module: IndustryModule) -> IndustryModule {
        self.state.hardware.configure_industry(module)
    }
    pub(crate) fn economy(&self, module: EconomyModule) -> EconomyModule {
        let module = module
            .with_core_integration(self.state.economy.clone())
            .with_core_refit(self.state.fitting.clone());
        match &self.state.bay_economy {
            Some(config) => module.with_bay_refit(config.clone()),
            None => module,
        }
    }
    pub(crate) fn tactics(&self) -> TacticsModule {
        let module = TacticsModule::with_catalog(
            self.state
                .hardware
                .state
                .registration
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
                .roots()
                .cloned()
                .collect(),
        );
        self.state.hardware.with_bay(module)
    }
}
/// **The two fitting families over `layer`'s fielded catalog** (doc 17h cut M8b C1): the line
/// family, then the rig family built on its catalog. Each holds a whole catalog, so they are
/// built here, boxed, off the caller's frame (the M7 memory-size gate).
#[inline(never)]
fn families(
    layer: &TacsimRuleset,
) -> Result<(Box<GovernedLineCores>, Box<SurfaceRigCores>), ModError> {
    let catalog = crate::definitions::Catalog::build_fielded(&layer.loaded_packs()?, crate::NS)
        .map_err(bad)?;
    let family = Box::new(GovernedLineCores::build(catalog).map_err(bad)?);
    let rig = Box::new(SurfaceRigCores::build(family.catalog().clone()).map_err(bad)?);
    Ok((family, rig))
}

/// **The refit bay, when the deployment names one** (doc 31 UD.4b), and its
/// recipe; built here, off `prepare`'s frame (the M7 memory-size gate).
#[inline(never)]
#[allow(clippy::type_complexity)]
fn refit_bay(
    layer: &TacsimRuleset,
    site: Option<&super::RefitBay>,
    issuer: &SystemPrincipalId,
    instrument: &InstrumentId,
    economy: &mush_core::DefinitionRef,
) -> Result<
    (
        Option<Box<mush_industry::bay_refit::BayRefitConfig>>,
        Option<DefinitionDescriptor>,
    ),
    ModError,
> {
    let Some(site) = site else {
        return Ok((None, None));
    };
    let recipe = super::bay::recipe_descriptor()?;
    let recipe_ref = DefinitionRegistryBuilder::new()
        .register(OwnerId::new(crate::NS).map_err(bad)?, recipe.clone())
        .map_err(bad)?;
    let fielded = crate::definitions::Catalog::build_fielded(&layer.loaded_packs()?, crate::NS)
        .map_err(bad)?
        .bindings
        .values()
        .map(|b| b.spec.definition.clone())
        .collect();
    let config = super::bay::config(
        site,
        issuer.clone(),
        instrument.clone(),
        recipe_ref,
        fielded,
        economy.clone(),
    )?;
    Ok((Some(Box::new(config)), Some(recipe)))
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
                bay_economy: self
                    .state
                    .bay
                    .as_ref()
                    .map(|b| b.economy_config())
                    .transpose()
                    .map_err(bad)?,
                bay_parts: self
                    .state
                    .bay
                    .as_ref()
                    .map(|b| b.part_binding())
                    .transpose()
                    .map_err(bad)?,
            }),
        })
    }
    /// Doc 31 UD.4b: Tactics' refit participant for the bay, when composed.
    fn with_bay(&self, module: TacticsModule) -> TacticsModule {
        match self.state.bay.as_deref() {
            Some(bay) => match bay.action(mush_industry::bay_refit::INSTALL) {
                Ok(action) => module.with_component_refit(
                    bay.tactics_policy(),
                    action,
                    bay.roots().clone(),
                    bay.derived().to_vec(),
                ),
                // `prepare` already derived this action; it cannot fail here.
                Err(_) => module,
            },
            None => module,
        }
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
        let (family, rig) = families(layer)?;
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
            // Doc 17h cut M8b-2 (H2): the ungoverning shop also integrates heirloom and grey
            // dies, paid to its own provider. Absent at launch (doc 17h §4), so the launch's
            // payment policy keeps its digest.
            ungoverned: deployment.ungovern.as_ref().map(|site| {
                mush_economy::core_integration::UngovernedIntegration {
                    integrator: site.provider.clone(),
                    factory: site.factory,
                    output_container: site.output_container,
                }
            }),
        };
        let core_classes = [
            crate::arena::core::CoreClass::I,
            crate::arena::core::CoreClass::II,
            crate::arena::core::CoreClass::III,
        ];
        // Doc 17h cut M8b: one `[I, II, III]` per family, governed or the twins.
        let roots_of = |family: &crate::definitions::core_replacement::CoreFamily,
                        governed: bool|
         -> Result<[mush_core::DefinitionRef; 3], ModError> {
            core_classes
                .map(|class| family.root(class, governed).cloned())
                .into_iter()
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| bad("finite fitted family"))?
                .try_into()
                .map_err(|_| bad("three fitted roots"))
        };
        let fitted_units = roots_of(&family, true)?;
        let ungovern = match &deployment.ungovern {
            Some(site) => Some(UngovernTerms {
                provider: site.provider.clone(),
                factory: site.factory,
                recipe: publication.definition(HardwareProcess::Ungovern).clone(),
                fee_minor: site
                    .fee_minor
                    .unwrap_or(publication.catalogue().ungovern_minor()),
                twins: vec![roots_of(&family, false)?, roots_of(&rig, false)?],
            }),
            None => None,
        };
        let fitting = RefitPaymentPolicy {
            issuer: deployment.issuer,
            instrument: payment.instrument.clone(),
            provider: deployment.integrator,
            factory: deployment.factory,
            recipe: publication.definition(HardwareProcess::Refit).clone(),
            fitted_units,
            families: vec![roots_of(&rig, true)?],
            ungovern,
            // Doc 17h cut M8b G2 (the lead's ruling): the shipped coreless terrain
            // vehicle is the rig family's base; `core_refit` fits a core into it.
            coreless: vec![CorelessBase {
                base: rig
                    .catalog()
                    .units
                    .get(crate::definitions::content::TV_PROSPECTOR)
                    .ok_or_else(|| bad("the shipped rig"))?
                    .definition
                    .clone(),
                family: 1,
            }],
        };
        let component = ComponentId::new("core").map_err(bad)?;
        let roots = fitting.roots().cloned().collect::<BTreeSet<_>>();
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
        // Doc 31 UD.4b: the refit bay, refitting within the fielded units.
        let (bay, bay_recipe) = refit_bay(
            layer,
            deployment.bay.as_ref(),
            &payment.issuer,
            &payment.instrument,
            &economy,
        )?;
        let factory = CoreFactoryConfig::new(mode, payment, Arc::new(publication.clone()), economy)
            .map_err(bad)?
            .with_refits(refit.clone())
            .map_err(bad)?;
        Ok(Self {
            state: Box::new(GovernedHardwareState {
                publication,
                registration: Registration {
                    family,
                    rig,
                    content,
                    bay_recipe,
                },
                origin,
                factory,
                refit,
                bay,
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
    /// The surface rig's fitting family (doc 17h cut M8b C1).
    pub fn rig_family(&self) -> &SurfaceRigCores {
        &self.state.registration.rig
    }

    /// Uses the normal acknowledged pack-load route. Preparing composition alone
    /// does not create a publication receipt or authorize either recipe callback.
    pub fn configure_tacsim(&self, mut layer: TacsimRuleset) -> Result<TacsimRuleset, ModError> {
        if !layer.config().enabled || layer.governed_hardware.is_some() {
            return Err(bad(
                "governed hardware requires one explicitly enabled composition",
            ));
        }
        let (_, rig) = families(&layer)?;
        if rig.catalog().descriptors != self.state.registration.catalog().descriptors {
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
        let module = match self.state.bay.as_deref() {
            Some(bay) => module.with_bay_parts(bay.part_binding().map_err(bad)?),
            None => module,
        };
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
        // Doc 17h cut M8a fix round 1 (F6): tacsim reads the retained die certificates for a
        // core's provenance; every roster this composition builds carries tacsim.
        let module = module
            .with_core_origins(self.state.origin.clone().with_certificate_reader(crate::NS))
            .with_core_factory(self.state.factory.clone());
        match self.state.bay.as_deref() {
            Some(bay) => module.with_bay_refit(bay.clone()),
            None => module,
        }
    }
    /// Preserves the caller's already configured initial filing owner and all
    /// other native profiles. Registration requires the exact captured owner.
    pub fn configure_economy(&self, module: EconomyModule) -> Result<EconomyModule, ModError> {
        let module = match self.state.bay.as_deref() {
            Some(bay) => module.with_bay_refit(bay.economy_config().map_err(bad)?),
            None => module,
        };
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
        Ok(self.with_bay(
            TacticsModule::with_catalog(self.state.registration.catalog().units.clone())
                .with_component_replacement(
                    self.state.refit.replacement_policy(),
                    self.state
                        .refit
                        .action(mush_economy::core_refit::INSTALL_ACTION)
                        .map_err(bad)?,
                    self.state.refit.component().clone(),
                    self.state.refit.payment().roots().cloned().collect(),
                ),
        ))
    }
    /// Doc 31 UD.4b: the refit bay, when the deployment names one.
    pub fn bay(&self) -> Option<&mush_industry::bay_refit::BayRefitConfig> {
        self.state.bay.as_deref()
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
