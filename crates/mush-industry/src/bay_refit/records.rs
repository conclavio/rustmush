//! Strict retained bay owner data, in the exact shape the kernel's
//! `bay_refit` protocol joins. Decoding never grants a refit, a fee or a part.
use super::progress::BayProgress;
use super::{BayRefitError as Error, BayRefitPolicy, OFFER_SECONDS};
use crate::core_production::refit::records::SavedComponent;
use crate::workorder::WorkOrderId;
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use mush_economy::AccountId;
use mush_economy::bay_refit::{
    BayFundingReceipt, BayPaymentHistory, BayPaymentPolicy, BaySubject, BayTerminalReceipt,
    BayTerminalResult,
};
use mush_inventory::{EndpointRef, LotId};
use mush_tactics::ComponentId;
use mush_tactics::unit::replacement::RefitJournal;
use serde::{Deserialize, Serialize};

pub(crate) const MAX_ORDERS: u64 = 8192;
pub(crate) const OFFER_BYTES: usize = 32768;
pub(crate) const ORDER_BYTES: usize = 196_608;
pub(crate) const INSTALLATION_BYTES: usize = 69632;
pub(crate) const INDEX_BYTES: usize = 4096;
/// The admission census's module key: the owner participant's native image.
pub(crate) const INDEX_KEY: &str = "bay_refit_registry";

fn bad<T>(_: T) -> Error {
    Error::Integrity
}
pub(crate) fn encode<T: Serialize>(r: &T, limit: usize) -> Result<serde_json::Value, Error> {
    let v = serde_json::to_value(r).map_err(bad)?;
    mush_core::canonical_json_bytes_with_limit(&v, limit).map_err(bad)?;
    Ok(v)
}
pub(crate) fn decode<T: Serialize + serde::de::DeserializeOwned>(
    v: &serde_json::Value,
    limit: usize,
) -> Result<T, Error> {
    mush_core::canonical_json_bytes_with_limit(v, limit).map_err(bad)?;
    let r: T = serde_json::from_value(v.clone()).map_err(bad)?;
    if serde_json::to_value(&r).map_err(bad)? != *v {
        return Err(Error::Integrity);
    }
    Ok(r)
}
fn digest<T: Serialize>(domain: &str, v: &T, limit: usize) -> Result<DefinitionDigest, Error> {
    DefinitionDigest::for_canonical_value_with_limit(domain, &encode(v, limit)?, limit).map_err(bad)
}
fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}

/// The bay's recipe terms for one offer: how many work units it takes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BayRecipe {
    pub(crate) work_units: u64,
}

/// **A supplier's acknowledged offer** to refit one hull at one bay: what
/// comes out and what goes in, the lot the part comes from and the lot the
/// part taken out becomes, the work and the quoted fee. It stands for
/// [`OFFER_SECONDS`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BayOffer {
    pub(crate) format: u16,
    pub(crate) sequence: u64,
    pub(crate) operation: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) operator: ObjId,
    pub(crate) requester: ObjId,
    pub(crate) policy: BayRefitPolicy,
    pub(crate) facility: ObjId,
    pub(crate) stores: EndpointRef,
    pub(crate) hull: ObjId,
    pub(crate) unit_revision: u64,
    pub(crate) before: DefinitionRef,
    pub(crate) target: DefinitionRef,
    #[serde(deserialize_with = "required_option")]
    pub(crate) remove: Option<ComponentId>,
    #[serde(deserialize_with = "required_option")]
    pub(crate) install: Option<ComponentId>,
    #[serde(deserialize_with = "required_option")]
    pub(crate) part: Option<LotId>,
    #[serde(deserialize_with = "required_option")]
    pub(crate) returned: Option<LotId>,
    pub(crate) recipe: BayRecipe,
    pub(crate) fee_minor: i64,
    pub(crate) observed_ic: ClockStamp,
    pub(crate) observed_utc: i64,
    pub(crate) expires_utc: i64,
}
impl BayOffer {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        if self.format != 1
            || !(1..=MAX_ORDERS).contains(&self.sequence)
            || self.action != self.policy.action_ref(super::OFFER)?
            || !self.policy.facilities().contains(&self.facility)
            || (self.remove.is_none() && self.install.is_none())
            || self.part.is_some() != self.install.is_some()
            || self.returned.is_some() != self.remove.is_some()
            || self.part.is_some_and(|p| Some(p) == self.returned)
            || self.before == self.target
            || [self.requester, self.facility].contains(&self.hull)
            || self.requester == self.facility
            || self.unit_revision == u64::MAX
            || !(1..=super::MAX_WORK_UNITS).contains(&self.recipe.work_units)
            || self.fee_minor < 1
            || self.observed_utc.checked_add(OFFER_SECONDS) != Some(self.expires_utc)
            || !matches!(self.observed_ic.clock, ClockRef::WorldIc { generation } if generation > 0)
        {
            return Err(Error::Integrity);
        }
        encode(self, OFFER_BYTES)?;
        Ok(())
    }
    pub(crate) fn digest(&self) -> Result<DefinitionDigest, Error> {
        self.validate()?;
        digest("industry.bay_refit.offer.v1", self, OFFER_BYTES)
    }
    pub(crate) fn require_time(&self, at: &ClockStamp, utc: i64) -> Result<(), Error> {
        self.validate()?;
        if at.clock != self.observed_ic.clock
            || at.at < self.observed_ic.at
            || utc < self.observed_utc
            || utc >= self.expires_utc
        {
            return Err(Error::Expired);
        }
        Ok(())
    }
}

