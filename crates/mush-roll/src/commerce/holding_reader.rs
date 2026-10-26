//! Registered original-owner reads. Inputs name subjects; core supplies the
//! immutable requester. Results are ephemeral observations, never sortie claims.

use super::families::CommerceFamilies;
use super::owner_holdings::{
    OriginalOwnerHolding, OwnerProjectionError, project_original_owner_for,
};
use super::rating::{RatingCompatibility, RatingFamilies};
use super::records::Record;
use super::{CharterId, LicenceId};
use crate::RollStore;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::read_service::{
    OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService,
};
use mush_core::{DefinitionRef, ModError, ObjId, Registrar};
use mush_economy::EconomyStore;
use mush_org::PrincipalId;
use serde::Serialize;
use serde_json::json;
use std::any::TypeId;
use std::collections::BTreeSet;

const OWN_ASSETS_PER_PAGE: usize = 16;

/// There is deliberately no Deserialize or public service constructor. The
/// registered Tacsim adapter is the only granted consumer. Admission naming a
/// different holder must come from that adapter's actual pilot/lead selection;
/// it does not acquire source-display or listing authority for the requester.
#[derive(Serialize)]
pub struct OriginalReadRequest {
    kind: ReadKind,
}
#[derive(Serialize)]
enum ReadKind {
    Own { licence: LicenceId },
    OwnPage { after: Option<ObjId> },
    Admission { holder: ObjId, licence: LicenceId },
}
impl OriginalReadRequest {
    pub fn own(licence: LicenceId) -> Self {
        Self {
            kind: ReadKind::Own { licence },
        }
    }
    pub fn own_page(after: Option<ObjId>) -> Self {
        Self {
            kind: ReadKind::OwnPage { after },
        }
    }
    pub fn admission(holder: ObjId, licence: LicenceId) -> Self {
        Self {
            kind: ReadKind::Admission { holder, licence },
        }
    }
}

/// A page advances only through the requester's current E4 title index. A
/// continuation is an owned object, never a global licence/character counter.
/// At most one complete certified holding is resolved per bounded invocation.
#[derive(Serialize)]
pub struct OriginalReadResult {
    holding: Option<OriginalOwnerHolding>,
    next_after: Option<ObjId>,
}
impl OriginalReadResult {
    pub fn holding(&self) -> Option<&OriginalOwnerHolding> {
        self.holding.as_ref()
    }
    pub fn into_holding(self) -> Option<OriginalOwnerHolding> {
        self.holding
    }
    pub const fn next_after(&self) -> Option<ObjId> {
        self.next_after
    }
}

