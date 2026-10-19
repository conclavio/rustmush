//! Fresh issuance facts for the two explicit entitlement protocols. This is
//! separate from OriginalSaleReader and from paid customer's own-term access.

use std::any::TypeId;
use std::collections::BTreeSet;

use mush_core::action::PrincipalId;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::participant::OwnerPrepareContext;
use mush_core::read_service::{
    OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService,
};
use mush_core::{
    ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, Flags, ModError, ObjId, ObjKind,
    Registrar,
};
use mush_economy::{EconomyStore, managed_entitlement::EntitlementProtocol};
use serde::Serialize;

use super::super::dependent_owner::OwnerParentBinding;
use super::super::families::CommerceFamilies;
use super::super::filed::ParentContent;
use super::super::owner_holdings::{
    OriginalOwnerHolding, OwnerProjectionError, project_owner_with_parent_for,
};
use super::super::paid::PaidForm;
use super::super::rating::{RatingClass, RatingCompatibility, RatingFamilies, RatingRef};
use super::super::records::{PublicationError, schema_line_digest};
use super::super::{
    CharterId, FiledRevisionId, LicenceId, LineId, RevisionIdentity, TermGeneration, TermPhase,
    TermWindow, WorldIcSeconds,
};
use super::history;
use super::royalties::{self, RoyaltyMaterialization};
use crate::RollStore;

/// Market builds these selectors from its actual invocation or committed
/// listing. They are neither wire requests nor caller-selected credentials.
#[derive(Serialize)]
pub struct EntitlementSaleRequest {
    subject: Subject,
}
#[derive(Serialize)]
enum Subject {
    Own {
        licence: LicenceId,
        publication: CommitToken,
    },
    Listed {
        seller: ObjId,
        licence: LicenceId,
    },
}
impl EntitlementSaleRequest {
    pub fn own(licence: LicenceId, publication: CommitToken) -> Self {
        Self {
            subject: Subject::Own {
                licence,
                publication,
            },
        }
    }
    pub fn listed(seller: ObjId, licence: LicenceId) -> Self {
        Self {
            subject: Subject::Listed { seller, licence },
        }
    }
}

/// The current qualifying parent is not the immutable acquisition basis. It
/// is retained only for the exact fresh operation's pre-custody clock fence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct QualifyingParent {
    parent: ParentContent,
    licence: LicenceId,
    term: TermGeneration,
    window: TermWindow,
}