/// The requester's acceptance of one offer: the work order it opens, who
/// pays, and the seven-day reservation it holds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BayIntent {
    pub(crate) order: WorkOrderId,
    pub(crate) offer: BayOffer,
    pub(crate) payer: AccountId,
    pub(crate) accepted_at: ClockStamp,
    pub(crate) expires_at: ClockStamp,
}
impl BayIntent {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        self.offer.validate()?;
        if self.accepted_at.clock != self.offer.observed_ic.clock
            || self.accepted_at.at < self.offer.observed_ic.at
            || self.expires_at
                != mush_inventory::reservation_deadline(&self.accepted_at).map_err(bad)?
        {
            return Err(Error::Integrity);
        }
        encode(self, OFFER_BYTES + 2048)?;
        Ok(())
    }
    /// The Economy subject of this acceptance under `payment`.
    pub(crate) fn subject(&self, payment: &BayPaymentPolicy) -> Result<BaySubject, Error> {
        self.validate()?;
        let o = &self.offer;
        let s = BaySubject {
            order: self.order.0,
            intent: digest("industry.bay_refit.intent.v1", self, OFFER_BYTES + 2048)?,
            offer: o.operation,
            offer_digest: o.digest()?,
            requester: o.requester,
            payer: self.payer,
            hull: o.hull,
            unit_revision: o.unit_revision,
            before: o.before.clone(),
            target: o.target.clone(),
            facility: o.facility,
            fee_minor: o.fee_minor,
            policy: payment.digest().map_err(bad)?,
            accepted_at: self.accepted_at.clone(),
            expires_at: self.expires_at.clone(),
        };
        s.validate().map_err(bad)?;
        Ok(s)
    }
}

/// The refit as it happened, joined to the offer: the hull, its revision,
/// the roots before and after, and what came out and went in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RefitEvidence {
    pub(crate) object: ObjId,
    pub(crate) from_revision: u64,
    pub(crate) to_revision: u64,
    pub(crate) before: DefinitionRef,
    pub(crate) after: DefinitionRef,
    #[serde(deserialize_with = "required_option")]
    pub(crate) remove: Option<ComponentId>,
    #[serde(deserialize_with = "required_option")]
    pub(crate) install: Option<ComponentId>,
    #[serde(deserialize_with = "required_option")]
    pub(crate) removed: Option<SavedComponent>,
    #[serde(deserialize_with = "required_option")]
    pub(crate) inserted: Option<SavedComponent>,
}
impl RefitEvidence {
    pub(crate) fn capture(j: &RefitJournal) -> Self {
        Self {
            object: j.object,
            from_revision: j.from_revision,
            to_revision: j.to_revision,
            before: j.before.clone(),
            after: j.after.clone(),
            remove: j.removed.as_ref().map(|(c, _)| c.clone()),
            install: j.installed.as_ref().map(|(c, _)| c.clone()),
            removed: j.removed.as_ref().map(|(_, s)| SavedComponent::capture(s)),
            inserted: j
                .installed
                .as_ref()
                .map(|(_, s)| SavedComponent::capture(s)),
        }
    }
    /// The digest the kernel joins the installation under.
    pub(crate) fn digest(&self) -> Result<DefinitionDigest, Error> {
        if self.from_revision.checked_add(1) != Some(self.to_revision) {
            return Err(Error::Integrity);
        }
        super::digest(
            super::REFIT_DIGEST_DOMAIN,
            &encode(self, INSTALLATION_BYTES)?,
        )
        .map_err(bad)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BayInstallation {
    pub(crate) refit: RefitEvidence,
    pub(crate) participants: [DefinitionRef; 2],
    pub(crate) at_utc: i64,
}

/// **One accepted bay work order**, the kernel's sponsor row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BayOrder {
    pub(crate) format: u16,
    pub(crate) sequence: u64,
    pub(crate) accepted: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) intent: BayIntent,
    pub(crate) funding: BayFundingReceipt,
    pub(crate) capacity: mush_core::ReservationId,
    pub(crate) progress: BayProgress,
    #[serde(deserialize_with = "required_option")]
    pub(crate) terminal: Option<BayTerminalReceipt>,
    #[serde(deserialize_with = "required_option")]
    pub(crate) installation: Option<BayInstallation>,
}
impl BayOrder {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        self.intent.validate()?;
        self.progress.validate()?;
        let o = &self.intent.offer;
        if self.format != 1
            || !(1..=MAX_ORDERS).contains(&self.sequence)
            || self.action != o.policy.action_ref(super::ACCEPT)?
            || self.funding.commit() != self.accepted
            || self.funding.action() != &self.action
            || self.funding.posted_at_utc() < o.observed_utc
            || self.funding.posted_at_utc() >= o.expires_utc
            || self.progress.order() != self.intent.order
            || self.progress.production().started() != &self.intent.accepted_at
            || self.progress.production().total_units() != o.recipe.work_units
        {
            return Err(Error::Integrity);
        }
        match (&self.terminal, &self.installation) {
            (None, None) => {}
            (Some(t), None) if matches!(t.result(), BayTerminalResult::Cancelled {}) => {
                if t.commit() == self.accepted
                    || t.action() != &o.policy.action_ref(super::CANCEL)?
                {
                    return Err(Error::Integrity);
                }
            }
            (Some(t), Some(i)) => {
                encode(i, INSTALLATION_BYTES)?;
                let r = &i.refit;
                if !self.progress.production().is_complete()
                    || t.result().refit() != Some(r.digest()?)
                    || t.action() != &o.policy.action_ref(super::INSTALL)?
                    || t.commit() == self.accepted
                    || i.at_utc != t.posted_at_utc()
                    || r.object != o.hull
                    || r.from_revision != o.unit_revision
                    || r.before != o.before
                    || r.after != o.target
                    || r.remove != o.remove
                    || r.install != o.install
                    || r.removed.is_some() != o.remove.is_some()
                    || r.inserted.is_some() != o.install.is_some()
                {
                    return Err(Error::Integrity);
                }
            }
            _ => return Err(Error::Integrity),
        }
        encode(self, ORDER_BYTES)?;
        Ok(())
    }
    /// The Economy history must be exactly this row's funding and terminal.
    pub(crate) fn verify_payment(
        &self,
        h: &BayPaymentHistory,
        payment: &BayPaymentPolicy,
    ) -> Result<(), Error> {
        self.validate()?;
        if h.subject() != &self.intent.subject(payment)?
            || h.funding() != &self.funding
            || h.terminal() != self.terminal.as_ref()
        {
            return Err(Error::Integrity);
        }
        Ok(())
    }
}

