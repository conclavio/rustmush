//! Permanent accepted identity and actual counterpart receipts. These records
//! are comparison data; only the installed providers mint owner preparation.
use super::{
    IntegrationError as Error, PublishedCoreRecipe, bad, offer::FactoryOffer,
    progress::CoreProgress,
};
use crate::{
    core_production::{DieClass, DieSource, UncertifiedDie, records::DieCertificate},
    facility::FacilityRecord,
    workorder::WorkOrderId,
};
use mush_core::{ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId, ReservationId};
use mush_economy::{
    AccountId,
    core_integration::{
        CoreFundingReceipt, CoreFundingSubject, CorePaymentHistory, CorePaymentPolicy,
        CoreTerminalReceipt, CoreTerminalResult,
    },
};
use mush_inventory::Lot;
use mush_inventory::core_input::{CoreInputIdentity, CoreInputOperation, CoreInputState};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) const MAX_ORDERS: u64 = 8192;
pub(crate) const ORDER_BYTES: usize = 98_304;
pub(crate) const INTENT_BYTES: usize = 24_576;
pub(crate) const INDEX_BYTES: usize = 12_288;
pub(crate) const INDEX_KEY: &str = "core_integration_registry";

pub(crate) fn digest<T: Serialize>(
    domain: &str,
    row: &T,
    limit: usize,
) -> Result<DefinitionDigest, Error> {
    let value = serde_json::to_value(row).map_err(bad)?;
    mush_core::mechanics::canonical_json_bytes_with_limit(&value, limit)
        .map_err(|_| Error::Capacity)?;
    DefinitionDigest::for_canonical_value_with_limit(domain, &value, limit).map_err(bad)
}
pub(crate) fn encode<T: Serialize>(row: &T, limit: usize) -> Result<Value, String> {
    let value = serde_json::to_value(row).map_err(|e| e.to_string())?;
    mush_core::mechanics::canonical_json_bytes_with_limit(&value, limit)
        .map_err(|e| e.to_string())?;
    Ok(value)
}
pub(crate) fn decode<T: Serialize + serde::de::DeserializeOwned>(
    value: &Value,
    limit: usize,
) -> Result<T, String> {
    mush_core::mechanics::canonical_json_bytes_with_limit(value, limit)
        .map_err(|e| e.to_string())?;
    let row: T = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    if serde_json::to_value(&row).map_err(|e| e.to_string())? != *value {
        return Err("core integration exact shape".into());
    }
    Ok(row)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AcceptedIntent {
    pub(crate) format: u16,
    pub(crate) order: WorkOrderId,
    pub(crate) offer: CommitToken,
    pub(crate) offer_digest: DefinitionDigest,
    pub(crate) requester: ObjId,
    pub(crate) payer: AccountId,
    pub(crate) policy: CorePaymentPolicy,
    pub(crate) facility: FacilityRecord,
    pub(crate) recipe: PublishedCoreRecipe,
    /// The importer's certificate for a certified die; absent for a heirloom or grey one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) certificate: Option<Box<DieCertificate>>,
    /// **A heirloom or grey die's staff issue** (doc 17h cut M8b-2, H2), in place of a
    /// certificate: exactly one of the two is present. Absent for a certified die, so a
    /// certified intent keeps its bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) uncertified: Option<Box<UncertifiedDie>>,
    pub(crate) lot: Lot,
    pub(crate) accepted_at: ClockStamp,
    pub(crate) expires_at: ClockStamp,
}
impl AcceptedIntent {
    /// `true` for a certified die: the site it integrates at.
    pub(crate) fn certified(&self) -> bool {
        self.uncertified.is_none()
    }
    /// The die's source word (doc 17h cut M8b-2, H1).
    pub(crate) fn source(&self) -> DieSource {
        self.uncertified
            .as_deref()
            .map_or(DieSource::Certified, UncertifiedDie::source)
    }
    /// The die's class and issuing operation, from whichever record the intent carries.
    pub(crate) fn die(&self) -> Result<(DieClass, CommitToken), Error> {
        match (&self.certificate, &self.uncertified) {
            (Some(c), None) => Ok((c.class, c.operation)),
            (None, Some(u)) => Ok((u.class(), u.operation())),
            _ => Err(Error::Integrity),
        }
    }
    /// The factory this intent's site integrates at.
    pub(crate) fn site_factory(&self) -> Result<ObjId, Error> {
        Ok(self.policy.site(self.certified()).map_err(bad)?.factory)
    }
    /// The container this intent's site births its cores into (fix round 1, L2).
    pub(crate) fn site_container(&self) -> Result<ObjId, Error> {
        Ok(self
            .policy
            .site(self.certified())
            .map_err(bad)?
            .output_container)
    }
    pub(crate) fn validate(&self) -> Result<(), Error> {
        self.policy.validate().map_err(bad)?;
        self.recipe.validate()?;
        match (&self.certificate, &self.uncertified) {
            (Some(c), None) => c.matches_lot(&self.lot).map_err(bad)?,
            (None, Some(u)) => u.matches_lot(&self.lot).map_err(bad)?,
            _ => return Err(Error::Integrity),
        }
        let (class, _) = self.die()?;
        if self.format != 1
            || self.offer_digest.as_bytes() == &[0; 32]
            || !super::offer::holds_site(
                &self.facility,
                &self.policy.site(self.certified()).map_err(bad)?,
            )
            || self.recipe.definition() != &self.policy.recipe
            || self.recipe.class() != class
            || self.expires_at
                != mush_inventory::reservation_deadline(&self.accepted_at).map_err(bad)?
        {
            return Err(Error::Integrity);
        }
        CoreProgress::new(
            self.order,
            self.accepted_at.clone(),
            self.recipe.work_units(),
        )?;
        encode(self, INTENT_BYTES).map_err(bad)?;
        Ok(())
    }
    pub(crate) fn digest(&self) -> Result<DefinitionDigest, Error> {
        self.validate()?;
        digest("industry.core_integration_intent.v1", self, INTENT_BYTES)
    }
    pub(crate) fn input_identity(&self) -> Result<CoreInputIdentity, Error> {
        Ok(CoreInputIdentity::new(self.order.0, self.digest()?))
    }
    pub(crate) fn subject(&self) -> Result<CoreFundingSubject, Error> {
        self.validate()?;
        let (class, operation) = self.die()?;
        let class = match class {
            DieClass::I => mush_economy::core_integration::CoreClass::I,
            DieClass::II => mush_economy::core_integration::CoreClass::II,
            DieClass::III => mush_economy::core_integration::CoreClass::III,
        };
        let s = CoreFundingSubject {
            order: self.order.0,
            intent: self.digest()?,
            offer: self.offer,
            offer_digest: self.offer_digest,
            requester: self.requester,
            payer: self.payer,
            input_vault: self.lot.endpoint().0,
            die_lot: self.lot.id().0,
            die_certificate: operation,
            class,
            recipe: self.policy.recipe.clone(),
            policy: self.policy.digest().map_err(bad)?,
            accepted_at: self.accepted_at.clone(),
            expires_at: self.expires_at.clone(),
            source: self.source(),
        };
        s.validate().map_err(bad)?;
        Ok(s)
    }
    pub(crate) fn matches_offer(&self, offer: &FactoryOffer) -> Result<(), Error> {
        self.validate()?;
        offer.validate()?;
        if self.offer != offer.operation
            || self.offer_digest != offer.digest()?
            || self.requester != offer.requester
            || self.policy != offer.policy
            || self.facility != offer.facility
            || self.recipe != offer.recipe
        {
            return Err(Error::Integrity);
        }
        match &offer.input {
            super::offer::OfferedInput::SupplierStock { certificate, lot }
                if self.certificate.as_deref() == Some(certificate.as_ref())
                    && lot.as_ref() == &self.lot => {}
            super::offer::OfferedInput::RequesterSupplied {}
                if self.certificate.is_some()
                    && !self.policy.supplier_stock.contains(&self.lot.endpoint().0) => {}
            super::offer::OfferedInput::Uncertified {}
                if self.uncertified.is_some()
                    && !self.policy.supplier_stock.contains(&self.lot.endpoint().0) => {}
            _ => return Err(Error::Integrity),
        }
        Ok(())
    }
    pub(crate) fn output_order(&self) -> Result<mush_core::core_output::CoreOutputOrder, Error> {
        mush_core::core_output::CoreOutputOrder::new(
            self.order.0,
            self.digest()?,
            self.requester,
            self.policy.recipe.clone(),
            self.site_factory()?,
            self.site_container()?,
        )
        .map_err(bad)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReservedOutput {
    pub(crate) object: ObjId,
    pub(crate) reservation: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) object_digest: DefinitionDigest,
}
impl ReservedOutput {
    pub(crate) fn capture(
        output: &mush_core::core_output::ReservedCoreOutput,
        intent: &AcceptedIntent,
        commit: CommitToken,
        action: &DefinitionRef,
    ) -> Result<Self, Error> {
        if output.order() != &intent.output_order()?
            || output.commit() != commit
            || output.action() != action
            || action != &intent.policy.action_ref(super::RESERVE).map_err(bad)?
        {
            return Err(Error::Integrity);
        }
        Ok(Self {
            object: output.object(),
            reservation: output.reservation(),
            action: output.reservation_action().clone(),
            object_digest: output.object_digest(),
        })
    }
    pub(crate) fn validate(&self, intent: &AcceptedIntent) -> Result<(), Error> {
        if self.object.0 < 2
            || self.object == intent.policy.factory
            || self.object == intent.site_factory()?
            || self.object == intent.policy.output_container
            || self.object == intent.site_container()?
            || self.object_digest.as_bytes() == &[0; 32]
            || self.action != intent.policy.action_ref(super::RESERVE).map_err(bad)?
        {
            return Err(Error::Integrity);
        }
        Ok(())
    }
    pub(crate) fn matches_history(
        &self,
        h: &mush_core::core_output::CoreOutputHistory,
        intent: &AcceptedIntent,
    ) -> Result<(), Error> {
        self.validate(intent)?;
        if h.order() != &intent.output_order()?
            || h.object() != self.object
            || h.reservation() != self.reservation
            || h.action() != &self.action
            || h.object_digest() != self.object_digest
        {
            return Err(Error::Integrity);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CoreIntegrationOrder {
    pub(crate) format: u16,
    pub(crate) sequence: u64,
    pub(crate) intent: AcceptedIntent,
    pub(crate) accepted: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) funding: CoreFundingReceipt,
    pub(crate) capacity: ReservationId,
    pub(crate) progress: CoreProgress,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) output: Option<ReservedOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) terminal: Option<CoreTerminalReceipt>,
}
impl CoreIntegrationOrder {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        self.intent.validate()?;
        self.progress.validate()?;
        if self.format != 1
            || !(1..=MAX_ORDERS).contains(&self.sequence)
            || self.action != self.intent.policy.action_ref(super::ACCEPT).map_err(bad)?
            || self.funding.commit() != self.accepted
            || self.funding.action() != &self.action
            || self.progress.production().started() != &self.intent.accepted_at
            || self.progress.production().total_units() != self.intent.recipe.work_units()
            || self.progress.order() != self.intent.order
        {
            return Err(Error::Integrity);
        }
        if let Some(output) = &self.output {
            output.validate(&self.intent)?;
        }
        if let Some(t) = &self.terminal {
            if t.commit() == self.accepted || t.posted_at_utc() < self.funding.posted_at_utc() {
                return Err(Error::Integrity);
            }
            match t.result() {
                CoreTerminalResult::Completed { output, title } => {
                    let reserved = self.output.as_ref().ok_or(Error::Integrity)?;
                    if !self.progress.production().is_complete()
                        || t.action()
                            != &self
                                .intent
                                .policy
                                .action_ref(super::COMPLETE)
                                .map_err(bad)?
                        || t.at_ic().clock != self.intent.accepted_at.clock
                        || t.at_ic().at < self.intent.accepted_at.at
                        || t.at_ic().at >= self.intent.expires_at.at
                        || output.object != reserved.object
                        || output.reservation != reserved.reservation
                        || output.reservation_action != reserved.action
                        || output.object_digest != reserved.object_digest
                        || title.object != reserved.object
                        || title.title_owner != mush_org::PrincipalId::Object(self.intent.requester)
                    {
                        return Err(Error::Integrity);
                    }
                }
                CoreTerminalResult::Cancelled {} => {
                    if t.action() != &self.intent.policy.action_ref(super::CANCEL).map_err(bad)? {
                        return Err(Error::Integrity);
                    }
                }
            }
        }
        encode(self, ORDER_BYTES).map_err(bad)?;
        Ok(())
    }
    pub(crate) fn verify_payment(&self, h: &CorePaymentHistory) -> Result<(), Error> {
        self.validate()?;
        if h.subject() != &self.intent.subject()?
            || h.policy() != &self.intent.policy
            || h.funding() != &self.funding
            || h.terminal() != self.terminal.as_ref()
        {
            return Err(Error::Integrity);
        }
        Ok(())
    }
    pub(crate) fn verify_input(&self, input: &CoreInputOperation) -> Result<(), Error> {
        self.validate()?;
        if input.identity() != self.intent.input_identity()?
            || input.lot() != &self.intent.lot
            || input.accepted() != self.accepted
            || input.action() != &self.action
        {
            return Err(Error::Integrity);
        }
        let expected = match self.terminal.as_ref().map(|t| t.result()) {
            None => CoreInputState::Held,
            Some(CoreTerminalResult::Completed { .. }) => CoreInputState::Consumed,
            Some(CoreTerminalResult::Cancelled {}) => CoreInputState::Released,
        };
        if input.state().map_err(bad)? != expected
            || input.terminal_commit() != self.terminal.as_ref().map(|t| t.commit())
            || input.terminal_action() != self.terminal.as_ref().map(|t| t.action())
        {
            return Err(Error::Integrity);
        }
        Ok(())
    }
    pub(crate) fn admission_digest(&self) -> Result<DefinitionDigest, Error> {
        digest(
            "industry.core_integration_admission.v1",
            &serde_json::json!({"sequence":self.sequence,"intent":self.intent,"accepted":self.accepted,"action":self.action,"funding":self.funding,"capacity":self.capacity}),
            ORDER_BYTES,
        )
    }
}

/// Independently persisted owner census. Output births and terminal operations
/// have their own actual Core/Inventory/E4 inverse; these immutable chains make
/// whole loss of the typed offer/admission inventory detectable as well.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IntegrationIndex {
    pub(crate) format: u16,
    pub(crate) initialized: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) policy: CorePaymentPolicy,
    pub(crate) offered: u64,
    pub(crate) accepted: u64,
    pub(crate) offer_chain: DefinitionDigest,
    pub(crate) order_chain: DefinitionDigest,
}
impl IntegrationIndex {
    pub(crate) fn new(operation: CommitToken, policy: CorePaymentPolicy) -> Result<Self, Error> {
        let action = policy.action_ref(super::INITIALIZE).map_err(bad)?;
        if !super::super::fixed(&action, "action", super::INITIALIZE) {
            return Err(Error::Invalid);
        }
        let genesis = digest(
            "industry.core_integration_genesis.v1",
            &serde_json::json!({"operation":operation,"action":action}),
            4096,
        )?;
        Ok(Self {
            format: 1,
            initialized: operation,
            action,
            policy,
            offered: 0,
            accepted: 0,
            offer_chain: genesis,
            order_chain: genesis,
        })
    }
    pub(crate) fn validate(&self) -> Result<(), Error> {
        self.policy.validate().map_err(bad)?;
        if self.action != self.policy.action_ref(super::INITIALIZE).map_err(bad)? {
            return Err(Error::Integrity);
        }
        if self.format != 1
            || self.offered > MAX_ORDERS
            || self.accepted > self.offered
            || !super::super::fixed(&self.action, "action", super::INITIALIZE)
            || self.offer_chain.as_bytes() == &[0; 32]
            || self.order_chain.as_bytes() == &[0; 32]
        {
            return Err(Error::Integrity);
        }
        Ok(())
    }
    pub(crate) fn offer(&self, offer: &FactoryOffer) -> Result<Self, Error> {
        self.validate()?;
        offer.validate()?;
        if self.offered >= MAX_ORDERS || offer.sequence != self.offered + 1 {
            return Err(Error::Capacity);
        }
        let mut next = self.clone();
        next.offered = offer.sequence;
        next.offer_chain = digest(
            "industry.core_integration_offers.v1",
            &serde_json::json!({"previous":self.offer_chain,"offer":offer}),
            super::offer::OFFER_BYTES + 1024,
        )?;
        Ok(next)
    }
    pub(crate) fn accept(&self, order: &CoreIntegrationOrder) -> Result<Self, Error> {
        self.validate()?;
        order.validate()?;
        if self.accepted >= self.offered || order.sequence != self.accepted + 1 {
            return Err(Error::Capacity);
        }
        let mut next = self.clone();
        next.accepted = order.sequence;
        next.order_chain = digest(
            "industry.core_integration_orders.v1",
            &serde_json::json!({"previous":self.order_chain,"admission":order.admission_digest()?}),
            4096,
        )?;
        Ok(next)
    }
}