/// Source-free, owner-created observation. No Deserialize or public constructor
/// exists. A serialized observation is data and cannot be supplied back as one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EntitlementSaleFacts {
    requester: ObjId,
    seller: ObjId,
    charter: CharterId,
    title_revision: u64,
    line: LineId,
    filed: FiledRevisionId,
    sequence: u64,
    owner_licence: LicenceId,
    owner_term: TermGeneration,
    owner_window: TermWindow,
    revision: RevisionIdentity,
    schema: DefinitionDigest,
    definition: DefinitionDigest,
    rating: RatingRef,
    compatibility: RatingCompatibility,
    declared: RatingClass,
    minimum: RatingClass,
    qualifying_parent: Option<QualifyingParent>,
    royalty: RoyaltyMaterialization,
    observed_ic: ClockStamp,
}
impl EntitlementSaleFacts {
    pub const fn requester(&self) -> ObjId {
        self.requester
    }
    pub const fn seller(&self) -> ObjId {
        self.seller
    }
    pub const fn charter(&self) -> CharterId {
        self.charter
    }
    pub const fn title_revision(&self) -> u64 {
        self.title_revision
    }
    pub const fn line(&self) -> LineId {
        self.line
    }
    pub const fn filed(&self) -> FiledRevisionId {
        self.filed
    }
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }
    pub const fn owner_licence(&self) -> LicenceId {
        self.owner_licence
    }
    pub const fn owner_term(&self) -> TermGeneration {
        self.owner_term
    }
    pub const fn owner_window(&self) -> TermWindow {
        self.owner_window
    }
    pub const fn revision(&self) -> RevisionIdentity {
        self.revision
    }
    pub const fn schema(&self) -> DefinitionDigest {
        self.schema
    }
    pub const fn definition(&self) -> DefinitionDigest {
        self.definition
    }
    pub const fn rating(&self) -> RatingRef {
        self.rating
    }
    pub fn compatibility(&self) -> &RatingCompatibility {
        &self.compatibility
    }
    pub const fn declared(&self) -> RatingClass {
        self.declared
    }
    pub const fn minimum(&self) -> RatingClass {
        self.minimum
    }
    pub fn parent(&self) -> Option<ParentContent> {
        self.qualifying_parent.as_ref().map(|p| p.parent)
    }
    pub fn protocol(&self) -> EntitlementProtocol {
        if self.qualifying_parent.is_some() {
            EntitlementProtocol::DerivedV1
        } else {
            EntitlementProtocol::OriginalV3
        }
    }
    pub fn royalty(&self) -> &super::FiledRoyaltyTermsV1 {
        self.royalty.primary()
    }
    pub(crate) fn materialization(&self) -> &RoyaltyMaterialization {
        &self.royalty
    }

    fn capture(
        requester: ObjId,
        holding: OriginalOwnerHolding,
        parent: Option<QualifyingParent>,
        royalty: RoyaltyMaterialization,
        observed_ic: ClockStamp,
    ) -> Result<Self, OwnerReadError> {
        let schema = schema_line_digest(holding.revision().envelope())
            .map_err(|_| OwnerReadError::Integrity)?;
        Ok(Self {
            requester,
            seller: holding.actor(),
            charter: holding.charter(),
            title_revision: holding.title_revision(),
            line: holding.line(),
            filed: holding.filed(),
            sequence: holding.sequence(),
            owner_licence: holding.licence(),
            owner_term: holding.term(),
            owner_window: holding.window(),
            revision: holding.identity(),
            schema,
            definition: holding.definition(),
            rating: holding.rating(),
            compatibility: holding.compatibility().clone(),
            declared: holding.declared(),
            minimum: holding.minimum(),
            qualifying_parent: parent,
            royalty,
            observed_ic,
        })
    }

    pub(crate) fn check_fresh(
        &self,
        ctx: &OwnerPrepareContext<'_>,
    ) -> Result<(), PublicationError> {
        let now = ctx.world().world_ic_now();
        if ctx.invocation().actor() != self.requester
            || now.clock != self.observed_ic.clock
            || now.at < self.observed_ic.at
            || [self.requester, self.seller].into_iter().any(|actor| {
                ctx.world().get(actor).is_none_or(|object| {
                    object.kind != ObjKind::Player || object.flags.contains(Flags::GOING)
                })
            })
        {
            return Err(PublicationError::Mismatch);
        }
        let at = WorldIcSeconds::from_stamp(&now).map_err(|_| PublicationError::Invalid)?;
        let eligible = |window: TermWindow| {
            matches!(window.phase_at(at), TermPhase::Active | TermPhase::Grace)
        };
        if !eligible(self.owner_window)
            || self
                .qualifying_parent
                .as_ref()
                .is_some_and(|p| !eligible(p.window))
        {
            return Err(PublicationError::Unavailable);
        }
        // Listing uses only Roll's participant. Its earlier owner read is not
        // an E4 prepared image, so recheck the actual mutable title here as
        // well. Funding/Grant additionally bind E4's exact complete plan.
        let economy = ctx
            .world()
            .module_state::<EconomyStore>()
            .ok_or(PublicationError::Unavailable)?;
        let title = economy
            .snapshot()
            .asset(ObjId(self.charter.get()))
            .ok_or(PublicationError::Unavailable)?;
        if economy.is_frozen()
            || ctx
                .world()
                .module_state_owner(TypeId::of::<EconomyStore>())
                .is_none_or(|owner| owner.as_str() != "economy")
            || ctx
                .world()
                .get(ObjId(self.charter.get()))
                .is_none_or(|object| object.flags.contains(Flags::GOING))
            || title.object != ObjId(self.charter.get())
            || title.revision != self.title_revision
            || title.title_owner != mush_org::PrincipalId::Object(self.seller)
            || title.scope.is_some()
            || title.frozen
            || title.stolen.is_some()
            || title.lifecycle != mush_economy::AssetLifecycle::Active
        {
            return Err(PublicationError::Unavailable);
        }
        Ok(())
    }
}

