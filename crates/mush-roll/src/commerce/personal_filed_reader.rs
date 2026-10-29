//! Closed original-owner/original-personal-paid exact-ID reads for Tacsim.
//! No listing scan, source bytes, account identity, seller lookup or IC claim.
use super::LicenceId;
use super::families::CommerceFamilies;
use super::owner_holdings::{
    OriginalOwnerHolding, OwnerProjectionError, project_original_owner_for,
};
use super::personal_paid_holdings::{
    PersonalPaidHolding, project_personal_paid_for, reader_holder,
};
use super::rating::{RatingCompatibility, RatingFamilies};
use crate::RollStore;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::read_service::{
    OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService,
};
use mush_core::{DefinitionRef, ModError, ObjId, Registrar};
use mush_economy::EconomyStore;
use serde::Serialize;
use serde_json::json;
use std::{any::TypeId, collections::BTreeSet};

#[derive(Serialize)]
pub struct PersonalFiledRequest {
    kind: ReadKind,
}
#[derive(Serialize)]
enum ReadKind {
    Own { licence: LicenceId },
    Admission { holder: ObjId, licence: LicenceId },
}
impl PersonalFiledRequest {
    pub fn own(licence: LicenceId) -> Self {
        Self {
            kind: ReadKind::Own { licence },
        }
    }
    /// The granted Tacsim adapter supplies only its actual slot/lead subject.
    /// This is not source or listing authority for the requester.
    pub fn admission(holder: ObjId, licence: LicenceId) -> Self {
        Self {
            kind: ReadKind::Admission { holder, licence },
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
enum HoldingKind {
    Owner(OriginalOwnerHolding),
    Paid(PersonalPaidHolding),
}
/// Internal opaque observation. Not Deserialize, not accepted back as authority,
/// and never serialize the complete value into a player-facing feed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PersonalFiledHolding {
    kind: HoldingKind,
}
impl PersonalFiledHolding {
    pub(crate) fn paid(value: PersonalPaidHolding) -> Self {
        Self {
            kind: HoldingKind::Paid(value),
        }
    }
    /// Wrap an already owner-created original observation without widening it.
    pub fn original(value: OriginalOwnerHolding) -> Self {
        Self {
            kind: HoldingKind::Owner(value),
        }
    }
    pub fn actor(&self) -> mush_core::ObjId {
        match &self.kind {
            HoldingKind::Owner(v) => v.actor(),
            HoldingKind::Paid(v) => v.actor(),
        }
    }
    pub fn charter(&self) -> super::CharterId {
        match &self.kind {
            HoldingKind::Owner(v) => v.charter(),
            HoldingKind::Paid(v) => v.charter(),
        }
    }
    pub fn line(&self) -> super::LineId {
        match &self.kind {
            HoldingKind::Owner(v) => v.line(),
            HoldingKind::Paid(v) => v.line(),
        }
    }
    pub fn filed(&self) -> super::FiledRevisionId {
        match &self.kind {
            HoldingKind::Owner(v) => v.filed(),
            HoldingKind::Paid(v) => v.filed(),
        }
    }
    pub fn sequence(&self) -> u64 {
        match &self.kind {
            HoldingKind::Owner(v) => v.sequence(),
            HoldingKind::Paid(v) => v.sequence(),
        }
    }
    pub fn licence(&self) -> super::LicenceId {
        match &self.kind {
            HoldingKind::Owner(v) => v.licence(),
            HoldingKind::Paid(v) => v.licence(),
        }
    }
    pub fn term(&self) -> super::TermGeneration {
        match &self.kind {
            HoldingKind::Owner(v) => v.term(),
            HoldingKind::Paid(v) => v.term(),
        }
    }
    pub fn window(&self) -> super::TermWindow {
        match &self.kind {
            HoldingKind::Owner(v) => v.window(),
            HoldingKind::Paid(v) => v.window(),
        }
    }
    pub fn name(&self) -> &str {
        match &self.kind {
            HoldingKind::Owner(v) => v.name(),
            HoldingKind::Paid(v) => v.name(),
        }
    }
    pub fn revision(&self) -> &crate::Revision {
        match &self.kind {
            HoldingKind::Owner(v) => v.revision(),
            HoldingKind::Paid(v) => v.revision(),
        }
    }
    pub fn identity(&self) -> super::RevisionIdentity {
        match &self.kind {
            HoldingKind::Owner(v) => v.identity(),
            HoldingKind::Paid(v) => v.identity(),
        }
    }
    pub fn definition(&self) -> mush_core::DefinitionDigest {
        match &self.kind {
            HoldingKind::Owner(v) => v.definition(),
            HoldingKind::Paid(v) => v.definition(),
        }
    }
    pub fn rating(&self) -> super::rating::RatingRef {
        match &self.kind {
            HoldingKind::Owner(v) => v.rating(),
            HoldingKind::Paid(v) => v.rating(),
        }
    }
    pub fn compatibility(&self) -> &RatingCompatibility {
        match &self.kind {
            HoldingKind::Owner(v) => v.compatibility(),
            HoldingKind::Paid(v) => v.compatibility(),
        }
    }
    pub fn declared(&self) -> super::rating::RatingClass {
        match &self.kind {
            HoldingKind::Owner(v) => v.declared(),
            HoldingKind::Paid(v) => v.declared(),
        }
    }
    pub fn minimum(&self) -> super::rating::RatingClass {
        match &self.kind {
            HoldingKind::Owner(v) => v.minimum(),
            HoldingKind::Paid(v) => v.minimum(),
        }
    }
    pub fn paid_terms(&self) -> Option<super::paid::PaidTerms> {
        match &self.kind {
            HoldingKind::Owner(_) => None,
            HoldingKind::Paid(value) => Some(value.terms()),
        }
    }
}
pub struct PersonalFiledReader {
    families: CommerceFamilies,
    ratings: RatingFamilies,
    expected: RatingCompatibility,
    source_forms: bool,
    dependent_owners: bool,
}
impl PersonalFiledReader {
    pub fn request(
        reg: &mut Registrar<'_>,
        manifest: &DefinitionRef,
    ) -> Result<OwnerReadHandle<Self>, ModError> {
        Self::request_protocol(reg, manifest, false)
    }
    /// Form-aware execution observation only; never source display authority.
    pub fn request_source_forms(
        reg: &mut Registrar<'_>,
        manifest: &DefinitionRef,
    ) -> Result<OwnerReadHandle<Self>, ModError> {
        Self::request_protocol(reg, manifest, true)
    }
    /// Explicit v3 owner execution observation. This grants no source display
    /// or resale permission and never changes the existing paid predicates.
    pub fn request_dependent_owners(
        reg: &mut Registrar<'_>,
        manifest: &DefinitionRef,
    ) -> Result<OwnerReadHandle<Self>, ModError> {
        let definition = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(crate::NS).map_err(error)?,
                descriptor_dependent(manifest)?,
            )
            .map_err(error)?;
        reg.request_owner_read::<Self>(definition)
    }
    fn request_protocol(
        reg: &mut Registrar<'_>,
        manifest: &DefinitionRef,
        source_forms: bool,
    ) -> Result<OwnerReadHandle<Self>, ModError> {
        let definition = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(crate::NS).map_err(error)?,
                if source_forms {
                    descriptor_protocol(manifest, true)?
                } else {
                    descriptor(manifest)?
                },
            )
            .map_err(error)?;
        reg.request_owner_read::<Self>(definition)
    }
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub(crate) fn descriptor(manifest: &DefinitionRef) -> Result<DefinitionDescriptor, ModError> {
    descriptor_protocol(manifest, false)
}
pub(crate) fn descriptor_protocol(
    manifest: &DefinitionRef,
    source_forms: bool,
) -> Result<DefinitionDescriptor, ModError> {
    if manifest.key().owner.as_str() != "tacsim"
        || manifest.key().kind.as_str() != "registry_manifest"
        || manifest.key().id.as_str() != "combat"
        || manifest.version() != 1
    {
        return Err(ModError::msg(
            "personal filed holdings require the registered combat manifest",
        ));
    }
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new("owner_read").map_err(error)?,
        DefinitionId::new("personal_filed_holding").map_err(error)?,
        if source_forms { 2 } else { 1 },
        if source_forms {
            json!({"format":2,"original_owner":true,"original_personal_paid_pin_forms":["sealed","open","open_resale"],
                "source_bytes":false,"listing":false,"manifest":manifest})
        } else {
            json!({"format":1,"original_owner":true,"original_personal_paid_sealed_pin":true,
                "source_bytes":false,"listing":false,"manifest":manifest})
        },
    )
    .map_err(error)?
    .with_references(vec![manifest.clone()]))
}
pub(crate) fn descriptor_dependent(
    manifest: &DefinitionRef,
) -> Result<DefinitionDescriptor, ModError> {
    // Reuse exact manifest validation, never its original-only descriptor.
    let _ = descriptor_protocol(manifest, true)?;
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new("owner_read").map_err(error)?,
        DefinitionId::new("personal_filed_holding").map_err(error)?,
        3,
        json!({"format":3,"original_owner":true,"dependent_owner":"exact_current_owner_parent",
            "original_personal_paid_pin_forms":["sealed","open","open_resale"],"source_bytes":false,
            "listing":false,"manifest":manifest}),
    )
    .map_err(error)?
    .with_references(vec![manifest.clone()]))
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    families: CommerceFamilies,
    ratings: RatingFamilies,
    manifest: &DefinitionRef,
    expected: RatingCompatibility,
    source_forms: bool,
    dependent_owners: bool,
) -> Result<(), ModError> {
    if reg.module() != crate::NS || manifest.digest() != expected.digest() {
        return Err(ModError::msg(
            "personal filed reader composition differs from Registry compatibility",
        ));
    }
    if dependent_owners && !source_forms {
        return Err(ModError::msg(
            "dependent owners require explicit source-form composition",
        ));
    }
    let definition = reg.definition(if dependent_owners {
        descriptor_dependent(manifest)?
    } else {
        descriptor_protocol(manifest, source_forms)?
    })?;
    let owner = |name| OwnerId::new(name).map_err(error);
    reg.owner_read_service(
        definition,
        BTreeSet::from([owner("tacsim")?]),
        BTreeSet::from([owner("economy")?, owner("tacsim")?]),
        PersonalFiledReader {
            families,
            ratings,
            expected,
            source_forms,
            dependent_owners,
        },
    )
}
impl OwnerReadService for PersonalFiledReader {
    type State = RollStore;
    type Request = PersonalFiledRequest;
    type Output = PersonalFiledHolding;
    fn read(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        request: PersonalFiledRequest,
    ) -> Result<PersonalFiledHolding, OwnerReadError> {
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
            ReadKind::Admission { holder, licence } => (holder, licence),
        };
        // The common allocator makes these namespaces disjoint. Never turn an
        // original-owner refusal into a second attempt with weaker predicates.
        let owner = ctx
            .action_state(&self.families.licences, &licence.key())?
            .is_some();
        let paid = ctx
            .action_state(&self.families.paid_licences, &licence.key())?
            .is_some();
        let kind = match (owner, paid) {
            (true, false) => HoldingKind::Owner(
                (if self.dependent_owners {
                    super::owner_holdings::project_owner_with_parent_for
                } else {
                    project_original_owner_for
                })(
                    ctx,
                    holder,
                    &self.families,
                    &self.ratings,
                    licence,
                    &self.expected,
                )
                .map_err(refusal)?,
            ),
            // M8c F1: typed by the pointer's arm (`reader_holder`).
            (false, true) => HoldingKind::Paid(
                reader_holder(ctx, &self.families, holder, licence)
                    .and_then(|holder| {
                        project_personal_paid_for(
                            ctx,
                            holder,
                            &self.families,
                            &self.ratings,
                            licence,
                            &self.expected,
                        )
                    })
                    .map_err(refusal)?,
            ),
            (false, false) => return Err(OwnerReadError::Refused),
            (true, true) => return Err(OwnerReadError::Integrity),
        };
        if !self.source_forms
            && matches!(&kind, HoldingKind::Paid(value) if value.terms().form() != (super::paid::PaidForm::Sealed {}))
        {
            return Err(OwnerReadError::Refused);
        }
        Ok(PersonalFiledHolding { kind })
    }
}
fn refusal(error: OwnerProjectionError) -> OwnerReadError {
    match error {
        OwnerProjectionError::Unavailable => OwnerReadError::Unavailable,
        OwnerProjectionError::NotHeld => OwnerReadError::Refused,
        OwnerProjectionError::Integrity => OwnerReadError::Integrity,
    }
}
