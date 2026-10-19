//! Immutable product declarations. Decoding a declaration proves neither its
//! publication nor the referenced organization's authority, certification,
//! funding or live host. The closed owner service must establish those facts.
use super::{CoverageError, CoverageIllustration, CoverageTermsV1};
use mush_core::{DefinitionDigest, DefinitionRef, ObjId};
use mush_economy::{AccountId, Amount};
use mush_org::{OrgId, OrgScopeId};
use mush_roll::commerce::{CharterId, FiledRevisionId, LineId, RevisionIdentity};
use serde::{Deserialize, Deserializer, Serialize, de};
use smol_str::SmolStr;
use std::{collections::BTreeSet, fmt, marker::PhantomData};

/// The actual role whose trial and executable must match. These tags do not
/// register a Registry role or create an authored profile by themselves.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeConductRole {
    Escort,
    Transit,
}

/// V1 covers only a trusted, committed IC destruction in the policy's exact
/// zone and interval. Capture, disappearance, generic journey failures and OOC
/// elimination are excluded. Paid claims convey any surviving wreck title to
/// the insurer in that same transition. Transfer requires cancellation and a
/// fresh quote; there is no coverage assignment or retroactive activation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LossRule {
    RecordedIcDestructionWithWreckTitle,
}

/// An exact proposed approval, with no source text and no mutable "latest"
/// selector. A matching record is data, never a discount capability. Binding
/// must join the published product, actual certification and role-trial result
/// to the private permit for the acknowledged IC Open and chosen profile.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "ApprovalWire")]
pub struct ApprovedConductV1 {
    format: u16,
    charter: CharterId,
    line: LineId,
    filed: FiledRevisionId,
    revision: RevisionIdentity,
    certification: DefinitionDigest,
    role_trial: DefinitionDigest,
    role: SafeConductRole,
    profile: SmolStr,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalWire {
    format: u16,
    charter: CharterId,
    line: LineId,
    filed: FiledRevisionId,
    revision: RevisionIdentity,
    certification: DefinitionDigest,
    role_trial: DefinitionDigest,
    role: SafeConductRole,
    profile: SmolStr,
}
impl TryFrom<ApprovalWire> for ApprovedConductV1 {
    type Error = CoverageError;
    fn try_from(w: ApprovalWire) -> Result<Self, Self::Error> {
        if w.format != 1
            || w.certification.as_bytes() == &[0; 32]
            || w.role_trial.as_bytes() == &[0; 32]
            || w.profile.is_empty()
            || w.profile.len() > 64
            || w.profile.trim() != w.profile.as_str()
            || w.profile.chars().any(char::is_control)
        {
            return Err(CoverageError::Product);
        }
        Ok(Self {
            format: w.format,
            charter: w.charter,
            line: w.line,
            filed: w.filed,
            revision: w.revision,
            certification: w.certification,
            role_trial: w.role_trial,
            role: w.role,
            profile: w.profile,
        })
    }
}
impl ApprovedConductV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        charter: CharterId,
        line: LineId,
        filed: FiledRevisionId,
        revision: RevisionIdentity,
        certification: DefinitionDigest,
        role_trial: DefinitionDigest,
        role: SafeConductRole,
        profile: SmolStr,
    ) -> Result<Self, CoverageError> {
        Self::try_from(ApprovalWire {
            format: 1,
            charter,
            line,
            filed,
            revision,
            certification,
            role_trial,
            role,
            profile,
        })
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
    pub const fn certification(&self) -> DefinitionDigest {
        self.certification
    }
    pub const fn role_trial(&self) -> DefinitionDigest {
        self.role_trial
    }
    pub const fn role(&self) -> SafeConductRole {
        self.role
    }
    pub fn profile(&self) -> &str {
        &self.profile
    }
}

