//! Strict retained fitting terms. They are data, never authority to charge,
//! withdraw a core, replace a unit, or bypass a title/custody predicate.
use super::{FITTING_MINOR, RESERVATION_SECONDS, RefitError as Error, SUBJECT_BYTES};
use crate::core_integration::{CoreClass, ScopedCoreAccount};
use crate::{AccountId, Amount, InstrumentId};
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use mush_org::SystemPrincipalId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefitPaymentPolicy {
    pub issuer: SystemPrincipalId,
    pub instrument: InstrumentId,
    pub provider: ScopedCoreAccount,
    pub factory: ObjId,
    pub recipe: DefinitionRef,
    /// Class I, II, III, in this exact order. Trusted content validates the
    /// finite compatible family through the actual Tactics definitions.
    pub fitted_units: [DefinitionRef; 3],
}
impl RefitPaymentPolicy {
    pub fn validate(&self) -> Result<(), Error> {
        if self.recipe.key().kind.as_str() != "recipe"
            || self.recipe.key().id.as_str() != "core_refit"
            || self.recipe.version() != 1
            || self.recipe.digest().as_bytes() == &[0; 32]
        {
            return Err(Error::Invalid);
        }
        for (i, root) in self.fitted_units.iter().enumerate() {
            if root.key().kind.as_str() != "unit"
                || root.key().owner.as_str() != "tacsim"
                || root.digest().as_bytes() == &[0; 32]
                || self.fitted_units.iter().skip(i + 1).any(|r| r == root)
            {
                return Err(Error::Invalid);
            }
        }
        bounded(self, SUBJECT_BYTES)?;
        Ok(())
    }
    pub fn price(&self) -> Amount {
        Amount::from_minor(FITTING_MINOR)
    }
    pub fn digest(&self) -> Result<DefinitionDigest, Error> {
        self.validate()?;
        digest(self, SUBJECT_BYTES)
    }
    pub fn target(&self, class: CoreClass) -> Result<&DefinitionRef, Error> {
        self.fitted_units
            .get(class.catalog_index())
            .ok_or(Error::Invalid)
    }
    pub fn action_descriptor(
        &self,
        action: &str,
    ) -> Result<mush_core::DefinitionDescriptor, Error> {
        self.validate()?;
        if action == super::INSTALL_ACTION {
            return mush_core::core_exchange::action_descriptor().map_err(|_| Error::Invalid);
        }
        let kind = mush_core::DefinitionKind::new("action").map_err(|_| Error::Invalid)?;
        let id = mush_core::DefinitionId::new(action).map_err(|_| Error::Invalid)?;
        if action == super::CANCEL_ACTION {
            return mush_core::DefinitionDescriptor::new(kind,id,1,serde_json::json!({
                "format":1,"protocol":"certified_core_refit_cancel_v1",
                "authority":"actual_acknowledged_requester","effect":"release_exact_fee_and_slot",
                "current_recipe_required":false
            })).map_err(|_| Error::Invalid);
        }
        if !matches!(action, super::OFFER_ACTION | super::ACCEPT_ACTION) {
            return Err(Error::Invalid);
        }
        let mut refs = vec![self.recipe.clone()];
        refs.extend(self.fitted_units.iter().cloned());
        refs.sort();
        refs.dedup();
        mush_core::DefinitionDescriptor::new(kind,id,1,serde_json::json!({
            "format":1,"protocol":"certified_core_refit_v1","policy":self,
            "fee_minor":FITTING_MINOR,"reservation_ic_seconds":RESERVATION_SECONDS,
            "incoming":"delivered_certified_core","supplier_consent":"acknowledged_exact_offer_v1"
        })).map(|d|d.with_references(refs)).map_err(|_| Error::Invalid)
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RefitSubject {
    pub order: u64,
    pub intent: DefinitionDigest,
    pub offer: CommitToken,
    pub offer_digest: DefinitionDigest,
    pub requester: ObjId,
    pub payer: AccountId,
    /// Actual native manufacture acceptance; not a caller's catalogue label.
    pub core_acceptance: CommitToken,
    pub incoming: ObjId,
    pub hull: ObjId,
    pub unit_revision: u64,
    pub target: DefinitionRef,
    pub class: CoreClass,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub displaced: Option<ObjId>,
    pub policy: DefinitionDigest,
    pub accepted_at: ClockStamp,
    pub expires_at: ClockStamp,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    order: u64,
    intent: DefinitionDigest,
    offer: CommitToken,
    offer_digest: DefinitionDigest,
    requester: ObjId,
    payer: AccountId,
    core_acceptance: CommitToken,
    incoming: ObjId,
    hull: ObjId,
    unit_revision: u64,
    target: DefinitionRef,
    class: CoreClass,
    #[serde(default, deserialize_with = "present_object")]
    displaced: Option<ObjId>,
    policy: DefinitionDigest,
    accepted_at: ClockStamp,
    expires_at: ClockStamp,
}
fn present_object<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<ObjId>, D::Error> {
    ObjId::deserialize(d).map(Some)
}
impl<'de> Deserialize<'de> for RefitSubject {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = Wire::deserialize(d)?;
        let s = Self {
            order: w.order,
            intent: w.intent,
            offer: w.offer,
            offer_digest: w.offer_digest,
            requester: w.requester,
            payer: w.payer,
            core_acceptance: w.core_acceptance,
            incoming: w.incoming,
            hull: w.hull,
            unit_revision: w.unit_revision,
            target: w.target,
            class: w.class,
            displaced: w.displaced,
            policy: w.policy,
            accepted_at: w.accepted_at,
            expires_at: w.expires_at,
        };
        s.validate().map_err(serde::de::Error::custom)?;
        Ok(s)
    }
}
impl RefitSubject {
    pub fn validate(&self) -> Result<(), Error> {
        if self.intent.as_bytes() == &[0; 32]
            || self.offer_digest.as_bytes() == &[0; 32]
            || self.policy.as_bytes() == &[0; 32]
            || self.incoming == self.hull
            || [self.incoming, self.hull].contains(&self.requester)
            || self
                .displaced
                .is_some_and(|v| [self.incoming, self.hull, self.requester].contains(&v))
            || self.target.key().kind.as_str() != "unit"
            || self.target.key().owner.as_str() != "tacsim"
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
    pub(crate) fn matches_policy(&self, p: &RefitPaymentPolicy) -> Result<(), Error> {
        self.validate()?;
        p.validate()?;
        if self.policy != p.digest()?
            || &self.target != p.target(self.class)?
            || self.payer == p.provider.account
            || [self.incoming, self.hull, self.requester].contains(&p.factory)
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