pub struct OriginalOwnerReader {
    families: CommerceFamilies,
    ratings: RatingFamilies,
    expected: RatingCompatibility,
}
impl OriginalOwnerReader {
    /// The requesting game passes its actual frozen manifest reference. The
    /// owner's registration must independently name exactly the same reference.
    pub fn request(
        reg: &mut Registrar<'_>,
        manifest: &DefinitionRef,
    ) -> Result<OwnerReadHandle<Self>, ModError> {
        reg.request_owner_read::<Self>(definition(manifest)?)
    }
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub(crate) fn descriptor(manifest: &DefinitionRef) -> Result<DefinitionDescriptor, ModError> {
    if manifest.key().owner.as_str() != "tacsim"
        || manifest.key().kind.as_str() != "registry_manifest"
        || manifest.key().id.as_str() != "combat"
        || manifest.version() != 1
    {
        return Err(ModError::msg(
            "original holdings require the registered combat manifest",
        ));
    }
    Ok(DefinitionDescriptor::new(DefinitionKind::new("owner_read").map_err(error)?,DefinitionId::new("original_owner_holding").map_err(error)?,1,
        json!({"format":1,"original_only":true,"own_assets_per_page":OWN_ASSETS_PER_PAGE,"source_bytes":false,"manifest":manifest}))
        .map_err(error)?.with_references(vec![manifest.clone()]))
}
fn definition(manifest: &DefinitionRef) -> Result<DefinitionRef, ModError> {
    DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new(crate::NS).map_err(error)?,
            descriptor(manifest)?,
        )
        .map_err(error)
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    families: CommerceFamilies,
    ratings: RatingFamilies,
    manifest: &DefinitionRef,
    expected: RatingCompatibility,
) -> Result<(), ModError> {
    if reg.module() != crate::NS || manifest.digest() != expected.digest() {
        return Err(ModError::msg(
            "holding reader composition differs from Registry compatibility",
        ));
    }
    let declaration = descriptor(manifest)?;
    let definition = reg.definition(declaration)?;
    let owner = |name| OwnerId::new(name).map_err(error);
    reg.owner_read_service(
        definition,
        BTreeSet::from([owner("tacsim")?]),
        BTreeSet::from([owner("economy")?, owner("tacsim")?]),
        OriginalOwnerReader {
            families,
            ratings,
            expected,
        },
    )
}
impl OwnerReadService for OriginalOwnerReader {
    type State = RollStore;
    type Request = OriginalReadRequest;
    type Output = OriginalReadResult;
    fn read(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        request: OriginalReadRequest,
    ) -> Result<OriginalReadResult, OwnerReadError> {
        roll.commerce_ready()
            .map_err(|_| OwnerReadError::Unavailable)?;
        if ctx
            .world()
            .module_state_owner(TypeId::of::<EconomyStore>())
            .is_none_or(|owner| owner.as_str() != "economy")
        {
            return Err(OwnerReadError::Integrity);
        }
        let (holder, licence) = match request.kind {
            ReadKind::Own { licence } => (ctx.actor(), licence),
            ReadKind::Admission { holder, licence } => {
                if ctx.world().get(holder).is_none() {
                    return Err(OwnerReadError::Refused);
                }
                (holder, licence)
            }
            ReadKind::OwnPage { after } => return self.page(ctx, after),
        };
        let holding = self.project(ctx, holder, licence)?;
        Ok(OriginalReadResult {
            holding: Some(holding),
            next_after: None,
        })
    }
}
impl OriginalOwnerReader {
    fn project(
        &self,
        ctx: &OwnerReadContext<'_>,
        holder: ObjId,
        licence: LicenceId,
    ) -> Result<OriginalOwnerHolding, OwnerReadError> {
        project_original_owner_for(
            ctx,
            holder,
            &self.families,
            &self.ratings,
            licence,
            &self.expected,
        )
        .map_err(|e| match e {
            OwnerProjectionError::Unavailable => OwnerReadError::Unavailable,
            OwnerProjectionError::NotHeld => OwnerReadError::Refused,
            OwnerProjectionError::Integrity => OwnerReadError::Integrity,
        })
    }
    fn page(
        &self,
        ctx: &OwnerReadContext<'_>,
        after: Option<ObjId>,
    ) -> Result<OriginalReadResult, OwnerReadError> {
        let economy = ctx
            .world()
            .module_state::<EconomyStore>()
            .ok_or(OwnerReadError::Unavailable)?;
        if economy.is_frozen() {
            return Err(OwnerReadError::Unavailable);
        }
        let owned: Vec<_> = economy
            .snapshot()
            .assets_of_after(
                &PrincipalId::Object(ctx.actor()),
                after,
                OWN_ASSETS_PER_PAGE + 1,
            )
            .collect();
        let mut last = None;
        for (offset, asset) in owned.iter().take(OWN_ASSETS_PER_PAGE).enumerate() {
            last = Some(asset.object);
            let charter_id =
                CharterId::from_object(asset.object).map_err(|_| OwnerReadError::Integrity)?;
            let Some(charter) = ctx.action_state(&self.families.charters, &charter_id.key())?
            else {
                continue;
            };
            charter.validate().map_err(|_| OwnerReadError::Integrity)?;
            if charter.id != charter_id {
                return Err(OwnerReadError::Integrity);
            }
            let operation = ctx
                .action_state(&self.families.operations, &charter.operation.to_string())?
                .ok_or(OwnerReadError::Integrity)?;
            operation
                .validate()
                .map_err(|_| OwnerReadError::Integrity)?;
            if operation.charter != charter.id || operation.commit != charter.operation {
                return Err(OwnerReadError::Integrity);
            }
            let holding = match self.project(ctx, ctx.actor(), operation.licence) {
                Ok(row) => Some(row),
                Err(OwnerReadError::Refused) => None,
                Err(error) => return Err(error),
            };
            return Ok(OriginalReadResult {
                holding,
                next_after: (offset + 1 < owned.len()).then_some(asset.object),
            });
        }
        Ok(OriginalReadResult {
            holding: None,
            next_after: if owned.len() > OWN_ASSETS_PER_PAGE {
                last
            } else {
                None
            },
        })
    }
}
