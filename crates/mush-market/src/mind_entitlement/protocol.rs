//! Exact closed action reconstruction. Data refs confer no participant grant.
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKey, DefinitionKind, DefinitionRegistryBuilder,
    OwnerId,
};
use mush_core::{DefinitionDigest, DefinitionRef};
use mush_economy::managed_entitlement::EntitlementProtocol;
use mush_roll::commerce::purchase::PurchaseParticipants;

#[derive(Clone, Copy)]
pub(super) enum Transition {
    Fund,
    Grant,
    Refund,
}
impl Transition {
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Fund => "fund",
            Self::Grant => "grant",
            Self::Refund => "refund",
        }
    }
}
pub(super) fn action(protocol: EntitlementProtocol, transition: Transition) -> &'static str {
    match (protocol, transition) {
        (EntitlementProtocol::OriginalV3, Transition::Fund) => "purchase_original_mind",
        (EntitlementProtocol::OriginalV3, Transition::Grant) => "settle_original_mind",
        (EntitlementProtocol::OriginalV3, Transition::Refund) => "refund_original_mind",
        (EntitlementProtocol::DerivedV1, Transition::Fund) => "purchase_derived_mind",
        (EntitlementProtocol::DerivedV1, Transition::Grant) => "settle_derived_mind",
        (EntitlementProtocol::DerivedV1, Transition::Refund) => "refund_derived_mind",
    }
}
pub(super) fn domain(protocol: EntitlementProtocol, kind: &str) -> Result<&'static str, String> {
    match (protocol, kind) {
        (EntitlementProtocol::OriginalV3, "product") => Ok("market.original_product.v3"),
        (EntitlementProtocol::OriginalV3, "listing") => Ok("market.original_listing.v3"),
        (EntitlementProtocol::OriginalV3, "quote") => Ok("market.original_quote.v3"),
        (EntitlementProtocol::DerivedV1, "product") => Ok("market.derived_product.v1"),
        (EntitlementProtocol::DerivedV1, "listing") => Ok("market.derived_listing.v1"),
        (EntitlementProtocol::DerivedV1, "quote") => Ok("market.derived_quote.v1"),
        _ => Err("unknown entitlement content domain".into()),
    }
}
pub(super) fn descriptor(
    protocol: EntitlementProtocol,
    transition: Transition,
    manifest: &DefinitionRef,
    participants: &PurchaseParticipants,
) -> Result<DefinitionDescriptor, String> {
    if manifest.key().owner.as_str() != "tacsim"
        || manifest.key().kind.as_str() != "registry_manifest"
        || manifest.key().id.as_str() != "combat"
        || manifest.version() != 1
    {
        return Err("invalid entitlement manifest locator".into());
    }
    let participants = PurchaseParticipants::new(
        participants.economy().clone(),
        participants.retention().clone(),
    )
    .map_err(|e| e.to_string())?;
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new("action").map_err(|e|e.to_string())?,DefinitionId::new(action(protocol,transition)).map_err(|e|e.to_string())?,
        protocol.version(),
        serde_json::json!({"format":protocol.version(),"protocol":protocol.name(),"transition":transition.name()}),
    ).map_err(|e|e.to_string())?.with_references(vec![manifest.clone(),participants.economy().clone(),participants.retention().clone()]))
}
pub(super) fn historical_fund(
    protocol: EntitlementProtocol,
    compatibility: DefinitionDigest,
    participants: &PurchaseParticipants,
) -> Result<DefinitionRef, String> {
    let manifest = DefinitionRef::new(
        DefinitionKey {
            owner: OwnerId::new("tacsim").map_err(|e| e.to_string())?,
            kind: DefinitionKind::new("registry_manifest").map_err(|e| e.to_string())?,
            id: DefinitionId::new("combat").map_err(|e| e.to_string())?,
        },
        1,
        compatibility,
    )
    .map_err(|e| e.to_string())?;
    let mut builder = DefinitionRegistryBuilder::new();
    builder
        .register(
            OwnerId::new("market").map_err(|e| e.to_string())?,
            descriptor(protocol, Transition::Fund, &manifest, participants)?,
        )
        .map_err(|e| e.to_string())
}
