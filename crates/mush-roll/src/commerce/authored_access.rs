//! Current immediate-parent rights for one marked draft and one author use.
//! No authoring permission is inferred from an executable paid holding or wire.

use super::families::CommerceFamilies;
use super::parent_grant::CurrentParentGrant;
use super::parent_selector::DraftParentSelection;
use super::rating::{RatingCompatibility, RatingFamilies};
use super::read::CommerceRead;
use super::records::Record;
use crate::{Mind, RollStore};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::read_service::{
    OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService,
};
use mush_core::{DefinitionRef, ModError, Registrar};
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Source,
    WorkingEdit,
    Check,
    Save,
}

#[derive(Serialize)]
pub struct AuthoredDraftRequest {
    mind: u64,
    operation: Operation,
}
/// Exact asynchronous author operation; no execution/filing permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthoredJobKind {
    Check,
    Save,
}
impl AuthoredDraftRequest {
    pub fn job(mind: u64, kind: AuthoredJobKind) -> Self {
        Self {
            mind,
            operation: match kind {
                AuthoredJobKind::Check => Operation::Check,
                AuthoredJobKind::Save => Operation::Save,
            },
        }
    }
    pub fn source(mind: u64) -> Self {
        Self {
            mind,
            operation: Operation::Source,
        }
    }
    pub fn working_edit(mind: u64) -> Self {
        Self {
            mind,
            operation: Operation::WorkingEdit,
        }
    }
}

/// Only the actual registered owner reader constructs this observation. It has
/// no decoder or Clone, and must never be retained as a later operation's ACL.
#[derive(Debug, Serialize)]
pub struct AuthoredDraftAccess {
    operation: Operation,
    mind: Mind,
    selector: DraftParentSelection,
    parent: CurrentParentGrant,
}
impl AuthoredDraftAccess {
    pub fn job_kind(&self) -> Option<AuthoredJobKind> {
        match self.operation {
            Operation::Check => Some(AuthoredJobKind::Check),
            Operation::Save => Some(AuthoredJobKind::Save),
            _ => None,
        }
    }
    /// Compare admission evidence to a separately resolved current job read.
    /// Source/Edit observations cannot be reused as job permission.
    pub fn same_job_subject(&self, current: &Self) -> bool {
        self.job_kind().is_some()
            && self.operation == current.operation
            && self.mind == current.mind
            && self.selector == current.selector
            && self.parent.same_grant_later(&current.parent)
    }
    pub fn actor(&self) -> mush_core::ObjId {
        self.parent.actor()
    }
    pub fn mind(&self) -> &Mind {
        &self.mind
    }
    /// Equality with a separately resolved current observation is useful for a
    /// same-dispatch final check; this method alone establishes no currentness.
    pub fn same_observation(&self, current: &Self) -> bool {
        self.operation == current.operation
            && self.mind == current.mind
            && self.selector == current.selector
            && self.parent == current.parent
    }
}

pub struct AuthoredDraftReader {
    families: CommerceFamilies,
    ratings: RatingFamilies,
    expected: RatingCompatibility,
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
            "authored draft requires exact combat manifest",
        ));
    }
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new("owner_read").map_err(error)?,
        DefinitionId::new("authored_draft_access").map_err(error)?,
        2,
        json!({"format":2,"subject":"actual_actor","selector":"exact_immediate_parent",
            "operations":["source","working_edit","check","save"],"current_draft_only":true,
            "forms":["open","open_resale"],"async":"fresh_completion_read","manifest":manifest}),
    )
    .map_err(error)?
    .with_references(vec![manifest.clone()]))
}
impl AuthoredDraftReader {
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
        return Err(ModError::msg("authored draft composition differs"));
    }
    let definition = reg.definition(descriptor(manifest)?)?;
    let owner = |name| OwnerId::new(name).map_err(error);
    reg.owner_read_service(
        definition,
        BTreeSet::from([owner("tacsim")?]),
        BTreeSet::from([owner("economy")?, owner("tacsim")?]),
        AuthoredDraftReader::new(families, ratings, expected),
    )
}
impl AuthoredDraftReader {
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
impl OwnerReadService for AuthoredDraftReader {
    type State = RollStore;
    type Request = AuthoredDraftRequest;
    type Output = AuthoredDraftAccess;
    fn read(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        request: AuthoredDraftRequest,
    ) -> Result<AuthoredDraftAccess, OwnerReadError> {
        if ctx.requester().as_str() != "tacsim" || request.mind == 0 {
            return Err(OwnerReadError::Authority);
        }
        roll.commerce_ready()
            .map_err(|_| OwnerReadError::Unavailable)?;
        let mind = roll
            .owned_mind(ctx.actor(), request.mind)
            .map_err(|_| OwnerReadError::Refused)?;
        let provenance = mind.provenance.as_ref().ok_or(OwnerReadError::Refused)?;
        let selector = ctx
            .get(
                &self.families.draft_parents,
                &DraftParentSelection::key_for(ctx.actor(), request.mind),
            )
            .map_err(|_| OwnerReadError::Unavailable)?
            .ok_or(OwnerReadError::Integrity)?;
        selector
            .check_mind(mind)
            .map_err(|_| OwnerReadError::Integrity)?;
        selector.validate().map_err(|_| OwnerReadError::Integrity)?;
        let parent = super::parent_grant::resolve(
            roll,
            ctx,
            &self.families,
            &self.ratings,
            &self.expected,
            selector.licence,
        )?;
        // Operation-specific requests are closed to these author uses. The actual
        // reader derives Open form/current grant; no generic ParentRights value
        // can turn a private-source grant into resale/filing/execution rights.
        if parent.actor() != ctx.actor()
            || parent.provenance() != *provenance
            || selector.provenance() != parent.provenance()
        {
            return Err(OwnerReadError::Integrity);
        }
        parent
            .check_fresh(ctx.actor(), ctx.world())
            .map_err(|_| OwnerReadError::Refused)?;
        Ok(AuthoredDraftAccess {
            operation: request.operation,
            mind: mind.clone(),
            selector: selector.clone(),
            parent,
        })
    }
}
