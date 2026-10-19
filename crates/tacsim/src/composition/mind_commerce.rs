//! Explicit, typed composition of the original personal mind commerce owners.
//!
//! Ordinary server/module constructors never call this API. The host supplies
//! real economic terms and existing owner modules; this prepares no certificate,
//! account, grant or deployment. Initialize still requires acknowledged setup
//! followed by restart under Active and every owner's real final audit.

use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::{DefinitionRef, ModError};
use mush_economy::{EconomyModule, registry_filing::InitialFilingPolicy};
use mush_market::{
    MarketModule,
    mind_trade::{OriginalPurchaseConfig, OriginalQuoteConfig, OriginalQuotePolicy},
};
use mush_roll::{
    RollModule,
    commerce::{
        filing::InitialFilingTerms,
        module::{CommerceConfig, CommerceMode},
        purchase::PurchaseParticipants,
        purchase_writer::OriginalPurchaseWriter,
        rating::registration::commercial_roots_descriptor,
    },
};

use crate::{TacsimRuleset, minds::registry::composition::Registration};

/// Prepared composition data, not an authority or runtime resource handle.
/// Its private worker policy comes only from the actual configured Tacsim
/// catalog and genuine application of the fixed shipped Registry ground.
pub struct MindCommerce {
    registry: Registration,
    mode: CommerceMode,
    roll: CommerceConfig,
    filing: InitialFilingPolicy,
    filing_action: DefinitionRef,
    house_filing_action: Option<DefinitionRef>,
    dependent_filing_action: Option<DefinitionRef>,
    purchases: Option<OriginalPurchaseConfig>,
    entitlements: Option<mush_market::mind_entitlement::EntitlementConfig>,
    entitlement_actions: Option<[[DefinitionRef; 3]; 2]>,
    purchase_actions: Option<[DefinitionRef; 3]>,
    authored_forks: bool,
    marked_registry: bool,
}

fn error(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}

impl MindCommerce {
    /// Freeze a fixed shipped certification ground with this layer's exact
    /// validated definitions and configuration. No live Atlas data is read.
    /// Filing accounts, issuer consent and all commercial readiness remain
    /// checked by their real owners; these arguments create none of them.
    pub fn prepare(
        layer: &TacsimRuleset,
        mode: CommerceMode,
        terms: InitialFilingTerms,
        filing: InitialFilingPolicy,
    ) -> Result<Self, ModError> {
        let registry = Registration::prepare(layer, mode)?;
        let roll = CommerceConfig::new(
            mode,
            terms,
            registry.reference().clone(),
            registry.compatibility(),
        )?;
        let filing_action = roll.initial_filing_action()?;
        Ok(Self {
            registry,
            mode,
            roll,
            filing,
            filing_action,
            house_filing_action: None,
            dependent_filing_action: None,
            purchases: None,
            entitlements: None,
            entitlement_actions: None,
            purchase_actions: None,
            authored_forks: false,
            marked_registry: false,
        })
    }

    /// Explicit scoped filing only. The real curator must independently hold
    /// every required current scoped authorization. House sales/use stay closed.
    pub fn with_house_filings(mut self) -> Result<Self, ModError> {
        self.roll = self.roll.with_house_filings()?;
        self.house_filing_action = Some(self.roll.house_filing_action()?);
        Ok(self)
    }
    /// Opt in only to personal dependent filing and its current owner rights.
    pub fn with_dependent_owner_filings(mut self) -> Result<Self, ModError> {
        self.roll = self.roll.with_dependent_owner_filings()?;
        self.dependent_filing_action = Some(self.roll.dependent_filing_action()?);
        Ok(self)
    }

    /// Add only the existing original-personal Sealed Pin purchase protocol.
    /// The same private writer cell and exact action identities configure all
    /// three actual owners. No source, derivative, Follow or IC grant is added.
    pub fn with_original_purchases(self, policy: OriginalQuotePolicy) -> Result<Self, ModError> {
        self.with_purchase_protocol(policy, false)
    }

