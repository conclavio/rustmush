//! Explicit host policy for new immutable-consent protocols. No default/TOML
//! switch or caller-supplied account/price is introduced.
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::{DefinitionRef, ModError};
use mush_economy::managed_entitlement::GrossRecipientCut;
use mush_economy::{Amount, InstrumentId};
use mush_roll::commerce::house_purchase::HouseProtocol;
use mush_roll::commerce::house_purchase::writer::{
    HousePurchaseWriter, listing_descriptor, purchase_descriptors,
};
use mush_roll::commerce::purchase::PurchaseParticipants;
use serde::Serialize;

use super::records::{Cut, Venue};

#[derive(Clone, Debug, Serialize)]
pub struct HousePolicy {
    pub price: Amount,
    pub instrument: InstrumentId,
    pub registry: GrossRecipientCut,
    pub venue_name: crate::MarketId,
    pub venue: GrossRecipientCut,
    pub transferable: bool,
    pub term_seconds: i64,
    pub grace_seconds: i64,
    pub quote_seconds: i64,
    pub renewal_price: Amount,
    pub renewal_term_seconds: i64,
    pub renewal_grace_seconds: i64,
}
impl HousePolicy {
    fn validate(&self) -> Result<(), ModError> {
        let cut = |cut: &GrossRecipientCut| {
            cut.permille <= 1000 && (cut.permille == 0) == cut.recipient.is_none()
        };
        if self.price.is_negative()
            || self.price.is_zero()
            || self.renewal_price.is_negative()
            || self.renewal_price.is_zero()
            || !cut(&self.registry)
            || !cut(&self.venue)
            || u32::from(self.registry.permille) + u32::from(self.venue.permille) > 1000
            || crate::MarketId::new(self.venue_name.as_str()).as_ref() != Some(&self.venue_name)
            || !(1..=mush_roll::commerce::MAX_TERM_SECONDS).contains(&self.term_seconds)
            || !(1..=mush_roll::commerce::MAX_TERM_SECONDS).contains(&self.renewal_term_seconds)
            || !(0..=mush_roll::commerce::MAX_GRACE_SECONDS).contains(&self.grace_seconds)
            || !(0..=mush_roll::commerce::MAX_GRACE_SECONDS).contains(&self.renewal_grace_seconds)
            || !(1..=600).contains(&self.quote_seconds)
        {
            return Err(error("invalid house host policy"));
        }
        super::codec::bounded(self, 8192).map_err(error)?;
        Ok(())
    }
    pub(super) fn venue(&self) -> Venue {
        Venue {
            name: self.venue_name.clone(),
            cut: Cut(self.venue.clone()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct HouseConfig {
    pub(super) policy: HousePolicy,
    pub(super) manifest: DefinitionRef,
    pub(super) participants: PurchaseParticipants,
    pub(super) writer: HousePurchaseWriter,
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub(super) fn descriptor(
    kind: &str,
    id: &str,
    version: u32,
    payload: serde_json::Value,
) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(error)?,
        DefinitionId::new(id).map_err(error)?,
        version,
        payload,
    )
    .map_err(error)
}
impl HouseConfig {
    pub fn new(
        policy: HousePolicy,
        manifest: DefinitionRef,
        participants: PurchaseParticipants,
        writer: HousePurchaseWriter,
    ) -> Result<Self, ModError> {
        policy.validate()?;
        purchase_descriptors(HouseProtocol::HouseV1, &manifest, &participants)?;
        Ok(Self {
            policy,
            manifest,
            participants,
            writer,
        })
    }
    pub fn participants(&self) -> &PurchaseParticipants {
        &self.participants
    }
    pub fn writer(&self) -> HousePurchaseWriter {
        self.writer.clone()
    }
    pub fn manifest(&self) -> &DefinitionRef {
        &self.manifest
    }
    pub(super) fn policy_descriptor(&self) -> Result<DefinitionDescriptor, ModError> {
        Ok(descriptor(
            "entitlement_policy",
            "house_personal",
            1,
            serde_json::to_value(&self.policy).map_err(error)?,
        )?
        .with_references(vec![self.manifest.clone()]))
    }
    pub fn policy_ref(&self) -> Result<DefinitionRef, ModError> {
        self.reference(self.policy_descriptor()?)
    }
    fn reference(&self, d: DefinitionDescriptor) -> Result<DefinitionRef, ModError> {
        DefinitionRegistryBuilder::new()
            .register(OwnerId::new(crate::NS).map_err(error)?, d)
            .map_err(error)
    }
    /// Exact [list,fund,grant,refund] identities passed to the private Roll
    /// writer. Quote is typed-only and receives no owner-participant grant.
    pub fn owner_actions(&self, protocol: HouseProtocol) -> Result<[DefinitionRef; 4], ModError> {
        let [fund, grant, refund] =
            purchase_descriptors(protocol, &self.manifest, &self.participants)?;
        Ok([
            self.reference(listing_descriptor(
                protocol,
                &self.manifest,
                &self.participants,
            )?)?,
            self.reference(fund)?,
            self.reference(grant)?,
            self.reference(refund)?,
        ])
    }
    pub fn purchase_actions(
        &self,
        protocol: HouseProtocol,
    ) -> Result<[DefinitionRef; 3], ModError> {
        let [_, fund, grant, refund] = self.owner_actions(protocol)?;
        Ok([fund, grant, refund])
    }
    pub(super) fn quote_descriptor(
        &self,
        protocol: HouseProtocol,
    ) -> Result<DefinitionDescriptor, ModError> {
        Ok(descriptor("action","quote_house_mind",1,serde_json::json!({"format":1,"protocol":"house_mind_purchase_v1","transition":"quote","funding":false}))?.with_references(vec![self.manifest.clone(),self.policy_ref()?]))
    }
    pub fn quote_action(&self, protocol: HouseProtocol) -> Result<DefinitionRef, ModError> {
        self.reference(self.quote_descriptor(protocol)?)
    }
    pub(super) fn outcome_descriptor(&self) -> Result<DefinitionDescriptor, ModError> {
        descriptor(
            "outcome",
            "mind_house_recorded",
            1,
            serde_json::json!({"format":1,"fields":["kind","operation","commit"]}),
        )
    }
    pub(crate) fn declarations(&self) -> Result<Vec<DefinitionDescriptor>, ModError> {
        let mut out = vec![self.policy_descriptor()?, self.outcome_descriptor()?];
        for protocol in [HouseProtocol::HouseV1] {
            out.push(listing_descriptor(
                protocol,
                &self.manifest,
                &self.participants,
            )?);
            out.push(self.quote_descriptor(protocol)?);
            out.extend(purchase_descriptors(
                protocol,
                &self.manifest,
                &self.participants,
            )?);
        }
        Ok(out)
    }
}

impl HouseConfig {
    pub(super) fn product(
        &self,
        f: &mush_roll::commerce::house_catalogue::HouseSaleFacts,
    ) -> Result<super::HouseProductV1, String> {
        use mush_roll::commerce::house_purchase::{HouseGrantEconomicsV1, HouseRenewalTemplateV1};
        let terms =
            mush_roll::commerce::house_purchase::writer::HousePurchaseTerms::personal_terms(
                f.minimum(),
                self.policy.transferable,
            )
            .map_err(|e| e.to_string())?;
        let revision = mush_roll::commerce::records::RevisionPolicy::Pin {
            filed: f.filed(),
            revision: f.revision(),
        };
        let renewal = HouseRenewalTemplateV1::new(
            self.policy.instrument.clone(),
            self.policy.renewal_price,
            self.policy.renewal_term_seconds,
            self.policy.renewal_grace_seconds,
            terms,
            revision,
            self.policy_ref().map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let product = super::HouseProductV1 {
            protocol: HouseProtocol::HouseV1,
            terms,
            revision,
            term_seconds: self.policy.term_seconds,
            grace_seconds: self.policy.grace_seconds,
            grant: HouseGrantEconomicsV1::new(renewal).map_err(|e| e.to_string())?,
        };
        product.matches_subject(&super::HouseSubjectV1::capture(f))?;
        Ok(product)
    }
}
