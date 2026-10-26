//! Exact closed action reconstruction. Data refs confer no participant grant.
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKey, DefinitionKind, DefinitionRegistryBuilder,
    OwnerId,
};
use mush_core::{DefinitionDigest, DefinitionRef};
use mush_roll::commerce::house_purchase::HouseProtocol;
use mush_roll::commerce::purchase::PurchaseParticipants;

// Grant and Refund are the closed protocol's other two names, pinned by the
// tests; production names Fund here and the grant/refund providers carry their
// own `Kind`.
#[cfg_attr(not(test), allow(dead_code))]
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
pub(super) fn action(protocol: HouseProtocol, transition: Transition) -> &'static str {
    match (protocol, transition) {
        (HouseProtocol::HouseV1 | HouseProtocol::HouseV2, Transition::Fund) => {
            "purchase_house_mind"
        }
        (HouseProtocol::HouseV1 | HouseProtocol::HouseV2, Transition::Grant) => "settle_house_mind",
        (HouseProtocol::HouseV1 | HouseProtocol::HouseV2, Transition::Refund) => {
            "refund_house_mind"
        }
    }
}
pub(super) fn domain(protocol: HouseProtocol, kind: &str) -> Result<&'static str, String> {
    match (protocol, kind) {
        (HouseProtocol::HouseV1, "product") => Ok("market.house_product.v1"),
        (HouseProtocol::HouseV2, "product") => Ok("market.house_product.v2"),
        (HouseProtocol::HouseV1, "listing") => Ok("market.house_listing.v1"),
        (HouseProtocol::HouseV2, "listing") => Ok("market.house_listing.v2"),
        (HouseProtocol::HouseV1, "quote") => Ok("market.house_quote.v1"),
        (HouseProtocol::HouseV2, "quote") => Ok("market.house_quote.v2"),
        _ => Err("unknown house content domain".into()),
    }
}
pub(super) fn descriptor(
    protocol: HouseProtocol,
    transition: Transition,
    manifest: &DefinitionRef,
    participants: &PurchaseParticipants,
) -> Result<DefinitionDescriptor, String> {
    if manifest.key().owner.as_str() != "tacsim"
        || manifest.key().kind.as_str() != "registry_manifest"
        || manifest.key().id.as_str() != "combat"
        || manifest.version() != 1
    {
        return Err("invalid house manifest locator".into());
    }
    let participants = PurchaseParticipants::new(
        participants.economy().clone(),
        participants.retention().clone(),
    )
    .map_err(|e| e.to_string())?;
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new("action").map_err(|e|e.to_string())?,DefinitionId::new(action(protocol,transition)).map_err(|e|e.to_string())?,
        protocol.version(),
        serde_json::json!({"format":protocol.format(),"protocol":protocol.protocol_name(),"transition":transition.name()}),
    ).map_err(|e|e.to_string())?.with_references(vec![manifest.clone(),participants.economy().clone(),participants.retention().clone()]))
}
pub(super) fn historical_fund(
    protocol: HouseProtocol,
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
