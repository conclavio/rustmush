//! Strict shared wire and Roll records for the fixed seven-target protocol.
//! All constructors are structural; actual E4 output and current rights are
//! checked separately before any owner image or typed row may be published.

use super::super::{
    CharterId, FiledRevisionId, LicenceId, LineId, MAX_GRACE_SECONDS, MAX_TERM_SECONDS,
    RevisionIdentity, TermGeneration, TermWindow, WorldIcSeconds,
    house_catalogue::HouseIssuer,
    house_purchase::{HouseGrantEconomicsV1, HouseTermEconomicsV1},
    paid::{PaidTermRecord, PaidTerms, PersonalPaidLicence},
    records::{PublicationError, Record, RevisionPolicy},
};
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use mush_economy::{
    ContractId,
    managed_house_renewal::{
        RenewalFundingReceiptV1, RenewalOutcome, RenewalServiceCloseReason,
        RenewalServiceClosureV1, RenewalTerminalReceiptV1, UnfundedCloseReason,
    },
};
use serde::{Deserialize, Serialize};

pub(crate) const MAX_OPERATIONS: usize = 32768;
pub(crate) const MAX_OPERATION_BYTES: usize = 16384;
pub(crate) const MAX_HEAD_BYTES: usize = 4096;

fn invalid(_: impl std::fmt::Display) -> PublicationError {
    PublicationError::Invalid
}
pub(super) fn bounded<T: Serialize>(value: &T, max: usize) -> Result<(), PublicationError> {
    let value = serde_json::to_value(value).map_err(invalid)?;
    mush_core::canonical_json_bytes_with_limit(&value, max)
        .map_err(|_| PublicationError::Capacity)?;
    Ok(())
}
pub(super) fn digest<T: Serialize>(
    domain: &str,
    value: &T,
) -> Result<DefinitionDigest, PublicationError> {
    DefinitionDigest::for_canonical_value(domain, &serde_json::to_value(value).map_err(invalid)?)
        .map_err(invalid)
}
fn versioned(value: &ClockStamp) -> bool {
    matches!(value.clock, ClockRef::WorldIc {generation} if generation!=0)
}
fn action(value: &DefinitionRef, id: &str) -> bool {
    value.key().owner.as_str() == "market"
        && value.key().kind.as_str() == "action"
        && value.key().id.as_str() == id
        && value.version() == 1
}
fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}
fn decode<T: serde::de::DeserializeOwned + Serialize>(
    bytes: &[u8],
    max: usize,
) -> Result<T, PublicationError> {
    if bytes.is_empty() || bytes.len() > max {
        return Err(PublicationError::Capacity);
    }
    // Typed first preserves duplicate-field rejection before any Value map.
    let row: T = serde_json::from_slice(bytes).map_err(invalid)?;
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(invalid)?;
    if serde_json::to_value(&row).map_err(invalid)? != value {
        return Err(PublicationError::Invalid);
    }
    Ok(row)
}

macro_rules! checked {
    ($name:ident,$wire:ident,{$($field:ident:$ty:ty),* $(,)?}) => {
        #[derive(Clone,Debug,PartialEq,Eq,Serialize)]
        pub struct $name {$(pub(crate) $field:$ty),*}
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct $wire {$($field:$ty),*}
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D:serde::Deserializer<'de>>(d:D)->Result<Self,D::Error> {
                let row=$wire::deserialize(d)?;
                let value=Self {$($field:row.$field),*};
                value.validate().map_err(serde::de::Error::custom)?;
                Ok(value)
            }
        }
    };
}