pub struct EntitlementSaleReader {
    families: CommerceFamilies,
    ratings: RatingFamilies,
    expected: RatingCompatibility,
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn integrity(_: impl std::fmt::Display) -> OwnerReadError {
    OwnerReadError::Integrity
}
fn projection(error: OwnerProjectionError) -> OwnerReadError {
    match error {
        OwnerProjectionError::Unavailable => OwnerReadError::Unavailable,
        OwnerProjectionError::NotHeld => OwnerReadError::Refused,
        OwnerProjectionError::Integrity => OwnerReadError::Integrity,
    }
}
impl EntitlementSaleReader {
    /// Reuse only inside another actual Roll-owned reader with its own exact
    /// registered protocol. This does not issue a handle or caller capability.
    pub(crate) fn for_renewal(
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

    pub fn request(
        reg: &mut Registrar<'_>,
        manifest: &DefinitionRef,
    ) -> Result<OwnerReadHandle<Self>, ModError> {
        let reference = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(crate::NS).map_err(error)?,
                descriptor(manifest)?,
            )
            .map_err(error)?;
        reg.request_owner_read::<Self>(reference)
    }
}
pub(crate) fn descriptor(manifest: &DefinitionRef) -> Result<DefinitionDescriptor, ModError> {
    super::super::holding_reader::descriptor(manifest)?;
    Ok(DefinitionDescriptor::new(DefinitionKind::new("owner_read").map_err(error)?,DefinitionId::new("personal_entitlement_sale").map_err(error)?,1,
        serde_json::json!({"format":1,"protocols":["original_v3","derived_v1"],"personal_pin":true,"first_generation_only":true,"current_parent_resale":true,"immutable_acquisition":true,"source_bytes":false,"manifest":manifest}))
        .map_err(error)?.with_references(vec![manifest.clone()]))
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
            "entitlement sale reader differs from Registry composition",
        ));
    }
    let definition = reg.definition(descriptor(manifest)?)?;
    reg.owner_read_service(
        definition,
        BTreeSet::from([OwnerId::new("market").map_err(error)?]),
        BTreeSet::from([
            OwnerId::new("economy").map_err(error)?,
            OwnerId::new("tacsim").map_err(error)?,
        ]),
        EntitlementSaleReader {
            families,
            ratings,
            expected,
        },
    )
}
impl OwnerReadService for EntitlementSaleReader {
    type State = RollStore;
    type Request = EntitlementSaleRequest;
    type Output = EntitlementSaleFacts;
    fn read(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        request: Self::Request,
    ) -> Result<Self::Output, OwnerReadError> {
        if ctx.requester().as_str() != "market"
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
        let listed = matches!(&request.subject, Subject::Listed { .. });
        let (seller, licence, publication) = match request.subject {
            Subject::Own {
                licence,
                publication,
            } => (ctx.actor(), licence, Some(publication)),
            Subject::Listed { seller, licence } => (seller, licence, None),
        };
        for actor in [ctx.actor(), seller] {
            if ctx.world().get(actor).is_none_or(|object| {
                object.kind != ObjKind::Player || object.flags.contains(Flags::GOING)
            }) {
                return Err(OwnerReadError::Refused);
            }
        }
        let holding = project_owner_with_parent_for(
            ctx,
            seller,
            &self.families,
            &self.ratings,
            licence,
            &self.expected,
        )
        .map_err(projection)?;
        let status =
            history::read(ctx, &self.families.status, holding.filed().key()).map_err(integrity)?;
        if status.filed != holding.filed() {
            return Err(OwnerReadError::Integrity);
        }
        if status.withdrawn_by.is_some() || status.recalled_by.is_some() {
            return Err(OwnerReadError::Refused);
        }
        let filed =
            history::read(ctx, &self.families.filed, holding.filed().key()).map_err(integrity)?;
        let qualifying_parent = if let Some(parent) = filed.content.parent() {
            // The owner projection above has already verified this binding's
            // historical integrity and the current own paid grant, including
            // its complete current certificate. The same immutable owner read
            // rereads only exact metadata to require the stronger resale form.
            let binding = history::read(
                ctx,
                &self.families.owner_parents,
                OwnerParentBinding::key_for(holding.licence(), seller),
            )
            .map_err(integrity)?;
            let paid = history::read(
                ctx,
                &self.families.paid_licences,
                binding.parent.licence.key(),
            )
            .map_err(integrity)?;
            let term = history::read(
                ctx,
                &self.families.paid_terms,
                format!("{}/{}", paid.id().key(), paid.term().key()),
            )
            .map_err(integrity)?;
            term.matches_current(paid).map_err(integrity)?;
            if binding.parent.parent != parent
                || binding.parent.owner != seller
                || paid.holder() != seller
                || term.terms().form() != (PaidForm::OpenResale {})
            {
                return Err(OwnerReadError::Refused);
            }
            Some(QualifyingParent {
                parent,
                licence: paid.id(),
                term: paid.term(),
                window: term.window(),
            })
        } else {
            None
        };
        let publication = match publication {
            Some(value) => value,
            None => history::read(ctx, &self.families.filed_royalties, holding.filed().key())
                .map_err(integrity)?
                .publication(),
        };
        let royalty = royalties::prepare(
            ctx,
            roll,
            &self.families,
            &self.ratings,
            holding.filed(),
            publication,
        )
        .map_err(integrity)?;
        if listed && !royalty.creates().is_empty() {
            return Err(OwnerReadError::Integrity);
        }
        EntitlementSaleFacts::capture(
            ctx.actor(),
            holding,
            qualifying_parent,
            royalty,
            ctx.world().world_ic_now(),
        )
    }
}
