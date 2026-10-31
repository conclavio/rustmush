//! Strict retained bay terms. They are data, never authority to charge, move
//! a part, refit a unit, or bypass a title/custody predicate.
use super::{BayRefitError as Error, RESERVATION_SECONDS, SUBJECT_BYTES};
use crate::core_integration::ScopedCoreAccount;
use crate::{AccountId, Amount, InstrumentId};
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use mush_org::SystemPrincipalId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// One bay: the facility it is and the organization account it pays.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BaySite {
    pub facility: ObjId,
    pub provider: ScopedCoreAccount,
}

/// **The bay fee terms**: who issues the plan, in which currency, the recipe
/// the bays work to, every bay and its provider, and the highest fee a quote
/// may charge. The fee itself is the supplier's quote, in the subject.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BayPaymentPolicy {
    pub issuer: SystemPrincipalId,
    pub instrument: InstrumentId,
    pub recipe: DefinitionRef,
    /// Strictly ascending by facility.
    pub bays: Vec<BaySite>,
    pub max_fee_minor: i64,
}
impl BayPaymentPolicy {
    pub fn validate(&self) -> Result<(), Error> {
        if self.recipe.key().kind.as_str() != "recipe"
            || self.recipe.key().id.as_str() != "bay_refit"
            || self.recipe.version() != 1
            || self.recipe.digest().as_bytes() == &[0; 32]
            || self.bays.is_empty()
            || self.bays.len() > 64
            || self.bays.windows(2).any(|w| w[0].facility >= w[1].facility)
            || !(1..=1_000_000_000).contains(&self.max_fee_minor)
        {
            return Err(Error::Invalid);
        }
        bounded(self, SUBJECT_BYTES)
    }
    /// The bay at `facility`.
    pub fn site(&self, facility: ObjId) -> Result<&BaySite, Error> {
        self.bays
            .iter()
            .find(|b| b.facility == facility)
            .ok_or(Error::Invalid)
    }
    /// The Industry policy the kernel pins the paid actions under: the
    /// recipe and the bays.
    pub fn industry_policy(&self) -> serde_json::Value {
        serde_json::json!({"format":1,"recipe":self.recipe,"facilities":self.bays.iter().map(|b| b.facility).collect::<Vec<_>>()})
    }
    pub fn digest(&self) -> Result<DefinitionDigest, Error> {
        self.validate()?;
        digest(self, SUBJECT_BYTES)
    }
    /// The descriptor of `action`, read from the kernel's one definition of
    /// the bay flow so the owners can never differ.
    pub fn action_descriptor(
        &self,
        action: &str,
    ) -> Result<mush_core::DefinitionDescriptor, Error> {
        self.validate()?;
        let policy = self.industry_policy();
        let kernel = mush_core::bay_refit_protocol::ACTIONS;
        match kernel.iter().position(|a| *a == action) {
            Some(index) => mush_core::bay_refit_protocol::action_descriptor(&policy, index),
            None => mush_core::bay_refit_protocol::owner_action_descriptor(&policy, action),
        }
        .map_err(|_| Error::Invalid)
    }
    pub fn action_ref(&self, action: &str) -> Result<DefinitionRef, Error> {
        mush_core::DefinitionRegistryBuilder::new()
            .register(
                mush_core::OwnerId::new("industry").map_err(|_| Error::Invalid)?,
                self.action_descriptor(action)?,
            )
            .map_err(|_| Error::Invalid)
    }
}