checked!(RenewalBindingV1,BindingWire,{
    format:u16,licence:LicenceId,holder:ObjId,issuer:HouseIssuer,charter:CharterId,line:LineId,
    birth_operation:CommitToken,predecessor:TermGeneration,next:TermGeneration,
    predecessor_operation:CommitToken,predecessor_term:DefinitionDigest,
    predecessor_economics:DefinitionDigest,filed:FiledRevisionId,revision:RevisionIdentity,
    terms:PaidTerms,term_seconds:i64,grace_seconds:i64,grant:HouseGrantEconomicsV1,
    product_digest:DefinitionDigest,
});
impl RenewalBindingV1 {
    pub fn validate(&self) -> Result<(), PublicationError> {
        self.grant.validate()?;
        let template = self.grant.renewal();
        if self.format != 1
            || self.holder.0 == 0
            || self.terms.seats().get() != 1
            || self.predecessor.get().checked_add(1) != Some(self.next.get())
            || !(1..=MAX_TERM_SECONDS).contains(&self.term_seconds)
            || !(0..=MAX_GRACE_SECONDS).contains(&self.grace_seconds)
            || template.terms() != self.terms
            || template.revision() != self.policy()
            || template.term_seconds() != self.term_seconds
            || template.grace_seconds() != self.grace_seconds
        {
            return Err(PublicationError::Mismatch);
        }
        bounded(self, MAX_OPERATION_BYTES)
    }
    pub(crate) fn capture(
        licence: &PersonalPaidLicence,
        term: &PaidTermRecord,
        economics: &HouseTermEconomicsV1,
        issuer: HouseIssuer,
    ) -> Result<Self, PublicationError> {
        term.matches_current(licence)?;
        economics.validate()?;
        let RevisionPolicy::Pin { filed, revision } = term.policy() else {
            return Err(PublicationError::Mismatch);
        };
        let template = economics.grant().renewal();
        let row = Self {
            format: 1,
            licence: licence.id(),
            holder: licence.holder(),
            issuer,
            charter: licence.charter(),
            line: licence.line(),
            birth_operation: licence.operation(),
            predecessor: term.generation(),
            next: TermGeneration::new(
                term.generation()
                    .get()
                    .checked_add(1)
                    .ok_or(PublicationError::Capacity)?,
            )
            .map_err(invalid)?,
            predecessor_operation: term.operation(),
            predecessor_term: Self::term_digest(term)?,
            predecessor_economics: economics.digest()?,
            filed,
            revision,
            terms: term.terms(),
            term_seconds: template.term_seconds(),
            grace_seconds: template.grace_seconds(),
            grant: economics.grant().clone(),
            product_digest: economics.product_digest(),
        };
        row.check_predecessor(licence, term, economics)?;
        Ok(row)
    }
    pub(crate) fn term_digest(term: &PaidTermRecord) -> Result<DefinitionDigest, PublicationError> {
        term.validate()?;
        digest("roll.mind.house_renewal_predecessor.v1", term)
    }
    pub fn template_digest(&self) -> Result<DefinitionDigest, PublicationError> {
        self.validate()?;
        digest("roll.mind.house_renewal_template.v1", self.grant.renewal())
    }
    /// Immutable predecessor proof: deliberately does not compare the live
    /// licence term pointer. Current admission separately calls matches_current.
    pub(crate) fn check_predecessor(
        &self,
        licence: &PersonalPaidLicence,
        term: &PaidTermRecord,
        economics: &HouseTermEconomicsV1,
    ) -> Result<(), PublicationError> {
        self.validate()?;
        licence.validate()?;
        term.validate()?;
        economics.validate()?;
        if !self.matches_birth(licence)
            || term.licence() != self.licence
            || term.generation() != self.predecessor
            || term.operation() != self.predecessor_operation
            || Self::term_digest(term)? != self.predecessor_term
            || term.policy() != self.policy()
            || term.terms() != self.terms
            || economics.licence() != self.licence
            || economics.generation() != self.predecessor
            || economics.operation() != self.predecessor_operation
            || economics.digest()? != self.predecessor_economics
            || economics.product_digest() != self.product_digest
            || economics.grant() != &self.grant
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
    pub(crate) fn matches_birth(&self, licence: &PersonalPaidLicence) -> bool {
        licence.id() == self.licence
            && licence.holder() == self.holder
            && licence.charter() == self.charter
            && licence.line() == self.line
            && licence.operation() == self.birth_operation
    }
    pub const fn issuer(&self) -> HouseIssuer {
        self.issuer
    }
    pub const fn charter(&self) -> CharterId {
        self.charter
    }
    pub const fn line(&self) -> LineId {
        self.line
    }
    pub const fn filed(&self) -> FiledRevisionId {
        self.filed
    }
    pub const fn revision(&self) -> RevisionIdentity {
        self.revision
    }
    pub const fn terms(&self) -> PaidTerms {
        self.terms
    }
    pub const fn term_seconds(&self) -> i64 {
        self.term_seconds
    }
    pub const fn grace_seconds(&self) -> i64 {
        self.grace_seconds
    }
    pub const fn predecessor_term(&self) -> DefinitionDigest {
        self.predecessor_term
    }
    pub const fn predecessor_economics(&self) -> DefinitionDigest {
        self.predecessor_economics
    }
    pub const fn licence(&self) -> LicenceId {
        self.licence
    }
    pub const fn holder(&self) -> ObjId {
        self.holder
    }
    pub const fn predecessor(&self) -> TermGeneration {
        self.predecessor
    }
    pub const fn next(&self) -> TermGeneration {
        self.next
    }
    pub const fn birth_operation(&self) -> CommitToken {
        self.birth_operation
    }
    pub const fn predecessor_operation(&self) -> CommitToken {
        self.predecessor_operation
    }
    pub const fn policy(&self) -> RevisionPolicy {
        RevisionPolicy::Pin {
            filed: self.filed,
            revision: self.revision,
        }
    }
    pub fn grant(&self) -> &HouseGrantEconomicsV1 {
        &self.grant
    }
    pub const fn product_digest(&self) -> DefinitionDigest {
        self.product_digest
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RenewalAttemptV1 {
    Automatic {},
    BuyerRetry { previous_operation: CommitToken },
}

impl RenewalAttemptV1 {
    /// Pure closed data mapping. Only an actual opaque E4 output/audit view
    /// supplies the attempt used at the publication and recovery boundaries.
    pub(crate) fn from_economy(
        value: mush_economy::managed_house_renewal::RenewalAttemptV1,
    ) -> Self {
        match value {
            mush_economy::managed_house_renewal::RenewalAttemptV1::Automatic => Self::Automatic {},
            mush_economy::managed_house_renewal::RenewalAttemptV1::BuyerRetry {
                previous_operation,
            } => Self::BuyerRetry { previous_operation },
        }
    }
}

checked!(RenewalClosureV1,ClosureWire,{
    commit:CommitToken,action:DefinitionRef,at_ic:ClockStamp,posted_at_utc:i64,reason:UnfundedCloseReason,
});
impl RenewalClosureV1 {
    pub fn validate(&self) -> Result<(), PublicationError> {
        if !action(&self.action, "close_house_mind_renewal") || !versioned(&self.at_ic) {
            return Err(PublicationError::Invalid);
        }
        bounded(
            self,
            mush_economy::managed_house_renewal::MAX_RENEWAL_RECEIPT_BYTES,
        )
    }
    pub const fn commit(&self) -> CommitToken {
        self.commit
    }
    pub fn action(&self) -> &DefinitionRef {
        &self.action
    }
    pub fn at_ic(&self) -> &ClockStamp {
        &self.at_ic
    }
    pub const fn posted_at_utc(&self) -> i64 {
        self.posted_at_utc
    }
    pub const fn reason(&self) -> UnfundedCloseReason {
        self.reason
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RenewalStateV1 {
    Drafted {},
    Funded {
        funding: RenewalFundingReceiptV1,
    },
    Granted {
        funding: RenewalFundingReceiptV1,
        terminal: RenewalTerminalReceiptV1,
    },
    Refunded {
        funding: RenewalFundingReceiptV1,
        terminal: RenewalTerminalReceiptV1,
    },
    UnfundedClosed {
        closure: RenewalClosureV1,
    },
    UnfundedServiceClosed {
        closure: RenewalServiceClosureV1,
    },
}
impl RenewalStateV1 {
    pub const fn pending(&self) -> bool {
        matches!(self, Self::Drafted {} | Self::Funded { .. })
    }
    pub const fn granted(&self) -> bool {
        matches!(self, Self::Granted { .. })
    }
    pub const fn retryable(&self) -> bool {
        matches!(
            self,
            Self::Refunded { .. }
                | Self::UnfundedClosed { .. }
                | Self::UnfundedServiceClosed { .. }
        )
    }
    pub fn funding(&self) -> Option<&RenewalFundingReceiptV1> {
        match self {
            Self::Funded { funding }
            | Self::Granted { funding, .. }
            | Self::Refunded { funding, .. } => Some(funding),
            _ => None,
        }
    }
    pub fn terminal(&self) -> Option<&RenewalTerminalReceiptV1> {
        match self {
            Self::Granted { terminal, .. } | Self::Refunded { terminal, .. } => Some(terminal),
            _ => None,
        }
    }
    pub const fn terminal_commit(&self) -> Option<CommitToken> {
        match self {
            Self::Granted { terminal, .. } | Self::Refunded { terminal, .. } => {
                Some(terminal.commit())
            }
            Self::UnfundedClosed { closure } => Some(closure.commit),
            Self::UnfundedServiceClosed { closure } => Some(closure.commit()),
            _ => None,
        }
    }
}

checked!(RenewalOperationV1,OperationWire,{
    format:u16,operation:CommitToken,action:DefinitionRef,attempt:RenewalAttemptV1,
    binding:RenewalBindingV1,offer_digest:DefinitionDigest,contract:ContractId,consent_hash:[u8;32],
    created_at_ic:ClockStamp,created_at_utc:i64,state:RenewalStateV1,
});
impl RenewalOperationV1 {
    pub fn validate(&self) -> Result<(), PublicationError> {
        <Self as Record>::validate(self)
    }
    /// Read-only data relation. This cannot certify current eligibility or
    /// replace the actual Economy owner's replayed history.
    pub fn matches_offer(
        &self,
        offer: &mush_economy::managed_house_renewal::RenewalOffer,
    ) -> Result<(), PublicationError> {
        super::history::check_offer(self, offer)
    }
    /// Joins this record to a borrowed view minted by the actual Economy
    /// audit. There is no decoded or caller-constructed history alternative.
    pub fn verify_history(
        &self,
        actual: &mush_economy::managed_house_renewal::RenewalOfferView<'_>,
    ) -> Result<(), PublicationError> {
        super::history::verify(self, actual)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, PublicationError> {
        decode(bytes, Self::MAX_BYTES)
    }
    pub const fn operation(&self) -> CommitToken {
        self.operation
    }
    pub fn action(&self) -> &DefinitionRef {
        &self.action
    }
    pub const fn attempt(&self) -> RenewalAttemptV1 {
        self.attempt
    }
    pub fn binding(&self) -> &RenewalBindingV1 {
        &self.binding
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
    pub fn state(&self) -> &RenewalStateV1 {
        &self.state
    }
    pub(crate) fn same_origin(&self, other: &Self) -> bool {
        self.operation == other.operation
            && self.action == other.action
            && self.attempt == other.attempt
            && self.binding == other.binding
            && self.offer_digest == other.offer_digest
            && self.contract == other.contract
            && self.consent_hash == other.consent_hash
            && self.created_at_ic == other.created_at_ic
            && self.created_at_utc == other.created_at_utc
    }
    /// Closed phase relation; actual owner receipts and action refs are checked
    /// by the paired producer before this structural relation may publish.
    pub(crate) fn follows(&self, old: &Self) -> Result<(), PublicationError> {
        self.validate()?;
        old.validate()?;
        if !self.same_origin(old) {
            return Err(PublicationError::Mismatch);
        }
        match (&old.state, &self.state) {
            (
                RenewalStateV1::Drafted {},
                RenewalStateV1::Funded { .. }
                | RenewalStateV1::UnfundedClosed { .. }
                | RenewalStateV1::UnfundedServiceClosed { .. },
            ) => Ok(()),
            (
                RenewalStateV1::Funded { funding: a },
                RenewalStateV1::Granted { funding: b, .. }
                | RenewalStateV1::Refunded { funding: b, .. },
            ) if a == b => Ok(()),
            _ => Err(PublicationError::Mismatch),
        }
    }
    /// Exact granted rows from the actual issued stamp. No parent window is
    /// used to cap a paid customer's own dates. This is data, not publication.
    pub(crate) fn granted_rows(
        &self,
    ) -> Result<(PersonalPaidLicence, PaidTermRecord, HouseTermEconomicsV1), PublicationError> {
        self.validate()?;
        let RenewalStateV1::Granted { terminal, .. } = &self.state else {
            return Err(PublicationError::Mismatch);
        };
        let stamp = terminal.issued_at_ic().ok_or(PublicationError::Missing)?;
        let issue = WorldIcSeconds::from_stamp(stamp).map_err(invalid)?;
        let end = stamp
            .at
            .checked_add(self.binding.term_seconds)
            .ok_or(PublicationError::Capacity)?;
        let grace = end
            .checked_add(self.binding.grace_seconds)
            .ok_or(PublicationError::Capacity)?;
        let end = WorldIcSeconds::from_stamp(&ClockStamp {
            clock: stamp.clock.clone(),
            at: end,
        })
        .map_err(invalid)?;
        let grace = WorldIcSeconds::from_stamp(&ClockStamp {
            clock: stamp.clock.clone(),
            at: grace,
        })
        .map_err(invalid)?;
        let b = &self.binding;
        Ok((
            PersonalPaidLicence::original(
                b.licence,
                b.charter,
                b.line,
                b.holder,
                b.next,
                b.birth_operation,
            )?,
            PaidTermRecord::original(
                b.licence,
                b.next,
                TermWindow::new(issue, issue, end, grace).map_err(invalid)?,
                b.terms,
                b.policy(),
                self.operation,
            )?,
            HouseTermEconomicsV1::capture(
                b.licence,
                b.next,
                self.operation,
                b.product_digest,
                b.grant.clone(),
            )?,
        ))
    }
}
impl Record for RenewalOperationV1 {
    const FAMILY: &'static str = "roll_house_mind_renewal_operation";
    const MAX_BYTES: usize = MAX_OPERATION_BYTES;
    fn key(&self) -> String {
        self.operation.to_hex()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        self.binding.validate()?;
        if self.format != 1
            || !action(&self.action, "draft_house_mind_renewal")
            || !versioned(&self.created_at_ic)
            || self.operation == self.binding.birth_operation
            || self.operation == self.binding.predecessor_operation
            || matches!(self.attempt,RenewalAttemptV1::BuyerRetry{previous_operation} if previous_operation==self.operation)
        {
            return Err(PublicationError::Invalid);
        }
        let distinct = |commit: CommitToken| {
            commit != self.operation
                && commit != self.binding.predecessor_operation
                && commit != self.binding.birth_operation
        };
        if let Some(funding) = self.state.funding() {
            if !distinct(funding.commit())
                || funding.contract() != self.contract
                || funding.consent_hash() != &self.consent_hash
                || funding.at_ic().clock != self.created_at_ic.clock
                || funding.at_ic().at < self.created_at_ic.at
                || funding.posted_at_utc() < self.created_at_utc
            {
                return Err(PublicationError::Mismatch);
            }
        }
        if let Some(terminal) = self.state.terminal() {
            let funding = self.state.funding().ok_or(PublicationError::Missing)?;
            let grant = self.state.granted();
            if !distinct(terminal.commit())
                || terminal.commit() == funding.commit()
                || terminal.transaction() == funding.transaction()
                || terminal.journal_seq() <= funding.journal_seq()
                || terminal.posted_at_utc() < funding.posted_at_utc()
                || (terminal.outcome() == RenewalOutcome::Granted) != grant
                || (grant
                    && (terminal.at_ic().clock != funding.at_ic().clock
                        || terminal.at_ic().at < funding.at_ic().at))
            {
                return Err(PublicationError::Mismatch);
            }
        }
        if let RenewalStateV1::UnfundedClosed { closure } = &self.state {
            closure.validate()?;
            if !distinct(closure.commit) || closure.posted_at_utc < self.created_at_utc {
                return Err(PublicationError::Mismatch);
            }
        }
        if let RenewalStateV1::UnfundedServiceClosed { closure } = &self.state {
            closure.validate().map_err(invalid)?;
            if !distinct(closure.commit())
                || closure.posted_at_utc() < self.created_at_utc
                || matches!(self.attempt, RenewalAttemptV1::BuyerRetry { previous_operation } if previous_operation == closure.commit())
                || match closure.reason() {
                    RenewalServiceCloseReason::DeadlineReached => {
                        closure.at_ic().clock != self.created_at_ic.clock
                            || closure.at_ic().at < self.created_at_ic.at
                    }
                    RenewalServiceCloseReason::ClockInvalidated => {
                        closure.at_ic().clock == self.created_at_ic.clock
                    }
                }
            {
                return Err(PublicationError::Mismatch);
            }
        }
        bounded(self, Self::MAX_BYTES)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RenewalHeadV1 {
    pub(crate) format: u16,
    pub(crate) licence: LicenceId,
    pub(crate) holder: ObjId,
    pub(crate) birth_operation: CommitToken,
    pub(crate) current: TermGeneration,
    pub(crate) attempted_predecessor: TermGeneration,
    pub(crate) last_operation: CommitToken,
    pub(crate) pending: Option<CommitToken>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HeadWire {
    format: u16,
    licence: LicenceId,
    holder: ObjId,
    birth_operation: CommitToken,
    current: TermGeneration,
    attempted_predecessor: TermGeneration,
    last_operation: CommitToken,
    #[serde(deserialize_with = "required_option")]
    pending: Option<CommitToken>,
}
impl<'de> Deserialize<'de> for RenewalHeadV1 {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let row = HeadWire::deserialize(d)?;
        let value = Self {
            format: row.format,
            licence: row.licence,
            holder: row.holder,
            birth_operation: row.birth_operation,
            current: row.current,
            attempted_predecessor: row.attempted_predecessor,
            last_operation: row.last_operation,
            pending: row.pending,
        };
        value.validate().map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}
impl RenewalHeadV1 {
    pub fn validate(&self) -> Result<(), PublicationError> {
        <Self as Record>::validate(self)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, PublicationError> {
        decode(bytes, Self::MAX_BYTES)
    }
    pub(crate) fn for_operation(row: &RenewalOperationV1) -> Result<Self, PublicationError> {
        row.validate()?;
        let b = &row.binding;
        let value = Self {
            format: 1,
            licence: b.licence,
            holder: b.holder,
            birth_operation: b.birth_operation,
            current: if row.state.granted() {
                b.next
            } else {
                b.predecessor
            },
            attempted_predecessor: b.predecessor,
            last_operation: row.operation,
            pending: row.state.pending().then_some(row.operation),
        };
        value.validate()?;
        Ok(value)
    }
    pub const fn licence(&self) -> LicenceId {
        self.licence
    }
    pub const fn current(&self) -> TermGeneration {
        self.current
    }
    pub const fn pending(&self) -> Option<CommitToken> {
        self.pending
    }
    pub const fn last_operation(&self) -> CommitToken {
        self.last_operation
    }
}
impl Record for RenewalHeadV1 {
    const FAMILY: &'static str = "roll_house_mind_renewal_head";
    const MAX_BYTES: usize = MAX_HEAD_BYTES;
    fn key(&self) -> String {
        self.licence.key()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        if self.format != 1
            || self.holder.0 == 0
            || self.last_operation == self.birth_operation
            || (self.current != self.attempted_predecessor
                && self.attempted_predecessor.get().checked_add(1) != Some(self.current.get()))
            || self.pending.is_some_and(|op| {
                op != self.last_operation || self.current != self.attempted_predecessor
            })
        {
            return Err(PublicationError::Invalid);
        }
        bounded(self, Self::MAX_BYTES)
    }
}