/// The exact publication payload. The owner must resolve insurer/scope/account
/// to the same active real organization, check publication rights, and retain
/// all references before admitting it. Decoding cannot perform that work.
/// Values are agreed minor units bounded by the immutable financial terms.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "ProductWire")]
pub struct CoverageProductV1 {
    format: u16,
    insurer: OrgId,
    scope: OrgScopeId,
    treasury: AccountId,
    adjudicator: ObjId,
    terms: CoverageTermsV1,
    hulls: Vec<DefinitionRef>,
    zones: Vec<DefinitionRef>,
    approvals: Vec<ApprovedConductV1>,
    quote_seconds: i64,
    loss_rule: LossRule,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProductWire {
    format: u16,
    insurer: OrgId,
    scope: OrgScopeId,
    treasury: AccountId,
    adjudicator: ObjId,
    terms: CoverageTermsV1,
    hulls: Rows<DefinitionRef, 32>,
    zones: Rows<DefinitionRef, 32>,
    approvals: Rows<ApprovedConductV1, 16>,
    quote_seconds: i64,
    loss_rule: LossRule,
}
impl TryFrom<ProductWire> for CoverageProductV1 {
    type Error = CoverageError;
    fn try_from(w: ProductWire) -> Result<Self, Self::Error> {
        if w.format != 1
            || w.adjudicator.0 == 0
            || !(1..=3600).contains(&w.quote_seconds)
            || w.hulls.0.is_empty()
            || w.zones.0.is_empty()
            || w.hulls.0.len() > 32
            || w.zones.0.len() > 32
            || w.approvals.0.len() > 16
        {
            return Err(CoverageError::Product);
        }
        // Require canonical set ordering, rather than silently sorting captured
        // policy bytes. Exact versions and digests matter, not names alone.
        for refs in [&w.hulls.0, &w.zones.0] {
            if refs.iter().any(|r| r.digest().as_bytes() == &[0; 32])
                || refs.windows(2).any(|pair| pair[0] >= pair[1])
            {
                return Err(CoverageError::Product);
            }
        }
        let mut seen = BTreeSet::new();
        for approval in &w.approvals.0 {
            // One pinned role/trial per exact executable profile. Contradictory
            // duplicate approvals must not make qualification order-dependent.
            if !seen.insert((
                approval.charter,
                approval.line,
                approval.filed,
                approval.revision.revision(),
                approval.profile.clone(),
            )) {
                return Err(CoverageError::Product);
            }
        }
        let product = Self {
            format: w.format,
            insurer: w.insurer,
            scope: w.scope,
            treasury: w.treasury,
            adjudicator: w.adjudicator,
            terms: w.terms,
            hulls: w.hulls.0,
            zones: w.zones.0,
            approvals: w.approvals.0,
            quote_seconds: w.quote_seconds,
            loss_rule: w.loss_rule,
        };
        product.encoded()?;
        Ok(product)
    }
}
impl CoverageProductV1 {
    pub const MAX_BYTES: usize = 65_536;
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        insurer: OrgId,
        scope: OrgScopeId,
        treasury: AccountId,
        adjudicator: ObjId,
        terms: CoverageTermsV1,
        hulls: Vec<DefinitionRef>,
        zones: Vec<DefinitionRef>,
        approvals: Vec<ApprovedConductV1>,
        quote_seconds: i64,
    ) -> Result<Self, CoverageError> {
        Self::try_from(ProductWire {
            format: 1,
            insurer,
            scope,
            treasury,
            adjudicator,
            terms,
            hulls: Rows(hulls),
            zones: Rows(zones),
            approvals: Rows(approvals),
            quote_seconds,
            loss_rule: LossRule::RecordedIcDestructionWithWreckTitle,
        })
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, CoverageError> {
        if bytes.is_empty() || bytes.len() > Self::MAX_BYTES {
            return Err(CoverageError::Bound);
        }
        serde_json::from_slice(bytes).map_err(|_| CoverageError::Product)
    }
    fn encoded(&self) -> Result<serde_json::Value, CoverageError> {
        let value = serde_json::to_value(self).map_err(|_| CoverageError::Product)?;
        mush_core::canonical_json_bytes_with_limit(&value, Self::MAX_BYTES)
            .map_err(|_| CoverageError::Bound)?;
        Ok(value)
    }
    pub fn digest(&self) -> Result<DefinitionDigest, CoverageError> {
        DefinitionDigest::for_canonical_value_with_limit(
            "tacsim.coverage.product.v1",
            &self.encoded()?,
            Self::MAX_BYTES,
        )
        .map_err(|_| CoverageError::Product)
    }
    /// Both prices remain conditional. Publication, current authority and exact
    /// IC proof must be resolved by the real service before selecting a price.
    pub fn illustrate(&self, value: Amount) -> Result<CoverageIllustration, CoverageError> {
        self.terms.illustrate(value)
    }
    pub const fn insurer(&self) -> OrgId {
        self.insurer
    }
    pub const fn scope(&self) -> OrgScopeId {
        self.scope
    }
    pub const fn treasury(&self) -> AccountId {
        self.treasury
    }
    pub const fn adjudicator(&self) -> ObjId {
        self.adjudicator
    }
    pub fn terms(&self) -> &CoverageTermsV1 {
        &self.terms
    }
    pub fn hulls(&self) -> &[DefinitionRef] {
        &self.hulls
    }
    pub fn zones(&self) -> &[DefinitionRef] {
        &self.zones
    }
    pub fn approvals(&self) -> &[ApprovedConductV1] {
        &self.approvals
    }
    pub const fn quote_seconds(&self) -> i64 {
        self.quote_seconds
    }
    pub const fn loss_rule(&self) -> LossRule {
        self.loss_rule
    }
}

// Direct serde callers also stop at the entry ceiling; the byte decoder adds
// the complete document bound before parsing. No unbounded intermediate list.
struct Rows<T, const N: usize>(Vec<T>);
impl<'de, T: Deserialize<'de>, const N: usize> Deserialize<'de> for Rows<T, N> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Bounded<T, const N: usize>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>, const N: usize> de::Visitor<'de> for Bounded<T, N> {
            type Value = Rows<T, N>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "at most {N} coverage entries")
            }
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                if seq.size_hint().is_some_and(|size| size > N) {
                    return Err(de::Error::custom(CoverageError::Bound));
                }
                let mut items = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(N));
                while let Some(item) = seq.next_element()? {
                    if items.len() == N {
                        return Err(de::Error::custom(CoverageError::Bound));
                    }
                    items.push(item);
                }
                Ok(Rows(items))
            }
        }
        d.deserialize_seq(Bounded::<T, N>(PhantomData))
    }
}
