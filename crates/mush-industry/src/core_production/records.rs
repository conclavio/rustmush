//! Retained die Origin and quota history. A decoded record alone grants no right.
use mush_core::{
    CanonicalId, ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId,
    RequestUid,
};
use mush_economy::ExternalRef;
use mush_inventory::{EndpointRef, Grade, Lineage, Lot, LotId};
use mush_org::OrgId;
use serde::{Deserialize, Serialize};

use super::{Error, MAX_CERTIFICATES, OriginPolicy, bad, encode};

/// Governed physical die classes. The ROM class has no manufactured die.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DieClass {
    I,
    II,
    III,
}
impl DieClass {
    pub const fn label(self) -> &'static str {
        match self {
            Self::I => "die.c1",
            Self::II => "die.c2",
            Self::III => "die.c3",
        }
    }
    pub const ALL: [Self; 3] = [Self::I, Self::II, Self::III];
    pub const fn ordinal(self) -> u8 {
        match self {
            Self::I => 1,
            Self::II => 2,
            Self::III => 3,
        }
    }
    pub const fn index(self) -> usize {
        self.ordinal() as usize - 1
    }
}

/// A civil month on the one WorldIc calendar, independent of rate generation.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImportMonth {
    pub(crate) year: i64,
    pub(crate) month: u32,
}
impl ImportMonth {
    pub(crate) fn from_stamp(stamp: &ClockStamp) -> Result<Self, Error> {
        if !matches!(stamp.clock, ClockRef::WorldIc { generation } if generation > 0) {
            return Err(Error::Invalid);
        }
        let date = mush_core::clock::CivilDate::from_ic_seconds(stamp.at);
        Ok(Self {
            year: date.year,
            month: date.month,
        })
    }
    pub(crate) fn validate(self) -> Result<(), Error> {
        if !(1..=12).contains(&self.month) {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialOrd, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct QuotaKey {
    pub(crate) importer: OrgId,
    pub(crate) month: ImportMonth,
    pub(crate) class: DieClass,
}

/// The first receipt freezes the month's cap. Later configuration cannot reset it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Quota {
    pub(crate) format: u16,
    pub(crate) key: QuotaKey,
    pub(crate) maximum: u16,
    pub(crate) issued: u16,
    pub(crate) first: CommitToken,
    pub(crate) last: CommitToken,
}
impl Quota {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        self.key.month.validate()?;
        if self.format != 1
            || !(1..=1024).contains(&self.maximum)
            || self.issued == 0
            || self.issued > self.maximum
            || ((self.issued == 1) != (self.first == self.last))
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub(crate) fn issue(
        key: QuotaKey,
        configured_maximum: u16,
        previous: Option<&Self>,
        commit: CommitToken,
    ) -> Result<Self, Error> {
        key.month.validate()?;
        if !(1..=1024).contains(&configured_maximum) {
            return Err(Error::Invalid);
        }
        let next = if let Some(old) = previous {
            old.validate()?;
            if old.key != key || commit == old.first || commit == old.last {
                return Err(Error::Integrity);
            }
            // A lower current policy may stop new stock; a higher one never
            // expands the cap already assigned to this actual month.
            if old.issued >= old.maximum.min(configured_maximum) {
                return Err(Error::Capacity);
            }
            Self {
                issued: old.issued.checked_add(1).ok_or(Error::Capacity)?,
                last: commit,
                ..old.clone()
            }
        } else {
            Self {
                format: 1,
                key,
                maximum: configured_maximum,
                issued: 1,
                first: commit,
                last: commit,
            }
        };
        next.validate()?;
        Ok(next)
    }
}

impl QuotaKey {
    pub(crate) fn key(&self) -> Result<String, Error> {
        self.month.validate()?;
        Ok(format!(
            "{}:{}:{:02}:{}",
            self.importer.0,
            self.month.year,
            self.month.month,
            self.class.ordinal()
        ))
    }
}
/// Immutable issuance proof, not an ownership or current-stock token. Public as an opaque
/// type so a granted reader ([`DieCertificates`]) can read the retained proof's words through
/// its accessors (doc 17h cut M8a fix round 1, F6); its fields stay this crate's.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DieCertificate {
    pub(crate) format: u16,
    pub(crate) sequence: u64,
    pub(crate) operation: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) policy: OriginPolicy,
    pub(crate) agent: ObjId,
    pub(crate) class: DieClass,
    pub(crate) label: CanonicalId,
    pub(crate) lot: LotId,
    pub(crate) commodity: CanonicalId,
    pub(crate) grade: Grade,
    pub(crate) source: ExternalRef,
    pub(crate) origin_endpoint: EndpointRef,
    pub(crate) observed_ic: ClockStamp,
    pub(crate) observed_utc: i64,
    pub(crate) ordinal: u16,
    pub(crate) quota_maximum: u16,
}
impl DieCertificate {
    /// The issuing operation: the certificate's key and the die lot's origin id.
    #[must_use]
    pub fn operation(&self) -> CommitToken {
        self.operation
    }
    /// The importer the origin policy names: whose stock the die came from.
    #[must_use]
    pub fn importer(&self) -> mush_org::OrgId {
        self.policy.importer
    }
    /// The die's class.
    #[must_use]
    pub fn class(&self) -> DieClass {
        self.class
    }
    /// The die lot the certificate issued.
    #[must_use]
    pub fn lot(&self) -> LotId {
        self.lot
    }
    pub(crate) fn key(&self) -> String {
        self.operation.to_hex()
    }
    pub(crate) fn quota_key(&self) -> Result<QuotaKey, Error> {
        Ok(QuotaKey {
            importer: self.policy.importer,
            month: ImportMonth::from_stamp(&self.observed_ic)?,
            class: self.class,
        })
    }
    pub(crate) fn validate(&self) -> Result<(), Error> {
        self.policy.validate()?;
        self.quota_key()?;
        if self.format != 1
            || !(1..=MAX_CERTIFICATES).contains(&self.sequence)
            || self.action != self.policy.action_ref(false)?
            || self.origin_endpoint != EndpointRef(self.policy.agent(self.agent)?.stock)
            || self.label.as_str() != self.class.label()
            || self.commodity.as_str() != "tacsim.electronics"
            || self.grade != self.policy.grades[self.class.index()]
            || self.lot != lot_id(self.operation)?
            || self.source != source(self.operation)
            || !(1..=1024).contains(&self.quota_maximum)
            || self.ordinal == 0
            || self.ordinal > self.quota_maximum
            || self.ordinal > self.policy.monthly_quotas[self.class.index()]
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub(crate) fn matches_lot(&self, lot: &Lot) -> Result<(), Error> {
        self.validate()?;
        if lot.id() != self.lot
            || lot.quantity() != 1
            || lot.commodity() != &self.commodity
            || lot.grade() != self.grade
            || lot.lineage()
                != &(Lineage::Origin {
                    source: self.source.clone(),
                })
            || lot.depth() != 0
        {
            return Err(Error::Integrity);
        }
        Ok(())
    }
}
pub(crate) fn lot_id(operation: CommitToken) -> Result<LotId, Error> {
    mush_inventory::derive_lot_id(RequestUid::from_bytes(*operation.as_bytes()), 0).map_err(bad)
}
pub(crate) fn source(operation: CommitToken) -> ExternalRef {
    ExternalRef {
        provider: crate::NS.into(),
        kind: super::SOURCE_KIND.into(),
        id: operation.to_hex().into(),
        revision: Some(1),
    }
}
/// **The retained die certificates, for a granted reader** (doc 17h cut M8a fix round 1,
/// F6): the family handle, published as module state when the composition names a reader
/// ([`super::CoreOriginConfig::with_certificate_reader`]). A typed read through it is refused
/// to any owner the grant does not name. Data, not custody or consumption authority.
#[derive(Clone)]
pub struct DieCertificates(pub(crate) mush_core::StateFamilyHandle<String, DieCertificate>);
impl DieCertificates {
    /// The family handle, keyed by the issuing operation's hex.
    #[must_use]
    pub fn handle(&self) -> &mush_core::StateFamilyHandle<String, DieCertificate> {
        &self.0
    }
}
/// A live owner read returns this only after joining the retained receipt with
/// the exact present Origin lot. This is data, not custody/consumption authority.
#[derive(Clone, Debug)]
pub struct CertifiedOrigin {
    pub(crate) certificate: DieCertificate,
    pub(crate) lot: Lot,
}
impl CertifiedOrigin {
    pub fn class(&self) -> DieClass {
        self.certificate.class
    }
    pub fn lot(&self) -> &Lot {
        &self.lot
    }
    pub fn operation(&self) -> CommitToken {
        self.certificate.operation
    }
    pub fn content(&self) -> &DefinitionRef {
        &self.certificate.policy.content
    }
}
/// Independent owner checkpoint, acknowledged beside all typed Origin writes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OriginIndex {
    pub(crate) format: u16,
    pub(crate) initialized_by: CommitToken,
    pub(crate) initialize_action: DefinitionRef,
    pub(crate) initialize_policy: OriginPolicy,
    pub(crate) issued: u64,
    pub(crate) chain: DefinitionDigest,
}
impl OriginIndex {
    pub(crate) fn new(policy: &OriginPolicy, operation: CommitToken) -> Result<Self, Error> {
        let mut r = Self {
            format: 1,
            initialized_by: operation,
            initialize_action: policy.action_ref(true)?,
            initialize_policy: policy.clone(),
            issued: 0,
            chain: DefinitionDigest::from_bytes([0; 32]),
        };
        r.chain = DefinitionDigest::for_canonical_value(
            "industry.core_origin_genesis.v1",
            &serde_json::json!({"operation":operation,"policy":policy}),
        )
        .map_err(bad)?;
        r.validate()?;
        Ok(r)
    }
    pub(crate) fn validate(&self) -> Result<(), Error> {
        self.initialize_policy.validate()?;
        if self.format != 1
            || self.issued > MAX_CERTIFICATES
            || self.initialize_action != self.initialize_policy.action_ref(true)?
            || self.chain.as_bytes() == &[0; 32]
        {
            return Err(Error::Integrity);
        }
        Ok(())
    }
    pub(crate) fn issue(&self, certificate: &DieCertificate) -> Result<Self, Error> {
        self.validate()?;
        certificate.validate()?;
        if certificate.sequence != self.issued.checked_add(1).ok_or(Error::Capacity)? {
            return Err(Error::Integrity);
        }
        let chain=DefinitionDigest::for_canonical_value("industry.core_origin_chain.v1",&serde_json::json!({"prior":self.chain,"certificate":encode(certificate).map_err(bad)?})).map_err(bad)?;
        Ok(Self {
            issued: certificate.sequence,
            chain,
            ..self.clone()
        })
    }
}
