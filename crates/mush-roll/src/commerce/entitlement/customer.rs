//! Separate opted-in customer services. Existing readers keep their exact
//! original-only descriptors. A derived customer's own paid grant never asks
//! whether its seller currently holds title, an owner term or parent rights.

use std::{any::TypeId, collections::BTreeSet};

use mush_core::action::PrincipalId;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::read_service::{
    OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService,
};
use mush_core::{DefinitionRef, ModError, ObjId, Registrar};
use mush_economy::EconomyStore;
use serde::Serialize;

use super::super::LicenceId;
use super::super::families::CommerceFamilies;
use super::super::owner_holdings::{OwnerProjectionError, project_owner_with_parent_for};
use super::super::paid::PaidForm;
use super::super::paid_source_reader::PaidSourceAccess;
use super::super::personal_filed_reader::PersonalFiledHolding;
use super::super::personal_paid_holdings::{project_entitlement_paid_for, reader_holder};
use super::super::rating::{RatingCompatibility, RatingFamilies};
use crate::RollStore;

#[derive(Serialize)]
pub struct EntitlementHoldingRequest {
    kind: HoldingKind,
}
#[derive(Serialize)]
enum HoldingKind {
    Own { licence: LicenceId },
    Admission { holder: ObjId, licence: LicenceId },
}
impl EntitlementHoldingRequest {
    pub fn own(licence: LicenceId) -> Self {
        Self {
            kind: HoldingKind::Own { licence },
        }
    }
    /// Tacsim derives this subject from its real slot/lead roster. It is not
    /// source display permission or a replacement authenticated requester.
    pub fn admission(holder: ObjId, licence: LicenceId) -> Self {
        Self {
            kind: HoldingKind::Admission { holder, licence },
        }
    }
}
#[derive(Serialize)]
pub struct EntitlementSourceRequest {
    licence: LicenceId,
}
impl EntitlementSourceRequest {
    pub fn own(licence: LicenceId) -> Self {
        Self { licence }
    }
}

#[derive(Clone)]
struct Facts {
    families: CommerceFamilies,
    ratings: RatingFamilies,
    expected: RatingCompatibility,
}
pub struct EntitlementHoldingReader(Facts);
pub struct EntitlementSourceReader(Facts);

fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn refused(e: OwnerProjectionError) -> OwnerReadError {
    match e {
        OwnerProjectionError::NotHeld => OwnerReadError::Refused,
        OwnerProjectionError::Unavailable => OwnerReadError::Unavailable,
        OwnerProjectionError::Integrity => OwnerReadError::Integrity,
    }
}
pub(crate) fn holding_descriptor(
    manifest: &DefinitionRef,
) -> Result<DefinitionDescriptor, ModError> {
    super::super::holding_reader::descriptor(manifest)?;
    Ok(DefinitionDescriptor::new(DefinitionKind::new("owner_read").map_err(error)?,DefinitionId::new("personal_entitlement_holding").map_err(error)?,1,
        serde_json::json!({"format":1,"owner":"exact_current_owner_parent","paid":"own_granted_term_status_only","revision":"pin","forms":["sealed","open","open_resale"],"source_display":false,"listing":false,"manifest":manifest})).map_err(error)?.with_references(vec![manifest.clone()]))
}
pub(crate) fn source_descriptor(
    manifest: &DefinitionRef,
) -> Result<DefinitionDescriptor, ModError> {
    super::super::holding_reader::descriptor(manifest)?;
    Ok(DefinitionDescriptor::new(DefinitionKind::new("owner_read").map_err(error)?,DefinitionId::new("personal_entitlement_source").map_err(error)?,1,
        serde_json::json!({"format":1,"subject":"actual_actor","paid":"own_granted_term_status_only","revision":"pin","forms":["open","open_resale"],"source_bytes":false,"fork":false,"listing":false,"manifest":manifest})).map_err(error)?.with_references(vec![manifest.clone()]))
}
impl EntitlementHoldingReader {
    pub fn request(
        reg: &mut Registrar<'_>,
        manifest: &DefinitionRef,
    ) -> Result<OwnerReadHandle<Self>, ModError> {
        let definition = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(crate::NS).map_err(error)?,
                holding_descriptor(manifest)?,
            )
            .map_err(error)?;
        reg.request_owner_read::<Self>(definition)
    }
}
impl EntitlementSourceReader {
    pub fn request(
        reg: &mut Registrar<'_>,
        manifest: &DefinitionRef,
    ) -> Result<OwnerReadHandle<Self>, ModError> {
        let definition = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(crate::NS).map_err(error)?,
                source_descriptor(manifest)?,
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
        return Err(ModError::msg("entitlement customer composition differs"));
    }
    let facts = Facts {
        families,
        ratings,
        expected,
    };
    let requester = BTreeSet::from([OwnerId::new("tacsim").map_err(error)?]);
    let dependencies = BTreeSet::from([
        OwnerId::new("economy").map_err(error)?,
        OwnerId::new("tacsim").map_err(error)?,
    ]);
    let holding = reg.definition(holding_descriptor(manifest)?)?;
    reg.owner_read_service(
        holding,
        requester.clone(),
        dependencies.clone(),
        EntitlementHoldingReader(facts.clone()),
    )?;
    let source = reg.definition(source_descriptor(manifest)?)?;
    reg.owner_read_service(
        source,
        requester,
        dependencies,
        EntitlementSourceReader(facts),
    )
}
fn ready(roll: &RollStore, ctx: &OwnerReadContext<'_>) -> Result<(), OwnerReadError> {
    if ctx.requester().as_str() != "tacsim"
        || ctx.principal() != &(PrincipalId::Object { id: ctx.actor() })
    {
        return Err(OwnerReadError::Authority);
    }
    roll.commerce_ready()
        .map_err(|_| OwnerReadError::Unavailable)?;
    if ctx
        .world()
        .module_state_owner(TypeId::of::<EconomyStore>())
        .is_none_or(|owner| owner.as_str() != "economy")
    {
        return Err(OwnerReadError::Integrity);
    }
    Ok(())
}
impl OwnerReadService for EntitlementHoldingReader {
    type State = RollStore;
    type Request = EntitlementHoldingRequest;
    type Output = PersonalFiledHolding;
    fn read(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        request: Self::Request,
    ) -> Result<Self::Output, OwnerReadError> {
        ready(roll, ctx)?;
        let (holder, licence) = match request.kind {
            HoldingKind::Own { licence } => (ctx.actor(), licence),
            HoldingKind::Admission { holder, licence } => (holder, licence),
        };
        let f = &self.0;
        // Exact family-presence routing; a refused owner projection never falls
        // back to a paid projection, and crossed namespaces remain integrity errors.
        let owner = ctx
            .action_state(&f.families.licences, &licence.key())?
            .is_some();
        let paid = ctx
            .action_state(&f.families.paid_licences, &licence.key())?
            .is_some();
        match (owner, paid) {
            (true, false) => project_owner_with_parent_for(
                ctx,
                holder,
                &f.families,
                &f.ratings,
                licence,
                &f.expected,
            )
            .map(PersonalFiledHolding::original)
            .map_err(refused),
            // M8c F1: the admission's pilot or the own read's actor, typed
            // by the pointer's arm (`reader_holder`): an organisation's
            // licence admits any character acting for it now.
            (false, true) => reader_holder(ctx, &f.families, holder, licence)
                .and_then(|holder| {
                    project_entitlement_paid_for(
                        ctx,
                        holder,
                        &f.families,
                        &f.ratings,
                        licence,
                        &f.expected,
                    )
                })
                .map(PersonalFiledHolding::paid)
                .map_err(refused),
            (false, false) => Err(OwnerReadError::Refused),
            (true, true) => Err(OwnerReadError::Integrity),
        }
    }
}
impl OwnerReadService for EntitlementSourceReader {
    type State = RollStore;
    type Request = EntitlementSourceRequest;
    type Output = PaidSourceAccess;
    fn read(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        request: Self::Request,
    ) -> Result<Self::Output, OwnerReadError> {
        ready(roll, ctx)?;
        let f = &self.0;
        if ctx
            .action_state(&f.families.licences, &request.licence.key())?
            .is_some()
        {
            return Err(OwnerReadError::Refused);
        }
        // M8c F1: the source read admits any character acting for the
        // holding organisation now (`reader_holder`).
        let holding = reader_holder(ctx, &f.families, ctx.actor(), request.licence)
            .and_then(|holder| {
                project_entitlement_paid_for(
                    ctx,
                    holder,
                    &f.families,
                    &f.ratings,
                    request.licence,
                    &f.expected,
                )
            })
            .map_err(refused)?;
        if !matches!(
            holding.terms().form(),
            PaidForm::Open {} | PaidForm::OpenResale {}
        ) {
            return Err(OwnerReadError::Refused);
        }
        Ok(PaidSourceAccess::paid(holding))
    }
}
