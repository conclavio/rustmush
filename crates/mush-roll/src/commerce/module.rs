//! Closed composition of original filing, its real owners and final boot audit.
//! M6 deployments remain on the existing path unless this explicit configuration
//! is supplied. Initialize never claims commercial health or falls back to it.

use std::collections::BTreeSet;

use mush_core::action::{ActionError, ActionErrorCode};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, DefinitionRegistryBuilder,
    DefinitionValidationContext, OwnerId,
};
use mush_core::reserved_shell::{RESERVE_ACTION, RESERVE_OUTCOME, ShellReservations};
use mush_core::{ModError, ModuleCtx, Registrar};
use mush_economy::registry_filing::InitialFilingParticipant;
use serde_json::{Value, json};

use super::families::CommerceFamilies;
use super::filing::{InitialFilingBuilder, InitialFilingTerms};
use super::participant::RootParticipant;
use super::rating::registration::{Installer, RegistryDefinitions, commercial_roots_descriptor};
use super::rating::{RatingCompatibility, RatingFamilies};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommerceMode {
    /// Explicit operator migration. Initial filing remains unavailable; restart
    /// under Active only after the acknowledged bootstrap succeeds.
    Initialize,
    Active,
}

#[derive(Clone, Debug)]
pub struct CommerceConfig {
    pub(crate) mode: CommerceMode,
    terms: InitialFilingTerms,
    manifest: DefinitionRef,
    compatibility: RatingCompatibility,
    original_market_quotes: bool,
    author_withdrawal: bool,
    charter_transfers: bool,
    minds_facts: Option<super::facts::MindsFactProvider>,
    registry_recall: Option<super::registry_recall::RegistryRecallPolicy>,
    registry_revocation: Option<super::registry_revocation::RegistryRevocationPolicy>,
    owner_qualification: bool,
    owner_attestations: bool,
    compatible_updates: bool,
    authored_forks: bool,
    marked_registry: bool,
    reconformance: bool,
    house_filings: bool,
    dependent_owners: bool,
    original_purchases: Option<PurchaseRegistration>,
    entitlements: Option<EntitlementRegistration>,
    renewals: Option<RenewalRegistration>,
    ic: Option<std::sync::Arc<dyn super::ic_claims::registration::Installer>>,
}
#[derive(Clone, Debug)]
struct PurchaseRegistration {
    writer: super::purchase_writer::OriginalPurchaseWriter,
    participants: super::purchase::PurchaseParticipants,
    actions: [DefinitionRef; 3],
}
#[derive(Clone, Debug)]
struct EntitlementRegistration {
    writer: super::entitlement::writer::EntitlementPurchaseWriter,
    participants: super::purchase::PurchaseParticipants,
    original: [DefinitionRef; 4],
    derived: [DefinitionRef; 4],
}
#[derive(Clone, Debug)]
struct RenewalRegistration {
    writer: super::renewal::writer::RenewalWriter,
    participants: super::purchase::PurchaseParticipants,
    actions: [DefinitionRef; 5],
    service_close: Option<DefinitionRef>,
}
impl CommerceConfig {
    /// The game supplies the pair constructed from its actual frozen Registry
    /// manifest during composition, never from a client or a stored revision.
    pub fn new(
        mode: CommerceMode,
        terms: InitialFilingTerms,
        manifest: DefinitionRef,
        compatibility: RatingCompatibility,
    ) -> Result<Self, ModError> {
        if manifest.key().owner.as_str() != "tacsim"
            || manifest.key().kind.as_str() != "registry_manifest"
            || manifest.key().id.as_str() != "combat"
            || manifest.version() != 1
            || manifest.digest() != compatibility.digest()
        {
            return Err(ModError::msg(
                "initial filing needs the actual combat Registry manifest",
            ));
        }
        Ok(Self {
            mode,
            terms,
            manifest,
            compatibility,
            original_market_quotes: false,
            author_withdrawal: false,
            charter_transfers: false,
            minds_facts: None,
            registry_recall: None,
            registry_revocation: None,
            owner_qualification: false,
            owner_attestations: false,
            compatible_updates: false,
            authored_forks: false,
            marked_registry: false,
            reconformance: false,
            house_filings: false,
            dependent_owners: false,
            original_purchases: None,
            entitlements: None,
            renewals: None,
            ic: None,
        })
    }

    /// Explicit Registry-service re-conformance of retained filed revisions.
    /// The actual Registry owners, worker quota and final audit remain required.
    pub fn with_registry_reconformance(mut self) -> Result<Self, ModError> {
        if self.mode != CommerceMode::Active || self.reconformance {
            return Err(ModError::msg(
                "reconformance requires one Active composition",
            ));
        }
        self.reconformance = true;
        Ok(self)
    }

    /// Explicit historical facts and automatic personal term-lapse observation.
    /// Pass the same opaque provider to the actual Economy fact table.
    pub fn with_minds_facts(
        mut self,
        provider: super::facts::MindsFactProvider,
    ) -> Result<Self, ModError> {
        if self.mode != CommerceMode::Active || self.minds_facts.is_some() {
            return Err(ModError::msg(
                "minds facts require one Active configuration",
            ));
        }
        self.minds_facts = Some(provider);
        Ok(self)
    }

    /// Explicit free personal-owner generation transitions. No automatic renewal,
    /// monetary participant or paid-customer row is enabled by this option.
    pub fn with_owner_attestations(mut self) -> Result<Self, ModError> {
        if self.mode != CommerceMode::Active || self.owner_attestations {
            return Err(ModError::msg("owner attestations require Active commerce"));
        }
        self.owner_attestations = true;
        Ok(self)
    }

    /// Select the actual closed seal owner alongside the concrete Tactics and
    /// Sim participants. This registers no host transition and enables nothing
    /// in default/Initialize composition.
    pub fn with_ic_minds<S>(
        mut self,
        config: super::ic_claims::IcClaimConfig,
        writer: super::ic_claims::IcWriter,
    ) -> Result<Self, ModError>
    where
        S: super::ic_claims::IcSealParticipant,
        S::Output: super::ic_claims::IcSealOutput,
    {
        if self.mode != CommerceMode::Active
            || self.ic.is_some()
            || config.manifest() != &self.manifest
        {
            return Err(ModError::msg(
                "IC requires one exact Active owner composition",
            ));
        }
        self.ic = Some(std::sync::Arc::new(
            super::ic_claims::registration::Selected::<S>::new(config, writer),
        ));
        Ok(self)
    }

