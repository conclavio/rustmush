//! Strict retained owner data. Decoding never grants a refit or Tactics write.
use super::{Error, PublishedRefitRecipe, bad, progress::RefitProgress};
use crate::{facility::FacilityRecord, workorder::WorkOrderId};
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use mush_economy::{
    AccountId,
    core_integration::CoreClass,
    core_refit::{
        RefitFundingReceipt, RefitPaymentHistory, RefitPaymentPolicy, RefitSubject,
        RefitTerminalReceipt, RefitTerminalResult,
    },
};
use mush_tactics::unit::replacement::{ComponentState, ReplacementJournal};
use mush_tactics::unit::{CapabilityId, CellId, InstalledComponent, RulesValue, ScalarCell};
use mush_tactics::{ComponentId, TacticalUnit};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(crate) const MAX_ORDERS: u64 = 8192;
pub(crate) const OFFER_BYTES: usize = 32768;
pub(crate) const ORDER_BYTES: usize = 196608;
pub(crate) const INSTALLATION_BYTES: usize = 69632;
pub(crate) const INDEX_BYTES: usize = 4096;
pub(crate) const ASSET_BYTES: usize = 32768;
pub(crate) const INDEX_KEY: &str = "core_refit_registry";
pub(crate) const OFFER_SECONDS: i64 = 300;
pub(crate) use crate::core_production::integration::records::{decode, digest, encode};

/// Exact public projection of all component fields. It is deliberately not
/// convertible to a ComponentState authority; later damaged reinstallation is
/// a separate policy version. All maps retain their canonical typed keys.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SavedComponent {
    pub(crate) installed: InstalledComponent,
    cells: BTreeMap<CellId, ScalarCell>,
    capabilities: BTreeMap<CapabilityId, RulesValue>,
}
impl SavedComponent {
    pub(crate) fn capture(value: &ComponentState) -> Self {
        Self {
            installed: value.installed().clone(),
            cells: value.cells().clone(),
            capabilities: value.capabilities().clone(),
        }
    }
    pub(crate) fn from_unit(unit: &TacticalUnit, component: &ComponentId) -> Result<Self, Error> {
        Ok(Self::capture(
            &ComponentState::capture(unit, component).map_err(bad)?,
        ))
    }
    fn validate(&self) -> Result<(), Error> {
        // The actual registered Tactics producer validates semantic ranges.
        // This outer shape is bounded before public strict decoder allocation.
        encode(self, ASSET_BYTES).map_err(bad)?;
        if self
            .cells
            .values()
            .any(|v| v.min > v.max || v.current < v.min || v.current > v.max)
        {
            return Err(Error::Integrity);
        }
        Ok(())
    }
}

