//! Strict shared wire and Roll records for the fixed seven-target protocol.
//! All constructors are structural; actual E4 output and current rights are
//! checked separately before any owner image or typed row may be published.
//!
//! Doc 46 CT.5: what both families' `records.rs` held word for word. The attempt is one type
//! for both families, the state and the head are generic over the issuer, and the three
//! `checked!` records stay each family's own (the binding has another shape in each; the
//! macro takes no type parameter): [`binding_records`] and [`operation_records`] give them
//! one text, expanded in each family's file beside its own records.

use super::super::records::{PublicationError, Record};
use super::super::{Holder, LicenceId, TermGeneration};
use super::Issuer;
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef};
use mush_economy::managed_paid_resale::RenewalHolderBasisV1;
use mush_economy::renewal_twin::{
    RenewalFundingReceiptV1, RenewalServiceClosureV1, RenewalTerminalReceiptV1,
};
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;

pub(crate) const MAX_OPERATIONS: usize = 32768;
pub(crate) const MAX_OPERATION_BYTES: usize = 16384;
pub(crate) const MAX_HEAD_BYTES: usize = 4096;

pub(crate) fn invalid(_: impl std::fmt::Display) -> PublicationError {
    PublicationError::Invalid
}
pub(crate) fn bounded<T: Serialize>(value: &T, max: usize) -> Result<(), PublicationError> {
    let value = serde_json::to_value(value).map_err(invalid)?;
    mush_core::canonical_json_bytes_with_limit(&value, max)
        .map_err(|_| PublicationError::Capacity)?;
    Ok(())
}
pub(crate) fn digest<T: Serialize>(
    domain: &str,
    value: &T,
) -> Result<DefinitionDigest, PublicationError> {
    DefinitionDigest::for_canonical_value(domain, &serde_json::to_value(value).map_err(invalid)?)
        .map_err(invalid)
}
pub(crate) fn versioned(value: &ClockStamp) -> bool {
    matches!(value.clock, ClockRef::WorldIc {generation} if generation!=0)
}
pub(crate) fn action(value: &DefinitionRef, id: &str) -> bool {
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
pub(crate) fn decode<T: serde::de::DeserializeOwned + Serialize>(
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

pub(crate) fn present_holder<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<RenewalHolderBasisV1>, D::Error> {
    RenewalHolderBasisV1::deserialize(d).map(Some)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RenewalAttemptV1 {
    Automatic {},
    BuyerRetry {
        previous_operation: CommitToken,
    },
    HolderAutomatic {
        basis: RenewalHolderBasisV1,
    },
    HolderRetry {
        basis: RenewalHolderBasisV1,
        previous_operation: CommitToken,
    },
}

impl RenewalAttemptV1 {
    pub const fn holder_basis(self) -> Option<RenewalHolderBasisV1> {
        match self {
            Self::HolderAutomatic { basis } | Self::HolderRetry { basis, .. } => Some(basis),
            _ => None,
        }
    }
    pub const fn previous_operation(self) -> Option<CommitToken> {
        match self {
            Self::BuyerRetry { previous_operation }
            | Self::HolderRetry {
                previous_operation, ..
            } => Some(previous_operation),
            _ => None,
        }
    }
    pub const fn record_format(self) -> u16 {
        match self {
            Self::Automatic {} => 1,
            Self::BuyerRetry { .. } => 2,
            Self::HolderAutomatic { .. } => 3,
            Self::HolderRetry { .. } => 4,
        }
    }

    /// Pure closed data mapping. Only an actual opaque E4 output/audit view
    /// supplies the attempt used at the publication and recovery boundaries.
    pub(crate) fn from_economy(value: mush_economy::renewal_twin::RenewalAttemptV1) -> Self {
        match value {
            mush_economy::renewal_twin::RenewalAttemptV1::Automatic => Self::Automatic {},
            mush_economy::renewal_twin::RenewalAttemptV1::HolderAutomatic { basis } => {
                Self::HolderAutomatic { basis }
            }
            mush_economy::renewal_twin::RenewalAttemptV1::HolderRetry {
                basis,
                previous_operation,
            } => Self::HolderRetry {
                basis,
                previous_operation,
            },
            mush_economy::renewal_twin::RenewalAttemptV1::BuyerRetry { previous_operation } => {
                Self::BuyerRetry { previous_operation }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    deny_unknown_fields,
    bound = ""
)]
pub enum RenewalStateV1<I: Issuer> {
    Drafted {},
    Funded {
        funding: RenewalFundingReceiptV1<I>,
    },
    Granted {
        funding: RenewalFundingReceiptV1<I>,
        terminal: RenewalTerminalReceiptV1<I>,
    },
    Refunded {
        funding: RenewalFundingReceiptV1<I>,
        terminal: RenewalTerminalReceiptV1<I>,
    },
    UnfundedClosed {
        closure: I::Closure,
    },
    UnfundedServiceClosed {
        closure: RenewalServiceClosureV1<I>,
    },
}
impl<I: Issuer> RenewalStateV1<I> {
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
    pub fn funding(&self) -> Option<&RenewalFundingReceiptV1<I>> {
        match self {
            Self::Funded { funding }
            | Self::Granted { funding, .. }
            | Self::Refunded { funding, .. } => Some(funding),
            _ => None,
        }
    }
    pub fn terminal(&self) -> Option<&RenewalTerminalReceiptV1<I>> {
        match self {
            Self::Granted { terminal, .. } | Self::Refunded { terminal, .. } => Some(terminal),
            _ => None,
        }
    }
    pub fn terminal_commit(&self) -> Option<CommitToken> {
        match self {
            Self::Granted { terminal, .. } | Self::Refunded { terminal, .. } => {
                Some(terminal.commit())
            }
            Self::UnfundedClosed { closure } => Some(super::Closure::commit(closure)),
            Self::UnfundedServiceClosed { closure } => Some(closure.commit()),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(bound = "")]
pub struct RenewalHeadV1<I: Issuer> {
    pub(crate) format: u16,
    pub(crate) licence: LicenceId,
    pub(crate) holder: Holder,
    pub(crate) birth_operation: CommitToken,
    pub(crate) current: TermGeneration,
    pub(crate) attempted_predecessor: TermGeneration,
    pub(crate) last_operation: CommitToken,
    pub(crate) pending: Option<CommitToken>,
    /// Which issuer's head this is. Not written: the family is the head's state family.
    #[serde(skip)]
    pub(crate) issuer: PhantomData<I>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HeadWire {
    format: u16,
    licence: LicenceId,
    holder: Holder,
    birth_operation: CommitToken,
    current: TermGeneration,
    attempted_predecessor: TermGeneration,
    last_operation: CommitToken,
    #[serde(deserialize_with = "required_option")]
    pending: Option<CommitToken>,
}
impl<'de, I: Issuer> Deserialize<'de> for RenewalHeadV1<I> {
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
            issuer: PhantomData,
        };
        value.validate().map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}
impl<I: Issuer> RenewalHeadV1<I> {
    pub fn validate(&self) -> Result<(), PublicationError> {
        <Self as Record>::validate(self)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, PublicationError> {
        decode(bytes, Self::MAX_BYTES)
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
#[expect(
    private_bounds,
    reason = "doc 46 CT.5: the issuer's rules are private to the crate, as the records they name are"
)]
impl<I: super::Rules> RenewalHeadV1<I> {
    pub(crate) fn for_operation(row: &I::Operation) -> Result<Self, PublicationError> {
        use super::{Binding, Operation};
        row.validate()?;
        let b = row.binding();
        let value = Self {
            format: 1,
            licence: b.licence(),
            holder: b.holder(),
            birth_operation: b.birth_operation(),
            current: if row.state().granted() {
                b.next()
            } else {
                b.predecessor()
            },
            attempted_predecessor: b.predecessor(),
            last_operation: row.operation(),
            pending: row.state().pending().then_some(row.operation()),
            issuer: PhantomData,
        };
        value.validate()?;
        Ok(value)
    }
}
impl<I: Issuer> Record for RenewalHeadV1<I> {
    const FAMILY: &'static str = I::HEAD_FAMILY;
    const MAX_BYTES: usize = MAX_HEAD_BYTES;
    fn key(&self) -> String {
        self.licence.key()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        if self.format != 1
            || self.holder.is_nobody()
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

/// The methods a family's `RenewalBindingV1` has, one text for both. `extra` is what the
/// binding records beside the shared fields (the House's issuer), taken by `capture_holder`
/// after the economics.
macro_rules! binding_records {
    (
        issuer: $issuer:ty,
        economics: $economics:ty,
        grant: $grant:ty,
        extra: { $($extra:ident : $extra_ty:ty),* $(,)? } $(,)?
    ) => {
        impl RenewalBindingV1 {
            /// The renewal's price in minor units (doc 17h §3 M8f P2a: the renewal
            /// steps that move money ask an organisation's grant with it).
            #[must_use]
            pub fn price_minor(&self) -> i64 {
                self.grant.renewal().price().minor()
            }
            pub fn validate(&self) -> Result<(), $crate::commerce::records::PublicationError> {
                use $crate::commerce::records::PublicationError;
                self.grant.validate()?;
                let template = self.grant.renewal();
                if let Some(basis) = self.holder_basis {
                    basis
                        .validate()
                        .map_err($crate::commerce::renewal_twin::records::invalid)?;
                }
                if !matches!((self.format, self.holder_basis), (1, None) | (2, Some(_)))
                    || self.holder.is_nobody()
                    || <$issuer as $crate::commerce::renewal_twin::Rules>::seats_differ(
                        self.terms,
                        &self.grant,
                    )?
                    || self.predecessor.get().checked_add(1) != Some(self.next.get())
                    || !(1..=$crate::commerce::MAX_TERM_SECONDS).contains(&self.term_seconds)
                    || !(0..=$crate::commerce::MAX_GRACE_SECONDS).contains(&self.grace_seconds)
                    || template.terms() != self.terms
                    || template.revision() != self.policy()
                    || template.term_seconds() != self.term_seconds
                    || template.grace_seconds() != self.grace_seconds
                {
                    return Err(PublicationError::Mismatch);
                }
                $crate::commerce::renewal_twin::records::bounded(
                    self,
                    $crate::commerce::renewal_twin::records::MAX_OPERATION_BYTES,
                )
            }
            #[cfg(test)]
            pub(crate) fn capture(
                licence: &$crate::commerce::paid::PersonalPaidLicence,
                term: &$crate::commerce::paid::PaidTermRecord,
                economics: &$economics,
                $($extra: $extra_ty,)*
            ) -> Result<Self, $crate::commerce::records::PublicationError> {
                Self::capture_holder(licence, term, economics, $($extra,)* None, licence.holder())
            }
            /// M8c F4: `holder` is the pointer's holder as THIS renewal records it:
            /// a character unchanged, an organisation re-stamped with the renewing
            /// actor as its acting character (`org_holder::acting_as`); a different
            /// principal is `Mismatch`.
            pub(crate) fn capture_holder(
                licence: &$crate::commerce::paid::PersonalPaidLicence,
                term: &$crate::commerce::paid::PaidTermRecord,
                economics: &$economics,
                $($extra: $extra_ty,)*
                holder_basis: Option<mush_economy::managed_paid_resale::RenewalHolderBasisV1>,
                holder: $crate::commerce::Holder,
            ) -> Result<Self, $crate::commerce::records::PublicationError> {
                use $crate::commerce::records::PublicationError;
                term.matches_current(licence)?;
                economics.validate()?;
                if !holder.same_principal(licence.holder()) {
                    return Err(PublicationError::Mismatch);
                }
                let $crate::commerce::records::RevisionPolicy::Pin { filed, revision } =
                    term.policy()
                else {
                    return Err(PublicationError::Mismatch);
                };
                let template = economics.grant().renewal();
                let row = Self {
                    format: if holder_basis.is_some() { 2 } else { 1 },
                    holder_basis,
                    licence: licence.id(),
                    holder,
                    $($extra,)*
                    charter: licence.charter(),
                    line: licence.line(),
                    birth_operation: licence.operation(),
                    predecessor: term.generation(),
                    next: $crate::commerce::TermGeneration::new(
                        term.generation()
                            .get()
                            .checked_add(1)
                            .ok_or(PublicationError::Capacity)?,
                    )
                    .map_err($crate::commerce::renewal_twin::records::invalid)?,
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
            pub(crate) fn term_digest(
                term: &$crate::commerce::paid::PaidTermRecord,
            ) -> Result<mush_core::DefinitionDigest, $crate::commerce::records::PublicationError>
            {
                term.validate()?;
                $crate::commerce::renewal_twin::records::digest(
                    <$issuer as $crate::commerce::renewal_twin::Issuer>::PREDECESSOR_DOMAIN,
                    term,
                )
            }
            pub fn template_digest(
                &self,
            ) -> Result<mush_core::DefinitionDigest, $crate::commerce::records::PublicationError>
            {
                self.validate()?;
                $crate::commerce::renewal_twin::records::digest(
                    <$issuer as $crate::commerce::renewal_twin::Issuer>::TEMPLATE_DOMAIN,
                    self.grant.renewal(),
                )
            }
            /// Immutable predecessor proof: deliberately does not compare the live
            /// licence term pointer. Current admission separately calls matches_current.
            pub(crate) fn check_predecessor(
                &self,
                licence: &$crate::commerce::paid::PersonalPaidLicence,
                term: &$crate::commerce::paid::PaidTermRecord,
                economics: &$economics,
            ) -> Result<(), $crate::commerce::records::PublicationError> {
                use $crate::commerce::records::PublicationError;
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
            pub(crate) fn matches_birth(
                &self,
                licence: &$crate::commerce::paid::PersonalPaidLicence,
            ) -> bool {
                licence.id() == self.licence
                    // M8c F4: by principal (the binding names the renewing actor).
                    && licence.holder().same_principal(self.holder)
                    && licence.charter() == self.charter
                    && licence.line() == self.line
                    && licence.operation() == self.birth_operation
                    && licence.ownership().copied() == self.holder_basis.map(|b| b.ownership)
            }
            pub const fn holder_basis(
                &self,
            ) -> Option<mush_economy::managed_paid_resale::RenewalHolderBasisV1> {
                self.holder_basis
            }
            /// Historical data projection, never a current owner observation.
            pub(crate) fn accepted_pointer(
                &self,
                generation: $crate::commerce::TermGeneration,
            ) -> Result<
                $crate::commerce::paid::PersonalPaidLicence,
                $crate::commerce::records::PublicationError,
            > {
                $crate::commerce::paid::PersonalPaidLicence::original(
                    self.licence,
                    self.charter,
                    self.line,
                    self.holder,
                    generation,
                    self.birth_operation,
                )?
                .with_captured_ownership(self.holder_basis.map(|b| b.ownership))
            }
            pub const fn charter(&self) -> $crate::commerce::CharterId {
                self.charter
            }
            pub const fn line(&self) -> $crate::commerce::LineId {
                self.line
            }
            pub const fn filed(&self) -> $crate::commerce::FiledRevisionId {
                self.filed
            }
            pub const fn revision(&self) -> $crate::commerce::RevisionIdentity {
                self.revision
            }
            pub const fn terms(&self) -> $crate::commerce::paid::PaidTerms {
                self.terms
            }
            pub const fn term_seconds(&self) -> i64 {
                self.term_seconds
            }
            pub const fn grace_seconds(&self) -> i64 {
                self.grace_seconds
            }
            pub const fn predecessor_term(&self) -> mush_core::DefinitionDigest {
                self.predecessor_term
            }
            pub const fn predecessor_economics(&self) -> mush_core::DefinitionDigest {
                self.predecessor_economics
            }
            pub const fn licence(&self) -> $crate::commerce::LicenceId {
                self.licence
            }
            pub const fn holder(&self) -> $crate::commerce::Holder {
                self.holder
            }
            pub const fn predecessor(&self) -> $crate::commerce::TermGeneration {
                self.predecessor
            }
            pub const fn next(&self) -> $crate::commerce::TermGeneration {
                self.next
            }
            pub const fn birth_operation(&self) -> mush_core::CommitToken {
                self.birth_operation
            }
            pub const fn predecessor_operation(&self) -> mush_core::CommitToken {
                self.predecessor_operation
            }
            pub const fn policy(&self) -> $crate::commerce::records::RevisionPolicy {
                $crate::commerce::records::RevisionPolicy::Pin {
                    filed: self.filed,
                    revision: self.revision,
                }
            }
            pub fn grant(&self) -> &$grant {
                &self.grant
            }
            pub const fn product_digest(&self) -> mush_core::DefinitionDigest {
                self.product_digest
            }
        }
        impl $crate::commerce::renewal_twin::Binding<$issuer> for RenewalBindingV1 {
            fn licence(&self) -> $crate::commerce::LicenceId {
                self.licence
            }
            fn holder(&self) -> $crate::commerce::Holder {
                self.holder
            }
            fn charter(&self) -> $crate::commerce::CharterId {
                self.charter
            }
            fn line(&self) -> $crate::commerce::LineId {
                self.line
            }
            fn birth_operation(&self) -> mush_core::CommitToken {
                self.birth_operation
            }
            fn predecessor(&self) -> $crate::commerce::TermGeneration {
                self.predecessor
            }
            fn next(&self) -> $crate::commerce::TermGeneration {
                self.next
            }
            fn predecessor_operation(&self) -> mush_core::CommitToken {
                self.predecessor_operation
            }
            fn revision(&self) -> $crate::commerce::RevisionIdentity {
                self.revision
            }
            fn terms(&self) -> $crate::commerce::paid::PaidTerms {
                self.terms
            }
            fn term_seconds(&self) -> i64 {
                self.term_seconds
            }
            fn grace_seconds(&self) -> i64 {
                self.grace_seconds
            }
            fn product_digest(&self) -> mush_core::DefinitionDigest {
                self.product_digest
            }
            fn holder_basis(
                &self,
            ) -> Option<mush_economy::managed_paid_resale::RenewalHolderBasisV1> {
                self.holder_basis
            }
            fn grant(&self) -> &$grant {
                &self.grant
            }
            fn price_minor(&self) -> i64 {
                RenewalBindingV1::price_minor(self)
            }
            fn validate(&self) -> Result<(), $crate::commerce::records::PublicationError> {
                RenewalBindingV1::validate(self)
            }
            fn template_digest(
                &self,
            ) -> Result<mush_core::DefinitionDigest, $crate::commerce::records::PublicationError>
            {
                RenewalBindingV1::template_digest(self)
            }
            fn grant_digest(
                &self,
            ) -> Result<mush_core::DefinitionDigest, $crate::commerce::records::PublicationError>
            {
                self.grant.digest()
            }
            fn template_instrument(&self) -> &mush_economy::InstrumentId {
                self.grant.renewal().instrument()
            }
            fn template_price(&self) -> mush_economy::Amount {
                self.grant.renewal().price()
            }
            fn matches_birth(&self, licence: &$crate::commerce::paid::PersonalPaidLicence) -> bool {
                RenewalBindingV1::matches_birth(self, licence)
            }
            fn accepted_pointer(
                &self,
                generation: $crate::commerce::TermGeneration,
            ) -> Result<
                $crate::commerce::paid::PersonalPaidLicence,
                $crate::commerce::records::PublicationError,
            > {
                RenewalBindingV1::accepted_pointer(self, generation)
            }
            fn check_predecessor(
                &self,
                licence: &$crate::commerce::paid::PersonalPaidLicence,
                term: &$crate::commerce::paid::PaidTermRecord,
                economics: &$economics,
            ) -> Result<(), $crate::commerce::records::PublicationError> {
                RenewalBindingV1::check_predecessor(self, licence, term, economics)
            }
            fn economics_at(
                &self,
                generation: $crate::commerce::TermGeneration,
                operation: mush_core::CommitToken,
            ) -> Result<$economics, $crate::commerce::records::PublicationError> {
                <$economics>::capture(
                    self.licence,
                    generation,
                    operation,
                    self.product_digest,
                    self.grant.clone(),
                )
            }
        }
    };
}
pub(crate) use binding_records;

/// A family's `RenewalClosureV1` and `RenewalOperationV1`, one text for both: the two
/// `checked!` records, their methods and the record traits over them. Expanded in each
/// family's `records.rs`, which holds its `RenewalBindingV1` and the `RenewalStateV1` alias
/// these name.
macro_rules! operation_records {
    ($issuer:ty) => {
        mush_core::checked!(pub struct RenewalClosureV1, ClosureWire, pub(crate) fields, {
            commit:mush_core::CommitToken,action:mush_core::DefinitionRef,at_ic:mush_core::ClockStamp,posted_at_utc:i64,reason:mush_economy::renewal_twin::UnfundedCloseReason,
        });
        impl RenewalClosureV1 {
            pub fn validate(&self) -> Result<(), $crate::commerce::records::PublicationError> {
                if !$crate::commerce::renewal_twin::records::action(
                    &self.action,
                    <$issuer as mush_economy::renewal_twin::Issuer>::CLOSE_ACTION,
                ) || !$crate::commerce::renewal_twin::records::versioned(&self.at_ic)
                {
                    return Err($crate::commerce::records::PublicationError::Invalid);
                }
                $crate::commerce::renewal_twin::records::bounded(
                    self,
                    mush_economy::renewal_twin::MAX_RENEWAL_RECEIPT_BYTES,
                )
            }
            pub const fn commit(&self) -> mush_core::CommitToken {
                self.commit
            }
            pub fn action(&self) -> &mush_core::DefinitionRef {
                &self.action
            }
            pub fn at_ic(&self) -> &mush_core::ClockStamp {
                &self.at_ic
            }
            pub const fn posted_at_utc(&self) -> i64 {
                self.posted_at_utc
            }
            pub const fn reason(&self) -> mush_economy::renewal_twin::UnfundedCloseReason {
                self.reason
            }
        }
        impl $crate::commerce::renewal_twin::Closure for RenewalClosureV1 {
            fn commit(&self) -> mush_core::CommitToken {
                self.commit
            }
            fn action(&self) -> &mush_core::DefinitionRef {
                &self.action
            }
            fn at_ic(&self) -> &mush_core::ClockStamp {
                &self.at_ic
            }
            fn posted_at_utc(&self) -> i64 {
                self.posted_at_utc
            }
            fn reason(&self) -> mush_economy::renewal_twin::UnfundedCloseReason {
                self.reason
            }
            fn new(
                commit: mush_core::CommitToken,
                action: mush_core::DefinitionRef,
                at_ic: mush_core::ClockStamp,
                posted_at_utc: i64,
                reason: mush_economy::renewal_twin::UnfundedCloseReason,
            ) -> Self {
                Self {
                    commit,
                    action,
                    at_ic,
                    posted_at_utc,
                    reason,
                }
            }
        }

        mush_core::checked!(pub struct RenewalOperationV1, OperationWire, pub(crate) fields, {
            format:u16,operation:mush_core::CommitToken,action:mush_core::DefinitionRef,attempt:RenewalAttemptV1,
            binding:Box<RenewalBindingV1>,offer_digest:mush_core::DefinitionDigest,contract:mush_economy::ContractId,consent_hash:[u8;32],
            created_at_ic:mush_core::ClockStamp,created_at_utc:i64,state:RenewalStateV1,
        });
        impl RenewalOperationV1 {
            pub fn validate(&self) -> Result<(), $crate::commerce::records::PublicationError> {
                <Self as $crate::commerce::records::Record>::validate(self)
            }
            /// Read-only data relation. This cannot certify current eligibility or
            /// replace the actual Economy owner's replayed history.
            pub fn matches_offer(
                &self,
                offer: &<$issuer as mush_economy::renewal_twin::Issuer>::Offer,
            ) -> Result<(), $crate::commerce::records::PublicationError> {
                $crate::commerce::renewal_twin::history::check_offer::<$issuer>(self, offer)
            }
            /// Joins this record to a borrowed view minted by the actual Economy
            /// audit. There is no decoded or caller-constructed history alternative.
            pub fn verify_history(
                &self,
                actual: &mush_economy::renewal_twin::RenewalOfferView<'_, $issuer>,
            ) -> Result<(), $crate::commerce::records::PublicationError> {
                $crate::commerce::renewal_twin::history::verify::<$issuer>(self, actual)
            }
            pub fn decode(bytes: &[u8]) -> Result<Self, $crate::commerce::records::PublicationError> {
                $crate::commerce::renewal_twin::records::decode(
                    bytes,
                    <Self as $crate::commerce::records::Record>::MAX_BYTES,
                )
            }
            pub const fn operation(&self) -> mush_core::CommitToken {
                self.operation
            }
            pub fn action(&self) -> &mush_core::DefinitionRef {
                &self.action
            }
            pub const fn attempt(&self) -> RenewalAttemptV1 {
                self.attempt
            }
            pub fn binding(&self) -> &RenewalBindingV1 {
                &self.binding
            }
            pub const fn offer_digest(&self) -> mush_core::DefinitionDigest {
                self.offer_digest
            }
            pub const fn contract(&self) -> mush_economy::ContractId {
                self.contract
            }
            pub const fn consent_hash(&self) -> &[u8; 32] {
                &self.consent_hash
            }
            pub fn created_at_ic(&self) -> &mush_core::ClockStamp {
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
            pub(crate) fn follows(
                &self,
                old: &Self,
            ) -> Result<(), $crate::commerce::records::PublicationError> {
                use $crate::commerce::records::PublicationError;
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
            ) -> Result<
                (
                    $crate::commerce::paid::PersonalPaidLicence,
                    $crate::commerce::paid::PaidTermRecord,
                    <$issuer as $crate::commerce::renewal_twin::Issuer>::Economics,
                ),
                $crate::commerce::records::PublicationError,
            > {
                use $crate::commerce::records::PublicationError;
                use $crate::commerce::renewal_twin::records::invalid;
                self.validate()?;
                let RenewalStateV1::Granted { terminal, .. } = &self.state else {
                    return Err(PublicationError::Mismatch);
                };
                let stamp = terminal.issued_at_ic().ok_or(PublicationError::Missing)?;
                let issue = $crate::commerce::WorldIcSeconds::from_stamp(stamp).map_err(invalid)?;
                let end = stamp
                    .at
                    .checked_add(self.binding.term_seconds)
                    .ok_or(PublicationError::Capacity)?;
                let grace = end
                    .checked_add(self.binding.grace_seconds)
                    .ok_or(PublicationError::Capacity)?;
                let end = $crate::commerce::WorldIcSeconds::from_stamp(&mush_core::ClockStamp {
                    clock: stamp.clock.clone(),
                    at: end,
                })
                .map_err(invalid)?;
                let grace = $crate::commerce::WorldIcSeconds::from_stamp(&mush_core::ClockStamp {
                    clock: stamp.clock.clone(),
                    at: grace,
                })
                .map_err(invalid)?;
                let b = &self.binding;
                Ok((
                    b.accepted_pointer(b.next)?,
                    $crate::commerce::paid::PaidTermRecord::original(
                        b.licence,
                        b.next,
                        $crate::commerce::TermWindow::new(issue, issue, end, grace)
                            .map_err(invalid)?,
                        b.terms,
                        b.policy(),
                        self.operation,
                    )?,
                    <RenewalBindingV1 as $crate::commerce::renewal_twin::Binding<$issuer>>::economics_at(
                        b,
                        b.next,
                        self.operation,
                    )?,
                ))
            }
        }
        impl $crate::commerce::records::Record for RenewalOperationV1 {
            const FAMILY: &'static str =
                <$issuer as $crate::commerce::renewal_twin::Issuer>::OPERATION_FAMILY;
            const MAX_BYTES: usize = $crate::commerce::renewal_twin::records::MAX_OPERATION_BYTES;
            fn key(&self) -> String {
                self.operation.to_hex()
            }
            fn validate(&self) -> Result<(), $crate::commerce::records::PublicationError> {
                use $crate::commerce::records::PublicationError;
                self.binding.validate()?;
                if self.attempt.holder_basis() != self.binding.holder_basis {
                    return Err(PublicationError::Mismatch);
                }
                if self.format != 1
                    || !$crate::commerce::renewal_twin::records::action(
                        &self.action,
                        <$issuer as mush_economy::renewal_twin::Issuer>::DRAFT_ACTION,
                    )
                    || !$crate::commerce::renewal_twin::records::versioned(&self.created_at_ic)
                    || self.operation == self.binding.birth_operation
                    || self.operation == self.binding.predecessor_operation
                    || self.attempt.previous_operation() == Some(self.operation)
                {
                    return Err(PublicationError::Invalid);
                }
                let distinct = |commit: mush_core::CommitToken| {
                    commit != self.operation
                        && commit != self.binding.predecessor_operation
                        && commit != self.binding.birth_operation
                };
                if let Some(funding) = self.state.funding()
                    && (!distinct(funding.commit())
                        || funding.contract() != self.contract
                        || funding.consent_hash() != &self.consent_hash
                        || funding.at_ic().clock != self.created_at_ic.clock
                        || funding.at_ic().at < self.created_at_ic.at
                        || funding.posted_at_utc() < self.created_at_utc)
                {
                    return Err(PublicationError::Mismatch);
                }
                if let Some(terminal) = self.state.terminal() {
                    let funding = self.state.funding().ok_or(PublicationError::Missing)?;
                    let grant = self.state.granted();
                    if !distinct(terminal.commit())
                        || terminal.commit() == funding.commit()
                        || terminal.transaction() == funding.transaction()
                        || terminal.journal_seq() <= funding.journal_seq()
                        || terminal.posted_at_utc() < funding.posted_at_utc()
                        || (terminal.outcome() == mush_economy::renewal_twin::RenewalOutcome::Granted)
                            != grant
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
                    closure
                        .validate()
                        .map_err($crate::commerce::renewal_twin::records::invalid)?;
                    if !distinct(closure.commit())
                        || closure.posted_at_utc() < self.created_at_utc
                        || self.attempt.previous_operation() == Some(closure.commit())
                        || match closure.reason() {
                            mush_economy::renewal_twin::RenewalServiceCloseReason::DeadlineReached => {
                                closure.at_ic().clock != self.created_at_ic.clock
                                    || closure.at_ic().at < self.created_at_ic.at
                            }
                            mush_economy::renewal_twin::RenewalServiceCloseReason::ClockInvalidated => {
                                closure.at_ic().clock == self.created_at_ic.clock
                            }
                        }
                    {
                        return Err(PublicationError::Mismatch);
                    }
                }
                $crate::commerce::renewal_twin::records::bounded(
                    self,
                    <Self as $crate::commerce::records::Record>::MAX_BYTES,
                )
            }
        }
        impl $crate::commerce::renewal_twin::Operation<$issuer> for RenewalOperationV1 {
            fn operation(&self) -> mush_core::CommitToken {
                self.operation
            }
            fn action(&self) -> &mush_core::DefinitionRef {
                &self.action
            }
            fn attempt(&self) -> RenewalAttemptV1 {
                self.attempt
            }
            fn binding(&self) -> &RenewalBindingV1 {
                &self.binding
            }
            fn offer_digest(&self) -> mush_core::DefinitionDigest {
                self.offer_digest
            }
            fn contract(&self) -> mush_economy::ContractId {
                self.contract
            }
            fn consent_hash(&self) -> &[u8; 32] {
                &self.consent_hash
            }
            fn created_at_ic(&self) -> &mush_core::ClockStamp {
                &self.created_at_ic
            }
            fn created_at_utc(&self) -> i64 {
                self.created_at_utc
            }
            fn state(&self) -> &RenewalStateV1 {
                &self.state
            }
            fn follows(&self, old: &Self) -> Result<(), $crate::commerce::records::PublicationError> {
                RenewalOperationV1::follows(self, old)
            }
            fn granted_rows(
                &self,
            ) -> Result<
                (
                    $crate::commerce::paid::PersonalPaidLicence,
                    $crate::commerce::paid::PaidTermRecord,
                    <$issuer as $crate::commerce::renewal_twin::Issuer>::Economics,
                ),
                $crate::commerce::records::PublicationError,
            > {
                RenewalOperationV1::granted_rows(self)
            }
            fn with_state(&self, state: RenewalStateV1) -> Self {
                Self {
                    state,
                    ..self.clone()
                }
            }
            fn drafted(
                operation: mush_core::CommitToken,
                action: mush_core::DefinitionRef,
                attempt: RenewalAttemptV1,
                binding: RenewalBindingV1,
                offer_digest: mush_core::DefinitionDigest,
                contract: mush_economy::ContractId,
                consent_hash: [u8; 32],
                created_at_ic: mush_core::ClockStamp,
                created_at_utc: i64,
            ) -> Self {
                Self {
                    format: 1,
                    operation,
                    action,
                    attempt,
                    binding: Box::new(binding),
                    offer_digest,
                    contract,
                    consent_hash,
                    created_at_ic,
                    created_at_utc,
                    state: RenewalStateV1::Drafted {},
                }
            }
        }
    };
}
pub(crate) use operation_records;
