//! Source-free observations for original personal listings and quotes.
//!
//! Market is the only granted consumer. Its listing provider uses `own`; its
//! quote provider must obtain `listed` selectors from an actual committed
//! Market listing. Neither a selector nor this observation grants a licence.
//! Funding must resolve the subject again and retain the real Roll, E4, time,
//! venue and account fences through the composed installation.

use std::any::TypeId;
use std::collections::BTreeSet;

use mush_core::action::PrincipalId;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::read_service::{
    OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService,
};
use mush_core::{
    ClockStamp, DefinitionDigest, DefinitionRef, Flags, ModError, ObjId, ObjKind, Registrar,
};
use mush_economy::EconomyStore;
use serde::Serialize;
use serde_json::json;

use super::families::CommerceFamilies;
use super::owner_holdings::{
    OriginalOwnerHolding, OwnerProjectionError, project_original_owner_for,
};
use super::rating::{RatingClass, RatingCompatibility, RatingFamilies, RatingRef};
use super::records::{Record, schema_line_digest};
use super::{
    CharterId, FiledRevisionId, LicenceId, LineId, RevisionIdentity, TermGeneration, TermWindow,
};
use crate::RollStore;

/// No deserializer or caller-selected requester. `listed` is for the trusted
/// Market provider's committed listing, never transport delegation of an actor.
#[derive(Serialize)]
pub struct OriginalSaleRequest {
    subject: Subject,
}

#[derive(Serialize)]
enum Subject {
    Own { licence: LicenceId },
    Listed { seller: ObjId, licence: LicenceId },
}

impl OriginalSaleRequest {
    pub fn own(licence: LicenceId) -> Self {
        Self {
            subject: Subject::Own { licence },
        }
    }
    pub fn listed(seller: ObjId, licence: LicenceId) -> Self {
        Self {
            subject: Subject::Listed { seller, licence },
        }
    }
}

/// Ephemeral exact facts, with no source, card values, programs or schema body.
/// Serialize supports the core size bound. No Deserialize/public constructor
/// exists; durable quote rows must remain explicitly data, never capabilities.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OriginalSaleFacts {
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
    observed_ic: ClockStamp,
}

impl OriginalSaleFacts {
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
    pub fn observed_ic(&self) -> &ClockStamp {
        &self.observed_ic
    }

    fn capture(
        requester: ObjId,
        holding: OriginalOwnerHolding,
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
            observed_ic,
        })
    }
}

pub struct OriginalSaleReader {
    families: CommerceFamilies,
    ratings: RatingFamilies,
    expected: RatingCompatibility,
}
impl OriginalSaleReader {
    /// The Market composition supplies the same actual frozen combat manifest
    /// as Roll. A client-supplied hash cannot request a different compatibility.
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
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}

pub(crate) fn descriptor(manifest: &DefinitionRef) -> Result<DefinitionDescriptor, ModError> {
    // Reuse the exact manifest identity check, not a second looser spelling.
    super::holding_reader::descriptor(manifest)?;
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new("owner_read").map_err(error)?,
        DefinitionId::new("original_personal_sale").map_err(error)?,
        1,
        json!({"format":1,"original_only":true,"personal_only":true,
            "fresh_sale":true,"source_bytes":false,"executable_metadata":false,
            "manifest":manifest}),
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
) -> Result<(), ModError> {
    if reg.module() != crate::NS || manifest.digest() != expected.digest() {
        return Err(ModError::msg(
            "sale reader differs from Registry composition",
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
        OriginalSaleReader {
            families,
            ratings,
            expected,
        },
    )
}

impl OwnerReadService for OriginalSaleReader {
    type State = RollStore;
    type Request = OriginalSaleRequest;
    type Output = OriginalSaleFacts;

    fn read(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        request: OriginalSaleRequest,
    ) -> Result<OriginalSaleFacts, OwnerReadError> {
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
        let (seller, licence) = match request.subject {
            Subject::Own { licence } => (ctx.actor(), licence),
            Subject::Listed { seller, licence } => (seller, licence),
        };
        for actor in [ctx.actor(), seller] {
            if ctx.world().get(actor).is_none_or(|object| {
                object.kind != ObjKind::Player || object.flags.contains(Flags::GOING)
            }) {
                return Err(OwnerReadError::Refused);
            }
        }
        let holding = project_original_owner_for(
            ctx,
            seller,
            &self.families,
            &self.ratings,
            licence,
            &self.expected,
        )
        .map_err(|e| match e {
            OwnerProjectionError::Unavailable => OwnerReadError::Unavailable,
            OwnerProjectionError::NotHeld => OwnerReadError::Refused,
            OwnerProjectionError::Integrity => OwnerReadError::Integrity,
        })?;
        let status = ctx
            .action_state(&self.families.status, &holding.filed().key())?
            .ok_or(OwnerReadError::Refused)?;
        status.validate().map_err(|_| OwnerReadError::Integrity)?;
        if status.filed != holding.filed() {
            return Err(OwnerReadError::Integrity);
        }
        // Existing paid and owner pins keep their separate admission semantics.
        // Withdrawal cannot authorize a newly quoted licence, even for a pin.
        if status.withdrawn_by.is_some() || status.recalled_by.is_some() {
            return Err(OwnerReadError::Refused);
        }
        OriginalSaleFacts::capture(ctx.actor(), holding, ctx.world().world_ic_now())
    }
}
