//! Explicit host policy for new immutable-consent protocols. No default/TOML
//! switch or caller-supplied account/price is introduced.
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::{DefinitionRef, ModError};
use mush_economy::managed_entitlement::{EntitlementProtocol, GrossRecipientCut};
use mush_economy::{Amount, InstrumentId};
use mush_roll::commerce::entitlement::writer::{
    EntitlementPurchaseWriter, listing_descriptor, purchase_descriptors,
};
use mush_roll::commerce::purchase::PurchaseParticipants;
use serde::Serialize;

use super::records::{Cut, Venue};

#[derive(Clone, Debug, Serialize)]
pub struct EntitlementPolicy {
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
impl EntitlementPolicy {
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
            return Err(error("invalid entitlement host policy"));
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
pub struct EntitlementConfig {
    pub(super) policy: EntitlementPolicy,
    pub(super) manifest: DefinitionRef,
    pub(super) participants: PurchaseParticipants,
    pub(super) writer: EntitlementPurchaseWriter,
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
impl EntitlementConfig {
    pub fn new(
        policy: EntitlementPolicy,
        manifest: DefinitionRef,
        participants: PurchaseParticipants,
        writer: EntitlementPurchaseWriter,
    ) -> Result<Self, ModError> {
        policy.validate()?;
        purchase_descriptors(EntitlementProtocol::OriginalV3, &manifest, &participants)?;
        purchase_descriptors(EntitlementProtocol::DerivedV1, &manifest, &participants)?;
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
    pub fn writer(&self) -> EntitlementPurchaseWriter {
        self.writer.clone()
    }
    pub fn manifest(&self) -> &DefinitionRef {
        &self.manifest
    }
    pub(super) fn policy_descriptor(&self) -> Result<DefinitionDescriptor, ModError> {
        Ok(descriptor(
            "entitlement_policy",
            "personal",
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
    /// Exact `[list,fund,grant,refund]` identities passed to the private Roll
    /// writer. Quote is typed-only and receives no owner-participant grant.
    pub fn owner_actions(
        &self,
        protocol: EntitlementProtocol,
    ) -> Result<[DefinitionRef; 4], ModError> {
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
        protocol: EntitlementProtocol,
    ) -> Result<[DefinitionRef; 3], ModError> {
        let [_, fund, grant, refund] = self.owner_actions(protocol)?;
        Ok([fund, grant, refund])
    }
    pub(super) fn quote_descriptor(
        &self,
        protocol: EntitlementProtocol,
    ) -> Result<DefinitionDescriptor, ModError> {
        Ok(descriptor("action",match protocol {EntitlementProtocol::OriginalV3=>"quote_original_mind",EntitlementProtocol::DerivedV1=>"quote_derived_mind"},protocol.version(),serde_json::json!({"format":protocol.version(),"protocol":protocol.name(),"transition":"quote","funding":false}))?.with_references(vec![self.manifest.clone(),self.policy_ref()?]))
    }
    pub fn quote_action(&self, protocol: EntitlementProtocol) -> Result<DefinitionRef, ModError> {
        self.reference(self.quote_descriptor(protocol)?)
    }
    pub(super) fn outcome_descriptor(&self) -> Result<DefinitionDescriptor, ModError> {
        descriptor(
            "outcome",
            "mind_entitlement_recorded",
            1,
            serde_json::json!({"format":1,"fields":["kind","operation","commit"]}),
        )
    }
    pub(crate) fn declarations(&self) -> Result<Vec<DefinitionDescriptor>, ModError> {
        let mut out = vec![self.policy_descriptor()?, self.outcome_descriptor()?];
        for protocol in [
            EntitlementProtocol::OriginalV3,
            EntitlementProtocol::DerivedV1,
        ] {
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

/// The author's product choice a listing is built from, and the same choice
/// read back from a listing's frozen product. 17g C2b (D-UM-40, C2 plan
/// §2.5): `transferable` is the author's per-listing mark. The List branch
/// builds it from the request (`offers::List::choice`); `current_listing`
/// rebuilds it from the listing's own product (`ProductChoice::frozen`), so a
/// marked listing recomputes to its own non-transferable terms and stays
/// current. One type for both sides keeps the two from drifting apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ProductChoice {
    pub(super) form: mush_roll::commerce::paid::PaidForm,
    pub(super) royalty_permille: u16,
    pub(super) transferable: bool,
}
impl ProductChoice {
    /// The choice a frozen listed product was built from: its form, its
    /// downstream royalty (zero when sealed) and its own transferability.
    pub(super) fn frozen(p: &super::EntitlementProductV1) -> Self {
        Self {
            form: p.terms.form(),
            royalty_permille: p.grant.downstream().royalty_permille().unwrap_or(0),
            transferable: p.terms.transferable(),
        }
    }
}

impl EntitlementConfig {
    /// The listed product's paid terms. 17g C2b (D-UM-40, C2 plan §2.5): the
    /// author's `transferable` mark only narrows the deployment policy
    /// (`policy && mark`): a mark of `true` under a `false` policy stays
    /// `false`. The renewal template carries these same terms
    /// (`EntitlementProductV1::validate`), so a marked licence renews
    /// non-transferable, and the paid-resale reader refuses its resale.
    pub(super) fn terms(
        &self,
        minimum: mush_roll::commerce::rating::RatingClass,
        form: mush_roll::commerce::paid::PaidForm,
        transferable: bool,
    ) -> Result<mush_roll::commerce::paid::PaidTerms, String> {
        mush_roll::commerce::entitlement::writer::EntitlementPurchaseTerms::personal_terms(
            minimum,
            form,
            self.policy.transferable && transferable,
        )
        .map_err(|e| e.to_string())
    }
    /// The product Roll's actual sale facts admit for `choice`: the subject
    /// is captured from the facts, so this is `product_for` over exactly what
    /// the owner observed.
    pub(super) fn product(
        &self,
        f: &mush_roll::commerce::entitlement::reader::EntitlementSaleFacts,
        choice: ProductChoice,
    ) -> Result<super::EntitlementProductV1, String> {
        self.product_for(
            f.protocol(),
            &super::EntitlementSubjectV1::capture(f),
            f.royalty(),
            choice,
        )
    }
    /// The product body over a captured subject and its royalty chain. 17g
    /// C2b: split from `product` (whose sale facts have no constructor
    /// outside Roll) so the transfer mark's path through the product and its
    /// frozen recompute are pinned by mush-market's own tests; `product`
    /// passes `EntitlementSubjectV1::capture(f)` and `f.royalty()`, so no
    /// production input changed.
    pub(super) fn product_for(
        &self,
        protocol: EntitlementProtocol,
        subject: &super::EntitlementSubjectV1,
        royalty: &mush_roll::commerce::entitlement::FiledRoyaltyTermsV1,
        choice: ProductChoice,
    ) -> Result<super::EntitlementProductV1, String> {
        use mush_roll::commerce::entitlement::{
            DownstreamTermsV1, GrantEconomicsV1, RenewalTemplateV1,
        };
        let ProductChoice {
            form,
            royalty_permille: rate,
            transferable,
        } = choice;
        let terms = self.terms(subject.minimum, form, transferable)?;
        let downstream = match form {
            mush_roll::commerce::paid::PaidForm::Sealed {} if rate == 0 => {
                DownstreamTermsV1::Sealed {}
            }
            mush_roll::commerce::paid::PaidForm::Sealed {} => {
                return Err("sealed product has a source royalty".into());
            }
            _ => DownstreamTermsV1::Source {
                royalty_permille: rate,
            },
        };
        let revision = mush_roll::commerce::records::RevisionPolicy::Pin {
            filed: subject.filed,
            revision: subject.revision,
        };
        let renewal = RenewalTemplateV1::new(
            self.policy.instrument.clone(),
            self.policy.renewal_price,
            self.policy.renewal_term_seconds,
            self.policy.renewal_grace_seconds,
            terms,
            revision,
            self.policy_ref().map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let product = super::EntitlementProductV1 {
            protocol,
            terms,
            revision,
            term_seconds: self.policy.term_seconds,
            grace_seconds: self.policy.grace_seconds,
            grant: GrantEconomicsV1::new(
                downstream,
                renewal,
                royalty.reference().map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?,
        };
        product.matches_subject(subject)?;
        if form == (mush_roll::commerce::paid::PaidForm::OpenResale {}) {
            royalty.check_next_resale(rate).map_err(|e| e.to_string())?;
        }
        Ok(product)
    }
}
