//! Strict source-free publication and quote data. These rows cannot construct
//! a scoped grant, role result, withheld departure, loss event or owner output.
use super::{ApprovedConductV1, CoverageError, CoverageProductV1};
use super::{authority::PublicationAuthority, protocol::Protocol};
use mush_core::{CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use mush_economy::managed_coverage::{CoverageBasisV1, CoverageQuoteTermsV1};
use serde::{Deserialize, Serialize, de};
use std::{fmt, marker::PhantomData};

pub(super) const MAX_PUBLICATION_BYTES: usize = 98304;
pub(super) const MAX_QUOTE_BYTES: usize = 16384;

pub(super) fn bounded_rows<'de, D, T, const N: usize>(d: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Rows<T, const N: usize>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>, const N: usize> de::Visitor<'de> for Rows<T, N> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "at most {N} rows")
        }
        fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            if seq.size_hint().is_some_and(|n| n > N) {
                return Err(de::Error::custom("coverage row bound"));
            }
            let mut rows = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(N));
            while let Some(row) = seq.next_element()? {
                if rows.len() == N {
                    return Err(de::Error::custom("coverage row bound"));
                }
                rows.push(row);
            }
            Ok(rows)
        }
    }
    d.deserialize_seq(Rows::<T, N>(PhantomData))
}
pub(super) fn option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}
fn nonzero(d: DefinitionDigest) -> bool {
    d.as_bytes() != &[0; 32]
}
pub(super) fn value<T: Serialize>(
    row: &T,
    limit: usize,
) -> Result<serde_json::Value, CoverageError> {
    let value = serde_json::to_value(row).map_err(|_| CoverageError::Product)?;
    mush_core::canonical_json_bytes_with_limit(&value, limit).map_err(|_| CoverageError::Bound)?;
    Ok(value)
}
pub(super) fn digest<T: Serialize>(
    domain: &str,
    row: &T,
    limit: usize,
) -> Result<DefinitionDigest, CoverageError> {
    DefinitionDigest::for_canonical_value_with_limit(domain, &value(row, limit)?, limit)
        .map_err(|_| CoverageError::Bound)
}
pub(super) fn decode<T: for<'de> Deserialize<'de> + Serialize>(
    raw: &[u8],
    limit: usize,
) -> Result<T, CoverageError> {
    if raw.is_empty() || raw.len() > limit {
        return Err(CoverageError::Bound);
    }
    // Decode the closed typed form first so duplicate fields cannot disappear
    // through an intermediate Value map.
    let row: T = serde_json::from_slice(raw).map_err(|_| CoverageError::Product)?;
    let actual: serde_json::Value =
        serde_json::from_slice(raw).map_err(|_| CoverageError::Product)?;
    if value(&row, limit)? != actual {
        return Err(CoverageError::Product);
    }
    Ok(row)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Publication {
    pub(super) format: u16,
    pub(super) operation: CommitToken,
    pub(super) action: DefinitionRef,
    pub(super) protocol: Protocol,
    pub(super) publisher: ObjId,
    pub(super) authorization: PublicationAuthority,
    pub(super) product: CoverageProductV1,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "bounded_rows::<_, ConductSelection, 16>"
    )]
    pub(super) conduct: Vec<ConductSelection>,
    pub(super) posted_at_utc: i64,
}
impl Publication {
    pub(super) fn validate(&self) -> Result<(), CoverageError> {
        self.protocol.validate()?;
        self.authorization
            .validate()
            .map_err(|_| CoverageError::Authority)?;
        self.product.digest()?;
        if self.conduct.len() != self.product.approvals().len()
            || self
                .conduct
                .iter()
                .zip(self.product.approvals())
                .any(|(s, a)| &s.approval != a || s.validate().is_err())
            || self
                .conduct
                .iter()
                .map(|s| s.trial)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.conduct.len()
        {
            return Err(CoverageError::Product);
        }
        if self.format != 1
            || self.publisher.0 == 0
            || self.action != self.protocol.action(0)?
            || self.posted_at_utc != self.authorization.at()
        {
            return Err(CoverageError::Product);
        }
        value(self, MAX_PUBLICATION_BYTES)?;
        Ok(())
    }
    pub(super) fn decode(raw: &[u8]) -> Result<Self, CoverageError> {
        let row: Self = decode(raw, MAX_PUBLICATION_BYTES)?;
        row.validate()?;
        Ok(row)
    }
    pub(super) fn digest(&self) -> Result<DefinitionDigest, CoverageError> {
        self.validate()?;
        digest(
            "tacsim.coverage.publication.v1",
            self,
            MAX_PUBLICATION_BYTES,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ConductSelection {
    pub(super) approval: ApprovedConductV1,
    pub(super) trial: CommitToken,
    pub(super) trial_digest: DefinitionDigest,
}
impl ConductSelection {
    fn validate(&self) -> Result<(), CoverageError> {
        if !nonzero(self.trial_digest) || self.trial_digest != self.approval.role_trial() {
            return Err(CoverageError::Product);
        }
        value(self, 4096)?;
        Ok(())
    }
    pub(super) fn digest(&self) -> Result<DefinitionDigest, CoverageError> {
        self.validate()?;
        digest("tacsim.coverage.qualified_role.v1", self, 4096)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Risk {
    pub(super) ic_open: CommitToken,
    pub(super) seal_digest: DefinitionDigest,
    pub(super) admission_digest: DefinitionDigest,
    pub(super) departure_digest: DefinitionDigest,
    pub(super) hull: ObjId,
    pub(super) hull_title_revision: u64,
    pub(super) hull_revision: u64,
    pub(super) hull_schema: DefinitionRef,
    pub(super) zone: DefinitionRef,
    #[serde(deserialize_with = "option")]
    pub(super) qualified: Option<ConductSelection>,
}
impl Risk {
    fn validate(&self) -> Result<(), CoverageError> {
        if self.hull.0 == 0
            || self.hull_title_revision == 0
            || [
                self.seal_digest,
                self.admission_digest,
                self.departure_digest,
                self.hull_schema.digest(),
                self.zone.digest(),
            ]
            .iter()
            .any(|d| !nonzero(*d))
        {
            return Err(CoverageError::Product);
        }
        // The actual Tactics revision starts at zero; do not narrow its domain.
        if let Some(qualified) = &self.qualified {
            qualified.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Quote {
    pub(super) format: u16,
    pub(super) request: DefinitionDigest,
    pub(super) operation: CommitToken,
    pub(super) action: DefinitionRef,
    pub(super) protocol: Protocol,
    pub(super) publication: CommitToken,
    pub(super) publication_digest: DefinitionDigest,
    pub(super) product_digest: DefinitionDigest,
    pub(super) risk: Risk,
    pub(super) terms: CoverageQuoteTermsV1,
}
impl Quote {
    pub(super) fn validate(&self) -> Result<(), CoverageError> {
        self.protocol.validate()?;
        self.risk.validate()?;
        self.terms.validate().map_err(|_| CoverageError::Terms)?;
        if self.format != 1
            || !nonzero(self.request)
            || self.action != self.protocol.action(1)?
            || self.operation == self.publication
            || self.risk.ic_open == self.operation
            || self.risk.ic_open == self.publication
            || self.risk.hull == self.terms.buyer
            || !nonzero(self.publication_digest)
            || !nonzero(self.product_digest)
            || (self.risk.qualified.is_none() && self.terms.discount != mush_economy::Amount::ZERO)
        {
            return Err(CoverageError::Product);
        }
        value(self, MAX_QUOTE_BYTES)?;
        Ok(())
    }
    pub(super) fn decode(raw: &[u8]) -> Result<Self, CoverageError> {
        let row: Self = decode(raw, MAX_QUOTE_BYTES)?;
        row.validate()?;
        Ok(row)
    }
    pub(super) fn digest(&self) -> Result<DefinitionDigest, CoverageError> {
        self.validate()?;
        digest("tacsim.coverage.quote.v1", self, MAX_QUOTE_BYTES)
    }
    pub(super) fn matches_publication(&self, p: &Publication) -> Result<(), CoverageError> {
        self.validate()?;
        p.validate()?;
        let product = &p.product;
        let illustration = product.illustrate(self.terms.insured_value)?;
        let amounts = if let Some(qualified) = &self.risk.qualified {
            if !product.approvals().contains(&qualified.approval) || !p.conduct.contains(qualified)
            {
                return Err(CoverageError::Product);
            }
            illustration.with_qualified_safe_conduct()
        } else {
            illustration.ordinary()
        };
        if self.publication != p.operation
            || self.publication_digest != p.digest()?
            || self.product_digest != product.digest()?
            || self.protocol != p.protocol
            || self.terms.insurer != product.insurer()
            || self.terms.scope != product.scope()
            || self.terms.treasury != product.treasury()
            || self.terms.adjudicator != product.adjudicator()
            || &self.terms.instrument != product.terms().instrument()
            || self.terms.base_premium != amounts.base_premium()
            || self.terms.discount != amounts.discount()
            || self.terms.premium != amounts.premium()
            || self.terms.liability != amounts.maximum_payout()
            || self.terms.duration_seconds != product.terms().term_seconds()
            || self.terms.claim_delay_seconds != product.terms().claim_delay_seconds()
            || self.terms.quoted_at_utc < p.posted_at_utc
            || self
                .terms
                .expires_at_utc
                .checked_sub(self.terms.quoted_at_utc)
                != Some(product.quote_seconds())
            || !product.hulls().contains(&self.risk.hull_schema)
            || !product.zones().contains(&self.risk.zone)
        {
            return Err(CoverageError::Product);
        }
        Ok(())
    }
    pub(super) fn native_basis(&self) -> Result<CoverageBasisV1, CoverageError> {
        self.validate()?;
        let basis = CoverageBasisV1 {
            format: 1,
            product: self.publication,
            // Native product identity binds the complete acknowledged publication,
            // including publisher attribution; the quote also pins the payload digest.
            product_digest: self.publication_digest,
            quote: self.operation,
            quote_digest: self.digest()?,
            ic_open: self.risk.ic_open,
            seal_digest: self.risk.seal_digest,
            admission_digest: self.risk.admission_digest,
            departure_digest: self.risk.departure_digest,
            hull: self.risk.hull,
            hull_title_revision: self.risk.hull_title_revision,
            hull_revision: self.risk.hull_revision,
            hull_schema: self.risk.hull_schema.clone(),
            zone: self.risk.zone.clone(),
            qualified_role: self
                .risk
                .qualified
                .as_ref()
                .map(ConductSelection::digest)
                .transpose()?,
        };
        basis.validate().map_err(|_| CoverageError::Product)?;
        Ok(basis)
    }
}