    /// Explicitly replace only an existing personal dependent owner's current
    /// qualifying parent. No owner term, ancestry or paid grant is issued.
    pub fn with_owner_parent_qualification(mut self) -> Result<Self, ModError> {
        if self.mode != CommerceMode::Active || !self.dependent_owners || self.owner_qualification {
            return Err(ModError::msg(
                "owner qualification requires one Active dependent-owner configuration",
            ));
        }
        self.owner_qualification = true;
        Ok(self)
    }

    /// Enable only original author's exact-schema certified line updates.
    pub fn with_compatible_updates(mut self) -> Result<Self, ModError> {
        if self.mode != CommerceMode::Active || self.compatible_updates {
            return Err(ModError::msg(
                "compatible updates require one Active configuration",
            ));
        }
        self.compatible_updates = true;
        Ok(self)
    }
    /// Exact configured reference for the existing Economy participant grant.
    pub fn compatible_update_action(&self) -> Result<DefinitionRef, ModError> {
        if !self.compatible_updates {
            return Err(ModError::msg("compatible updates are not configured"));
        }
        let [action, _] = super::line_update::registration::definitions(&self.manifest)?;
        DefinitionRegistryBuilder::new()
            .register(OwnerId::new(crate::NS).map_err(error)?, action)
            .map_err(error)
    }

    /// Explicit Registry recall authority. No organizations, grants or operators
    /// are created. Every actual recall repeats the configured live Org policy.
    pub fn with_registry_recall(
        mut self,
        policy: super::registry_recall::RegistryRecallPolicy,
    ) -> Result<Self, ModError> {
        if self.mode != CommerceMode::Active || self.registry_recall.is_some() {
            return Err(ModError::msg(
                "Registry recall requires one Active configuration",
            ));
        }
        policy.validate().map_err(error)?;
        self.registry_recall = Some(policy);
        Ok(self)
    }

    /// Explicit Registry revocation authority. No organizations, grants or operators
    /// are created. Every actual revocation repeats the configured live Org policy.
    pub fn with_registry_revocation(
        mut self,
        policy: super::registry_revocation::RegistryRevocationPolicy,
    ) -> Result<Self, ModError> {
        if self.mode != CommerceMode::Active || self.registry_revocation.is_some() {
            return Err(ModError::msg(
                "Registry revocation requires one Active configuration",
            ));
        }
        policy.validate().map_err(error)?;
        self.registry_revocation = Some(policy);
        Ok(self)
    }

    /// Enable source-free personal charter offers and native immediate settlement.
    /// Pending transfers and paid-licence resale remain unavailable.
    pub fn with_charter_transfers(
        mut self,
        _config: super::charter_transfer::CharterTransferConfig,
    ) -> Result<Self, ModError> {
        if self.mode != CommerceMode::Active || self.charter_transfers {
            return Err(error("charter transfer requires one Active configuration"));
        }
        self.charter_transfers = true;
        Ok(self)
    }

    /// Enable only personal initial-author withdrawal. No source, house,
    /// transferred-holder, or Registry recall authority is implied.
    pub fn with_author_withdrawal(mut self) -> Result<Self, ModError> {
        if self.mode != CommerceMode::Active || self.author_withdrawal {
            return Err(ModError::msg(
                "author withdrawal requires one Active configuration",
            ));
        }
        self.author_withdrawal = true;
        Ok(self)
    }

    /// Explicit composition grants only the source-free original-sale reader
    /// to Market. This is not a sale permission, grant, or funding endpoint.
    #[must_use]
    pub fn with_original_market_quotes(mut self) -> Self {
        self.original_market_quotes = true;
        self
    }

    /// Register the closed paid writer only in explicit Active composition.
    /// Raw families remain private and only these exact Market actions can
    /// request the shared retention slot. Existing quote-only mode is unchanged.
    pub fn with_original_purchases(
        mut self,
        writer: super::purchase_writer::OriginalPurchaseWriter,
        participants: super::purchase::PurchaseParticipants,
        actions: [DefinitionRef; 3],
    ) -> Result<Self, ModError> {
        if self.mode != CommerceMode::Active {
            return Err(ModError::msg("paid purchase requires Active commerce"));
        }
        for (action, (id, transition)) in actions.iter().zip([
            ("purchase_original_mind", "fund"),
            ("settle_original_mind", "grant"),
            ("refund_original_mind", "refund"),
        ]) {
            let version = actions[0].version();
            let protocol = match version {
                1 => "original_mind_purchase_v1",
                2 => "original_mind_purchase_v2",
                _ => return Err(ModError::msg("unknown paid purchase protocol")),
            };
            let expected = DefinitionRegistryBuilder::new()
                .register(
                    OwnerId::new("market").map_err(error)?,
                    DefinitionDescriptor::new(
                        DefinitionKind::new("action").map_err(error)?,
                        DefinitionId::new(id).map_err(error)?,
                        version,
                        json!({"format":version,"protocol":protocol,"transition":transition}),
                    )
                    .map_err(error)?
                    .with_references(vec![
                        self.manifest.clone(),
                        participants.economy().clone(),
                        participants.retention().clone(),
                    ]),
                )
                .map_err(error)?;
            if action != &expected {
                return Err(ModError::msg("paid action differs from closed composition"));
            }
        }
        self.original_market_quotes = true;
        self.original_purchases = Some(PurchaseRegistration {
            writer,
            participants,
            actions,
        });
        Ok(self)
    }

    /// Explicit host opt-in to acknowledged first-generation authored forks.
    /// The actual source-reader type is supplied separately by trusted composition.
    pub fn with_authored_forks(mut self) -> Result<Self, ModError> {
        if self.mode != CommerceMode::Active || !self.source_forms() || self.authored_forks {
            return Err(ModError::msg(
                "authored forks require Active source-form purchases",
            ));
        }
        self.authored_forks = true;
        Ok(self)
    }

