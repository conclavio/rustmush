//! Exact Market wire. Roll validates the common state relation; Market also
//! pins the retained manifest, complete offer and exact owner references.
//!
//! Doc 46 CT.6: the two records are `checked!` records, which cannot take a type parameter, so
//! each family expands [`records!`] once: one text, the family's own record types. The
//! generic owner reads them through [`OperationRow`] and [`ReceiptRow`].
use super::config::RenewalConfig;
use super::{Issuer, Offer, RollOperation};
use mush_core::{ClockStamp, CommitToken, DefinitionRef};
use mush_economy::ContractId;
use mush_economy::renewal_twin::RenewalOfferView;
use mush_roll::commerce::renewal_twin::{Issuer as Roll, protocol::Step, records::RenewalStateV1};
use serde::{Serialize, de::DeserializeOwned};
use std::fmt::Debug;

pub(crate) fn invalid() -> String {
    "renewal records disagree or exceed their bounds".into()
}
pub(crate) fn error(_: impl std::fmt::Display) -> String {
    invalid()
}
pub(crate) fn bounded<T: Serialize>(row: &T, max: usize) -> Result<(), String> {
    let value = serde_json::to_value(row).map_err(error)?;
    mush_core::canonical_json_bytes_with_limit(&value, max).map_err(error)?;
    Ok(())
}
pub(crate) fn decode<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
    max: usize,
) -> Result<T, String> {
    if bytes.is_empty() || bytes.len() > max {
        return Err(invalid());
    }
    // Typed first rejects duplicate fields before Value can collapse them.
    let row: T = serde_json::from_slice(bytes).map_err(error)?;
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(error)?;
    if serde_json::to_value(&row).map_err(error)? != value {
        return Err(invalid());
    }
    Ok(row)
}

/// A row of one of the market's two renewal state families.
pub(crate) trait Row: Clone + Serialize + DeserializeOwned + Send + Sync + 'static {
    const FAMILY: &'static str;
    const MAX_BYTES: usize;
    fn validate(&self) -> Result<(), String>;
    fn key(&self) -> String;
}