    /// Explicit form-aware v2 consent/issuance. This only enables the current
    /// original-personal Pin product; it creates no source permission or fork.
    /// Each listing must choose Sealed, Open or OpenResale under signed consent.
    pub fn with_original_source_purchases(
        self,
        policy: OriginalQuotePolicy,
    ) -> Result<Self, ModError> {
        self.with_purchase_protocol(policy, true)
    }

    /// Explicit first-generation fork preparation only. Existing source forms
    /// must already be selected; dependent editing, filing and sales stay closed.
    pub fn with_authored_forks(mut self) -> Result<Self, ModError> {
        self.roll = self.roll.with_authored_forks()?;
        self.authored_forks = true;
        Ok(self)
    }

    /// Explicit current-parent admission for saved authored forks. Rating is
    /// behavior evidence only; dependent filing/sales remain separate.
    pub fn with_marked_registry(mut self) -> Result<Self, ModError> {
        self.roll = self.roll.with_marked_registry()?;
        self.marked_registry = true;
        Ok(self)
    }

    /// Explicit personal Pin entitlement issuance. Existing original protocol
    /// remains installed for its own receipts and historical recovery.
    pub fn with_mind_entitlements(
        mut self,
        policy: mush_market::mind_entitlement::EntitlementPolicy,
    ) -> Result<Self, ModError> {
        if self.mode != CommerceMode::Active
            || self.dependent_filing_action.is_none()
            || self.entitlements.is_some()
        {
            return Err(ModError::msg(
                "entitlement purchase requires one Active dependent-owner composition",
            ));
        }
        let old = self
            .purchases
            .as_ref()
            .ok_or_else(|| ModError::msg("historical original purchase composition is missing"))?;
        let participants = old.participants().clone();
        let writer = mush_roll::commerce::entitlement::writer::EntitlementPurchaseWriter::new();
        let config = mush_market::mind_entitlement::EntitlementConfig::new(
            policy,
            self.registry.reference().clone(),
            participants.clone(),
            writer.clone(),
        )?;
        use mush_economy::managed_entitlement::EntitlementProtocol::{DerivedV1, OriginalV3};
        let original = config.owner_actions(OriginalV3)?;
        let derived = config.owner_actions(DerivedV1)?;
        self.roll = self
            .roll
            .with_mind_entitlements(writer, participants, original, derived)?;
        self.entitlement_actions = Some([
            config.purchase_actions(OriginalV3)?,
            config.purchase_actions(DerivedV1)?,
        ]);
        self.entitlements = Some(config);
        Ok(self)
    }