    /// New measured work only; a resulting certificate grants no dependent
    /// filing/use permission. Existing admitted jobs keep their recovery path.
    pub fn with_marked_registry(mut self) -> Result<Self, ModError> {
        if self.mode != CommerceMode::Active || !self.authored_forks || self.marked_registry {
            return Err(ModError::msg(
                "marked Registry requires Active authored forks",
            ));
        }
        self.marked_registry = true;
        Ok(self)
    }

    /// Enable only scoped original filing. This grants no organization rights,
    /// house sales, source access or owner execution capability.
    pub fn with_house_filings(mut self) -> Result<Self, ModError> {
        if self.mode != CommerceMode::Active || self.house_filings {
            return Err(ModError::msg(
                "house filing requires one Active configuration",
            ));
        }
        self.house_filings = true;
        Ok(self)
    }

    /// Explicit private personal derivative filing and owner use/source only.
    /// This does not enable derivative sale, renewal or organization authority.
    pub fn with_dependent_owner_filings(mut self) -> Result<Self, ModError> {
        if self.mode != CommerceMode::Active
            || !self.authored_forks
            || !self.marked_registry
            || !self.source_forms()
            || self.dependent_owners
        {
            return Err(ModError::msg(
                "dependent owner filing requires one Active marked-Registry configuration",
            ));
        }
        self.dependent_owners = true;
        Ok(self)
    }
    /// Explicit new original/derived protocols. Historical purchase readers
    /// remain distinct; the actual dependent owner configuration is required.
    pub fn with_mind_entitlements(
        mut self,
        writer: super::entitlement::writer::EntitlementPurchaseWriter,
        participants: super::purchase::PurchaseParticipants,
        original: [DefinitionRef; 4],
        derived: [DefinitionRef; 4],
    ) -> Result<Self, ModError> {
        if self.mode != CommerceMode::Active
            || !self.dependent_owners
            || self.entitlements.is_some()
        {
            return Err(ModError::msg(
                "entitlements require one Active dependent-owner configuration",
            ));
        }
        super::entitlement::writer::validate_refs(
            &self.manifest,
            &participants,
            &original,
            &derived,
        )?;
        self.entitlements = Some(EntitlementRegistration {
            writer,
            participants,
            original,
            derived,
        });
        Ok(self)
    }

    /// Explicit same-slot renewal composition. Exact Market and Economy
    /// providers and all final audits are required; no default activation.
    pub fn with_mind_renewals(
        mut self,
        writer: super::renewal::writer::RenewalWriter,
        participants: super::purchase::PurchaseParticipants,
        actions: [DefinitionRef; 5],
    ) -> Result<Self, ModError> {
        if self.mode != CommerceMode::Active
            || self.entitlements.is_none()
            || self.original_purchases.is_none()
            || self.renewals.is_some()
        {
            return Err(ModError::msg(
                "renewal requires one Active entitlement composition",
            ));
        }
        super::renewal::protocol::validate_refs(&self.manifest, &participants, &actions)?;
        self.renewals = Some(RenewalRegistration {
            writer,
            participants,
            actions,
            service_close: None,
        });
        Ok(self)
    }