/// Time is joined separately to the actual terminal receipt. Excluding it
/// here keeps the semantic image stable across monotonic custody rechecks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplacementEvidence {
    pub(crate) object: ObjId,
    pub(crate) from_revision: u64,
    pub(crate) to_revision: u64,
    pub(crate) component: ComponentId,
    pub(crate) before: DefinitionRef,
    pub(crate) after: DefinitionRef,
    pub(crate) displaced: SavedComponent,
    pub(crate) inserted: SavedComponent,
}
impl ReplacementEvidence {
    pub(crate) fn capture(j: &ReplacementJournal) -> Self {
        Self {
            object: j.object,
            from_revision: j.from_revision,
            to_revision: j.to_revision,
            component: j.component.clone(),
            before: j.before.clone(),
            after: j.after.clone(),
            displaced: SavedComponent::capture(&j.displaced),
            inserted: SavedComponent::capture(&j.inserted),
        }
    }
    pub(crate) fn digest(&self) -> Result<DefinitionDigest, Error> {
        self.displaced.validate()?;
        self.inserted.validate()?;
        if self.from_revision.checked_add(1) != Some(self.to_revision)
            || self.inserted.installed.asset.is_none()
        {
            return Err(Error::Integrity);
        }
        digest("industry.core_refit.replacement.v1", self, ORDER_BYTES)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RefitOffer {
    pub(crate) format: u16,
    pub(crate) sequence: u64,
    pub(crate) operation: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) operator: ObjId,
    pub(crate) requester: ObjId,
    pub(crate) policy: RefitPaymentPolicy,
    pub(crate) facility: FacilityRecord,
    pub(crate) recipe: PublishedRefitRecipe,
    pub(crate) core_acceptance: CommitToken,
    pub(crate) incoming: ObjId,
    pub(crate) hull: ObjId,
    pub(crate) unit_revision: u64,
    pub(crate) before: DefinitionRef,
    pub(crate) component: ComponentId,
    pub(crate) displaced: SavedComponent,
    pub(crate) observed_ic: ClockStamp,
    pub(crate) observed_utc: i64,
    pub(crate) expires_utc: i64,
}
impl RefitOffer {
    pub(crate) fn class(&self) -> CoreClass {
        match self.recipe.class() {
            crate::core_production::DieClass::I => CoreClass::I,
            crate::core_production::DieClass::II => CoreClass::II,
            crate::core_production::DieClass::III => CoreClass::III,
        }
    }
    pub(crate) fn validate(&self) -> Result<(), Error> {
        self.policy.validate().map_err(bad)?;
        self.recipe.validate()?;
        self.displaced.validate()?;
        if self.format != 1
            || !(1..=MAX_ORDERS).contains(&self.sequence)
            || self.action != self.policy.action_ref(super::OFFER).map_err(bad)?
            || self.facility.object() != self.policy.factory
            || self.facility.owner()
                != &mush_org::PrincipalId::Organization(self.policy.provider.organization)
            || !self
                .facility
                .capabilities()
                .iter()
                .any(|c| c.as_str() == "core_refit")
            || self.recipe.definition() != &self.policy.recipe
            || !self.policy.fitted_units.contains(&self.before)
            || self.unit_revision == u64::MAX
            || [self.hull, self.incoming].contains(&self.requester)
            || self.hull == self.incoming
            || [self.requester, self.hull, self.incoming].contains(&self.policy.factory)
            || self.displaced.installed.asset.is_some_and(|a| {
                [
                    self.requester,
                    self.hull,
                    self.incoming,
                    self.policy.factory,
                ]
                .contains(&a)
            })
            || self.observed_utc.checked_add(OFFER_SECONDS) != Some(self.expires_utc)
            || !matches!(self.observed_ic.clock,ClockRef::WorldIc{generation}if generation>0)
        {
            return Err(Error::Integrity);
        }
        encode(self, OFFER_BYTES).map_err(bad)?;
        Ok(())
    }
    pub(crate) fn digest(&self) -> Result<DefinitionDigest, Error> {
        self.validate()?;
        digest("industry.core_refit.offer.v1", self, OFFER_BYTES)
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AcceptedIntent {
    pub(crate) order: WorkOrderId,
    pub(crate) offer: RefitOffer,
    pub(crate) payer: AccountId,
    pub(crate) accepted_at: ClockStamp,
    pub(crate) expires_at: ClockStamp,
}
impl AcceptedIntent {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        self.offer.validate()?;
        if self.accepted_at.clock != self.offer.observed_ic.clock
            || self.accepted_at.at < self.offer.observed_ic.at
            || self.payer == self.offer.policy.provider.account
            || self.expires_at
                != mush_inventory::reservation_deadline(&self.accepted_at).map_err(bad)?
        {
            return Err(Error::Integrity);
        }
        encode(self, OFFER_BYTES + 2048).map_err(bad)?;
        Ok(())
    }
    pub(crate) fn subject(&self) -> Result<RefitSubject, Error> {
        self.validate()?;
        let o = &self.offer;
        let s = RefitSubject {
            order: self.order.0,
            intent: digest("industry.core_refit.intent.v1", self, OFFER_BYTES + 2048)?,
            offer: o.operation,
            offer_digest: o.digest()?,
            requester: o.requester,
            payer: self.payer,
            core_acceptance: o.core_acceptance,
            incoming: o.incoming,
            hull: o.hull,
            unit_revision: o.unit_revision,
            target: o.policy.target(o.class()).map_err(bad)?.clone(),
            class: o.class(),
            displaced: o.displaced.installed.asset,
            policy: o.policy.digest().map_err(bad)?,
            accepted_at: self.accepted_at.clone(),
            expires_at: self.expires_at.clone(),
        };
        s.validate().map_err(bad)?;
        Ok(s)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Installation {
    pub(crate) replacement: ReplacementEvidence,
    pub(crate) core_receipt: DefinitionDigest,
    pub(crate) at_utc: i64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RefitOrder {
    pub(crate) format: u16,
    pub(crate) sequence: u64,
    pub(crate) accepted: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) intent: AcceptedIntent,
    pub(crate) funding: RefitFundingReceipt,
    pub(crate) capacity: mush_core::ReservationId,
    pub(crate) progress: RefitProgress,
    #[serde(deserialize_with = "required_option")]
    pub(crate) terminal: Option<RefitTerminalReceipt>,
    #[serde(deserialize_with = "required_option")]
    pub(crate) installation: Option<Installation>,
}
fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}
impl RefitOrder {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        let s = self.intent.subject()?;
        let o = &self.intent.offer;
        self.progress.validate()?;
        if self.format != 1
            || !(1..=MAX_ORDERS).contains(&self.sequence)
            || self.action != o.policy.action_ref(super::ACCEPT).map_err(bad)?
            || self.funding.commit() != self.accepted
            || self.funding.action() != &self.action
            || self.funding.posted_at_utc() < o.observed_utc
            || self.funding.posted_at_utc() >= o.expires_utc
            || self.progress.order() != self.intent.order
            || self.progress.production().started() != &self.intent.accepted_at
            || self.progress.production().total_units() != o.recipe.work_units()
        {
            return Err(Error::Integrity);
        }
        match (&self.terminal, &self.installation) {
            (None, None) => {}
            (Some(t), None) if matches!(t.result(), RefitTerminalResult::Cancelled {}) => {
                if t.commit() == self.accepted
                    || t.action() != &o.policy.action_ref(super::CANCEL).map_err(bad)?
                {
                    return Err(Error::Integrity);
                }
            }
            (Some(t), Some(i)) => {
                encode(i, INSTALLATION_BYTES).map_err(bad)?;
                let r = &i.replacement;
                if !self.progress.production().is_complete()
                    || t.result().replacement() != Some(r.digest()?)
                    || t.action() != &o.policy.action_ref(super::INSTALL).map_err(bad)?
                    || t.commit() == self.accepted
                    || t.at_ic().clock != s.accepted_at.clock
                    || t.at_ic().at < s.accepted_at.at
                    || t.at_ic().at >= s.expires_at.at
                    || i.at_utc != t.posted_at_utc()
                    || i.core_receipt.as_bytes() == &[0; 32]
                    || r.object != o.hull
                    || r.from_revision != o.unit_revision
                    || r.before != o.before
                    || r.after != s.target
                    || r.component != o.component
                    || r.displaced != o.displaced
                    || r.inserted.installed.asset != Some(o.incoming)
                {
                    return Err(Error::Integrity);
                }
            }
            _ => return Err(Error::Integrity),
        }
        encode(self, ORDER_BYTES).map_err(bad)?;
        Ok(())
    }
    pub(crate) fn verify_payment(&self, h: &RefitPaymentHistory) -> Result<(), Error> {
        self.validate()?;
        if h.subject() != &self.intent.subject()?
            || h.policy() != &self.intent.offer.policy
            || h.funding() != &self.funding
            || h.terminal() != self.terminal.as_ref()
        {
            return Err(Error::Integrity);
        }
        Ok(())
    }
    pub(crate) fn install(
        &self,
        economy: &mush_economy::core_refit::RefitTerminalOutput,
        tactics: &mush_tactics::component_replacement::ReplacementOutput,
        core: &mush_core::core_exchange::ExchangedCoreComponent,
    ) -> Result<Self, Error> {
        if self.terminal.is_some() || economy.subject() != &self.intent.subject()? {
            return Err(Error::Integrity);
        }
        let t = economy.receipt();
        let j = tactics.journal();
        let o = &self.intent.offer;
        if tactics.commit() != t.commit()
            || tactics.action() != t.action()
            || tactics.actor() != o.requester
            || j.now != t.posted_at_utc()
            || core.commit() != t.commit()
            || core.action() != t.action()
            || core.order() != self.intent.order.0
            || core.requester() != o.requester
            || core.hull() != o.hull
            || core.unit_revision() != o.unit_revision
            || core.incoming() != o.incoming
            || core.source() != o.requester
            || core.displaced() != o.displaced.installed.asset.map(|a| (a, o.requester))
            || core.at_utc() != t.posted_at_utc()
        {
            return Err(Error::Integrity);
        }
        let row = Self {
            terminal: Some(t.clone()),
            installation: Some(Installation {
                replacement: ReplacementEvidence::capture(j),
                core_receipt: core.digest().map_err(bad)?,
                at_utc: j.now,
            }),
            ..self.clone()
        };
        row.validate()?;
        Ok(row)
    }
    pub(crate) fn matches_core(
        &self,
        core: &mush_core::core_exchange::ExchangedCoreComponent,
    ) -> Result<(), Error> {
        self.validate()?;
        let t = self.terminal.as_ref().ok_or(Error::Integrity)?;
        let i = self.installation.as_ref().ok_or(Error::Integrity)?;
        let o = &self.intent.offer;
        if core.commit() != t.commit()
            || core.action() != t.action()
            || core.order() != self.intent.order.0
            || core.requester() != o.requester
            || core.hull() != o.hull
            || core.unit_revision() != o.unit_revision
            || core.incoming() != o.incoming
            || core.source() != o.requester
            || core.displaced() != o.displaced.installed.asset.map(|a| (a, o.requester))
            || core.at_utc() != i.at_utc
            || core.digest().map_err(bad)? != i.core_receipt
        {
            return Err(Error::Integrity);
        }
        Ok(())
    }
}
/// Independent non-growing admission census, written by the real Industry
/// owner at Initialize/Offer/Accept. Install intentionally uses only E4+Tactics.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RefitIndex {
    format: u16,
    pub(crate) initialized: CommitToken,
    pub(crate) policy: RefitPaymentPolicy,
    pub(crate) offers: u64,
    pub(crate) accepted: u64,
    offer_chain: DefinitionDigest,
    order_chain: DefinitionDigest,
}
impl RefitIndex {
    pub(crate) fn new(initialized: CommitToken, policy: RefitPaymentPolicy) -> Result<Self, Error> {
        policy.validate().map_err(bad)?;
        let chain = digest(
            "industry.core_refit.genesis.v1",
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
        self.policy.validate().map_err(bad)?;
        if self.format != 1
            || self.offers > MAX_ORDERS
            || self.accepted > self.offers
            || self.offer_chain.as_bytes() == &[0; 32]
            || self.order_chain.as_bytes() == &[0; 32]
        {
            return Err(Error::Integrity);
        }
        encode(self, INDEX_BYTES).map_err(bad)?;
        Ok(())
    }
    pub(crate) fn genesis(&self) -> Result<Self, Error> {
        Self::new(self.initialized, self.policy.clone())
    }
    pub(crate) fn offer(&self, row: &RefitOffer) -> Result<Self, Error> {
        self.validate()?;
        row.validate()?;
        if self.offers.checked_add(1) != Some(row.sequence) || self.policy != row.policy {
            return Err(Error::Integrity);
        }
        let mut n = self.clone();
        n.offers = row.sequence;
        n.offer_chain = digest(
            "industry.core_refit.offers.v1",
            &serde_json::json!({"prior":self.offer_chain,"row":row.digest()?}),
            INDEX_BYTES,
        )?;
        n.validate()?;
        Ok(n)
    }
    pub(crate) fn accept(&self, row: &RefitOrder) -> Result<Self, Error> {
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
            "industry.core_refit.orders.v1",
            &serde_json::json!({"prior":self.order_chain,"accepted":row.accepted,"intent":row.intent.subject()?.intent,"capacity":row.capacity,"funding":row.funding}),
            INDEX_BYTES,
        )?;
        n.validate()?;
        Ok(n)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AssetPosition {
    Pending {
        order: WorkOrderId,
    },
    Mounted {
        order: WorkOrderId,
        hull: ObjId,
    },
    Released {
        order: WorkOrderId,
    },
    Detached {
        order: WorkOrderId,
        from_hull: ObjId,
        condition: DefinitionDigest,
    },
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AssetHead {
    pub(crate) format: u16,
    pub(crate) object: ObjId,
    pub(crate) operation: CommitToken,
    pub(crate) position: AssetPosition,
}
impl AssetHead {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        if self.format != 1
            || matches!(&self.position,AssetPosition::Mounted{hull,..}if *hull==self.object)
            || matches!(&self.position,AssetPosition::Detached{from_hull,condition,..}if *from_hull==self.object||condition.as_bytes()==&[0;32])
        {
            return Err(Error::Integrity);
        }
        encode(self, 4096).map_err(bad)?;
        Ok(())
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HullHead {
    pub(crate) format: u16,
    pub(crate) hull: ObjId,
    pub(crate) operation: CommitToken,
    #[serde(deserialize_with = "required_option")]
    pub(crate) pending: Option<WorkOrderId>,
    #[serde(deserialize_with = "required_option")]
    pub(crate) mounted: Option<ObjId>,
}
impl HullHead {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        if self.format != 1 || self.mounted == Some(self.hull) {
            return Err(Error::Integrity);
        }
        encode(self, 4096).map_err(bad)?;
        Ok(())
    }
}
