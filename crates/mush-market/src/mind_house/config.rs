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
    pub(super) two_seat_policy: Option<HousePolicy>,
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
            two_seat_policy: None,
            manifest,
            participants,
            writer,
        })
    }
    /// Add the exact two-seat sealed Pin while retaining every one-seat route.
    pub fn with_two_seat_pins(mut self, policy: HousePolicy) -> Result<Self, ModError> {
        if self.two_seat_policy.is_some() {
            return Err(error("two-seat House pins already configured"));
        }
        policy.validate()?;
        self.two_seat_policy = Some(policy);
        self.owner_actions(HouseProtocol::HouseV2)?;
        Ok(self)
    }
    pub fn has_two_seat_pins(&self) -> bool {
        self.two_seat_policy.is_some()
    }
    pub(super) fn protocols(&self) -> impl Iterator<Item = HouseProtocol> {
        std::iter::once(HouseProtocol::HouseV1).chain(
            self.two_seat_policy
                .as_ref()
                .map(|_| HouseProtocol::HouseV2),
        )
    }
    pub(super) fn policy_for_protocol(
        &self,
        protocol: HouseProtocol,
    ) -> Result<&HousePolicy, ModError> {
        match protocol {
            HouseProtocol::HouseV1 => Ok(&self.policy),
            HouseProtocol::HouseV2 => self
                .two_seat_policy
                .as_ref()
                .ok_or_else(|| error("two-seat House pins are not configured")),
        }
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
    pub(super) fn policy_descriptor_for_protocol(
        &self,
        protocol: HouseProtocol,
    ) -> Result<DefinitionDescriptor, ModError> {
        let policy = self.policy_for_protocol(protocol)?;
        let payload = match protocol {
            HouseProtocol::HouseV1 => serde_json::to_value(policy).map_err(error)?,
            HouseProtocol::HouseV2 => serde_json::json!({"format":2,"seats":2,"policy":policy}),
        };
        Ok(descriptor(
            "entitlement_policy",
            "house_personal",
            protocol.version(),
            payload,
        )?
        .with_references(vec![self.manifest.clone()]))
    }
    pub fn policy_ref(&self) -> Result<DefinitionRef, ModError> {
        self.policy_ref_for_protocol(HouseProtocol::HouseV1)
    }
    pub fn policy_ref_for_protocol(
        &self,
        protocol: HouseProtocol,
    ) -> Result<DefinitionRef, ModError> {
        self.reference(self.policy_descriptor_for_protocol(protocol)?)
    }
    fn reference(&self, d: DefinitionDescriptor) -> Result<DefinitionRef, ModError> {
        DefinitionRegistryBuilder::new()
            .register(OwnerId::new(crate::NS).map_err(error)?, d)
            .map_err(error)
    }
    /// Exact `[list,fund,grant,refund]` identities passed to the private Roll
    /// writer. Quote is typed-only and receives no owner-participant grant.
    pub fn owner_actions(&self, protocol: HouseProtocol) -> Result<[DefinitionRef; 4], ModError> {
        self.policy_for_protocol(protocol)?;
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
        Ok(descriptor("action","quote_house_mind",protocol.version(),serde_json::json!({"format":protocol.format(),"protocol":protocol.protocol_name(),"transition":"quote","funding":false}))?.with_references(vec![self.manifest.clone(),self.policy_ref_for_protocol(protocol)?]))
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
        let mut out = vec![self.outcome_descriptor()?];
        for protocol in self.protocols() {
            out.push(self.policy_descriptor_for_protocol(protocol)?);
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
    pub(super) fn product_for_protocol(
        &self,
        protocol: HouseProtocol,
        f: &mush_roll::commerce::house_catalogue::HouseSaleFacts,
    ) -> Result<super::HouseProductV1, String> {
        let policy = self
            .policy_for_protocol(protocol)
            .map_err(|e| e.to_string())?;
        use mush_roll::commerce::house_purchase::{HouseGrantEconomicsV1, HouseRenewalTemplateV1};
        let terms =
            mush_roll::commerce::house_purchase::writer::HousePurchaseTerms::personal_terms_for_protocol(
                protocol,
                f.minimum(),
                policy.transferable,
            )
            .map_err(|e| e.to_string())?;
        let revision = mush_roll::commerce::records::RevisionPolicy::Pin {
            filed: f.filed(),
            revision: f.revision(),
        };
        let renewal = HouseRenewalTemplateV1::for_protocol(
            protocol,
            policy.instrument.clone(),
            policy.renewal_price,
            policy.renewal_term_seconds,
            policy.renewal_grace_seconds,
            terms,
            revision,
            self.policy_ref_for_protocol(protocol)
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let product = super::HouseProductV1 {
            protocol,
            terms,
            revision,
            term_seconds: policy.term_seconds,
            grace_seconds: policy.grace_seconds,
            grant: HouseGrantEconomicsV1::new(renewal).map_err(|e| e.to_string())?,
        };
        product.matches_subject(&super::HouseSubjectV1::capture(f))?;
        Ok(product)
    }
}
