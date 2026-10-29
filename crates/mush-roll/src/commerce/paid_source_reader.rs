//! Current personal-paid source access, distinct from executable holdings.
//! No original authored-library path is changed by this service. No derivative
//! or fork authority is granted. A result is used only during the same dispatch;
//! every delayed/reconnected source request must resolve through this reader again.
use super::families::CommerceFamilies;
use super::owner_holdings::OwnerProjectionError;
use super::paid::PaidForm;
use super::personal_paid_holdings::{
    PersonalPaidHolding, project_personal_paid_for, reader_holder,
};
use super::rating::{RatingClass, RatingCompatibility, RatingFamilies};
use super::{LicenceId, RevisionIdentity};
use crate::{Revision, RollStore};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::read_service::{
    OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService,
};
use mush_core::{DefinitionDigest, DefinitionRef, ModError, ObjId, Registrar};
use mush_economy::EconomyStore;
use serde::Serialize;
use serde_json::json;
use std::{any::TypeId, collections::BTreeSet};

#[derive(Serialize)]
pub struct PaidSourceRequest {
    licence: LicenceId,
}
impl PaidSourceRequest {
    pub fn own(licence: LicenceId) -> Self {
        Self { licence }
    }
}
/// Owner-created point-in-time observation. No public constructor/Deserialize,
/// no caller-provided form or parent qualification, and no durable capability.
#[derive(Debug, Serialize)]
pub struct PaidSourceAccess {
    holding: PersonalPaidHolding,
}
impl PaidSourceAccess {
    pub(crate) fn paid(holding: PersonalPaidHolding) -> Self {
        Self { holding }
    }
    pub fn actor(&self) -> ObjId {
        self.holding.actor()
    }
    pub fn licence(&self) -> LicenceId {
        self.holding.licence()
    }
    pub fn identity(&self) -> RevisionIdentity {
        self.holding.identity()
    }
    pub fn revision(&self) -> &Revision {
        self.holding.revision()
    }
    pub fn definition(&self) -> DefinitionDigest {
        self.holding.definition()
    }
    pub fn declared(&self) -> RatingClass {
        self.holding.declared()
    }
    pub fn minimum(&self) -> RatingClass {
        self.holding.minimum()
    }
}
pub struct PaidSourceReader {
    families: CommerceFamilies,
    ratings: RatingFamilies,
    expected: RatingCompatibility,
}
impl PaidSourceReader {
    pub fn request(
        reg: &mut Registrar<'_>,
        manifest: &DefinitionRef,
    ) -> Result<OwnerReadHandle<Self>, ModError> {
        let definition = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(crate::NS).map_err(error)?,
                descriptor(manifest)?,
            )
            .map_err(error)?;
        reg.request_owner_read::<Self>(definition)
    }
}
fn error(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}
pub(crate) fn descriptor(manifest: &DefinitionRef) -> Result<DefinitionDescriptor, ModError> {
    if manifest.key().owner.as_str() != "tacsim"
        || manifest.key().kind.as_str() != "registry_manifest"
        || manifest.key().id.as_str() != "combat"
        || manifest.version() != 1
    {
        return Err(ModError::msg(
            "paid source requires the exact combat manifest",
        ));
    }
    Ok(DefinitionDescriptor::new(DefinitionKind::new("owner_read").map_err(error)?, DefinitionId::new("personal_paid_source").map_err(error)?, 1,
        json!({"format":1,"subject":"actual_actor","original_only":true,"personal_paid_pin":true,"forms":["open","open_resale"],"source_bytes":false,"fork":false,"listing":false,"manifest":manifest})).map_err(error)?.with_references(vec![manifest.clone()]))
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    families: CommerceFamilies,
    ratings: RatingFamilies,
    manifest: &DefinitionRef,
    expected: RatingCompatibility,
) -> Result<(), ModError> {
    if reg.module() != crate::NS || manifest.digest() != expected.digest() {
        return Err(ModError::msg("paid source composition differs"));
    }
    let definition = reg.definition(descriptor(manifest)?)?;
    let owner = |name| OwnerId::new(name).map_err(error);
    reg.owner_read_service(
        definition,
        BTreeSet::from([owner("tacsim")?]),
        BTreeSet::from([owner("economy")?, owner("tacsim")?]),
        PaidSourceReader::new(families, ratings, expected),
    )
}
impl PaidSourceReader {
    fn new(
        families: CommerceFamilies,
        ratings: RatingFamilies,
        expected: RatingCompatibility,
    ) -> Self {
        Self {
            families,
            ratings,
            expected,
        }
    }
}
impl OwnerReadService for PaidSourceReader {
    type State = RollStore;
    type Request = PaidSourceRequest;
    type Output = PaidSourceAccess;
    fn read(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        request: PaidSourceRequest,
    ) -> Result<PaidSourceAccess, OwnerReadError> {
        roll.commerce_ready()
            .map_err(|_| OwnerReadError::Unavailable)?;
        if ctx
            .world()
            .module_state_owner(TypeId::of::<EconomyStore>())
            .is_none_or(|owner| owner.as_str() != "economy")
        {
            return Err(OwnerReadError::Integrity);
        }
        // Actual paid projection proves holder, own term, complete Grant/E4
        // history, exact status/rating/source roots. It never asks whether the
        // issuer still holds title, its owner term, or an ancestor grant.
        // M8c F1: the source read admits any character acting for the
        // holding organisation now (`reader_holder`).
        let holding = reader_holder(ctx, &self.families, ctx.actor(), request.licence)
            .and_then(|holder| {
                project_personal_paid_for(
                    ctx,
                    holder,
                    &self.families,
                    &self.ratings,
                    request.licence,
                    &self.expected,
                )
            })
            .map_err(|error| match error {
                OwnerProjectionError::Unavailable => OwnerReadError::Unavailable,
                OwnerProjectionError::NotHeld => OwnerReadError::Refused,
                OwnerProjectionError::Integrity => OwnerReadError::Integrity,
            })?;
        if !matches!(
            holding.terms().form(),
            PaidForm::Open {} | PaidForm::OpenResale {}
        ) {
            return Err(OwnerReadError::Refused);
        }
        Ok(PaidSourceAccess { holding })
    }
}
