//! One complete current certificate plus the actual retained IC claim. Current
//! parent/issuer/licence eligibility is deliberately not re-admitted here: the
//! acknowledged sortie already owns its immutable warrant. New cover still
//! requires the exact current certificate, hull title and withheld host proof.
use crate::{
    RollStore,
    commerce::{
        CharterId, FiledRevisionId, LineId, RevisionIdentity,
        families::CommerceFamilies,
        ic_claims::ClaimRecord,
        rating::{RatingCompatibility, RatingFamilies, RatingRef},
        read::CommerceRead,
        records::{PublicationError, Record, canonical_duplicate_digest, schema_line_digest},
        roots::{RequiredRoot, RootAnchor},
    },
};
use mush_core::{
    ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, Flags, ModError, ObjId, ObjKind,
    Registrar, StateFamilyHandle, World,
    action::PrincipalId,
    mechanics::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
    },
    participant::OwnerPrepareContext,
    read_service::{OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService},
};
use mush_economy::{AssetLifecycle, EconomyStore};
use mush_sim::ic_exclusion::IcExclusions;
use mush_tactics::UnitStore;
use serde::Serialize;
use std::{any::TypeId, collections::BTreeSet};

/// An exact selector only. The actor comes from the actual dispatch context.
#[derive(Serialize)]
pub struct CoverageOpenRequest {
    open: CommitToken,
}
impl CoverageOpenRequest {
    pub fn acknowledged(open: CommitToken) -> Self {
        Self { open }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Hull {
    object: ObjId,
    holder: ObjId,
    title_revision: u64,
    unit_revision: u64,
    unit_schema: DefinitionRef,
}
impl Hull {
    fn capture(world: &World, row: &ClaimRecord) -> Result<Self, PublicationError> {
        let open = &row.open;
        for (slot, owner) in [
            (TypeId::of::<EconomyStore>(), "economy"),
            (TypeId::of::<UnitStore>(), "tactics"),
            (TypeId::of::<IcExclusions>(), "sim"),
        ] {
            if world
                .module_state_owner(slot)
                .is_none_or(|v| v.as_str() != owner)
            {
                return Err(PublicationError::Unavailable);
            }
        }
        let economy = world
            .module_state::<EconomyStore>()
            .ok_or(PublicationError::Unavailable)?;
        let title = economy
            .snapshot()
            .asset(open.unit)
            .ok_or(PublicationError::Missing)?;
        if economy.is_frozen()
            || world
                .get(open.unit)
                .is_none_or(|v| v.kind != ObjKind::Thing || v.flags.contains(Flags::GOING))
            || title.object != open.unit
            || title.title_owner != mush_org::PrincipalId::Object(open.dispatcher)
            || title.scope.is_some()
            || title.frozen
            || title.stolen.is_some()
            || title.lifecycle != AssetLifecycle::Active
            || title.revision != open.hull_title_revision
        {
            return Err(PublicationError::Unavailable);
        }
        let unit =
            mush_tactics::module::unit_of(world, open.unit).ok_or(PublicationError::Missing)?;
        let units = world
            .module_state::<UnitStore>()
            .ok_or(PublicationError::Unavailable)?;
        if !units.is_ic_custodied(open.unit) {
            return Err(PublicationError::Mismatch);
        }
        let inventory = units.ic_custody_inventory().map_err(integrity)?;
        let custody = inventory
            .iter()
            .find(|v| v.open() == open.operation)
            .ok_or(PublicationError::Missing)?;
        custody.verify_open_unit(unit).map_err(integrity)?;
        let exclusions = world
            .module_state::<IcExclusions>()
            .ok_or(PublicationError::Unavailable)?;
        let inventory = exclusions.inventory().map_err(integrity)?;
        let spatial = inventory
            .iter()
            .find(|v| v.identity().open == open.operation)
            .ok_or(PublicationError::Missing)?;
        let id = spatial.identity();
        if row.closed.is_some()
            || custody.closed().is_some()
            || spatial.closed().is_some()
            || custody.unit() != open.unit
            || custody.sortie() != open.sortie.get()
            || custody.seal_digest() != open.seal_digest
            || custody.digest().map_err(integrity)? != open.custody_digest
            || spatial.digest().map_err(integrity)? != open.exclusion_digest
            || spatial.dispatcher() != open.dispatcher
            || id.unit != open.unit
            || id.sortie != open.sortie.get()
            || id.seal_digest != open.seal_digest
            || id.custody_digest != open.custody_digest
            || mush_sim::ic_exclusion::identity_of(world, open.unit) != Some(id)
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(Self {
            object: open.unit,
            holder: open.dispatcher,
            title_revision: title.revision,
            unit_revision: unit.revision(),
            unit_schema: unit.schema().clone(),
        })
    }
}
fn integrity(_: impl std::fmt::Display) -> PublicationError {
    PublicationError::Mismatch
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn refused(_: impl std::fmt::Display) -> OwnerReadError {
    OwnerReadError::Refused
}
fn row<'a, R: Record>(
    ctx: &'a impl CommerceRead,
    handle: &StateFamilyHandle<String, R>,
    key: String,
) -> Result<&'a R, OwnerReadError> {
    let value = ctx
        .get(handle, &key)
        .map_err(|_| OwnerReadError::Unavailable)?
        .ok_or(OwnerReadError::Refused)?;
    value.validate().map_err(|_| OwnerReadError::Integrity)?;
    if value.key() != key {
        return Err(OwnerReadError::Integrity);
    }
    Ok(value)
}

/// Minted only by the actual registered Roll read. There is no decoder or
/// public constructor, and neither these facts nor their digest release a host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CoverageOpenFacts {
    claim: ClaimRecord,
    hull: Hull,
    observed_ic: ClockStamp,
}
impl CoverageOpenFacts {
    pub(crate) fn claim(&self) -> &ClaimRecord {
        &self.claim
    }
    pub fn open(&self) -> CommitToken {
        self.claim.open.operation
    }
    pub fn actor(&self) -> ObjId {
        self.claim.open.dispatcher
    }
    pub fn unit(&self) -> ObjId {
        self.hull.object
    }
    pub fn hull_title_revision(&self) -> u64 {
        self.hull.title_revision
    }
    pub fn hull_revision(&self) -> u64 {
        self.hull.unit_revision
    }
    pub fn hull_schema(&self) -> &DefinitionRef {
        &self.hull.unit_schema
    }
    pub fn charter(&self) -> CharterId {
        self.claim.open.charter
    }
    pub fn line(&self) -> LineId {
        self.claim.open.line
    }
    pub fn filed(&self) -> FiledRevisionId {
        self.claim.open.filed
    }
    pub fn revision(&self) -> RevisionIdentity {
        self.claim.open.revision
    }
    pub fn rating(&self) -> RatingRef {
        self.claim.open.rating
    }
    pub fn seal_digest(&self) -> DefinitionDigest {
        self.claim.open.seal_digest
    }
    pub fn admission_digest(&self) -> Result<DefinitionDigest, PublicationError> {
        self.claim.open.admission_digest()
    }
    pub fn observed_ic(&self) -> &ClockStamp {
        &self.observed_ic
    }
    pub(crate) fn root(&self) -> Result<RequiredRoot, PublicationError> {
        crate::commerce::ic_claims::roots::required(&self.claim)
    }
    /// The actual Root participant retains this observation. Checked typed
    /// preimages protect the certificate/status; this fence repeats mutable
    /// physical/title/lifecycle and the acknowledged Roll claim at stage/take.
    pub(crate) fn check_fresh(
        &self,
        ctx: &OwnerPrepareContext<'_>,
    ) -> Result<(), PublicationError> {
        let world = ctx.world();
        let now = world.world_ic_now();
        if ctx.invocation().actor() != self.actor()
            || ctx.invocation().principal() != &(PrincipalId::Object { id: self.actor() })
            || now.clock != self.observed_ic.clock
            || now.at < self.observed_ic.at
            || world
                .get(self.actor())
                .is_none_or(|v| v.kind != ObjKind::Player || v.flags.contains(Flags::GOING))
            || world
                .module_state_owner(TypeId::of::<RollStore>())
                .is_none_or(|v| v.as_str() != crate::NS)
        {
            return Err(PublicationError::Stale);
        }
        let roll = world
            .module_state::<RollStore>()
            .ok_or(PublicationError::Unavailable)?;
        if roll.coverage_ic_claim(self.open()).map_err(integrity)? != &self.claim
            || Hull::capture(world, &self.claim)? != self.hull
        {
            return Err(PublicationError::Stale);
        }
        roll.check_required_root(&self.root()?).map_err(integrity)?;
        Ok(())
    }
}

