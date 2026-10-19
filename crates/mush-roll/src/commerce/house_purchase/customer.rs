//! Opt-in sealed House customer execution. Existing personal readers retain
//! their original contracts. This reads only the customer's own paid authority
//! and current certificate, without a current house title or curator predicate.

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
use super::super::owner_holdings::OwnerProjectionError;
use super::super::personal_filed_reader::PersonalFiledHolding;
use super::super::personal_paid_holdings::project_house_paid_for;
use super::super::rating::{RatingCompatibility, RatingFamilies};
use crate::RollStore;

#[derive(Serialize)]
pub struct HouseHoldingRequest {
    kind: HoldingKind,
}
#[derive(Serialize)]
enum HoldingKind {
    Own { licence: LicenceId },
    Admission { holder: ObjId, licence: LicenceId },
}
impl HouseHoldingRequest {
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
#[derive(Clone)]
struct Facts {
    families: CommerceFamilies,
    ratings: RatingFamilies,
    expected: RatingCompatibility,
}
pub struct HouseHoldingReader(Facts);
/// Exact family routing only. A House refusal is always an error and must not
/// be retried against a different protocol. Other carries no holding authority.
#[derive(Serialize)]
pub enum HouseHoldingResult {
    Other,
    Holding(PersonalFiledHolding),
}

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
    Ok(DefinitionDescriptor::new(DefinitionKind::new("owner_read").map_err(error)?,DefinitionId::new("personal_house_holding").map_err(error)?,1,
        serde_json::json!({"format":1,"paid":"own_granted_term_status_only","revision":"pin","forms":["sealed"],"routing":"exact_birth_family","source_display":false,"listing":false,"manifest":manifest})).map_err(error)?.with_references(vec![manifest.clone()]))
}
impl HouseHoldingReader {
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
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    families: CommerceFamilies,
    ratings: RatingFamilies,
    manifest: &DefinitionRef,
    expected: RatingCompatibility,
) -> Result<(), ModError> {
    if reg.module() != crate::NS || expected.digest() != manifest.digest() {
        return Err(ModError::msg("house customer composition differs"));
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
    reg.owner_read_service(holding, requester, dependencies, HouseHoldingReader(facts))
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
impl OwnerReadService for HouseHoldingReader {
    type State = RollStore;
    type Request = HouseHoldingRequest;
    type Output = HouseHoldingResult;
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
        let owner = ctx.action_state(&f.families.licences, &licence.key())?;
        let paid = ctx.action_state(&f.families.paid_licences, &licence.key())?;
        let Some(paid) = paid else {
            return Ok(HouseHoldingResult::Other);
        };
        if owner.is_some() {
            return Err(OwnerReadError::Integrity);
        }
        let key = paid.operation().to_hex();
        let house = ctx.action_state(&f.families.house_allocations, &key)?;
        let original = ctx.action_state(&f.families.paid_allocations, &key)?;
        let entitlement = ctx.action_state(&f.families.entitlement_allocations, &key)?;
        if usize::from(house.is_some())
            + usize::from(original.is_some())
            + usize::from(entitlement.is_some())
            != 1
        {
            return Err(OwnerReadError::Integrity);
        }
        if house.is_none() {
            return Ok(HouseHoldingResult::Other);
        }
        project_house_paid_for(ctx, holder, &f.families, &f.ratings, licence, &f.expected)
            .map(|p| HouseHoldingResult::Holding(PersonalFiledHolding::paid(p)))
            .map_err(refused)
    }
}