    /// Optional exact registered service, without changing old five-action config.
    pub fn with_mind_renewal_service(mut self, action: DefinitionRef) -> Result<Self, ModError> {
        let r = self
            .renewals
            .as_mut()
            .ok_or_else(|| ModError::msg("renewal service requires renewals"))?;
        if r.service_close.is_some()
            || action
                != super::renewal::protocol::service_reference(&self.manifest, &r.participants)?
        {
            return Err(ModError::msg("renewal service composition differs"));
        }
        r.service_close = Some(action);
        Ok(self)
    }
    fn dependent_action_descriptor(&self) -> Result<DefinitionDescriptor, ModError> {
        Ok(descriptor(
            "action",
            super::dependent_filing::ACTION,
            self.terms.dependent_payload(),
        )?
        .with_references(vec![self.manifest.clone()]))
    }
    pub fn dependent_filing_action(&self) -> Result<DefinitionRef, ModError> {
        if !self.dependent_owners {
            return Err(ModError::msg("dependent owner filing is not configured"));
        }
        DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(crate::NS).map_err(error)?,
                self.dependent_action_descriptor()?,
            )
            .map_err(error)
    }

    fn house_action_descriptor(&self) -> Result<DefinitionDescriptor, ModError> {
        Ok(descriptor(
            "action",
            mush_economy::registry_filing::HOUSE_FILING_ACTION,
            json!({"format":1,"protocol":"scoped_original_house_filing_v1",
                "terms":self.terms.payload(),
                "capabilities":[mush_economy::registry_filing::HOUSE_PUBLICATION_CAPABILITY,
                    "asset.title.transfer","economy.account.post"]}),
        )?
        .with_references(vec![self.manifest.clone()]))
    }

    /// Exact composition identity; no live grant or payer is carried here.
    pub fn house_filing_action(&self) -> Result<DefinitionRef, ModError> {
        if !self.house_filings {
            return Err(ModError::msg("house filing is not configured"));
        }
        DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(crate::NS).map_err(error)?,
                self.house_action_descriptor()?,
            )
            .map_err(error)
    }

    fn source_forms(&self) -> bool {
        self.original_purchases
            .as_ref()
            .is_some_and(|purchase| purchase.actions[0].version() == 2)
    }

    fn action_descriptor(&self) -> Result<DefinitionDescriptor, ModError> {
        Ok(
            descriptor("action", "initial_filing", self.terms.payload())?
                .with_references(vec![self.manifest.clone()]),
        )
    }

    fn definitions(&self) -> Result<Vec<DefinitionDescriptor>, ModError> {
        let mut definitions = vec![
            descriptor(
                "action",
                RESERVE_ACTION,
                json!({"format":1,"purpose":"roll.initial_filing","object":"inert"}),
            )?,
            descriptor("outcome", RESERVE_OUTCOME, json!({"format":1}))?,
            self.action_descriptor()?,
            descriptor("outcome", "initial_filing_committed", json!({"format":1}))?,
            commercial_roots_descriptor()?,
        ];
        if self.house_filings {
            definitions.push(self.house_action_descriptor()?);
            definitions.push(descriptor(
                "outcome",
                "house_initial_filing_committed",
                json!({"format":1}),
            )?);
        }
        if self.dependent_owners {
            definitions.push(self.dependent_action_descriptor()?);
            definitions.push(descriptor(
                "outcome",
                "dependent_initial_filing_committed",
                json!({"format":1}),
            )?);
        }
        if self.owner_attestations {
            let (term, grace) = self.terms.durations();
            definitions.extend(super::owner_attestation::protocol::definitions(
                &self.manifest,
                term,
                grace,
            )?);
        }
        if self.owner_qualification {
            definitions.extend(super::owner_qualification::protocol::definitions(
                &self.manifest,
            )?);
        }
        if self.compatible_updates {
            definitions.extend(super::line_update::registration::definitions(
                &self.manifest,
            )?);
        }
        if self.minds_facts.is_some() {
            definitions.extend(super::facts::protocol::definitions()?);
        }
        if let Some(policy) = &self.registry_recall {
            definitions.extend(policy.definitions()?);
        }
        if let Some(policy) = &self.registry_revocation {
            definitions.extend(policy.definitions()?);
        }
        if self.charter_transfers {
            definitions.extend(super::charter_transfer::protocol::definitions()?);
        }
        if self.author_withdrawal {
            let [action, outcome] = super::withdrawal::definitions()?;
            definitions.extend([action, outcome]);
        }
        if self.authored_forks {
            definitions.extend(super::authored_fork::definitions(&self.manifest)?);
            let [action, outcome, _] = super::authored_retirement::definitions()?;
            definitions.extend([action, outcome]);
            definitions.push(super::authored_retirement::captured_definition()?);
        }
        if self.mode == CommerceMode::Initialize {
            definitions.push(descriptor(
                "action",
                "initialize_commerce",
                json!({"format":1,"explicit_setup":true}),
            )?);
            definitions.push(descriptor(
                "outcome",
                "commerce_initialized",
                json!({"format":1,"restart_active":true}),
            )?);
        }
        Ok(definitions)
    }

    #[cfg(test)]
    fn register_validators(&self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        self.register_validators_for(reg, None)
    }

    fn register_validators_for(
        &self,
        reg: &mut Registrar<'_>,
        registry: Option<&RegistryDefinitions>,
    ) -> Result<(), ModError> {
        let mut definitions = self.definitions()?;
        if let Some(registry) = registry {
            definitions.extend(registry.definitions());
        }
        if self.reconformance {
            definitions.extend(super::rating::reconformance::recall::protocol::definitions(
                &self.manifest,
            )?);
            definitions.extend(
                super::rating::registration::ReconformanceDefinitions::new(self.manifest.clone())?
                    .definitions(),
            );
        }
        for kind in [
            "action",
            "outcome",
            "owner_participant",
            "registry_authority",
        ] {
            let expected = definitions
                .iter()
                .filter(|definition| definition.kind().as_str() == kind)
                .cloned()
                .collect::<Vec<_>>();
            reg.definition_validator(
                DefinitionKind::new(kind).map_err(error)?,
                move |ctx: &DefinitionValidationContext<'_>| {
                    validate_definitions(crate::NS, &expected, ctx)
                },
            )?;
        }
        Ok(())
    }

    /// Deterministic reference used by Economy composition to authorize only
    /// this exact Roll action. The actual registry still validates every ref.
    pub fn initial_filing_action(&self) -> Result<DefinitionRef, ModError> {
        DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(crate::NS).map_err(error)?,
                self.action_descriptor()?,
            )
            .map_err(error)
    }
}

pub(crate) struct CommerceRegistration {
    reconformance: bool,
    measured_recall: Option<super::rating::reconformance::recall::Readiness>,
    ic: Option<std::sync::Arc<dyn super::ic_claims::registration::Installer>>,
    withdrawal: Option<super::withdrawal::EconomyReadiness>,
    charter_transfer: Option<super::charter_transfer::Readiness>,
    facts_readiness: Option<super::facts::MindsFactProvider>,
    registry_recall: Option<super::registry_recall::Readiness>,
    registry_revocation: Option<super::registry_revocation::Readiness>,
    recall_readback: Option<super::registry_recall::notice::Readback>,
    revocation_readback: Option<super::registry_revocation::notice::Readback>,
    attestation_readiness: Option<super::withdrawal::EconomyReadiness>,
    owner_qualification: bool,
    owner_attestations: bool,
    update: Option<DefinitionRef>,
    pub(crate) families: CommerceFamilies,
    pub(crate) ratings: RatingFamilies,
    pub(crate) shells: ShellReservations,
    pub(crate) mode: CommerceMode,
    paid: bool,
    entitlements: bool,
    renewals: bool,
    forks: bool,
    dependent_owners: bool,
}

fn error(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}
fn descriptor(kind: &str, id: &str, payload: Value) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(error)?,
        DefinitionId::new(id).map_err(error)?,
        1,
        payload,
    )
    .map_err(error)
}

/// Complete equality is deliberate: these are closed, configured operation
/// schemas, with no client-selected extension fields. The filing declaration
/// binds both exact configured terms and the actual Registry manifest reference.
pub(super) fn validate_definitions(
    owner: &str,
    expected: &[DefinitionDescriptor],
    ctx: &DefinitionValidationContext<'_>,
) -> Result<(), String> {
    let entry = ctx.definition();
    let reference = entry.reference();
    if reference.key().owner.as_str() == owner
        && expected.iter().any(|definition| {
            // Core stores and hashes references in canonical sorted order.
            // Compare the exact configured set in that same order; construction
            // order is not part of a DefinitionRef's identity.
            let mut references = definition.references().to_vec();
            references.sort();
            &reference.key().kind == definition.kind()
                && &reference.key().id == definition.id()
                && reference.version() == definition.version()
                && entry.payload() == definition.payload()
                && entry.references() == references
                && entry.is_current() == definition.is_current()
        })
    {
        Ok(())
    } else {
        Err("definition differs from the closed owner configuration".into())
    }
}