    fn with_purchase_protocol(
        mut self,
        policy: OriginalQuotePolicy,
        source_forms: bool,
    ) -> Result<Self, ModError> {
        if self.mode != CommerceMode::Active || self.purchases.is_some() {
            return Err(ModError::msg(
                "original purchases require one Active commerce configuration",
            ));
        }
        let economy = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new("economy").map_err(error)?,
                DefinitionDescriptor::new(
                    DefinitionKind::new("owner_participant").map_err(error)?,
                    DefinitionId::new("initial_filing").map_err(error)?,
                    1,
                    self.filing.definition_payload(),
                )
                .map_err(error)?,
            )
            .map_err(error)?;
        let retention = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new("roll").map_err(error)?,
                commercial_roots_descriptor()?,
            )
            .map_err(error)?;
        let participants = PurchaseParticipants::new(economy, retention).map_err(error)?;
        let writer = OriginalPurchaseWriter::new();
        let quotes = OriginalQuoteConfig::new(policy, self.registry.reference().clone())?;
        let quotes = if source_forms {
            quotes.with_source_forms()
        } else {
            quotes
        };
        let purchases = OriginalPurchaseConfig::new(quotes, participants.clone(), writer.clone())?;
        let actions = purchases.actions()?;
        self.roll = self
            .roll
            .with_original_purchases(writer, participants, actions.clone())?;
        self.purchases = Some(purchases);
        self.purchase_actions = Some(actions);
        Ok(self)
    }

    /// Structural identity for trusted roster configuration only. The real
    /// Tacsim registrar must install this exact descriptor before freeze.
    pub fn manifest(&self) -> &DefinitionRef {
        self.registry.reference()
    }

    pub(crate) fn filing_issuer(&self) -> &mush_org::SystemPrincipalId {
        self.filing.issuer()
    }

    /// Preserve the actual layer and all its boot hooks. A stale configuration
    /// or a different loaded catalog refuses instead of silently recapturing.
    pub fn configure_tacsim(&self, mut layer: TacsimRuleset) -> Result<TacsimRuleset, ModError> {
        if layer.registry_commerce.is_some() || layer.original_holdings.is_some() {
            return Err(ModError::msg(
                "Tacsim already has explicit Registry or filed holdings configuration",
            ));
        }
        let catalog = crate::definitions::Catalog::build_fielded(&layer.loaded_packs()?, crate::NS)
            .map_err(error)?;
        self.registry.check(layer.config(), &catalog)?;
        if self.mode == CommerceMode::Active {
            layer = if self.entitlements.is_some() {
                layer.with_mind_entitlement_holdings(self.manifest().clone())
            } else if self.dependent_filing_action.is_some() {
                layer.with_dependent_owner_holdings(self.manifest().clone())
            } else if self.purchases.is_some() {
                if self
                    .purchase_actions
                    .as_ref()
                    .is_some_and(|actions| actions[0].version() == 2)
                {
                    layer.with_personal_filed_source_forms(self.manifest().clone())
                } else {
                    layer.with_personal_filed_holdings(self.manifest().clone())
                }
            } else {
                layer.with_original_holdings(self.manifest().clone())
            };
        }
        if self.authored_forks {
            layer.fork_material = Some(self.manifest().clone());
        }
        if self.marked_registry {
            layer.marked_registry = Some(self.manifest().clone());
        }
        layer.registry_commerce = Some(self.registry.clone());
        Ok(layer)
    }

    /// Keep the supplied Roll module's real hooks and source writers. The host
    /// must include Tacsim in those writers, as in the ordinary game roster.
    pub fn configure_roll(&self, module: RollModule) -> RollModule {
        let module = module.with_commerce(self.roll.clone());
        let module = if self.authored_forks {
            module.with_authored_fork_source::<crate::minds::workshop::fork_material::ForkMaterialReader>()
        } else {
            module
        };
        if self.mode == CommerceMode::Active {
            crate::minds::registry::configure_roll(module)
        } else {
            module
        }
    }

    /// Preserve custody, facts, observers and existing title issuers. The host
    /// must already allow the configured filing issuer; no issuer is added here.
    pub fn configure_economy(&self, module: EconomyModule) -> EconomyModule {
        let module = module.with_initial_filing(self.filing.clone(), self.filing_action.clone());
        let module = match &self.house_filing_action {
            Some(action) => module.with_house_initial_filing(action.clone()),
            None => module,
        };
        let module = match &self.dependent_filing_action {
            Some(action) => module.with_dependent_initial_filing(action.clone()),
            None => module,
        };
        let module = match &self.purchase_actions {
            Some(actions) => module.with_original_mind_purchases(actions.clone()),
            None => module,
        };
        match &self.entitlement_actions {
            Some([original, derived]) => {
                module.with_mind_entitlement_purchases(original.clone(), derived.clone())
            }
            None => module,
        }
    }

    /// Preserve the real venue gate and inventory/IC settings. The closed
    /// writer binding is checked at final definition freeze, so the existing
    /// Market-before-Roll order remains intact.
    pub fn configure_market(&self, module: MarketModule) -> MarketModule {
        let module = match &self.purchases {
            Some(config) => module.with_original_mind_purchases(config.clone()),
            None => module,
        };
        match &self.entitlements {
            Some(config) => module.with_mind_entitlements(config.clone()),
            None => module,
        }
    }
}

#[cfg(test)]
mod tests;