/// **One accepted bay order's money terms**: the order, the exact offer it
/// takes, who pays, the hull and the refit (its revision and the roots
/// before and after), the bay, and the quoted fee.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, try_from = "SubjectWire")]
pub struct BaySubject {
    pub order: u64,
    pub intent: DefinitionDigest,
    pub offer: CommitToken,
    pub offer_digest: DefinitionDigest,
    pub requester: ObjId,
    pub payer: AccountId,
    pub hull: ObjId,
    pub unit_revision: u64,
    pub before: DefinitionRef,
    pub target: DefinitionRef,
    pub facility: ObjId,
    pub fee_minor: i64,
    pub policy: DefinitionDigest,
    pub accepted_at: ClockStamp,
    pub expires_at: ClockStamp,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubjectWire {
    order: u64,
    intent: DefinitionDigest,
    offer: CommitToken,
    offer_digest: DefinitionDigest,
    requester: ObjId,
    payer: AccountId,
    hull: ObjId,
    unit_revision: u64,
    before: DefinitionRef,
    target: DefinitionRef,
    facility: ObjId,
    fee_minor: i64,
    policy: DefinitionDigest,
    accepted_at: ClockStamp,
    expires_at: ClockStamp,
}
impl TryFrom<SubjectWire> for BaySubject {
    type Error = Error;
    fn try_from(w: SubjectWire) -> Result<Self, Error> {
        let s = Self {
            order: w.order,
            intent: w.intent,
            offer: w.offer,
            offer_digest: w.offer_digest,
            requester: w.requester,
            payer: w.payer,
            hull: w.hull,
            unit_revision: w.unit_revision,
            before: w.before,
            target: w.target,
            facility: w.facility,
            fee_minor: w.fee_minor,
            policy: w.policy,
            accepted_at: w.accepted_at,
            expires_at: w.expires_at,
        };
        s.validate()?;
        Ok(s)
    }
}
fn unit(r: &DefinitionRef) -> bool {
    r.key().kind.as_str() == "unit"
        && r.key().owner.as_str() == "tacsim"
        && r.digest().as_bytes() != &[0; 32]
}
impl BaySubject {
    pub fn validate(&self) -> Result<(), Error> {
        if self.intent.as_bytes() == &[0; 32]
            || self.offer_digest.as_bytes() == &[0; 32]
            || self.policy.as_bytes() == &[0; 32]
            || [self.requester, self.facility].contains(&self.hull)
            || self.requester == self.facility
            || self.unit_revision == u64::MAX
            || !unit(&self.before)
            || !unit(&self.target)
            || self.before == self.target
            || self.fee_minor < 1
            || !matches!(self.accepted_at.clock,ClockRef::WorldIc{generation}if generation>0)
            || self.expires_at.clock != self.accepted_at.clock
            || self.accepted_at.at.checked_add(RESERVATION_SECONDS) != Some(self.expires_at.at)
        {
            return Err(Error::Invalid);
        }
        bounded(self, SUBJECT_BYTES)
    }
    pub fn digest(&self) -> Result<DefinitionDigest, Error> {
        self.validate()?;
        digest(self, SUBJECT_BYTES)
    }
    /// The quoted fee.
    pub fn price(&self) -> Amount {
        Amount::from_minor(self.fee_minor)
    }
    pub(crate) fn matches_policy(&self, p: &BayPaymentPolicy) -> Result<(), Error> {
        self.validate()?;
        p.validate()?;
        let site = p.site(self.facility)?;
        if self.policy != p.digest()?
            || self.fee_minor > p.max_fee_minor
            || self.payer == site.provider.account
        {
            return Err(Error::Integrity);
        }
        Ok(())
    }
}
pub(super) fn bounded<T: Serialize>(v: &T, n: usize) -> Result<(), Error> {
    bytes(v, n).map(|_| ())
}
pub(super) fn bytes<T: Serialize>(v: &T, n: usize) -> Result<Vec<u8>, Error> {
    mush_core::mechanics::canonical_json_bytes_with_limit(
        &serde_json::to_value(v).map_err(|_| Error::Invalid)?,
        n,
    )
    .map_err(|_| Error::Capacity)
}
pub(super) fn digest<T: Serialize>(v: &T, n: usize) -> Result<DefinitionDigest, Error> {
    Ok(DefinitionDigest::from_bytes(
        Sha256::digest(bytes(v, n)?).into(),
    ))
}
