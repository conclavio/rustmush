//! Immutable house birth and real E4 history; no current issuer eligibility.
use super::{HouseAllocationV1, HouseTermEconomicsV1};
use crate::commerce::{
    WorldIcSeconds,
    paid::{PaidTermRecord, PersonalPaidLicence},
    purchase::{PurchaseParticipants, PurchasePhase},
    records::{PublicationError, Record, RevisionPolicy},
};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKey, DefinitionKind, DefinitionRegistryBuilder,
    OwnerId,
};
use mush_core::{DefinitionDigest, DefinitionRef};
use mush_economy::{managed_house_purchase::HousePurchase, managed_purchase::PurchaseHistory};
fn invalid(_: impl std::fmt::Display) -> PublicationError {
    PublicationError::Mismatch
}
fn exact_fund(
    id: &str,
    version: u32,
    protocol: &str,
    compatibility: DefinitionDigest,
    p: &PurchaseParticipants,
) -> Result<DefinitionRef, PublicationError> {
    p.validate()?;
    let manifest = DefinitionRef::new(
        DefinitionKey {
            owner: OwnerId::new("tacsim").map_err(invalid)?,
            kind: DefinitionKind::new("registry_manifest").map_err(invalid)?,
            id: DefinitionId::new("combat").map_err(invalid)?,
        },
        1,
        compatibility,
    )
    .map_err(invalid)?;
    let descriptor = DefinitionDescriptor::new(
        DefinitionKind::new("action").map_err(invalid)?,
        DefinitionId::new(id).map_err(invalid)?,
        version,
        serde_json::json!({"format":version,"protocol":protocol,"transition":"fund"}),
    )
    .map_err(invalid)?
    .with_references(vec![manifest, p.economy().clone(), p.retention().clone()]);
    DefinitionRegistryBuilder::new()
        .register(OwnerId::new("market").map_err(invalid)?, descriptor)
        .map_err(invalid)
}

pub(crate) fn verify_history(
    row: &HouseAllocationV1,
    actual: &PurchaseHistory,
    compatibility: DefinitionDigest,
) -> Result<HousePurchase, PublicationError> {
    row.validate()?;
    let economic = actual
        .house()
        .map_err(invalid)?
        .ok_or(PublicationError::Mismatch)?;
    if actual.operation() != row.operation
        || actual.funding() != &row.funding
        || actual.effect()
            != &exact_fund(
                "purchase_house_mind",
                1,
                "house_mind_purchase_v1",
                compatibility,
                &row.participants,
            )?
        || economic.quote != row.quote
        || economic.quote_digest != row.quote_digest
        || economic.product_digest != row.product_digest
        || economic.buyer != row.holder
        || economic.organization != row.issuer.organization()
        || economic.scope != row.issuer.scope()
        || economic.charter != mush_core::ObjId(row.charter.get())
        || economic.grant_economics_digest != row.economics.grant().digest()?
        || !actual.matches_house(&economic, &row.funding)
    {
        return Err(PublicationError::Mismatch);
    }
    let phase = match actual.terminal() {
        None => PurchasePhase::Funded {},
        Some(t) if t.granted() => PurchasePhase::Granted { commit: t.commit() },
        Some(t) => PurchasePhase::Refunded { commit: t.commit() },
    };
    if row.phase != phase {
        return Err(PublicationError::Mismatch);
    }
    Ok(economic)
}

pub(crate) fn verify_grant(
    row: &HouseAllocationV1,
    licence: &PersonalPaidLicence,
    term: &PaidTermRecord,
    economics: &HouseTermEconomicsV1,
    actual: &PurchaseHistory,
    compatibility: DefinitionDigest,
) -> Result<(), PublicationError> {
    verify_history(row, actual, compatibility)?;
    if !matches!(row.phase, PurchasePhase::Granted { .. }) {
        return Err(PublicationError::Mismatch);
    }
    let issued = WorldIcSeconds::from_stamp(
        actual
            .terminal()
            .and_then(|t| t.issued_at_ic())
            .ok_or(PublicationError::Missing)?,
    )
    .map_err(invalid)?;
    let expected = PersonalPaidLicence::original(
        row.licence,
        row.charter,
        row.line,
        row.holder,
        row.term,
        row.operation,
    )?;
    if licence != &expected
        || economics != &row.economics
        || term.licence() != row.licence
        || term.generation() != row.term
        || term.operation() != row.operation
        || term.policy()
            != (RevisionPolicy::Pin {
                filed: row.filed,
                revision: row.revision,
            })
        || term.terms() != row.terms
        || term.window().issued_at() != issued
        || term.window().eligibility_start() != issued
    {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}