pub(crate) fn register(
    reg: &mut Registrar<'_>,
    config: &CommerceConfig,
    installer: Option<&dyn Installer>,
    fork: Option<&dyn super::authored_fork::Installer>,
) -> Result<CommerceRegistration, ModError> {
    if reg.module() != crate::NS {
        return Err(ModError::msg("commerce registration is Roll-owned"));
    }
    if installer.is_some() && config.mode != CommerceMode::Active {
        return Err(ModError::msg(
            "Registry jobs require Active commerce after acknowledged setup",
        ));
    }
    if config.authored_forks != fork.is_some() {
        return Err(ModError::msg(
            "authored fork configuration and actual source owner differ",
        ));
    }
    if config.marked_registry && (installer.is_none() || fork.is_none()) {
        return Err(ModError::msg(
            "marked Registry requires actual Registry and fork owners",
        ));
    }
    if config.reconformance && installer.is_none() {
        return Err(ModError::msg(
            "reconformance requires actual Registry owners",
        ));
    }
    let reconformance = config
        .reconformance
        .then(|| {
            super::rating::registration::ReconformanceDefinitions::new(config.manifest.clone())
        })
        .transpose()?;
    let registry = installer
        .map(|_| RegistryDefinitions::new(config.manifest.clone()))
        .transpose()?;
    config.register_validators_for(reg, registry.as_ref())?;
    let families = CommerceFamilies::register(reg)?;
    let ratings = RatingFamilies::register(reg)?;
    if config.mode == CommerceMode::Active {
        let mut declarations = vec![super::holding_reader::descriptor(&config.manifest)?];
        if config.house_filings {
            declarations.push(super::house_catalogue::descriptor(&config.manifest)?);
        }
        if config.minds_facts.is_some() {
            declarations.push(super::facts::reader::descriptor()?);
        }
        if config.ic.is_some() {
            declarations.push(super::ic_claims::reader::descriptor(&config.manifest)?);
        }
        if fork.is_some() {
            declarations.push(super::parent_grant::descriptor(&config.manifest)?);
            declarations.push(super::authored_access::descriptor(&config.manifest)?);
            declarations.push(super::authored_history::descriptor(&config.manifest)?);
            let [_, _, metadata] = super::authored_retirement::definitions()?;
            declarations.push(metadata);
        }
        if config.dependent_owners {
            declarations.push(super::dependent_filing::descriptor(&config.manifest)?);
            declarations.push(super::dependent_source::descriptor(&config.manifest)?);
        }
        if config.marked_registry {
            declarations.push(super::marked_rating::descriptor(&config.manifest)?);
        }
        if config.reconformance {
            declarations.push(super::rating::reconformance::reader::descriptor(
                &config.manifest,
            )?);
        }
        if installer.is_some() {
            declarations.push(super::rating::service_read::descriptor(&config.manifest)?);
        }
        if config.original_market_quotes {
            declarations.push(super::sale_reader::descriptor(&config.manifest)?);
        }
        if config.original_purchases.is_some() {
            declarations.push(if config.dependent_owners {
                super::personal_filed_reader::descriptor_dependent(&config.manifest)?
            } else {
                super::personal_filed_reader::descriptor_protocol(
                    &config.manifest,
                    config.source_forms(),
                )?
            });
            if config.source_forms() {
                declarations.push(super::paid_source_reader::descriptor(&config.manifest)?);
            }
        }
        if config.entitlements.is_some() {
            declarations.push(super::entitlement::reader::descriptor(&config.manifest)?);
            declarations.push(super::entitlement::customer::holding_descriptor(
                &config.manifest,
            )?);
            declarations.push(super::entitlement::customer::source_descriptor(
                &config.manifest,
            )?);
        }
        if config.renewals.is_some() {
            declarations.push(super::renewal::reader::configured_descriptor(
                &config.manifest,
                config
                    .renewals
                    .as_ref()
                    .is_some_and(|r| r.service_close.is_some()),
            )?);
        }
        reg.definition_validator(
            DefinitionKind::new("owner_read").map_err(error)?,
            move |ctx: &DefinitionValidationContext<'_>| {
                validate_definitions(crate::NS, &declarations, ctx)
            },
        )?;
        if config.marked_registry {
            super::marked_rating::register(
                reg,
                families.clone(),
                ratings.clone(),
                &config.manifest,
                config.compatibility.clone(),
            )?;
        }
        if config.reconformance {
            super::rating::reconformance::reader::register(
                reg,
                families.clone(),
                ratings.clone(),
                &config.manifest,
                config.compatibility.clone(),
            )?;
        }
        if installer.is_some() {
            super::rating::service_read::register(reg, ratings.clone(), &config.manifest)?;
        }
        if config.house_filings {
            super::house_catalogue::register(
                reg,
                families.clone(),
                ratings.clone(),
                &config.manifest,
                config.compatibility.clone(),
            )?;
        }
        super::holding_reader::register(
            reg,
            families.clone(),
            ratings.clone(),
            &config.manifest,
            config.compatibility.clone(),
        )?;
        if config.original_purchases.is_some() {
            super::personal_filed_reader::register(
                reg,
                families.clone(),
                ratings.clone(),
                &config.manifest,
                config.compatibility.clone(),
                config.source_forms(),
                config.dependent_owners,
            )?;
            if config.source_forms() {
                super::paid_source_reader::register(
                    reg,
                    families.clone(),
                    ratings.clone(),
                    &config.manifest,
                    config.compatibility.clone(),
                )?;
            }
        }
        if config.dependent_owners {
            super::dependent_source::register(
                reg,
                families.clone(),
                ratings.clone(),
                &config.manifest,
                config.compatibility.clone(),
            )?;
        }
        if config.entitlements.is_some() {
            super::entitlement::reader::register(
                reg,
                families.clone(),
                ratings.clone(),
                &config.manifest,
                config.compatibility.clone(),
            )?;
            super::entitlement::customer::register(
                reg,
                families.clone(),
                ratings.clone(),
                &config.manifest,
                config.compatibility.clone(),
            )?;
        }
        if config.renewals.is_some() {
            super::renewal::reader::register(
                reg,
                families.clone(),
                ratings.clone(),
                &config.manifest,
                config.compatibility.clone(),
                config
                    .renewals
                    .as_ref()
                    .is_some_and(|r| r.service_close.is_some()),
            )?;
        }
        if config.original_market_quotes {
            super::sale_reader::register(
                reg,
                families.clone(),
                ratings.clone(),
                &config.manifest,
                config.compatibility.clone(),
            )?;
        }
    }
    let reserve = reg.definition(descriptor(
        "action",
        RESERVE_ACTION,
        json!({"format":1,"purpose":"roll.initial_filing","object":"inert"}),
    )?)?;
    let reserved = reg.definition(descriptor("outcome", RESERVE_OUTCOME, json!({"format":1}))?)?;
    let shells = match config.mode {
        CommerceMode::Initialize => ShellReservations::register_for_setup(reg, reserve, reserved)?,
        CommerceMode::Active => ShellReservations::register(reg, reserve, reserved)?,
    };
    let action = reg.definition(config.action_descriptor()?)?;
    let outcome = reg.definition(descriptor(
        "outcome",
        "initial_filing_committed",
        json!({"format":1}),
    )?)?;
    let house_filing = if config.house_filings {
        Some((
            reg.definition(config.house_action_descriptor()?)?,
            reg.definition(descriptor(
                "outcome",
                "house_initial_filing_committed",
                json!({"format":1}),
            )?)?,
        ))
    } else {
        None
    };
    let dependent_filing = if config.dependent_owners {
        let action = reg.definition(config.dependent_action_descriptor()?)?;
        let outcome = reg.definition(descriptor(
            "outcome",
            "dependent_initial_filing_committed",
            json!({"format":1}),
        )?)?;
        let reader_definition = super::dependent_filing::register(
            reg,
            families.clone(),
            ratings.clone(),
            &config.manifest,
            config.compatibility.clone(),
        )?;
        let reader = reg
            .request_owner_read::<super::dependent_filing::FilingParentReader>(reader_definition)?;
        Some((action, outcome, reader))
    } else {
        None
    };
    let update_registration = if config.compatible_updates {
        let [action, outcome] = super::line_update::registration::definitions(&config.manifest)?;
        Some([reg.definition(action)?, reg.definition(outcome)?])
    } else {
        None
    };
    let update = update_registration.as_ref().map(|pair| pair[0].clone());
    let root_definition = reg.definition(commercial_roots_descriptor()?)?;
    let mut root_actions = BTreeSet::from([action.clone()]);
    if let Some(ic) = &config.ic {
        if ic.config().participants().first() != Some(&root_definition) {
            return Err(ModError::msg("IC retention definition differs"));
        }
        root_actions.extend(ic.config().actions()?);
        ic.register(
            reg,
            families.clone(),
            ratings.clone(),
            config.compatibility.clone(),
        )?;
    }
    if let Some(action) = &update {
        root_actions.insert(action.clone());
    }
    if let Some((house_action, _)) = &house_filing {
        root_actions.insert(house_action.clone());
    }
    if let Some((action, _, _)) = &dependent_filing {
        root_actions.insert(action.clone());
    }
    if let Some(registry) = &registry {
        root_actions.extend(registry.actions().iter().cloned());
    }
    if let Some(reconformance) = &reconformance {
        root_actions.extend(reconformance.actions().iter().cloned());
    }
    if let Some(purchase) = &config.original_purchases {
        if purchase.participants.retention() != &root_definition {
            return Err(ModError::msg("paid retention definition differs"));
        }
        root_actions.extend(purchase.actions.iter().cloned());
        purchase.writer.install(
            reg,
            families.clone(),
            config.manifest.clone(),
            purchase.participants.clone(),
            purchase.actions.clone(),
        )?;
    }
    if let Some(entitlement) = &config.entitlements {
        if entitlement.participants.retention() != &root_definition {
            return Err(ModError::msg("entitlement retention definition differs"));
        }
        root_actions.extend(
            entitlement
                .original
                .iter()
                .chain(&entitlement.derived)
                .cloned(),
        );
        entitlement.writer.install(
            reg,
            families.clone(),
            config.manifest.clone(),
            entitlement.participants.clone(),
            entitlement.original.clone(),
            entitlement.derived.clone(),
        )?;
    }
    if let Some(renewal) = &config.renewals {
        if renewal.participants.retention() != &root_definition {
            return Err(ModError::msg("renewal retention definition differs"));
        }
        root_actions.extend(renewal.actions.iter().cloned());
        root_actions.extend(renewal.service_close.iter().cloned());
        renewal.writer.install(
            reg,
            families.clone(),
            config.manifest.clone(),
            renewal.participants.clone(),
            renewal.actions.clone(),
            renewal.service_close.clone(),
        )?;
    }
    let fork_registration = if let Some(installer) = fork {
        let [action, outcome] = super::authored_fork::definitions(&config.manifest)?;
        let action = reg.definition(action)?;
        let outcome = reg.definition(outcome)?;
        root_actions.insert(action.clone());
        Some((installer, [action, outcome]))
    } else {
        None
    };
    let retirement_registration = if fork.is_some() {
        let [action, outcome, metadata] = super::authored_retirement::definitions()?;
        let action = reg.definition(action)?;
        let outcome = reg.definition(outcome)?;
        let metadata = reg.definition(metadata)?;
        let captured = reg.definition(super::authored_retirement::captured_definition()?)?;
        root_actions.insert(action.clone());
        root_actions.insert(captured.clone());
        Some([action, outcome, metadata, captured])
    } else {
        None
    };
    let attestation_registration = if config.owner_attestations {
        let (term, grace) = config.terms.durations();
        let [action, outcome] =
            super::owner_attestation::protocol::definitions(&config.manifest, term, grace)?;
        let action = reg.definition(action)?;
        let outcome = reg.definition(outcome)?;
        root_actions.insert(action.clone());
        Some([action, outcome])
    } else {
        None
    };
    let qualification_registration = if config.owner_qualification {
        let [action, outcome] =
            super::owner_qualification::protocol::definitions(&config.manifest)?;
        let action = reg.definition(action)?;
        let outcome = reg.definition(outcome)?;
        root_actions.insert(action.clone());
        Some([action, outcome])
    } else {
        None
    };
    let facts_registration = if config.minds_facts.is_some() {
        let [action, outcome] = super::facts::protocol::definitions()?;
        let action = reg.definition(action)?;
        let outcome = reg.definition(outcome)?;
        root_actions.insert(action.clone());
        Some([action, outcome])
    } else {
        None
    };
    let measured_recall_registration = if config.reconformance {
        let [action, outcome] =
            super::rating::reconformance::recall::protocol::definitions(&config.manifest)?;
        let action = reg.definition(action)?;
        let outcome = reg.definition(outcome)?;
        root_actions.insert(action.clone());
        Some([action, outcome])
    } else {
        None
    };
    let recall_registration = if let Some(policy) = &config.registry_recall {
        let [authority, action, outcome] = policy.definitions()?;
        let authority = reg.definition(authority)?;
        let action = reg.definition(action)?;
        let outcome = reg.definition(outcome)?;
        root_actions.insert(action.clone());
        Some((policy.clone(), [authority, action, outcome]))
    } else {
        None
    };
    let revocation_registration = if let Some(policy) = &config.registry_revocation {
        let [authority, action, outcome] = policy.definitions()?;
        let authority = reg.definition(authority)?;
        let action = reg.definition(action)?;
        let outcome = reg.definition(outcome)?;
        root_actions.insert(action.clone());
        Some((policy.clone(), [authority, action, outcome]))
    } else {
        None
    };
    let charter_transfer_registration = if config.charter_transfers {
        let [offer, accept, offered, transferred] =
            super::charter_transfer::protocol::definitions()?;
        let offer = reg.definition(offer)?;
        let accept = reg.definition(accept)?;
        let offered = reg.definition(offered)?;
        let transferred = reg.definition(transferred)?;
        root_actions.insert(offer.clone());
        root_actions.insert(accept.clone());
        Some([offer, accept, offered, transferred])
    } else {
        None
    };
    let withdrawal_registration = if config.author_withdrawal {
        let [action, outcome] = super::withdrawal::definitions()?;
        let action = reg.definition(action)?;
        let outcome = reg.definition(outcome)?;
        root_actions.insert(action.clone());
        Some([action, outcome])
    } else {
        None
    };
    reg.owner_participant(root_definition, root_actions, RootParticipant)?;
    if let Some(definitions) = update_registration {
        super::line_update::registration::register(
            reg,
            families.clone(),
            ratings.clone(),
            config.manifest.clone(),
            config.compatibility.clone(),
            definitions,
        )?;
    }
    let facts_readiness = facts_registration
        .map(|definitions| {
            super::facts::register(
                reg,
                families.clone(),
                ratings.clone(),
                config
                    .minds_facts
                    .as_ref()
                    .ok_or_else(|| error("minds facts configuration absent"))?,
                definitions,
            )
        })
        .transpose()?;
    let registry_recall = recall_registration
        .map(|(policy, definitions)| {
            super::registry_recall::register(reg, families.clone(), policy, definitions)
        })
        .transpose()?;
    let registry_revocation = revocation_registration
        .map(|(policy, definitions)| {
            super::registry_revocation::register(
                reg,
                families.clone(),
                ratings.clone(),
                policy,
                definitions,
            )
        })
        .transpose()?;
    let charter_transfer = charter_transfer_registration
        .map(|definitions| super::charter_transfer::register(reg, families.clone(), definitions))
        .transpose()?;
    let withdrawal = withdrawal_registration
        .map(|definitions| super::withdrawal::register(reg, families.clone(), definitions))
        .transpose()?;
    if let Some(definitions) = retirement_registration {
        super::authored_retirement::register(reg, families.clone(), definitions)?;
    }
    let mut attestation_parent = None;
    if let Some((fork, definitions)) = fork_registration {
        let parent = super::parent_grant::register(
            reg,
            families.clone(),
            ratings.clone(),
            &config.manifest,
            config.compatibility.clone(),
        )?;
        let parent = reg.request_owner_read::<super::parent_grant::ParentGrantReader>(parent)?;
        attestation_parent = Some(parent.clone());
        fork.register(
            reg,
            &config.manifest,
            families.clone(),
            parent.clone(),
            definitions,
        )?;
        if let Some(definitions) = qualification_registration {
            super::owner_qualification::register(
                reg,
                families.clone(),
                &config.manifest,
                parent,
                definitions,
            )?;
        }
        super::authored_history::register(
            reg,
            families.clone(),
            ratings.clone(),
            &config.manifest,
            config.compatibility.clone(),
        )?;
        super::authored_access::register(
            reg,
            families.clone(),
            ratings.clone(),
            &config.manifest,
            config.compatibility.clone(),
        )?;
    }
    let attestation_readiness = if let Some(definitions) = attestation_registration {
        Some(super::owner_attestation::register(
            reg,
            families.clone(),
            &config.manifest,
            attestation_parent,
            ratings.clone(),
            config.compatibility.clone(),
            config.terms.clone(),
            definitions,
        )?)
    } else {
        None
    };

    let economy = reg.request_owner_participant::<InitialFilingParticipant>(action.clone())?;
    let roots = reg.request_owner_participant::<RootParticipant>(action.clone())?;
    let refusal = ActionError::new(
        ActionErrorCode::ProviderRejected,
        false,
        "The owned rated revision cannot be filed under these terms.",
    )
    .map_err(error)?;
    reg.action_provider(
        action.clone(),
        InitialFilingBuilder {
            house: false,
            dependent: None,
            action,
            outcome,
            manifest: config.manifest.clone(),
            compatibility: config.compatibility.clone(),
            terms: config.terms.clone(),
            families: families.clone(),
            ratings: ratings.clone(),
            shells: shells.clone(),
            economy,
            roots,
            refusal,
        },
    )?;
    if let Some((action, outcome)) = house_filing {
        let economy = reg.request_owner_participant::<InitialFilingParticipant>(action.clone())?;
        let roots = reg.request_owner_participant::<RootParticipant>(action.clone())?;
        let refusal = ActionError::new(
            ActionErrorCode::ProviderRejected,
            false,
            "The rated revision cannot be filed for this organization scope.",
        )
        .map_err(error)?;
        reg.action_provider(
            action.clone(),
            InitialFilingBuilder {
                house: true,
                dependent: None,
                action,
                outcome,
                manifest: config.manifest.clone(),
                compatibility: config.compatibility.clone(),
                terms: config.terms.clone(),
                families: families.clone(),
                ratings: ratings.clone(),
                shells: shells.clone(),
                economy,
                roots,
                refusal,
            },
        )?;
    }
    if let Some((action, outcome, reader)) = dependent_filing {
        let economy = reg.request_owner_participant::<InitialFilingParticipant>(action.clone())?;
        let roots = reg.request_owner_participant::<RootParticipant>(action.clone())?;
        let refusal = ActionError::new(
            ActionErrorCode::ProviderRejected,
            false,
            "The rated derivative cannot be filed under current parent rights.",
        )
        .map_err(error)?;
        reg.action_provider(
            action.clone(),
            InitialFilingBuilder {
                house: false,
                dependent: Some(reader),
                action,
                outcome,
                manifest: config.manifest.clone(),
                compatibility: config.compatibility.clone(),
                terms: config.terms.clone(),
                families: families.clone(),
                ratings: ratings.clone(),
                shells: shells.clone(),
                economy,
                roots,
                refusal,
            },
        )?;
    }
    let measured_recall = if let (Some(installer), Some(registry)) = (installer, registry.as_ref())
    {
        installer.register(
            reg,
            registry,
            &config.compatibility,
            families.clone(),
            ratings.clone(),
            config.marked_registry,
            reconformance.as_ref(),
            measured_recall_registration,
        )?
    } else {
        None
    };
    if config.mode == CommerceMode::Initialize {
        let action = reg.definition(descriptor(
            "action",
            "initialize_commerce",
            json!({"format":1,"explicit_setup":true}),
        )?)?;
        let outcome = reg.definition(descriptor(
            "outcome",
            "commerce_initialized",
            json!({"format":1,"restart_active":true}),
        )?)?;
        super::bootstrap::register(
            reg,
            action,
            outcome,
            families.clone(),
            ratings.clone(),
            shells.clone(),
        )?;
    }
    let recall_readback = (config.mode == CommerceMode::Active)
        .then(|| super::registry_recall::notice::register(reg, families.clone(), ratings.clone()))
        .transpose()?;
    let revocation_readback = (config.mode == CommerceMode::Active)
        .then(|| {
            super::registry_revocation::notice::register(reg, families.clone(), ratings.clone())
        })
        .transpose()?;
    Ok(CommerceRegistration {
        reconformance: config.reconformance,
        measured_recall,
        ic: config.ic.clone(),
        withdrawal,
        charter_transfer,
        facts_readiness,
        registry_recall,
        registry_revocation,
        recall_readback,
        revocation_readback,
        attestation_readiness,
        owner_qualification: config.owner_qualification,
        owner_attestations: config.owner_attestations,
        update,
        families,
        ratings,
        shells,
        mode: config.mode,
        paid: config.original_purchases.is_some(),
        entitlements: config.entitlements.is_some(),
        renewals: config.renewals.is_some(),
        forks: fork.is_some(),
        dependent_owners: config.dependent_owners,
    })
}

