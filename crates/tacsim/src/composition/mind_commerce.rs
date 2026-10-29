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

#[cfg(test)]
#[path = "mind_commerce/conduct_tests.rs"]
mod conduct_tests;

/// Prepared composition data, not an authority or runtime resource handle.
/// Its private worker policy comes only from the actual configured Tacsim
/// catalog and genuine application of the fixed shipped Registry ground.
pub struct MindCommerce {
    state: Box<MindCommerceState>,
}

struct MindCommerceState {
    registry: Registration,
    conduct: Option<crate::minds::registry::conduct::composition::Registration>,
    coverage: Option<crate::underwriting::CoverageConfiguration>,
    minds_facts: Option<mush_roll::commerce::facts::MindsFactProvider>,
    ic: Option<crate::minds::ic_sortie::registration::Composition>,
    mode: CommerceMode,
    roll: CommerceConfig,
    filing: InitialFilingPolicy,
    filing_action: DefinitionRef,
    compatible_update_action: Option<DefinitionRef>,
    derived_update_action: Option<DefinitionRef>,
    charter_transfer_action: Option<DefinitionRef>,
    registry_revocation_action: Option<DefinitionRef>,
    registry_inspection: Option<crate::minds::registry_inspection::Composition>,
    paid_resales: Option<mush_market::mind_paid_resale::PaidResaleConfig>,
    pending_charters: Option<(
        mush_roll::commerce::PendingCharterTransferConfig,
        mush_economy::managed_charter_transfer::CharterPendingConfig,
    )>,
    house_filing_action: Option<DefinitionRef>,
    house_purchases: Option<mush_market::mind_house::HouseConfig>,
    house_purchase_actions: Option<[DefinitionRef; 3]>,
    two_seat_house_purchase_actions: Option<[DefinitionRef; 3]>,
    house_renewals: Option<mush_market::mind_house_renewal::RenewalConfig>,
    house_renewal_actions: Option<[DefinitionRef; 5]>,
    house_renewal_service: Option<DefinitionRef>,
    // 17g C1a: the House chain's buyer admission, kept so a later paid resale
    // writer repeats it for a resold House Pin.
    house_admission:
        Option<std::sync::Arc<dyn mush_roll::commerce::house_purchase::HouseBuyerAdmission>>,
    // 17g C1a: personal (entitlement-born) renewals and their bounded service.
    renewals: Option<mush_market::mind_renewal::RenewalConfig>,
    renewal_actions: Option<[DefinitionRef; 5]>,
    renewal_service: Option<DefinitionRef>,
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
    /// Opt in the actual five-action metered Conduct service on a catalogue
    /// that already contains its bound carrier. No trial or result is minted.
    pub fn with_conduct_trials(mut self, layer: &TacsimRuleset) -> Result<Self, ModError> {
        if self.state.conduct.is_some() {
            return Err(ModError::msg("Conduct already configured"));
        }
        self.state
            .registry
            .check(layer.config(), &layer.definition_catalog()?)?;
        let conduct = crate::minds::registry::conduct::composition::Registration::prepare(
            layer,
            self.state.mode,
            self.manifest(),
        )?;
        self.state.roll = self
            .state
            .roll
            .with_conduct_trials(conduct.roll_config()?)?;
        self.state.conduct = Some(conduct);
        Ok(self)
    }
    pub fn conduct_manifest(&self) -> Option<&DefinitionRef> {
        self.state.conduct.as_ref().map(|c| c.reference())
    }

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
            state: Box::new(MindCommerceState {
                registry,
                conduct: None,
                coverage: None,
                minds_facts: None,
                ic: None,
                mode,
                roll,
                filing,
                filing_action,
                compatible_update_action: None,
                derived_update_action: None,
                charter_transfer_action: None,
                registry_revocation_action: None,
                registry_inspection: None,
                pending_charters: None,
                paid_resales: None,
                house_filing_action: None,
                house_purchases: None,
                house_purchase_actions: None,
                two_seat_house_purchase_actions: None,
                house_renewals: None,
                house_renewal_actions: None,
                house_renewal_service: None,
                house_admission: None,
                renewals: None,
                renewal_actions: None,
                renewal_service: None,
                dependent_filing_action: None,
                purchases: None,
                entitlements: None,
                entitlement_actions: None,
                purchase_actions: None,
                authored_forks: false,
                marked_registry: false,
            }),
        })
    }

    /// Explicit issuer-bound renewal offers. A host may enable the bounded
    /// service separately; neither option accepts or pays on the buyer's behalf.
    pub fn with_house_mind_renewals(
        mut self,
        policy: mush_market::mind_house_renewal::RenewalPolicy,
    ) -> Result<Self, ModError> {
        if self.state.mode != CommerceMode::Active || self.state.house_renewals.is_some() {
            return Err(ModError::msg(
                "House renewals require one Active composition",
            ));
        }
        let house = self
            .state
            .house_purchases
            .as_ref()
            .ok_or_else(|| ModError::msg("House renewals require House purchases"))?;
        let config = mush_market::mind_house_renewal::RenewalConfig::new(
            house,
            mush_roll::commerce::house_renewal::writer::RenewalWriter::new(),
            policy,
        )?;
        let actions = config.actions()?;
        self.state.roll = self.state.roll.with_house_mind_renewals(
            config.writer(),
            config.participants().clone(),
            actions.clone(),
        )?;
        self.state.house_renewal_actions = Some(actions);
        self.state.house_renewals = Some(config);
        Ok(self)
    }
    /// Source-free offer/status and acknowledged terminal continuations.
    pub fn with_house_mind_renewal_service(mut self) -> Result<Self, ModError> {
        if self.state.house_renewal_service.is_some() {
            return Err(ModError::msg("House renewal service already configured"));
        }
        let config = self
            .state
            .house_renewals
            .take()
            .ok_or_else(|| ModError::msg("House renewal service requires renewals"))?
            .with_service()?;
        let action = config
            .service_action()?
            .ok_or_else(|| ModError::msg("House renewal service action"))?;
        self.state.roll = self
            .state
            .roll
            .with_house_mind_renewal_service(action.clone())?;
        self.state.house_renewal_service = Some(action);
        self.state.house_renewals = Some(config);
        Ok(self)
    }

    /// 17g C1a: explicit personal Pin renewal offers, the pair of
    /// [`Self::with_house_mind_renewals`] for entitlement-born licences. Prices
    /// and terms come from each licence's immutable acquired template; this
    /// policy carries only the Registry and venue cuts and the offer window.
    /// It needs the Active entitlement composition and the personal purchase
    /// participants it shares. No offer is accepted or paid for a holder.
    ///
    /// # Errors
    /// Initialize, a second call, no entitlements or personal purchases yet,
    /// or the Market/Roll owners' own refusal of the policy or references.
    pub fn with_mind_renewals(
        mut self,
        policy: mush_market::mind_renewal::RenewalPolicy,
    ) -> Result<Self, ModError> {
        if self.state.mode != CommerceMode::Active
            || self.state.renewals.is_some()
            || self.state.entitlements.is_none()
        {
            return Err(ModError::msg(
                "personal renewals require one Active entitlement composition",
            ));
        }
        let participants = self
            .state
            .purchases
            .as_ref()
            .map(|p| p.participants().clone())
            .ok_or_else(|| {
                ModError::msg("personal renewals require one Active entitlement composition")
            })?;
        let config = mush_market::mind_renewal::RenewalConfig::new(
            self.manifest().clone(),
            participants,
            mush_roll::commerce::renewal::writer::RenewalWriter::new(),
            policy,
        )?;
        let actions = config.actions()?;
        self.state.roll = self.state.roll.with_mind_renewals(
            config.writer(),
            config.participants().clone(),
            actions.clone(),
        )?;
        self.state.renewal_actions = Some(actions);
        self.state.renewals = Some(config);
        Ok(self)
    }
    /// 17g C1a: the personal renewals' source-free close of an unavailable
    /// unfunded offer, the shape of [`Self::with_house_mind_renewal_service`].
    /// Its service actor is fixed by the as-built protocol.
    ///
    /// # Errors
    /// A second call, no personal renewals, or the owners' own refusal.
    pub fn with_mind_renewal_service(mut self) -> Result<Self, ModError> {
        if self.state.renewal_service.is_some() {
            return Err(ModError::msg("personal renewal service already configured"));
        }
        let config = self
            .state
            .renewals
            .take()
            .ok_or_else(|| ModError::msg("personal renewal service requires renewals"))?
            .with_service()?;
        let action = config
            .service_action()?
            .ok_or_else(|| ModError::msg("personal renewal service action"))?;
        self.state.roll = self.state.roll.with_mind_renewal_service(action.clone())?;
        self.state.renewal_service = Some(action);
        self.state.renewals = Some(config);
        Ok(self)
    }

    /// 17g C1a: personal initial-author withdrawal (stop fresh offers for a
    /// filed revision). The Roll owner's guards are the refusals: Active, once.
    /// No source, House, transferred-holder or Registry recall authority.
    ///
    /// # Errors
    /// Initialize, or a second call.
    pub fn with_author_withdrawal(mut self) -> Result<Self, ModError> {
        self.state.roll = self.state.roll.with_author_withdrawal()?;
        Ok(self)
    }

    /// 17g C1a: free personal owner-licence generation transitions. The Roll
    /// owner's guards are the refusals: Active, once. No monetary participant
    /// or paid-customer row is enabled.
    ///
    /// # Errors
    /// Initialize, or a second call.
    pub fn with_owner_attestations(mut self) -> Result<Self, ModError> {
        self.state.roll = self.state.roll.with_owner_attestations()?;
        Ok(self)
    }

    /// 17g C1a: replace a personal dependent owner's current qualifying parent
    /// (Q-UM-17). The Roll owner's guards are the refusals: Active, once, and
    /// after [`Self::with_dependent_owner_filings`]. No term is issued.
    ///
    /// # Errors
    /// Initialize, a second call, or no dependent owners yet.
    pub fn with_owner_parent_qualification(mut self) -> Result<Self, ModError> {
        self.state.roll = self.state.roll.with_owner_parent_qualification()?;
        Ok(self)
    }

    /// Re-conform exact retained filed revisions under the current Registry.
    /// Default composition starts no automatic work or recall service.
    pub fn with_registry_reconformance(mut self) -> Result<Self, ModError> {
        self.state.roll = self.state.roll.with_registry_reconformance()?;
        self.state.registry = self.state.registry.with_reconformance()?;
        Ok(self)
    }

    /// Historical public filed facts and scoped personal term-lapse occurrences.
    /// This opts in the same opaque provider in the actual Roll and Economy
    /// owners; it grants no source, transfer or house authority.
    pub fn with_minds_facts(mut self) -> Result<Self, ModError> {
        if self.state.mode != CommerceMode::Active || self.state.minds_facts.is_some() {
            return Err(ModError::msg("minds facts require one Active composition"));
        }
        let provider = mush_roll::commerce::facts::MindsFactProvider::new();
        self.state.roll = self.state.roll.with_minds_facts(provider.clone())?;
        self.state.minds_facts = Some(provider);
        self.install_registry_findings()?;
        Ok(self)
    }

    /// Doc 17h cut M8d D6: with both the minds facts and the inspection composed (in either
    /// order), the public findings `minds.unwarranted` and `minds.seized` read the
    /// inspection's chronicle.
    fn install_registry_findings(&self) -> Result<(), ModError> {
        if let (Some(provider), Some(_)) =
            (&self.state.minds_facts, &self.state.registry_inspection)
        {
            provider.with_registry_findings(std::sync::Arc::new(
                crate::minds::registry_inspection::unwarranted::Findings,
            ))?;
        }
        Ok(())
    }

    /// Explicit fixed-base IC custody. No host event or start transition is
    /// minted, and ordinary rosters retain their previous configuration.
    pub fn with_ic_minds(
        mut self,
        bases: Vec<mush_sim::ic_exclusion::ConfiguredIcBase>,
    ) -> Result<Self, ModError> {
        if self.state.mode != CommerceMode::Active || self.state.ic.is_some() {
            return Err(ModError::msg(
                "IC requires explicit Active composition once",
            ));
        }
        let ic = crate::minds::ic_sortie::registration::Composition::new(
            self.manifest().clone(),
            bases,
        )?;
        self.state.roll = self
            .state
            .roll
            .with_ic_minds::<crate::minds::ic_sortie::IcSortieParticipant>(
                ic.claims.clone(),
                ic.writer.clone(),
            )?;
        self.state.ic = Some(ic);
        Ok(self)
    }
    /// Explicit ordinary hull cover. This composes real owners but produces no
    /// region event, withheld departure permit or qualified conduct proof.
    pub fn with_hull_coverage(
        mut self,
        service: mush_core::ObjId,
        products: Vec<crate::underwriting::CoverageProductV1>,
    ) -> Result<Self, ModError> {
        if self.state.mode != CommerceMode::Active
            || self.state.ic.is_none()
            || self.state.coverage.is_some()
        {
            return Err(ModError::msg(
                "coverage requires one actual Active IC composition",
            ));
        }
        let economy = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new("economy").map_err(error)?,
                DefinitionDescriptor::new(
                    DefinitionKind::new("owner_participant").map_err(error)?,
                    DefinitionId::new("initial_filing").map_err(error)?,
                    1,
                    self.state.filing.definition_payload(),
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
        let coverage = crate::underwriting::CoverageConfiguration::new(
            self.manifest().clone(),
            PurchaseParticipants::new(economy, retention).map_err(error)?,
            service,
            products,
        )?;
        self.state.roll = coverage.configure_roll(self.state.roll)?;
        self.state.coverage = Some(coverage);
        Ok(self)
    }
    /// Qualified cover requires the actual registered Conduct owner; selectors
    /// alone grant no publication, discount or departure authority.
    pub fn with_qualified_hull_coverage(
        mut self,
        service: mush_core::ObjId,
        products: Vec<crate::underwriting::CoverageProductV1>,
        conduct_manifest: mush_core::DefinitionRef,
        selections: std::collections::BTreeMap<
            mush_core::DefinitionDigest,
            Vec<mush_core::CommitToken>,
        >,
    ) -> Result<Self, ModError> {
        if self.state.mode != CommerceMode::Active
            || self.state.ic.is_none()
            || self.state.coverage.is_some()
        {
            return Err(ModError::msg(
                "coverage requires one actual Active IC composition",
            ));
        }
        let economy = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new("economy").map_err(error)?,
                DefinitionDescriptor::new(
                    DefinitionKind::new("owner_participant").map_err(error)?,
                    DefinitionId::new("initial_filing").map_err(error)?,
                    1,
                    self.state.filing.definition_payload(),
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
        let coverage = crate::underwriting::CoverageConfiguration::with_conduct(
            self.manifest().clone(),
            PurchaseParticipants::new(economy, retention).map_err(error)?,
            service,
            products,
            conduct_manifest,
            selections,
        )?;
        self.state.roll = coverage.configure_roll(self.state.roll)?;
        self.state.coverage = Some(coverage);
        Ok(self)
    }
    pub fn configure_ic_physical(
        &self,
        tactics: mush_tactics::TacticsModule,
        sim: mush_sim::SimulationModule,
    ) -> (mush_tactics::TacticsModule, mush_sim::SimulationModule) {
        match &self.state.ic {
            Some(ic) => ic.physical(tactics, sim),
            None => (tactics, sim),
        }
    }
    /// Explicit personal charter offers and immediate native payment/title
    /// settlement. Open OwnedDispatch claims refuse acceptance; this enables no
    /// paid-licence resale, pending hold, automatic acceptance or source grant.
    pub fn with_charter_transfers(mut self) -> Result<Self, ModError> {
        let config = mush_roll::commerce::charter_transfer::CharterTransferConfig::new();
        let [_, accept] = config.actions()?;
        self.state.roll = self.state.roll.with_charter_transfers(config)?;
        self.state.charter_transfer_action = Some(accept);
        Ok(self)
    }

    /// Explicit named-buyer escrow and trusted terminal processing. Existing
    /// claims retain their original warrants; this never accepts or debits for a buyer.
    pub fn with_pending_charter_transfers(
        mut self,
        config: mush_roll::commerce::PendingCharterTransferConfig,
    ) -> Result<Self, ModError> {
        if self.state.mode != CommerceMode::Active
            || self.state.pending_charters.is_some()
            || self.state.charter_transfer_action.is_none()
        {
            return Err(ModError::msg(
                "pending charters require one Active charter composition",
            ));
        }
        let economy = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new("economy").map_err(error)?,
                DefinitionDescriptor::new(
                    DefinitionKind::new("owner_participant").map_err(error)?,
                    DefinitionId::new("initial_filing").map_err(error)?,
                    1,
                    self.state.filing.definition_payload(),
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
        let native = config.economy_config(
            self.state.filing.issuer().clone(),
            self.state.filing.catalog().clone(),
            self.state.registry_revocation_action.clone(),
        )?;
        self.state.roll = self
            .state
            .roll
            .with_pending_charter_transfers(config.clone(), participants)?;
        self.state.pending_charters = Some((config, native));
        Ok(self)
    }

    /// Explicit positive-fee original-author updates; no new major or paid Follow.
    pub fn with_compatible_updates(mut self) -> Result<Self, ModError> {
        self.state.roll = self.state.roll.with_compatible_updates()?;
        self.state.compatible_update_action = Some(self.state.roll.compatible_update_action()?);
        Ok(self)
    }

    /// Explicit compatible revisions for an already filed personal derivative.
    /// Reuses the real parent-reader and fee/root owner slots.
    pub fn with_derived_updates(mut self) -> Result<Self, ModError> {
        self.state.roll = self.state.roll.with_derived_updates()?;
        self.state.derived_update_action = Some(self.state.roll.derived_update_action()?);
        Ok(self)
    }

    /// 17g C0: the House owner's free licence term, distinct from the personal
    /// term `prepare` took. Set it before [`Self::with_house_filings`], which
    /// fixes the House filing action's definition (the term is part of it).
    pub fn with_house_owner_terms(mut self, terms: InitialFilingTerms) -> Result<Self, ModError> {
        if self.state.house_filing_action.is_some() {
            return Err(ModError::msg(
                "the House owner term is set before House filing",
            ));
        }
        self.state.roll = self.state.roll.with_house_owner_terms(terms)?;
        Ok(self)
    }

    /// Explicit scoped filing only. The real curator must independently hold
    /// every required current scoped authorization. House sales/use stay closed.
    pub fn with_house_filings(mut self) -> Result<Self, ModError> {
        self.state.roll = self.state.roll.with_house_filings()?;
        self.state.house_filing_action = Some(self.state.roll.house_filing_action()?);
        Ok(self)
    }
    /// Opt in only to personal dependent filing and its current owner rights.
    pub fn with_dependent_owner_filings(mut self) -> Result<Self, ModError> {
        self.state.roll = self.state.roll.with_dependent_owner_filings()?;
        self.state.dependent_filing_action = Some(self.state.roll.dependent_filing_action()?);
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
        self.state.roll = self.state.roll.with_authored_forks()?;
        self.state.authored_forks = true;
        Ok(self)
    }

    /// Explicit current-parent admission for saved authored forks. Rating is
    /// behavior evidence only; dependent filing/sales remain separate.
    pub fn with_marked_registry(mut self) -> Result<Self, ModError> {
        self.state.roll = self.state.roll.with_marked_registry()?;
        self.state.marked_registry = true;
        Ok(self)
    }

    /// Explicit organization-issued sealed personal purchases. Curator publication
    /// and the organization's title/treasury remain distinct from buyer rights.
    /// No default catalogue, account, grant or renewal activation is created.
    pub fn with_house_minds(
        self,
        policy: mush_market::mind_house::HousePolicy,
    ) -> Result<Self, ModError> {
        self.with_house_minds_and_admission(policy, None)
    }
    /// Trusted refusal-only launch policy; all owner authority remains unchanged.
    /// 17g C1a: the admission is also kept for a later
    /// [`Self::with_paid_mind_resales`], whose writer then holds a resold House
    /// Pin's buyer to it. So an admission must come before paid resale.
    pub fn with_house_minds_and_admission(
        mut self,
        policy: mush_market::mind_house::HousePolicy,
        admission: Option<
            std::sync::Arc<dyn mush_roll::commerce::house_purchase::HouseBuyerAdmission>,
        >,
    ) -> Result<Self, ModError> {
        if self.state.mode != CommerceMode::Active
            || self.state.house_filing_action.is_none()
            || self.state.house_purchases.is_some()
        {
            return Err(ModError::msg(
                "house purchases require one Active house filing composition",
            ));
        }
        // 17g C1a: a paid resale writer composed earlier would sell a House
        // Pin without the House's condition. The order is a real guard.
        if admission.is_some() && self.state.paid_resales.is_some() {
            return Err(ModError::msg(
                "house buyer admission must precede paid resale",
            ));
        }
        let economy = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new("economy").map_err(error)?,
                DefinitionDescriptor::new(
                    DefinitionKind::new("owner_participant").map_err(error)?,
                    DefinitionId::new("initial_filing").map_err(error)?,
                    1,
                    self.state.filing.definition_payload(),
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
        let writer = mush_roll::commerce::house_purchase::writer::HousePurchaseWriter::new();
        let writer = match &admission {
            Some(policy) => writer.with_buyer_admission(policy.clone())?,
            None => writer,
        };
        let config = mush_market::mind_house::HouseConfig::new(
            policy,
            self.manifest().clone(),
            participants.clone(),
            writer.clone(),
        )?;
        self.state.roll = self.state.roll.with_house_mind_purchases(
            writer,
            participants,
            config.owner_actions(mush_roll::commerce::house_purchase::HouseProtocol::HouseV1)?,
        )?;
        self.state.house_purchase_actions = Some(
            config.purchase_actions(mush_roll::commerce::house_purchase::HouseProtocol::HouseV1)?,
        );
        self.state.house_purchases = Some(config);
        self.state.house_admission = admission;
        Ok(self)
    }

    /// Explicitly offer a second immutable two-seat House Pin. Configure this
    /// before renewals so their current template policy set is captured exactly.
    pub fn with_two_seat_house_pins(
        mut self,
        policy: mush_market::mind_house::HousePolicy,
    ) -> Result<Self, ModError> {
        if self.state.mode != CommerceMode::Active
            || self.state.two_seat_house_purchase_actions.is_some()
            || self.state.house_renewals.is_some()
        {
            return Err(ModError::msg(
                "two-seat House pins require one Active purchase composition before renewals",
            ));
        }
        let house = self
            .state
            .house_purchases
            .take()
            .ok_or_else(|| ModError::msg("House purchases unavailable"))?
            .with_two_seat_pins(policy)?;
        let protocol = mush_roll::commerce::house_purchase::HouseProtocol::HouseV2;
        self.state.roll = self
            .state
            .roll
            .with_two_seat_house_pins(house.owner_actions(protocol)?)?;
        self.state.two_seat_house_purchase_actions = Some(house.purchase_actions(protocol)?);
        self.state.house_purchases = Some(house);
        Ok(self)
    }

    /// Explicit personal Pin entitlement issuance. Existing original protocol
    /// remains installed for its own receipts and historical recovery.
    pub fn with_mind_entitlements(
        mut self,
        policy: mush_market::mind_entitlement::EntitlementPolicy,
    ) -> Result<Self, ModError> {
        if self.state.mode != CommerceMode::Active
            || self.state.dependent_filing_action.is_none()
            || self.state.entitlements.is_some()
        {
            return Err(ModError::msg(
                "entitlement purchase requires one Active dependent-owner composition",
            ));
        }
        let old =
            self.state.purchases.as_ref().ok_or_else(|| {
                ModError::msg("historical original purchase composition is missing")
            })?;
        let participants = old.participants().clone();
        let writer = mush_roll::commerce::entitlement::writer::EntitlementPurchaseWriter::new();
        let config = mush_market::mind_entitlement::EntitlementConfig::new(
            policy,
            self.state.registry.reference().clone(),
            participants.clone(),
            writer.clone(),
        )?;
        use mush_economy::managed_entitlement::EntitlementProtocol::{DerivedV1, OriginalV3};
        let original = config.owner_actions(OriginalV3)?;
        let derived = config.owner_actions(DerivedV1)?;
        self.state.roll =
            self.state
                .roll
                .with_mind_entitlements(writer, participants, original, derived)?;
        self.state.entitlement_actions = Some([
            config.purchase_actions(OriginalV3)?,
            config.purchase_actions(DerivedV1)?,
        ]);
        self.state.entitlements = Some(config);
        Ok(self)
    }

    /// Optional transfer of an existing personal paid instrument. Captured
    /// term dates, seats, source form, royalty basis and old warrants survive.
    /// No offer is accepted and no account is selected by this constructor.
    pub fn with_paid_mind_resales(
        mut self,
        service: mush_core::ObjId,
        offer_seconds: i64,
    ) -> Result<Self, ModError> {
        if self.state.mode != CommerceMode::Active || self.state.paid_resales.is_some() {
            return Err(ModError::msg(
                "paid resale requires one Active configuration",
            ));
        }
        let participants = self
            .state
            .purchases
            .as_ref()
            .map(|p| p.participants().clone())
            .or_else(|| {
                self.state
                    .house_purchases
                    .as_ref()
                    .map(|p| p.participants().clone())
            })
            .ok_or_else(|| ModError::msg("paid resale requires an actual purchase composition"))?;
        let writer = mush_roll::commerce::paid_resale::writer::PaidResaleWriter::new();
        // 17g C1a: a House chain's buyer admission (the launch's Coprates
        // membership) also binds the buyer of a resold House Pin. The one
        // writer reaches Roll and Market both, so their fences agree.
        let writer = match &self.state.house_admission {
            Some(policy) => writer.with_buyer_admission(policy.clone())?,
            None => writer,
        };
        let config = mush_market::mind_paid_resale::PaidResaleConfig::new(
            self.state.filing.issuer().clone(),
            service,
            offer_seconds,
            participants.clone(),
            writer.clone(),
            self.state.registry_revocation_action.clone(),
        )?;
        self.state.roll = self
            .state
            .roll
            .with_paid_mind_resale(writer, participants, service)?;
        self.state.paid_resales = Some(config);
        Ok(self)
    }

    /// Explicit scoped Registry authority for one current accepted term.
    /// Existing terms and captured sorties are not rewritten or reclaimed.
    pub fn with_registry_revocation(
        mut self,
        policy: mush_roll::commerce::registry_revocation::RegistryRevocationPolicy,
    ) -> Result<Self, ModError> {
        let action = policy.action()?;
        if let Some((config, native)) = &mut self.state.pending_charters {
            *native = config.economy_config(
                self.state.filing.issuer().clone(),
                self.state.filing.catalog().clone(),
                Some(action.clone()),
            )?;
        }
        if let Some(config) = self.state.paid_resales.take() {
            self.state.paid_resales = Some(config.with_registry_revocation(action.clone())?);
        }
        self.state.registry_revocation_action = Some(action);
        self.state.roll = self.state.roll.with_registry_revocation(policy)?;
        Ok(self)
    }

    /// Enable exact filed-revision Registry recall under configured live Org
    /// authorization. Historical holder readback remains independent of the
    /// currently configured authority; no source or sortie is rewritten.
    pub fn with_registry_recall(
        mut self,
        policy: mush_roll::commerce::registry_recall::RegistryRecallPolicy,
    ) -> Result<Self, ModError> {
        self.state.roll = self.state.roll.with_registry_recall(policy)?;
        Ok(self)
    }

    /// Doc 17h cut M8a: the Registry inspection (`@registry/inspect <hull>`) under the
    /// configured live Org authorization: one read of a hull, one record; nothing is
    /// rewritten, granted or reclaimed. The Roll takes the policy and the record family's
    /// writer; the layer takes the action, outcome, provider and command slot.
    pub fn with_registry_inspection(
        mut self,
        policy: mush_roll::commerce::registry_inspection::RegistryInspectionPolicy,
    ) -> Result<Self, ModError> {
        if self.state.registry_inspection.is_some() {
            return Err(ModError::msg("Registry inspection already configured"));
        }
        let writer = mush_roll::commerce::registry_inspection::InspectionWriter::new();
        self.state.roll = self
            .state
            .roll
            .with_registry_inspection(policy.clone(), writer.clone())?;
        self.state.registry_inspection = Some(crate::minds::registry_inspection::Composition {
            policy,
            writer,
            fines: None,
            principals: None,
            claims: false,
        });
        self.install_registry_findings()?;
        Ok(self)
    }

    /// Doc 17h cut M8d D3: `@registry/fine` over the composed inspection — the inspection's
    /// operators holding `roll.registry.fine`, the fine paid to `credit` (the deployment's
    /// levy sink). Needs [`Self::with_registry_inspection`] first.
    pub fn with_registry_fines(
        mut self,
        credit: mush_economy::AccountId,
    ) -> Result<Self, ModError> {
        let inspection = self
            .state
            .registry_inspection
            .as_mut()
            .ok_or_else(|| ModError::msg("Registry fines need the Registry inspection"))?;
        if inspection.fines.is_some() {
            return Err(ModError::msg("Registry fines already configured"));
        }
        inspection.fines = Some(credit);
        Ok(self)
    }

    /// **Doc 17h cut M8g G3: the quirk claim** over the composed inspection — `@mind/claim`
    /// for a charter title holder naming hulls they earned, filed with the inspection's
    /// Registry, and `@registry/claims` for its operators holding `roll.registry.claims`.
    /// Needs [`Self::with_registry_inspection`] first.
    pub fn with_registry_claims(mut self) -> Result<Self, ModError> {
        let inspection = self
            .state
            .registry_inspection
            .as_mut()
            .ok_or_else(|| ModError::msg("Registry claims need the Registry inspection"))?;
        if inspection.claims {
            return Err(ModError::msg("Registry claims already configured"));
        }
        inspection.claims = true;
        Ok(self)
    }

    /// Doc 17h cut M8d D4: `@registry/revoke principal=` over the composed inspection — the
    /// inspection's operators holding `roll.registry.revoke`, barring a principal for `days`
    /// IC days (`[tacsim.minds].revocation_ic_days`). Needs [`Self::with_registry_inspection`].
    pub fn with_registry_principal_revocation(mut self, days: u32) -> Result<Self, ModError> {
        let inspection =
            self.state.registry_inspection.as_mut().ok_or_else(|| {
                ModError::msg("principal revocation needs the Registry inspection")
            })?;
        if days == 0 {
            return Err(ModError::msg(
                "Registry principal revocation needs a term of at least one IC day",
            ));
        }
        if inspection.principals.is_some() {
            return Err(ModError::msg(
                "Registry principal revocation already configured",
            ));
        }
        inspection.principals = Some(days);
        Ok(self)
    }

    fn with_purchase_protocol(
        mut self,
        policy: OriginalQuotePolicy,
        source_forms: bool,
    ) -> Result<Self, ModError> {
        if self.state.mode != CommerceMode::Active || self.state.purchases.is_some() {
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
                    self.state.filing.definition_payload(),
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
        let quotes = OriginalQuoteConfig::new(policy, self.state.registry.reference().clone())?;
        let quotes = if source_forms {
            quotes.with_source_forms()
        } else {
            quotes
        };
        let purchases = OriginalPurchaseConfig::new(quotes, participants.clone(), writer.clone())?;
        let actions = purchases.actions()?;
        self.state.roll =
            self.state
                .roll
                .with_original_purchases(writer, participants, actions.clone())?;
        self.state.purchases = Some(purchases);
        self.state.purchase_actions = Some(actions);
        Ok(self)
    }

    /// Structural identity for trusted roster configuration only. The real
    /// Tacsim registrar must install this exact descriptor before freeze.
    pub fn manifest(&self) -> &DefinitionRef {
        self.state.registry.reference()
    }

    #[must_use]
    pub fn filing_issuer(&self) -> &mush_org::SystemPrincipalId {
        self.state.filing.issuer()
    }

    /// Preserve the actual layer and all its boot hooks. A stale configuration
    /// or a different loaded catalog refuses instead of silently recapturing.
    pub fn configure_tacsim(&self, mut layer: TacsimRuleset) -> Result<TacsimRuleset, ModError> {
        if layer.registry_commerce.is_some()
            || layer.original_holdings.is_some()
            || layer.house_holdings.is_some()
        {
            return Err(ModError::msg(
                "Tacsim already has explicit Registry or filed holdings configuration",
            ));
        }
        let catalog = layer.definition_catalog()?;
        self.state.registry.check(layer.config(), &catalog)?;
        // Doc 17h cut M8e E4: the seal-mark ring's writer, the Roll's cell, for the finish.
        layer.seal_marks = Some(self.state.roll.seal_mark_writer());
        if self.state.mode == CommerceMode::Active {
            layer = if self.state.entitlements.is_some() {
                layer.with_mind_entitlement_holdings(self.manifest().clone())
            } else if self.state.dependent_filing_action.is_some() {
                layer.with_dependent_owner_holdings(self.manifest().clone())
            } else if self.state.purchases.is_some() {
                if self
                    .state
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
        if self.state.house_purchases.is_some() {
            layer = layer.with_house_mind_holdings(self.manifest().clone());
        }
        if self.state.authored_forks {
            layer.fork_material = Some(self.manifest().clone());
        }
        if self.state.marked_registry {
            layer.marked_registry = Some(self.manifest().clone());
        }
        // 17g C2a (C2 plan §2.4): the line-mode command slot and its Roll read
        // exist only when Active commerce composes the player market, the
        // condition that selects `with_mind_entitlement_holdings` above. A
        // format-1 House-only launch is Active without it.
        if self.state.mode == CommerceMode::Active && self.state.entitlements.is_some() {
            layer.mind_commands = Some(self.manifest().clone());
        }
        layer.ic_minds = self
            .state
            .ic
            .as_ref()
            .map(|ic| ic.layer(&layer.cfg).map(Box::new))
            .transpose()?;
        layer.registry_commerce = Some(Box::new(self.state.registry.clone()));
        layer.registry_inspection = self.state.registry_inspection.clone().map(Box::new);
        if let Some(conduct) = &self.state.conduct {
            conduct.check(layer.config(), &layer.definition_catalog()?)?;
            layer.conduct_trials = Some(Box::new(conduct.clone()));
        }
        if let Some(coverage) = &self.state.coverage {
            layer = layer.with_hull_coverage(coverage.clone())?;
        }
        Ok(layer)
    }

    /// Keep the supplied Roll module's real hooks and source writers. The host
    /// must include Tacsim in those writers, as in the ordinary game roster.
    pub fn configure_roll(&self, module: RollModule) -> RollModule {
        let module = module.with_commerce(self.state.roll.clone());
        let module = if self.state.authored_forks {
            module.with_authored_fork_source::<crate::minds::workshop::fork_material::ForkMaterialReader>()
        } else {
            module
        };
        if self.state.mode == CommerceMode::Active {
            crate::minds::registry::configure_roll(module)
        } else {
            module
        }
    }

    /// Preserve custody, facts, observers and existing title issuers. The host
    /// must already allow the configured filing issuer; no issuer is added here.
    ///
    /// # Errors
    /// The native owner's exact configuration error if a paired service differs.
    pub fn configure_economy(&self, module: EconomyModule) -> Result<EconomyModule, ModError> {
        let module = match &self.state.minds_facts {
            Some(provider) => module.with_fact_provider(std::sync::Arc::new(provider.clone())),
            None => module,
        };
        let module =
            module.with_initial_filing(self.state.filing.clone(), self.state.filing_action.clone());
        let module = match &self.state.charter_transfer_action {
            Some(action) => module.with_mind_charter_transfers(action.clone()),
            None => module,
        };
        let module = match &self.state.pending_charters {
            Some((_, config)) => module.with_mind_charter_pending(config.clone()),
            None => module,
        };
        let module = match &self.state.compatible_update_action {
            Some(action) => module.with_compatible_mind_updates(action.clone()),
            None => module,
        };
        let module = match &self.state.derived_update_action {
            Some(action) => module.with_derived_mind_updates(action.clone()),
            None => module,
        };
        let module = match &self.state.house_filing_action {
            Some(action) => module.with_house_initial_filing(action.clone()),
            None => module,
        };
        let module = match &self.state.dependent_filing_action {
            Some(action) => module.with_dependent_initial_filing(action.clone()),
            None => module,
        };
        let module = match &self.state.purchase_actions {
            Some(actions) => module.with_original_mind_purchases(actions.clone()),
            None => module,
        };
        let module = match &self.state.entitlement_actions {
            Some([original, derived]) => {
                module.with_mind_entitlement_purchases(original.clone(), derived.clone())
            }
            None => module,
        };
        let module = match &self.state.house_purchase_actions {
            Some(actions) => module.with_house_mind_purchases(actions.clone()),
            None => module,
        };
        let module = match &self.state.two_seat_house_purchase_actions {
            Some(actions) => module.with_two_seat_house_pins(actions.clone()),
            None => module,
        };
        let module = match (
            &self.state.house_renewals,
            &self.state.house_renewal_actions,
        ) {
            (Some(config), Some(actions)) => module.with_house_mind_renewals(
                config.manifest().clone(),
                [
                    config.participants().economy().clone(),
                    config.participants().retention().clone(),
                ],
                actions.clone(),
            ),
            _ => module,
        };
        let module = match &self.state.house_renewal_service {
            // This private option is produced only by the checked Market/Roll
            // builder. The preceding branch installs those same immutable
            // participants and manifest afresh, with no service yet attached.
            Some(action) => module.with_house_mind_renewal_service(action.clone())?,
            None => module,
        };
        // 17g C1a: personal renewals, in the House branch's shape.
        let module = match (&self.state.renewals, &self.state.renewal_actions) {
            (Some(config), Some(actions)) => module.with_mind_renewals(
                config.manifest().clone(),
                [
                    config.participants().economy().clone(),
                    config.participants().retention().clone(),
                ],
                actions.clone(),
            ),
            _ => module,
        };
        let module = match &self.state.renewal_service {
            // This private option is produced only by the checked Market/Roll
            // builder. The preceding branch installs those same immutable
            // participants and manifest afresh, with no service yet attached.
            Some(action) => module.with_mind_renewal_service(action.clone())?,
            None => module,
        };
        let module = match &self.state.paid_resales {
            Some(config) => module.with_paid_mind_resale(config.economy()?),
            None => module,
        };
        match &self.state.coverage {
            Some(coverage) => coverage.configure_economy(module),
            None => Ok(module),
        }
    }

    /// 17g C2h (the lead's ruling R3): TacSim's own owner id, the grantee of
    /// both renewal services' player front, when Active commerce composes
    /// the player market (the `configure_tacsim` condition for the `+minds`
    /// slot); `None` otherwise (a House-only or a Dormant composition).
    fn player_front(&self) -> Option<OwnerId> {
        (self.state.mode == CommerceMode::Active && self.state.entitlements.is_some())
            .then(|| OwnerId::new(crate::NS).ok())
            .flatten()
    }

    /// Preserve the real venue gate and inventory/IC settings. The closed
    /// writer binding is checked at final definition freeze, so the existing
    /// Market-before-Roll order remains intact.
    pub fn configure_market(&self, module: MarketModule) -> MarketModule {
        let module = match &self.state.purchases {
            Some(config) => module.with_original_mind_purchases(config.clone()),
            None => module,
        };
        let module = match &self.state.entitlements {
            Some(config) => module.with_mind_entitlements(config.clone()),
            None => module,
        };
        let module = match &self.state.house_purchases {
            Some(config) => module.with_house_minds(config.clone()),
            None => module,
        };
        // 17g C2h (the lead's ruling R3): each renewal service's player front
        // is delegated to TacSim exactly where the player market composes,
        // the condition under which `configure_tacsim` sets
        // `TacsimRuleset::mind_commands` (the `+minds` slot). A format-1
        // House-only launch composes the House renewal service with no
        // grantee, so `+minds/renew` stays `NOT_OPEN` there.
        let front = self.player_front();
        let module = match &self.state.house_renewals {
            Some(config) => module.with_house_mind_renewals(match &front {
                Some(grantee) => config.clone().with_player_front(grantee.clone()),
                None => config.clone(),
            }),
            None => module,
        };
        // 17g C1a: personal renewals; the config carries its own service.
        let module = match &self.state.renewals {
            Some(config) => module.with_mind_renewals(match &front {
                Some(grantee) => config.clone().with_player_front(grantee.clone()),
                None => config.clone(),
            }),
            None => module,
        };
        match &self.state.paid_resales {
            Some(config) => module.with_paid_mind_resale(config.clone()),
            None => module,
        }
    }
}

#[cfg(test)]
mod tests;