/// The market's renewal operation, as the generic owner reads, captures and advances it.
pub(crate) trait OperationRow<I: Issuer>: Row + PartialEq + Debug {
    fn operation(&self) -> CommitToken;
    fn manifest(&self) -> &DefinitionRef;
    fn participants(&self) -> &[DefinitionRef; 2];
    fn binding(&self) -> &<I as Roll>::Binding;
    fn offer(&self) -> &Offer<I>;
    fn contract(&self) -> ContractId;
    fn consent_hash(&self) -> &[u8; 32];
    fn created_at_ic(&self) -> &ClockStamp;
    fn created_at_utc(&self) -> i64;
    fn state(&self) -> &RenewalStateV1<I>;
    /// The exact action reference of `step` under this row's manifest and participants.
    fn expected_step(&self, step: Step) -> Result<DefinitionRef, String>;
    fn matches_roll(&self, actual: &RollOperation<I>) -> Result<(), String>;
    fn verify_history(&self, actual: &RenewalOfferView<'_, I>) -> Result<(), String>;
    fn captured(
        config: &RenewalConfig<I>,
        offer: Offer<I>,
        row: &RollOperation<I>,
    ) -> Result<Self, String>;
    fn progressed(&self, row: &RollOperation<I>) -> Result<Self, String>;
    fn receipt(&self) -> Result<I::Receipt, String>;
}

/// The market's terminal receipt of a renewal operation.
pub(crate) trait ReceiptRow<I: Issuer>: Row + PartialEq + Debug {
    fn operation(&self) -> CommitToken;
    fn matches_operation(&self, operation: &I::Renewal) -> Result<(), String>;
    fn verify_history(
        &self,
        operation: &I::Renewal,
        actual: &RenewalOfferView<'_, I>,
    ) -> Result<(), String>;
}

/// The family's two records: `$issuer` the family's issuer, `$protocol` its Roll protocol
/// module (its `Transition`), each record's constants its issuer's.
macro_rules! records {
    ($issuer:ty, $protocol:path) => {
        use $crate::renewal_twin::records::{bounded, decode, error, invalid};
        #[cfg(test)]
        use $protocol as protocol;
        use mush_core::checked;
        use mush_core::{
            ClockStamp, CommitToken, DefinitionDigest, DefinitionRef,
            mechanics::{DefinitionRegistryBuilder, OwnerId},
        };
        use mush_economy::ContractId;
        use mush_economy::renewal_twin::{
            RenewalOfferView, RenewalOutcome, RenewalServiceCloseReason, UnfundedCloseReason,
        };
        use mush_roll::commerce::renewal_twin::{
            protocol::Step,
            records::{RenewalAttemptV1, RenewalStateV1},
        };
        use mush_roll::commerce::{LicenceId, TermGeneration, purchase::PurchaseParticipants};
        use serde::Serialize;

        type Binding = <$issuer as mush_roll::commerce::renewal_twin::Issuer>::Binding;
        type RollRow = <$issuer as mush_roll::commerce::renewal_twin::Issuer>::Operation;
        type Offer = <$issuer as mush_economy::renewal_twin::Issuer>::Offer;
        type State = RenewalStateV1<$issuer>;

        checked!(pub struct MindRenewalOperationV1, OperationWire, pub(crate) fields, {
            format:u16,operation:CommitToken,action:DefinitionRef,attempt:RenewalAttemptV1,
            manifest:DefinitionRef,participants:[DefinitionRef;2],binding:Box<Binding>,
            offer:Box<Offer>,offer_digest:DefinitionDigest,contract:ContractId,consent_hash:[u8;32],
            created_at_ic:ClockStamp,created_at_utc:i64,state:State,
        });

        /// A serialization projection, never an actual Roll owner output.
        #[derive(Serialize)]
        struct Shared<'a> {
            format: u16,
            operation: CommitToken,
            action: &'a DefinitionRef,
            attempt: RenewalAttemptV1,
            binding: &'a Binding,
            offer_digest: DefinitionDigest,
            contract: ContractId,
            consent_hash: [u8; 32],
            created_at_ic: &'a ClockStamp,
            created_at_utc: i64,
            state: &'a State,
        }
        impl MindRenewalOperationV1 {
            pub const FAMILY: &'static str =
                <$issuer as $crate::renewal_twin::Issuer>::OPERATION_FAMILY;
            /// Raw/body ceiling. Registration must separately bound the complete typed document.
            pub const MAX_BYTES: usize = 32_768;
            pub fn decode(bytes: &[u8]) -> Result<Self, String> {
                decode(bytes, Self::MAX_BYTES)
            }
            pub fn validate(&self) -> Result<(), String> {
                if self.format != 1 || self.action != self.expected_step(Step::Draft)? {
                    return Err(invalid());
                }
                bounded(&self.offer, mush_economy::renewal_twin::MAX_RENEWAL_OFFER_BYTES)?;
                let shared = self.roll_projection()?;
                shared.matches_offer(&self.offer).map_err(error)?;
                let start = &self.offer.terms.not_before;
                let expiry = &self.offer.terms.expires_at;
                if self.offer.digest().map_err(error)? != self.offer_digest
                    || self.created_at_ic.clock != start.clock
                    || self.created_at_ic.at < start.at
                    || self.created_at_ic.at >= expiry.at
                {
                    return Err(invalid());
                }
                if let Some(funding) = self.state.funding()
                    && (funding.action() != &self.expected_step(Step::Fund)?
                        || funding.at_ic().clock != start.clock
                        || funding.at_ic().at < start.at
                        || funding.at_ic().at >= expiry.at)
                {
                    return Err(invalid());
                }
                match &self.state {
                    RenewalStateV1::Granted { terminal, .. } => {
                        if terminal.action() != &self.expected_step(Step::Grant)?
                            || terminal.at_ic().clock != expiry.clock
                            || terminal.at_ic().at >= expiry.at
                        {
                            return Err(invalid());
                        }
                    }
                    RenewalStateV1::UnfundedClosed { closure } => {
                        if closure.at_ic().clock != self.created_at_ic.clock
                            || closure.at_ic().at < self.created_at_ic.at
                            || (closure.reason() == UnfundedCloseReason::Expired
                                && closure.at_ic().at < expiry.at)
                        {
                            return Err(invalid());
                        }
                    }
                    RenewalStateV1::UnfundedServiceClosed { closure } => match closure.reason() {
                        RenewalServiceCloseReason::DeadlineReached => {
                            if closure.at_ic().clock != self.created_at_ic.clock
                                || closure.at_ic().at < expiry.at
                            {
                                return Err(invalid());
                            }
                        }
                        RenewalServiceCloseReason::ClockInvalidated => {
                            if closure.at_ic().clock == self.created_at_ic.clock {
                                return Err(invalid());
                            }
                        }
                    },
                    _ => {}
                }
                // Refund/Close can use a current cleanup descriptor after a manifest
                // change. Their closed locator is checked by the shared codec; actual
                // owner history and dispatch establish the complete action reference.
                bounded(self, Self::MAX_BYTES)
            }
            #[cfg(test)]
            pub(crate) fn expected_action(
                &self,
                transition: protocol::Transition,
            ) -> Result<DefinitionRef, String> {
                self.expected_step(transition.step())
            }
            fn expected_step(&self, step: Step) -> Result<DefinitionRef, String> {
                let [economy, retention] = &self.participants;
                let participants =
                    PurchaseParticipants::new(economy.clone(), retention.clone()).map_err(error)?;
                DefinitionRegistryBuilder::new()
                    .register(
                        OwnerId::new("market").map_err(error)?,
                        mush_roll::commerce::renewal_twin::protocol::descriptor::<$issuer>(
                            step,
                            &self.manifest,
                            &participants,
                        )
                        .map_err(error)?,
                    )
                    .map_err(error)
            }
            pub(crate) fn roll_projection(&self) -> Result<RollRow, String> {
                let shared = Shared {
                    format: self.format,
                    operation: self.operation,
                    action: &self.action,
                    attempt: self.attempt,
                    binding: &self.binding,
                    offer_digest: self.offer_digest,
                    contract: self.contract,
                    consent_hash: self.consent_hash,
                    created_at_ic: &self.created_at_ic,
                    created_at_utc: self.created_at_utc,
                    state: &self.state,
                };
                serde_json::from_value(serde_json::to_value(shared).map_err(error)?).map_err(error)
            }
            pub const fn operation(&self) -> CommitToken {
                self.operation
            }
            pub fn key(&self) -> String {
                self.operation.to_hex()
            }
            pub fn action(&self) -> &DefinitionRef {
                &self.action
            }
            pub const fn attempt(&self) -> RenewalAttemptV1 {
                self.attempt
            }
            pub fn manifest(&self) -> &DefinitionRef {
                &self.manifest
            }
            pub fn participants(&self) -> &[DefinitionRef; 2] {
                &self.participants
            }
            pub fn binding(&self) -> &Binding {
                &self.binding
            }
            pub fn offer(&self) -> &Offer {
                &self.offer
            }
            pub const fn offer_digest(&self) -> DefinitionDigest {
                self.offer_digest
            }
            pub const fn contract(&self) -> ContractId {
                self.contract
            }
            pub const fn consent_hash(&self) -> &[u8; 32] {
                &self.consent_hash
            }
            pub fn created_at_ic(&self) -> &ClockStamp {
                &self.created_at_ic
            }
            pub const fn created_at_utc(&self) -> i64 {
                self.created_at_utc
            }
            pub fn state(&self) -> &State {
                &self.state
            }
        }

        checked!(pub struct MindRenewalReceiptV1, ReceiptWire, pub(crate) fields, {
            format:u16,operation:CommitToken,licence:LicenceId,predecessor:TermGeneration,
            next:TermGeneration,contract:ContractId,consent_hash:[u8;32],state:State,
        });
        impl MindRenewalReceiptV1 {
            pub const FAMILY: &'static str =
                <$issuer as $crate::renewal_twin::Issuer>::RECEIPT_FAMILY;
            pub const MAX_BYTES: usize = 8_192;
            pub fn decode(bytes: &[u8]) -> Result<Self, String> {
                decode(bytes, Self::MAX_BYTES)
            }
            pub fn validate(&self) -> Result<(), String> {
                if self.format != 1
                    || self.predecessor.get().checked_add(1) != Some(self.next.get())
                {
                    return Err(invalid());
                }
                match &self.state {
                    RenewalStateV1::Granted { funding, terminal }
                    | RenewalStateV1::Refunded { funding, terminal } => {
                        let granted = matches!(&self.state, RenewalStateV1::Granted { .. });
                        if funding.commit() == self.operation
                            || funding.contract() != self.contract
                            || funding.consent_hash() != &self.consent_hash
                            || terminal.commit() == self.operation
                            || terminal.commit() == funding.commit()
                            || terminal.transaction() == funding.transaction()
                            || terminal.journal_seq() <= funding.journal_seq()
                            || terminal.posted_at_utc() < funding.posted_at_utc()
                            || (terminal.outcome() == RenewalOutcome::Granted) != granted
                            || (granted
                                && (terminal.at_ic().clock != funding.at_ic().clock
                                    || terminal.at_ic().at < funding.at_ic().at))
                        {
                            return Err(invalid());
                        }
                    }
                    RenewalStateV1::UnfundedClosed { closure }
                        if closure.commit() != self.operation =>
                    {
                        closure.validate().map_err(error)?;
                    }
                    RenewalStateV1::UnfundedServiceClosed { closure }
                        if closure.commit() != self.operation =>
                    {
                        closure.validate().map_err(error)?;
                    }
                    _ => return Err(invalid()),
                }
                bounded(self, Self::MAX_BYTES)
            }
            pub const fn operation(&self) -> CommitToken {
                self.operation
            }
            pub fn key(&self) -> String {
                self.operation.to_hex()
            }
            pub const fn licence(&self) -> LicenceId {
                self.licence
            }
            pub const fn predecessor(&self) -> TermGeneration {
                self.predecessor
            }
            pub const fn next(&self) -> TermGeneration {
                self.next
            }
            pub const fn contract(&self) -> ContractId {
                self.contract
            }
            pub const fn consent_hash(&self) -> &[u8; 32] {
                &self.consent_hash
            }
            pub fn state(&self) -> &State {
                &self.state
            }
        }

        // Structural pairing is separate from actual borrowed owner evidence. Neither
        // path decides current eligibility or publishes a grant.
        impl MindRenewalOperationV1 {
            /// Exact data equality with a Roll row. The caller must obtain that row
            /// through the registered owner/typed preimage path, not an arbitrary decode.
            pub fn matches_roll(&self, actual: &RollRow) -> Result<(), String> {
                self.validate()?;
                actual.validate().map_err(|_| invalid())?;
                if &self.roll_projection()? != actual {
                    return Err(invalid());
                }
                Ok(())
            }
            /// Requires the Economy owner's private combined audit view. This cannot
            /// be called with a serialized receipt or a merely terminal contract.
            pub fn verify_history(
                &self,
                actual: &RenewalOfferView<'_, $issuer>,
            ) -> Result<(), String> {
                self.validate()?;
                if actual.offer() != self.offer.as_ref() {
                    return Err(invalid());
                }
                self.roll_projection()?
                    .verify_history(actual)
                    .map_err(|_| invalid())
            }
        }
        impl MindRenewalReceiptV1 {
            pub fn matches_operation(
                &self,
                operation: &MindRenewalOperationV1,
            ) -> Result<(), String> {
                self.validate()?;
                operation.validate()?;
                let binding = operation.binding();
                if self.operation != operation.operation()
                    || self.licence != binding.licence()
                    || self.predecessor != binding.predecessor()
                    || self.next != binding.next()
                    || self.contract != operation.contract()
                    || &self.consent_hash != operation.consent_hash()
                    || &self.state != operation.state()
                {
                    return Err(invalid());
                }
                Ok(())
            }
            pub fn verify_history(
                &self,
                operation: &MindRenewalOperationV1,
                actual: &RenewalOfferView<'_, $issuer>,
            ) -> Result<(), String> {
                self.matches_operation(operation)?;
                operation.verify_history(actual)
            }
        }

        // Only an actual closed writer result can supply the paired publication here.
        // These helpers do not mint owner evidence or install any rows.
        impl MindRenewalOperationV1 {
            pub(crate) fn captured(
                config: &$crate::renewal_twin::config::RenewalConfig<$issuer>,
                offer: Offer,
                row: &RollRow,
            ) -> Result<Self, String> {
                let value = Self {
                    format: 1,
                    operation: row.operation(),
                    action: row.action().clone(),
                    attempt: row.attempt(),
                    manifest: config.manifest.clone(),
                    participants: config.participants.ordered(),
                    binding: Box::new(row.binding().clone()),
                    offer: Box::new(offer),
                    offer_digest: row.offer_digest(),
                    contract: row.contract(),
                    consent_hash: *row.consent_hash(),
                    created_at_ic: row.created_at_ic().clone(),
                    created_at_utc: row.created_at_utc(),
                    state: row.state().clone(),
                };
                value.matches_roll(row)?;
                Ok(value)
            }
            pub(crate) fn progressed(&self, row: &RollRow) -> Result<Self, String> {
                let value = Self {
                    state: row.state().clone(),
                    ..self.clone()
                };
                value.matches_roll(row)?;
                Ok(value)
            }
            pub(crate) fn receipt(&self) -> Result<MindRenewalReceiptV1, String> {
                let value = MindRenewalReceiptV1 {
                    format: 1,
                    operation: self.operation,
                    licence: self.binding.licence(),
                    predecessor: self.binding.predecessor(),
                    next: self.binding.next(),
                    contract: self.contract,
                    consent_hash: self.consent_hash,
                    state: self.state.clone(),
                };
                value.matches_operation(self)?;
                Ok(value)
            }
        }

        impl $crate::renewal_twin::records::Row for MindRenewalOperationV1 {
            const FAMILY: &'static str = MindRenewalOperationV1::FAMILY;
            const MAX_BYTES: usize = MindRenewalOperationV1::MAX_BYTES;
            fn validate(&self) -> Result<(), String> {
                Self::validate(self)
            }
            fn key(&self) -> String {
                Self::key(self)
            }
        }
        impl $crate::renewal_twin::records::Row for MindRenewalReceiptV1 {
            const FAMILY: &'static str = MindRenewalReceiptV1::FAMILY;
            const MAX_BYTES: usize = MindRenewalReceiptV1::MAX_BYTES;
            fn validate(&self) -> Result<(), String> {
                Self::validate(self)
            }
            fn key(&self) -> String {
                self.operation().to_hex()
            }
        }
        impl $crate::renewal_twin::records::OperationRow<$issuer> for MindRenewalOperationV1 {
            fn operation(&self) -> CommitToken {
                Self::operation(self)
            }
            fn manifest(&self) -> &DefinitionRef {
                Self::manifest(self)
            }
            fn participants(&self) -> &[DefinitionRef; 2] {
                Self::participants(self)
            }
            fn binding(&self) -> &Binding {
                Self::binding(self)
            }
            fn offer(&self) -> &Offer {
                Self::offer(self)
            }
            fn contract(&self) -> ContractId {
                Self::contract(self)
            }
            fn consent_hash(&self) -> &[u8; 32] {
                Self::consent_hash(self)
            }
            fn created_at_ic(&self) -> &ClockStamp {
                Self::created_at_ic(self)
            }
            fn created_at_utc(&self) -> i64 {
                Self::created_at_utc(self)
            }
            fn state(&self) -> &State {
                Self::state(self)
            }
            fn expected_step(&self, step: Step) -> Result<DefinitionRef, String> {
                Self::expected_step(self, step)
            }
            fn matches_roll(&self, actual: &RollRow) -> Result<(), String> {
                Self::matches_roll(self, actual)
            }
            fn verify_history(&self, actual: &RenewalOfferView<'_, $issuer>) -> Result<(), String> {
                Self::verify_history(self, actual)
            }
            fn captured(
                config: &$crate::renewal_twin::config::RenewalConfig<$issuer>,
                offer: Offer,
                row: &RollRow,
            ) -> Result<Self, String> {
                Self::captured(config, offer, row)
            }
            fn progressed(&self, row: &RollRow) -> Result<Self, String> {
                Self::progressed(self, row)
            }
            fn receipt(&self) -> Result<MindRenewalReceiptV1, String> {
                Self::receipt(self)
            }
        }
        impl $crate::renewal_twin::records::ReceiptRow<$issuer> for MindRenewalReceiptV1 {
            fn operation(&self) -> CommitToken {
                Self::operation(self)
            }
            fn matches_operation(&self, operation: &MindRenewalOperationV1) -> Result<(), String> {
                Self::matches_operation(self, operation)
            }
            fn verify_history(
                &self,
                operation: &MindRenewalOperationV1,
                actual: &RenewalOfferView<'_, $issuer>,
            ) -> Result<(), String> {
                Self::verify_history(self, operation, actual)
            }
        }
    };
}
pub(crate) use records;
