//! Own dependent-owner source access, freshly resolved for each request.
//! An absent owner namespace can route to the separate paid reader; a refused
//! owner binding can never fall through to another right or issuer identity.

use super::families::CommerceFamilies;
use super::owner_holdings::{OriginalOwnerHolding, OwnerProjectionError};
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
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeSet;

#[derive(Serialize)]
pub struct DependentSourceRequest {
    licence: LicenceId,
}
impl DependentSourceRequest {
    pub fn own(licence: LicenceId) -> Self {
        Self { licence }
    }
}

#[derive(Debug, Serialize)]
pub struct DependentSourceAccess {
    holding: OriginalOwnerHolding,
}
impl DependentSourceAccess {
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

#[derive(Debug, Serialize)]
pub struct DependentSourceResult {
    access: Option<DependentSourceAccess>,
}
impl DependentSourceResult {
    /// None means the exact licence exists only in the paid namespace. It is
    /// never returned for a missing/invalid/refused dependent-owner licence.
    pub fn into_owner(self) -> Option<DependentSourceAccess> {
        self.access
    }
}

pub struct DependentSourceReader {
    families: CommerceFamilies,
    ratings: RatingFamilies,
    expected: RatingCompatibility,
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub(crate) fn descriptor(manifest: &DefinitionRef) -> Result<DefinitionDescriptor, ModError> {
    let _ = super::personal_filed_reader::descriptor_dependent(manifest)?;
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new("owner_read").map_err(error)?,
        DefinitionId::new("dependent_owner_source").map_err(error)?,
        1,
        json!({"format":1,"subject":"actual_actor","operation":"dependent_owner_source",
            "binding":"exact_current_owner_parent","paid":"namespace_route_only",
            "source_bytes":false,"sales":false,"manifest":manifest}),
    )
    .map_err(error)?
    .with_references(vec![manifest.clone()]))
}
impl DependentSourceReader {
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
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    families: CommerceFamilies,
    ratings: RatingFamilies,
    manifest: &DefinitionRef,
    expected: RatingCompatibility,
) -> Result<(), ModError> {
    if reg.module() != crate::NS || expected.digest() != manifest.digest() {
        return Err(ModError::msg("dependent source composition differs"));
    }
    let definition = reg.definition(descriptor(manifest)?)?;
    let owner = |name| OwnerId::new(name).map_err(error);
    reg.owner_read_service(
        definition,
        BTreeSet::from([owner("tacsim")?]),
        BTreeSet::from([owner("economy")?, owner("tacsim")?]),
        DependentSourceReader {
            families,
            ratings,
            expected,
        },
    )
}
impl OwnerReadService for DependentSourceReader {
    type State = RollStore;
    type Request = DependentSourceRequest;
    type Output = DependentSourceResult;
    fn read(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        request: DependentSourceRequest,
    ) -> Result<Self::Output, OwnerReadError> {
        roll.commerce_ready()
            .map_err(|_| OwnerReadError::Unavailable)?;
        if ctx
            .world()
            .module_state_owner(std::any::TypeId::of::<mush_economy::EconomyStore>())
            .is_none_or(|owner| owner.as_str() != "economy")
        {
            return Err(OwnerReadError::Integrity);
        }
        let owner = ctx.action_state(&self.families.licences, &request.licence.key())?;
        let paid = ctx.action_state(&self.families.paid_licences, &request.licence.key())?;
        let owner = match (owner, paid) {
            (None, Some(_)) => return Ok(DependentSourceResult { access: None }),
            (None, None) => return Err(OwnerReadError::Refused),
            (Some(_), Some(_)) => return Err(OwnerReadError::Integrity),
            (Some(owner), None) => owner,
        };
        let term = ctx
            .action_state(
                &self.families.terms,
                &super::records::OwnerTerm::encoded_key(owner.id, owner.term),
            )?
            .ok_or(OwnerReadError::Integrity)?;
        // Later dependent generations retain their parent qualification in the
        // attestation evidence. The projection below still proves that cause,
        // current selected parent grant, source identity and current window.
        let dependent = match term.format {
            2 => term.parent.is_some(),
            4 => term
                .attestation
                .as_ref()
                .is_some_and(|evidence| evidence.parent.is_some()),
            _ => false,
        };
        if !dependent {
            return Err(OwnerReadError::Refused);
        }
        let holding = super::owner_holdings::project_owner_with_parent_for(
            ctx,
            ctx.actor(),
            &self.families,
            &self.ratings,
            request.licence,
            &self.expected,
        )
        .map_err(|e| match e {
            OwnerProjectionError::Unavailable => OwnerReadError::Unavailable,
            OwnerProjectionError::NotHeld => OwnerReadError::Refused,
            OwnerProjectionError::Integrity => OwnerReadError::Integrity,
        })?;
        Ok(DependentSourceResult {
            access: Some(DependentSourceAccess { holding }),
        })
    }
}