pub struct CoverageOpenReader {
    f: CommerceFamilies,
    ratings: RatingFamilies,
    expected: RatingCompatibility,
}
impl CoverageOpenReader {
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
    crate::commerce::holding_reader::descriptor(manifest)?;
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new("owner_read").map_err(error)?, DefinitionId::new("hull_coverage_open").map_err(error)?, 1,
        serde_json::json!({"format":1,"protocol":"acknowledged_ic_coverage_v1","actor":"actual_dispatcher",
            "claim":"actual_audited_active","certificate":"exact_current_retained_seal","title":"actual_personal_hull",
            "renewed_warrant":false,"source_bytes":false,"manifest":manifest}),
    ).map_err(error)?.with_references(vec![manifest.clone()]))
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    f: CommerceFamilies,
    ratings: RatingFamilies,
    manifest: &DefinitionRef,
    expected: RatingCompatibility,
) -> Result<(), ModError> {
    if reg.module() != crate::NS || manifest.digest() != expected.digest() {
        return Err(error("coverage Registry composition differs"));
    }
    let definition = reg.definition(descriptor(manifest)?)?;
    reg.owner_read_service(
        definition,
        BTreeSet::from([OwnerId::new("tacsim").map_err(error)?]),
        ["economy", "tacsim", "tactics", "sim"]
            .into_iter()
            .map(|s| OwnerId::new(s).map_err(error))
            .collect::<Result<BTreeSet<_>, _>>()?,
        CoverageOpenReader {
            f,
            ratings,
            expected,
        },
    )
}
impl OwnerReadService for CoverageOpenReader {
    type State = RollStore;
    type Request = CoverageOpenRequest;
    type Output = CoverageOpenFacts;
    fn read(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        request: Self::Request,
    ) -> Result<Self::Output, OwnerReadError> {
        if ctx.requester().as_str() != "tacsim"
            || ctx.principal() != &(PrincipalId::Object { id: ctx.actor() })
            || ctx
                .world()
                .get(ctx.actor())
                .is_none_or(|v| v.kind != ObjKind::Player || v.flags.contains(Flags::GOING))
        {
            return Err(OwnerReadError::Authority);
        }
        let claim = row(ctx, &self.f.ic_claims, request.open.to_hex())?;
        if claim.closed.is_some() || claim.open.dispatcher != ctx.actor() {
            return Err(OwnerReadError::Refused);
        }
        if roll.coverage_ic_claim(request.open).map_err(refused)? != claim {
            return Err(OwnerReadError::Integrity);
        }
        let hull = Hull::capture(ctx.world(), claim).map_err(refused)?;
        let open = &claim.open;
        let filed = row(ctx, &self.f.filed, open.filed.key())?;
        let status = row(ctx, &self.f.status, open.filed.key())?;
        let line = row(ctx, &self.f.lines, open.line.key())?;
        if filed.content.revision() != open.revision
            || filed.content.position().charter != open.charter
            || filed.content.position().line != open.line
            || line.charter != open.charter
            || status.filed != open.filed
        {
            return Err(OwnerReadError::Integrity);
        }
        if status.withdrawn_by.is_some() || status.recalled_by.is_some() {
            return Err(OwnerReadError::Refused);
        }
        // At most one complete certificate walk (116 page rows at current
        // maximum), plus bounded claim/filed/status/line/head/job/metadata joins.
        let rating = crate::commerce::rating::reconformance::current::receipt(
            ctx,
            &self.f,
            &self.ratings,
            filed,
            &self.expected,
        )
        .map_err(refused)?;
        if rating.reference().map_err(refused)? != open.rating
            || rating.subject().revision() != open.revision
            || rating.subject().definition() != filed.definition
            || rating.subject().schema() != line.schema_digest
        {
            return Err(OwnerReadError::Refused);
        }
        for root in [
            crate::commerce::ic_claims::roots::required(claim).map_err(refused)?,
            RequiredRoot {
                anchor: RootAnchor::Filed(open.filed),
                identity: open.revision,
            },
            RequiredRoot {
                anchor: RootAnchor::Rating {
                    job: rating.publication().job(),
                    id: rating.id(),
                },
                identity: open.revision,
            },
        ] {
            roll.check_required_root(&root).map_err(refused)?;
        }
        let reference = open.revision.revision();
        let revision = roll
            .revision(reference.key.character, reference.key)
            .map_err(refused)?;
        if !open.revision.matches(revision).map_err(refused)?
            || schema_line_digest(revision.envelope()).map_err(refused)? != line.schema_digest
            || canonical_duplicate_digest(revision).map_err(refused)? != filed.duplicate
        {
            return Err(OwnerReadError::Integrity);
        }
        let observed_ic = ctx.world().world_ic_now();
        if observed_ic.clock != open.at_ic.clock || observed_ic.at < open.at_ic.at {
            return Err(OwnerReadError::Refused);
        }
        Ok(CoverageOpenFacts {
            claim: claim.clone(),
            hull,
            observed_ic,
        })
    }
}
