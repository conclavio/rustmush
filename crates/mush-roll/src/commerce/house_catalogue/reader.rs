use super::{HouseIssuer, authority};
use crate::{
    RollStore,
    commerce::{
        CharterId, FiledRevisionId, LicenceId, LineId, RevisionIdentity, TermGeneration, TermPhase,
        TermWindow, WorldIcSeconds,
        families::CommerceFamilies,
        rating::{RatingClass, RatingCompatibility, RatingFamilies, RatingRef},
        read::CommerceRead,
        records::{Record, RevisionPolicy, canonical_duplicate_digest, schema_line_digest},
        roots::{RequiredRoot, RootAnchor},
    },
};
use mush_core::action::PrincipalId;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::read_service::{
    OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService,
};
use mush_core::{
    ClockStamp, DefinitionDigest, DefinitionRef, ModError, ObjId, Registrar, StateFamilyHandle,
};
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeSet;

#[derive(Serialize)]
enum Subject {
    Publication {
        licence: LicenceId,
    },
    Listed {
        licence: LicenceId,
        issuer: HouseIssuer,
    },
}
/// Market constructs Listed only from its acknowledged issuer-bound listing.
/// Neither selector is a wire request or a replacement for the actual actor.
#[derive(Serialize)]
pub struct HouseSaleRequest {
    subject: Subject,
}
impl HouseSaleRequest {
    pub fn publication(licence: LicenceId) -> Self {
        Self {
            subject: Subject::Publication { licence },
        }
    }
    pub fn listed(licence: LicenceId, issuer: HouseIssuer) -> Self {
        Self {
            subject: Subject::Listed { licence, issuer },
        }
    }
}
/// Bounded source-free facts. A curator/member receives no executable, source,
/// free personal holding or purchase exemption through this observation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HouseSaleFacts {
    requester: ObjId,
    issuer: HouseIssuer,
    organization_revision: u64,
    scope_revision: u64,
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
impl HouseSaleFacts {
    pub const fn requester(&self) -> ObjId {
        self.requester
    }
    pub const fn issuer(&self) -> HouseIssuer {
        self.issuer
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
    pub const fn organization_revision(&self) -> u64 {
        self.organization_revision
    }
    pub const fn scope_revision(&self) -> u64 {
        self.scope_revision
    }
}
pub struct HouseSaleReader {
    f: CommerceFamilies,
    ratings: RatingFamilies,
    expected: RatingCompatibility,
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn integrity<T>(_: T) -> OwnerReadError {
    OwnerReadError::Integrity
}
fn row<'a, R: Record>(
    ctx: &'a impl CommerceRead,
    handle: &StateFamilyHandle<String, R>,
    key: String,
) -> Result<&'a R, OwnerReadError> {
    let r = ctx
        .get(handle, &key)
        .map_err(|_| OwnerReadError::Unavailable)?
        .ok_or(OwnerReadError::Refused)?;
    r.validate().map_err(integrity)?;
    if r.key() != key {
        return Err(OwnerReadError::Integrity);
    }
    Ok(r)
}
impl HouseSaleReader {
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
    Ok(DefinitionDescriptor::new(DefinitionKind::new("owner_read").map_err(error)?,DefinitionId::new("original_house_sale").map_err(error)?,1,
        json!({"format":1,"original_only":true,"issuer":"organization_and_exact_scope","publication":"current_explicit_allow","listed":"organization_survives_curator_departure","source_bytes":false,"personal_owner_use":false,"manifest":manifest})).map_err(error)?.with_references(vec![manifest.clone()]))
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    f: CommerceFamilies,
    ratings: RatingFamilies,
    manifest: &DefinitionRef,
    expected: RatingCompatibility,
) -> Result<(), ModError> {
    if reg.module() != crate::NS || manifest.digest() != expected.digest() {
        return Err(error("house sale Registry composition differs"));
    }
    let definition = reg.definition(descriptor(manifest)?)?;
    reg.owner_read_service(
        definition,
        BTreeSet::from([OwnerId::new("market").map_err(error)?]),
        BTreeSet::from([
            OwnerId::new("economy").map_err(error)?,
            OwnerId::new("org").map_err(error)?,
            OwnerId::new("tacsim").map_err(error)?,
        ]),
        HouseSaleReader {
            f,
            ratings,
            expected,
        },
    )
}
impl OwnerReadService for HouseSaleReader {
    type State = RollStore;
    type Request = HouseSaleRequest;
    type Output = HouseSaleFacts;
    fn read(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        request: HouseSaleRequest,
    ) -> Result<HouseSaleFacts, OwnerReadError> {
        if ctx.requester().as_str() != "market"
            || ctx.principal() != &(PrincipalId::Object { id: ctx.actor() })
        {
            return Err(OwnerReadError::Authority);
        }
        roll.commerce_ready()
            .map_err(|_| OwnerReadError::Unavailable)?;
        authority::player(ctx.world(), ctx.actor())?;
        let (licence, expected_issuer) = match request.subject {
            Subject::Publication { licence } => (licence, None),
            Subject::Listed { licence, issuer } => (licence, Some(issuer)),
        };
        let licence = row(ctx, &self.f.licences, licence.key())?;
        let birth = row(ctx, &self.f.operations, licence.operation.to_hex())?;
        let house = birth
            .economic
            .house
            .as_ref()
            .ok_or(OwnerReadError::Refused)?;
        let issuer = HouseIssuer::new(house.organization(), house.scope())?;
        if birth.format != 2
            || birth.parent.is_some()
            || birth.economic.issued_owner != issuer.principal()
            || house.curator() != birth.requester
            || expected_issuer.is_some_and(|e| e != issuer)
        {
            return Err(OwnerReadError::Integrity);
        }
        if expected_issuer.is_none() {
            issuer.publication(ctx.world(), ctx.actor())?;
        }
        let (organization_revision, scope_revision) = issuer.live(ctx.world())?;
        let title = issuer.title(ctx.world(), birth.charter)?;
        let economy = ctx
            .world()
            .module_state::<mush_economy::EconomyStore>()
            .ok_or(OwnerReadError::Unavailable)?;
        economy
            .filing_receipt_audit()
            .verify(&birth.audit_subject().map_err(integrity)?)
            .map_err(integrity)?;
        let charter = row(ctx, &self.f.charters, licence.charter.key())?;
        let line = row(ctx, &self.f.lines, licence.line.key())?;
        let term =
            crate::commerce::owner_attestation::history::current(ctx, roll, &self.f, licence)
                .map_err(integrity)?;
        let observed_ic = ctx.world().world_ic_now();
        let at = WorldIcSeconds::from_stamp(&observed_ic).map_err(integrity)?;
        if !matches!(
            term.window.phase_at(at),
            TermPhase::Active | TermPhase::Grace
        ) {
            return Err(OwnerReadError::Refused);
        }
        crate::commerce::registry_revocation::current::require(
            ctx,
            &self.f,
            licence.id,
            term.generation,
            None,
        )
        .map_err(|_| OwnerReadError::Refused)?;
        if charter.id != birth.charter
            || charter.line != line.id
            || line.charter != charter.id
            || licence.id != birth.licence
            || licence.line != birth.line
            || charter.operation != birth.commit
            || line.operation != birth.commit
            || term.parent.is_some()
        {
            return Err(OwnerReadError::Integrity);
        }
        let (filed_id, pin) = match term.revision {
            RevisionPolicy::Pin { filed, revision } => (filed, Some(revision)),
            RevisionPolicy::Follow { line: selected } if selected == line.id => (line.head, None),
            _ => return Err(OwnerReadError::Integrity),
        };
        let filed = row(ctx, &self.f.filed, filed_id.key())?;
        let status = row(ctx, &self.f.status, filed_id.key())?;
        let identity = filed.content.revision();
        if filed.content.position().charter != charter.id
            || filed.content.position().line != line.id
            || filed.content.parent().is_some()
            || status.filed != filed_id
            || pin.is_some_and(|r| r != identity)
            || (filed.format != 3 && filed.operation != birth.commit)
        {
            return Err(OwnerReadError::Integrity);
        }
        if status.withdrawn_by.is_some() || status.recalled_by.is_some() {
            return Err(OwnerReadError::Refused);
        }
        if filed.format == 3
            && crate::commerce::line_update::audit::birth(ctx, &self.f, filed)
                .map_err(integrity)?
                .commit
                != birth.commit
        {
            return Err(OwnerReadError::Integrity);
        }
        let rating = crate::commerce::rating::reconformance::current::receipt(
            ctx,
            &self.f,
            &self.ratings,
            filed,
            &self.expected,
        )
        .map_err(|_| OwnerReadError::Refused)?;
        if rating.subject().revision() != identity
            || rating.subject().definition() != filed.definition
            || rating.subject().schema() != line.schema_digest
        {
            return Err(OwnerReadError::Integrity);
        }
        for anchor in [
            RootAnchor::Filed(filed_id),
            RootAnchor::Rating {
                job: rating.publication().job(),
                id: rating.id(),
            },
        ] {
            roll.check_required_root(&RequiredRoot { anchor, identity })
                .map_err(integrity)?;
        }
        if pin.is_some() {
            roll.check_required_root(&RequiredRoot {
                anchor: RootAnchor::Term(licence.id, term.generation),
                identity,
            })
            .map_err(integrity)?;
        }
        let reference = identity.revision();
        let revision = roll
            .revision(reference.key.character, reference.key)
            .map_err(|_| OwnerReadError::Unavailable)?;
        if !identity.matches(revision).map_err(integrity)?
            || schema_line_digest(revision.envelope()).map_err(integrity)? != line.schema_digest
            || canonical_duplicate_digest(revision).map_err(integrity)? != filed.duplicate
        {
            return Err(OwnerReadError::Integrity);
        }
        Ok(HouseSaleFacts {
            requester: ctx.actor(),
            issuer,
            organization_revision,
            scope_revision,
            charter: charter.id,
            title_revision: title.revision,
            line: line.id,
            filed: filed_id,
            sequence: filed.content.position().sequence.get(),
            owner_licence: licence.id,
            owner_term: term.generation,
            owner_window: term.window,
            revision: identity,
            schema: line.schema_digest,
            definition: filed.definition,
            rating: rating.reference().map_err(integrity)?,
            compatibility: rating.compatibility().clone(),
            declared: rating.subject().declared(),
            minimum: rating.minimum(),
            observed_ic,
        })
    }
}
