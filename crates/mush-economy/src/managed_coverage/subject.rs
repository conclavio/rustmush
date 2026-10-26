//! Closed coverage consent data. This is not a host permit, published product,
//! premium authorization, funding reservation or mechanical loss proof.
use super::{MAX_SUBJECT_BYTES, refused};
use crate::{AccountId, Amount, InstrumentId};
use mush_core::participant::ParticipantError;
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use mush_org::{OrgId, OrgScopeId};
use serde::{Deserialize, Serialize};

macro_rules! record {
    ($name:ident,$wire:ident,{$($(#[$attr:meta])* $field:ident:$ty:ty),* $(,)?}) => {
        #[derive(Clone, Debug, PartialEq, Eq, Serialize)]
        pub struct $name { $(pub $field:$ty),* }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct $wire { $($(#[$attr])* $field:$ty),* }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D:serde::Deserializer<'de>>(d:D)->Result<Self,D::Error> {
                let w=$wire::deserialize(d)?;
                let value=Self {$($field:w.$field),*};
                value.validate().map_err(serde::de::Error::custom)?;
                Ok(value)
            }
        }
    }
}

fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}

record!(CoverageBasisV1, BasisWire, {
    format:u16, product:CommitToken, product_digest:DefinitionDigest,
    quote:CommitToken, quote_digest:DefinitionDigest,
    ic_open:CommitToken, seal_digest:DefinitionDigest, admission_digest:DefinitionDigest,
    departure_digest:DefinitionDigest,
    hull:ObjId, hull_title_revision:u64, hull_revision:u64,
    hull_schema:DefinitionRef, zone:DefinitionRef,
    #[serde(deserialize_with="required_option")]
    qualified_role:Option<DefinitionDigest>,
});
impl CoverageBasisV1 {
    pub fn validate(&self) -> Result<(), ParticipantError> {
        if self.format != 1
            || self.product == self.quote
            || self.ic_open == self.product
            || self.ic_open == self.quote
            || self.hull.0 == 0
            || self.hull_title_revision == 0
            || [
                self.product_digest,
                self.quote_digest,
                self.seal_digest,
                self.admission_digest,
                self.departure_digest,
                self.hull_schema.digest(),
                self.zone.digest(),
            ]
            .iter()
            .any(|digest| digest.as_bytes() == &[0; 32])
            || self
                .qualified_role
                .is_some_and(|digest| digest.as_bytes() == &[0; 32])
        {
            return Err(refused());
        }
        Ok(())
    }
}

record!(CoverageMoneyTermsV1, TermsWire, {
    format:u16, buyer:ObjId, insurer:OrgId, scope:OrgScopeId, treasury:AccountId,
    payer:AccountId, instrument:InstrumentId, adjudicator:ObjId,
    insured_value:Amount, base_premium:Amount, discount:Amount, premium:Amount,
    liability:Amount, starts_at:ClockStamp, ends_at:ClockStamp,
    claim_delay_seconds:i64, quoted_at_utc:i64, expires_at_utc:i64,
});
impl CoverageMoneyTermsV1 {
    pub fn validate(&self) -> Result<(), ParticipantError> {
        if self.format != 1
            || self.buyer.0 == 0
            || self.adjudicator.0 == 0
            || self.payer == self.treasury
            || self.instrument.as_str().len() > 128
            || self.insured_value.minor() <= 0
            || self.base_premium.minor() <= 0
            || self.base_premium > self.insured_value
            || self.discount.minor() < 0
            || self.premium.minor() <= 0
            || self.premium.checked_add(self.discount) != Some(self.base_premium)
            || self.liability.minor() <= 0
            || self.liability > self.insured_value
            || !matches!(self.starts_at.clock,ClockRef::WorldIc{generation} if generation != 0)
            || self.starts_at.clock != self.ends_at.clock
            || self
                .ends_at
                .at
                .checked_sub(self.starts_at.at)
                .is_none_or(|n| !(1..=90 * 86_400).contains(&n))
            || !(0..=30 * 86_400).contains(&self.claim_delay_seconds)
            || self
                .expires_at_utc
                .checked_sub(self.quoted_at_utc)
                .is_none_or(|n| !(1..=3600).contains(&n))
        {
            return Err(refused());
        }
        // Org/scope/account IDs deliberately have no invented nonzero test:
        // the actual authoritative allocators start those domains at zero.
        Ok(())
    }
}

record!(CoverageQuoteTermsV1, QuoteTermsWire, {
    format:u16, buyer:ObjId, insurer:OrgId, scope:OrgScopeId, treasury:AccountId,
    payer:AccountId, instrument:InstrumentId, adjudicator:ObjId,
    insured_value:Amount, base_premium:Amount, discount:Amount, premium:Amount,
    liability:Amount, duration_seconds:i64, claim_delay_seconds:i64,
    quoted_at_utc:i64, expires_at_utc:i64,
});
impl CoverageQuoteTermsV1 {
    pub fn validate(&self) -> Result<(), ParticipantError> {
        if self.format != 1
            || self.buyer.0 == 0
            || self.adjudicator.0 == 0
            || self.payer == self.treasury
            || self.instrument.as_str().len() > 128
            || self.insured_value.minor() <= 0
            || self.base_premium.minor() <= 0
            || self.base_premium > self.insured_value
            || self.discount.minor() < 0
            || self.premium.minor() <= 0
            || self.premium.checked_add(self.discount) != Some(self.base_premium)
            || self.liability.minor() <= 0
            || self.liability > self.insured_value
            || !(1..=90 * 86_400).contains(&self.duration_seconds)
            || !(0..=30 * 86_400).contains(&self.claim_delay_seconds)
            || self
                .expires_at_utc
                .checked_sub(self.quoted_at_utc)
                .is_none_or(|n| !(1..=3600).contains(&n))
        {
            return Err(refused());
        }
        Ok(())
    }
    /// Pure window arithmetic, not a binding capability. Actual native Bind
    /// supplies its current WorldIC stamp; no quote can choose or backdate it.
    pub fn bind_window(&self, start: ClockStamp) -> Result<CoverageMoneyTermsV1, ParticipantError> {
        self.validate()?;
        let end = ClockStamp {
            clock: start.clock.clone(),
            at: start
                .at
                .checked_add(self.duration_seconds)
                .ok_or_else(refused)?,
        };
        let terms = CoverageMoneyTermsV1 {
            format: 1,
            buyer: self.buyer,
            insurer: self.insurer,
            scope: self.scope,
            treasury: self.treasury,
            payer: self.payer,
            instrument: self.instrument.clone(),
            adjudicator: self.adjudicator,
            insured_value: self.insured_value,
            base_premium: self.base_premium,
            discount: self.discount,
            premium: self.premium,
            liability: self.liability,
            starts_at: start,
            ends_at: end,
            claim_delay_seconds: self.claim_delay_seconds,
            quoted_at_utc: self.quoted_at_utc,
            expires_at_utc: self.expires_at_utc,
        };
        terms.validate()?;
        Ok(terms)
    }
}

record!(CoverageConsentV1, ConsentWire, {
    format:u16, operation:CommitToken, effect:DefinitionRef, accepted_at_utc:i64,
    basis:CoverageBasisV1, terms:CoverageMoneyTermsV1,
});
impl CoverageConsentV1 {
    pub fn validate(&self) -> Result<(), ParticipantError> {
        if self.format != 1
            || !super::action_matches(&self.effect, super::BIND_ACTION)
            || [self.basis.product, self.basis.quote, self.basis.ic_open].contains(&self.operation)
            || self.accepted_at_utc < self.terms.quoted_at_utc
            || self.accepted_at_utc >= self.terms.expires_at_utc
            || self.basis.hull == self.terms.buyer
            || (self.basis.qualified_role.is_none() && self.terms.discount != Amount::ZERO)
        {
            return Err(refused());
        }
        self.basis.validate()?;
        self.terms.validate()?;
        mush_core::canonical_json_bytes_with_limit(
            &serde_json::to_value(self).map_err(|_| ParticipantError::Encoding)?,
            MAX_SUBJECT_BYTES,
        )
        .map_err(|_| refused())?;
        Ok(())
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, ParticipantError> {
        if bytes.is_empty() || bytes.len() > MAX_SUBJECT_BYTES {
            return Err(refused());
        }
        let subject: Self = serde_json::from_slice(bytes).map_err(|_| refused())?;
        let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| refused())?;
        if serde_json::to_value(&subject).map_err(|_| ParticipantError::Encoding)? != value {
            return Err(refused());
        }
        Ok(subject)
    }
    pub fn digest(&self) -> Result<DefinitionDigest, ParticipantError> {
        self.validate()?;
        DefinitionDigest::for_canonical_value_with_limit(
            "economy.coverage.consent.v1",
            &serde_json::to_value(self).map_err(|_| ParticipantError::Encoding)?,
            MAX_SUBJECT_BYTES,
        )
        .map_err(|_| refused())
    }
}