/// **The admission census**: the bay's offers and accepted orders, chained
/// so neither can be rewritten. The owner participant's native image.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BayIndex {
    format: u16,
    pub(crate) initialized: CommitToken,
    pub(crate) policy: BayRefitPolicy,
    pub(crate) offers: u64,
    pub(crate) accepted: u64,
    offer_chain: DefinitionDigest,
    order_chain: DefinitionDigest,
}
impl BayIndex {
    pub(crate) fn new(initialized: CommitToken, policy: BayRefitPolicy) -> Result<Self, Error> {
        let chain = digest(
            "industry.bay_refit.genesis.v1",
            &serde_json::json!({"initialized":initialized,"policy":policy}),
            INDEX_BYTES,
        )?;
        Ok(Self {
            format: 1,
            initialized,
            policy,
            offers: 0,
            accepted: 0,
            offer_chain: chain,
            order_chain: chain,
        })
    }
    pub(crate) fn validate(&self) -> Result<(), Error> {
        if self.format != 1
            || self.offers > MAX_ORDERS
            || self.accepted > self.offers
            || self.offer_chain.as_bytes() == &[0; 32]
            || self.order_chain.as_bytes() == &[0; 32]
        {
            return Err(Error::Integrity);
        }
        encode(self, INDEX_BYTES)?;
        Ok(())
    }
    pub(crate) fn offer(&self, row: &BayOffer) -> Result<Self, Error> {
        self.validate()?;
        row.validate()?;
        if self.offers.checked_add(1) != Some(row.sequence) || self.policy != row.policy {
            return Err(Error::Integrity);
        }
        let mut n = self.clone();
        n.offers = row.sequence;
        n.offer_chain = digest(
            "industry.bay_refit.offers.v1",
            &serde_json::json!({"prior":self.offer_chain,"row":row.digest()?}),
            INDEX_BYTES,
        )?;
        Ok(n)
    }
    pub(crate) fn accept(&self, row: &BayOrder) -> Result<Self, Error> {
        self.validate()?;
        row.validate()?;
        if self.accepted.checked_add(1) != Some(row.sequence)
            || self.policy != row.intent.offer.policy
            || row.terminal.is_some()
        {
            return Err(Error::Integrity);
        }
        let mut n = self.clone();
        n.accepted = row.sequence;
        n.order_chain = digest(
            "industry.bay_refit.orders.v1",
            &serde_json::json!({"prior":self.order_chain,"accepted":row.accepted,"intent":row.intent.offer.digest()?,"capacity":row.capacity,"funding":row.funding}),
            INDEX_BYTES,
        )?;
        Ok(n)
    }
}