#[cfg(test)]
mod definition_tests;

impl CommerceRegistration {
    pub(crate) fn finalize(
        &self,
        ctx: &mut ModuleCtx<'_>,
        registry: Option<&dyn super::rating::registration::Installer>,
    ) -> Result<(), ModError> {
        if let Some(readiness) = &self.registry_recall {
            readiness.capture(ctx)?;
        }
        if let Some(readiness) = &self.registry_revocation {
            readiness.capture(ctx)?;
        }
        if let Some(readiness) = &self.charter_transfer {
            readiness.capture(ctx)?;
        }
        if let Some(readiness) = &self.withdrawal {
            readiness.capture(ctx)?;
        }
        if let Some(readiness) = &self.attestation_readiness {
            readiness.capture(ctx)?;
        }
        ctx.state_mut::<crate::RollStore>()
            .ok_or_else(|| error("Roll audit state"))?
            .close_commerce_for_audit()
            .map_err(error)?;
        super::rating::reconformance::audit::audit(
            ctx,
            &self.families,
            &self.ratings,
            self.reconformance,
        )
        .map_err(error)?;
        if self.mode == CommerceMode::Active {
            super::charter_transfer::audit::audit(
                ctx,
                &self.families,
                self.charter_transfer.is_some(),
            )
            .map_err(error)?;
        }
        match self.mode {
            CommerceMode::Active => super::audit::audit_and_open_ic_configured(
                ctx,
                &self.families,
                &self.ratings,
                &self.shells,
                self.paid,
                self.forks,
                self.dependent_owners,
                self.entitlements,
                self.renewals,
                self.update.as_ref(),
                self.owner_qualification,
                self.owner_attestations,
                self.ic.as_deref(),
                registry,
            )
            .map_err(error),
            // Explicit setup remains Closed. The operator provider repeats its
            // actual empty/source/replay checks and carries the handoff guards.
            CommerceMode::Initialize => super::bootstrap::verify_initialize_mode(
                ctx,
                &self.families,
                &self.ratings,
                &self.shells,
            ),
        }?;
        if let Some(readiness) = &self.measured_recall {
            readiness.capture(ctx)?;
        }
        if let Some(readiness) = &self.facts_readiness {
            readiness.capture(ctx)?;
        }
        if let Some(readback) = &self.recall_readback {
            readback.enable(ctx)?;
        }
        if let Some(readback) = &self.revocation_readback {
            readback.enable(ctx)?;
        }
        Ok(())
    }
}
