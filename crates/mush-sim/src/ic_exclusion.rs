//! Durable exclusion owned by Sim for the closed IC mind custody union.
//!
//! These records are not permission to seal, launch, or close a sortie. Only
//! the fixed Tacsim action provider may request this participant; that provider
//! must pair actual Roll, Tactics and Tacsim owners in the same commit. Public
//! knowledge of an identity or a recorded close is never a close capability.

use mush_core::action::PrincipalId;
use mush_core::authoritative::{AuthoritativeKey, ReplayWrite};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistry,
    DefinitionRegistryBuilder, DefinitionValidationContext, OwnerId,
};
use mush_core::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPrepareContext, ParticipantError,
};
use mush_core::{
    CommitToken, DefinitionDigest, DefinitionRef, Flags, ModError, ModuleBootData, ModuleCtx,
    ObjId, ObjKind, Registrar, World,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};

const PREFIX: &str = "ic_exclusion:";
pub const MAX_IC_EXCLUSIONS: usize = 1024;
pub const MAX_IC_EXCLUSION_HISTORY: usize = 8192;
pub const MAX_IC_EXCLUSION_BYTES: usize = 8192;
pub const MAX_IC_BASES: usize = 32;
const MAX_BASE_BYTES: usize = 4096;
fn error(reason: impl std::fmt::Display) -> ParticipantError {
    tracing::debug!(target: "ic_claim", %reason, "sim: IC exclusion refused");
    ParticipantError::refused("IC exclusion is unavailable or changed")
}
fn bad(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}

/// Exact data identity supplied only through an actual registered preparation.
/// Constructing this value does not supply the corresponding owner grant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IcIdentity {
    pub open: CommitToken,
    pub unit: ObjId,
    pub sortie: u64,
    pub seal_digest: DefinitionDigest,
    pub custody_digest: DefinitionDigest,
}
impl IcIdentity {
    fn validate(&self) -> Result<(), ParticipantError> {
        if self.unit.0 == 0
            || self.sortie == 0
            || [self.seal_digest, self.custody_digest]
                .iter()
                .any(|d| d.as_bytes() == &[0; 32])
        {
            return Err(error("identity"));
        }
        Ok(())
    }
}
/// Structural event reference. The fixed higher owner must independently hold
/// the genuine opaque event; a decoded reference cannot request a close alone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IcEventRef {
    pub sequence: u64,
    pub digest: DefinitionDigest,
}
impl IcEventRef {
    fn validate(&self) -> Result<(), ParticipantError> {
        if self.sequence == 0 || self.digest.as_bytes() == &[0; 32] {
            return Err(error("event"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenImage {
    identity: IcIdentity,
    action: DefinitionRef,
    dispatcher: ObjId,
    placement: crate::data::PlacementRecord,
    /// A configured fixed base's image. Exactly one of `base` and `hangar` is present; a
    /// base-held record serializes as before doc 23 cut S.4b-1, so its digest is unmoved.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    base: Option<BaseImage>,
    /// **A carrier bay's image** (doc 23 cut S.4b-1, D-23-11/D-23-15): the hangar the claim
    /// opened in, by id.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    hangar: Option<HangarImage>,
}
fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}
/// Where an Open was admitted: a configured fixed base or a carrier's bay (doc 23 cut
/// S.4b-1). Never evidence of a later return.
#[derive(Clone, Debug, PartialEq)]
enum AnchorImage {
    Base(Box<BaseImage>),
    Hangar(Box<HangarImage>),
}
impl OpenImage {
    fn anchor(&self) -> Result<AnchorImage, ParticipantError> {
        match (&self.base, &self.hangar) {
            (Some(base), None) => Ok(AnchorImage::Base(Box::new(base.clone()))),
            (None, Some(hangar)) => Ok(AnchorImage::Hangar(Box::new(hangar.clone()))),
            _ => Err(error("exactly one admission anchor")),
        }
    }
    /// The interior the hull was admitted in.
    fn space(&self) -> Option<&crate::data::InteriorSpaceRecord> {
        self.base
            .as_ref()
            .map(|b| &b.space)
            .or(self.hangar.as_ref().map(|h| &h.space))
    }
    /// The object the admitting interior hangs on: a base domain's host or the carrier.
    fn host(&self) -> Option<ObjId> {
        self.base
            .as_ref()
            .map(|b| b.domain.host)
            .or(self.hangar.as_ref().map(|h| h.carrier))
    }
}
/// **A carrier bay at admission** (doc 23 cut S.4b-1, D-23-11): the bay's live interior,
/// anchored `MovingObject { host: carrier }`, and the carrier. The claim's anchor is the hangar
/// by id; this image records where the Open was admitted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HangarImage {
    space: crate::data::InteriorSpaceRecord,
    carrier: ObjId,
}
impl HangarImage {
    fn validate(&self, placement: &crate::data::PlacementRecord) -> Result<(), ParticipantError> {
        let crate::SpatialParent::Interior { space, slot } = &placement.parent else {
            return Err(error("not in a carrier's bay"));
        };
        let anchored = matches!(
            self.space.anchor,
            crate::SpaceAnchor::MovingObject { host } if host == self.carrier
        );
        if *space != self.space.object
            || placement.unplaced.is_some()
            || slot
                .as_ref()
                .is_some_and(|s| !self.space.compartments.contains_key(s))
            || self.carrier.0 == 0
            || self.carrier == placement.object
            || self.space.object == placement.object
            || !anchored
        {
            return Err(error("hangar identity"));
        }
        Ok(())
    }
}
/// An exact authored base and its live spatial image at admission. This image
/// records where Open was admitted; it is never evidence of a later return.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BaseImage {
    definition: DefinitionRef,
    space: crate::data::InteriorSpaceRecord,
    domain: crate::data::DomainRecord,
}
impl BaseImage {
    fn validate(&self, placement: &crate::data::PlacementRecord) -> Result<(), ParticipantError> {
        let expected =
            ConfiguredIcBase::new(self.definition.clone(), self.space.clone()).map_err(error)?;
        let crate::SpatialParent::Interior { space, slot } = &placement.parent else {
            return Err(error("not in a configured base"));
        };
        if *space != expected.space.object
            || placement.unplaced.is_some()
            || slot
                .as_ref()
                .is_some_and(|s| !expected.space.compartments.contains_key(s))
            || self.domain.id != expected.domain()?
            || self.domain.status != crate::DomainStatus::Active
        {
            return Err(error("base identity"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClosedImage {
    operation: CommitToken,
    action: DefinitionRef,
    event: IcEventRef,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    format: u16,
    open: OpenImage,
    #[serde(deserialize_with = "required_close")]
    closed: Option<ClosedImage>,
}
fn required_close<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<ClosedImage>, D::Error> {
    Option::deserialize(d)
}
impl Record {
    fn key(&self) -> String {
        format!("{PREFIX}{}", self.open.identity.open.to_hex())
    }
    fn validate(&self) -> Result<(), ParticipantError> {
        self.open.identity.validate()?;
        match self.open.anchor()? {
            AnchorImage::Base(base) => base.validate(&self.open.placement)?,
            AnchorImage::Hangar(hangar) => hangar.validate(&self.open.placement)?,
        }
        if self.format != 1
            || self.open.dispatcher.0 == 0
            || self.open.placement.object != self.open.identity.unit
        {
            return Err(error("record"));
        }
        for action in
            std::iter::once(&self.open.action).chain(self.closed.iter().map(|v| &v.action))
        {
            if action.key().owner.as_str() != "tacsim"
                || action.key().kind.as_str() != "action"
                || action.version() != 1
            {
                return Err(error("action"));
            }
        }
        if self.open.action.key().id.as_str() != "open_ic_mind" {
            return Err(error("open action"));
        }
        if let Some(close) = &self.closed {
            close.event.validate()?;
            if close.operation == self.open.identity.open
                || close.action.key().id.as_str() != "close_ic_mind"
            {
                return Err(error("close action"));
            }
        }
        mush_core::canonical_json_bytes_with_limit(
            &serde_json::to_value(self).map_err(error)?,
            MAX_IC_EXCLUSION_BYTES,
        )
        .map_err(error)?;
        Ok(())
    }
    fn digest(&self) -> Result<DefinitionDigest, ParticipantError> {
        self.validate()?;
        DefinitionDigest::for_canonical_value(
            "sim.ic_exclusion.open.v1",
            &serde_json::to_value(&self.open).map_err(error)?,
        )
        .map_err(error)
    }
}

/// Private complete projection reconstructed by the real Sim boot hook. Closed
/// histories stay durable, while only open identities exclude ordinary world use.
#[derive(Clone, Default)]
pub struct IcExclusions {
    generation: u64,
    loaded: bool,
    records: BTreeMap<CommitToken, Arc<Record>>,
    active: BTreeMap<ObjId, CommitToken>,
}
impl IcExclusions {
    fn reconstruct(rows: impl IntoIterator<Item = Record>) -> Result<Self, ParticipantError> {
        let mut value = Self {
            loaded: true,
            ..Self::default()
        };
        let mut closing = BTreeSet::new();
        let mut sorties = BTreeSet::new();
        for row in rows {
            row.validate()?;
            if value.records.len() >= MAX_IC_EXCLUSION_HISTORY
                || value.records.contains_key(&row.open.identity.open)
                || !sorties.insert(row.open.identity.sortie)
            {
                return Err(error("duplicate or full history"));
            }
            if let Some(close) = &row.closed {
                if !closing.insert(close.operation) {
                    return Err(error("duplicate close"));
                }
            } else {
                if value.active.len() >= MAX_IC_EXCLUSIONS
                    || value
                        .active
                        .insert(row.open.identity.unit, row.open.identity.open)
                        .is_some()
                {
                    return Err(error("duplicate or full exclusion"));
                }
            }
            value.records.insert(row.open.identity.open, Arc::new(row));
        }
        if closing
            .iter()
            .any(|commit| value.records.contains_key(commit))
        {
            return Err(error("crossed operation"));
        }
        Ok(value)
    }
    fn ready(&self) -> Result<(), ParticipantError> {
        if !self.loaded {
            return Err(error("not booted"));
        }
        Ok(())
    }
    /// Current movement exclusion preimage, without a custody capability or
    /// copying historical records. The actual owner generation also catches a
    /// custody open/close cycle while a movement is held by the writer.
    pub(crate) fn movement_preimage(
        &self,
        roots: &BTreeSet<ObjId>,
    ) -> Result<(u64, Vec<(ObjId, bool)>), ParticipantError> {
        self.ready()?;
        if roots.is_empty()
            || roots.len() > mush_core::physical_transition::MAX_PHYSICAL_READ_DEPENDENCIES
        {
            return Err(error("movement exclusion scope exceeds bounds"));
        }
        let mut rows = Vec::with_capacity(roots.len());
        for root in roots {
            let identity = self.identity(*root);
            if self.active.contains_key(root) != identity.is_some() {
                return Err(error("movement exclusion index differs"));
            }
            rows.push((*root, identity.is_some()));
        }
        Ok((self.generation, rows))
    }
    fn identity(&self, unit: ObjId) -> Option<&IcIdentity> {
        self.records
            .get(self.active.get(&unit)?)
            .map(|record| &record.open.identity)
    }
    /// Read-only historical identity for the mandatory cross-owner boot audit.
    /// This is not a custody capability or a current permission.
    pub fn inventory(&self) -> Result<Vec<IcExclusionView<'_>>, ParticipantError> {
        self.ready()?;
        Ok(self
            .records
            .values()
            .map(|record| IcExclusionView { record })
            .collect())
    }
}
/// Borrowed actual-owner view, without a decoder or authority constructor.
pub struct IcExclusionView<'a> {
    record: &'a Record,
}
impl IcExclusionView<'_> {
    pub fn identity(&self) -> &IcIdentity {
        &self.record.open.identity
    }
    pub fn dispatcher(&self) -> ObjId {
        self.record.open.dispatcher
    }
    pub fn digest(&self) -> Result<DefinitionDigest, ParticipantError> {
        self.record.digest()
    }
    pub fn closed(&self) -> Option<(CommitToken, &DefinitionRef, &IcEventRef)> {
        self.record
            .closed
            .as_ref()
            .map(|v| (v.operation, &v.action, &v.event))
    }
}
#[must_use]
pub fn identity_of(world: &World, unit: ObjId) -> Option<&IcIdentity> {
    world.module_state::<IcExclusions>()?.identity(unit)
}
/// **The carrier whose bay a held hull's claim opened in** (doc 23 cut S.4b-2, D-23-15): `Some`
/// for an active hold anchored in a hangar — the surface host's claim, which coexists with a
/// field's mark — `None` for a hold in a fixed base or no hold at all.
#[must_use]
pub fn held_in_hangar(world: &World, unit: ObjId) -> Option<ObjId> {
    let state = world.module_state::<IcExclusions>()?;
    let record = state.records.get(state.active.get(&unit)?)?;
    record.open.hangar.as_ref().map(|hangar| hangar.carrier)
}
/// Host-authored immutable base selection, not a player-supplied admission
/// parameter. Its exact descriptor must be registered by Tacsim, and its live
/// interior must still equal this authored image at preparation and custody.
#[derive(Clone, Debug)]
pub struct ConfiguredIcBase {
    definition: DefinitionRef,
    space: crate::data::InteriorSpaceRecord,
}
impl ConfiguredIcBase {
    pub fn descriptor(
        id: DefinitionId,
        space: &crate::data::InteriorSpaceRecord,
    ) -> Result<DefinitionDescriptor, ModError> {
        let payload = serde_json::json!({"format":1,"protocol":"ic_base_v1","interior":space});
        mush_core::canonical_json_bytes_with_limit(&payload, MAX_BASE_BYTES).map_err(bad)?;
        let mut refs = Vec::new();
        if let crate::SpaceAnchor::FixedDomain {
            anchor_pose: Some(pose),
            ..
        } = &space.anchor
        {
            refs.push(pose.schema().clone());
        }
        DefinitionDescriptor::new(DefinitionKind::new("ic_base").map_err(bad)?, id, 1, payload)
            .map(|d| d.with_references(refs))
            .map_err(bad)
    }
    pub fn new(
        definition: DefinitionRef,
        space: crate::data::InteriorSpaceRecord,
    ) -> Result<Self, ModError> {
        if definition.key().owner.as_str() != "tacsim"
            || definition.key().kind.as_str() != "ic_base"
            || definition.version() != 1
            || space.object.0 == 0
            || !matches!(space.anchor, crate::SpaceAnchor::FixedDomain { .. })
        {
            return Err(bad("invalid fixed IC base"));
        }
        let expected = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new("tacsim").map_err(bad)?,
                Self::descriptor(definition.key().id.clone(), &space)?,
            )
            .map_err(bad)?;
        if definition != expected {
            return Err(bad("IC base definition differs"));
        }
        Ok(Self { definition, space })
    }
    pub fn definition(&self) -> &DefinitionRef {
        &self.definition
    }
    /// Exact immutable declaration for the trusted Tacsim composition that
    /// selected this already-validated base. This exposes no placement writer.
    pub fn registration_descriptor(&self) -> Result<DefinitionDescriptor, ModError> {
        Self::descriptor(self.definition.key().id.clone(), &self.space)
    }
    fn domain(&self) -> Result<crate::DomainId, ParticipantError> {
        match self.space.anchor {
            crate::SpaceAnchor::FixedDomain { domain, .. } => Ok(domain),
            _ => Err(error("base anchor")),
        }
    }
}

/// Fixed trusted composition input. Neither a capacity nor arbitrary action or
/// participant locators can be selected. The exact refs must exist at freeze.
#[derive(Clone, Debug)]
pub struct IcExclusionConfig {
    manifest: DefinitionRef,
    participants: [DefinitionRef; 4],
    actions: [DefinitionRef; 2],
    bases: Vec<ConfiguredIcBase>,
    /// **Carrier bays admit an Open** (doc 23 cut S.4b-1, D-23-15): a hull `Interior` to an
    /// interior anchored `MovingObject { host }` is admitted with the bay as its hangar.
    hangars: bool,
    definitions: Arc<OnceLock<DefinitionRegistry>>,
}
pub fn participant_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(DefinitionKind::new("owner_participant").map_err(bad)?,DefinitionId::new("ic_exclusion").map_err(bad)?,1,serde_json::json!({"format":1,"protocol":"ic_exclusion_v1","max_open":MAX_IC_EXCLUSIONS,"max_history":MAX_IC_EXCLUSION_HISTORY,"max_record_bytes":MAX_IC_EXCLUSION_BYTES})).map_err(bad)
}
impl IcExclusionConfig {
    pub fn new(
        manifest: DefinitionRef,
        participants: [DefinitionRef; 4],
        bases: Vec<ConfiguredIcBase>,
    ) -> Result<Self, ModError> {
        Self::build(manifest, participants, bases, false)
    }
    /// **The composition that admits carrier bays** (doc 23 cut S.4b-1, D-23-11/D-23-15): as
    /// [`IcExclusionConfig::new`], and an Open is also admitted for a hull `Interior` to a
    /// carrier's bay (an interior anchored `MovingObject`), recording the hangar; `bases` may
    /// then be empty. Doc 23 S.4b-2 composes it for the surface host's claims.
    pub fn with_hangars(
        manifest: DefinitionRef,
        participants: [DefinitionRef; 4],
        bases: Vec<ConfiguredIcBase>,
    ) -> Result<Self, ModError> {
        Self::build(manifest, participants, bases, true)
    }
    fn build(
        manifest: DefinitionRef,
        participants: [DefinitionRef; 4],
        bases: Vec<ConfiguredIcBase>,
        hangars: bool,
    ) -> Result<Self, ModError> {
        let mut spaces = BTreeSet::new();
        let mut references = BTreeSet::new();
        if (bases.is_empty() && !hangars)
            || bases.len() > MAX_IC_BASES
            || bases.iter().any(|base| {
                !spaces.insert(base.space.object) || !references.insert(base.definition.clone())
            })
        {
            return Err(bad("IC bases are absent, duplicated or over bound"));
        }
        if manifest.key().owner.as_str() != "tacsim"
            || manifest.key().kind.as_str() != "registry_manifest"
            || manifest.key().id.as_str() != "combat"
            || manifest.version() != 1
        {
            return Err(bad("IC manifest differs"));
        }
        for (reference, (owner, id)) in participants.iter().zip([
            ("roll", "commercial_roots"),
            ("tacsim", "ic_sortie"),
            ("tactics", "ic_custody"),
            ("sim", "ic_exclusion"),
        ]) {
            if reference.key().owner.as_str() != owner
                || reference.key().kind.as_str() != "owner_participant"
                || reference.key().id.as_str() != id
                || reference.version() != 1
            {
                return Err(bad("IC participant differs"));
            }
        }
        let expected = DefinitionRegistryBuilder::new()
            .register(OwnerId::new("sim").map_err(bad)?, participant_descriptor()?)
            .map_err(bad)?;
        if participants.last() != Some(&expected) {
            return Err(bad("IC exclusion descriptor differs"));
        }
        let action = |transition: &str| -> Result<DefinitionRef, ModError> {
            let mut refs = vec![manifest.clone()];
            refs.extend(participants.iter().cloned());
            let descriptor=DefinitionDescriptor::new(DefinitionKind::new("action").map_err(bad)?,DefinitionId::new(format!("{transition}_ic_mind")).map_err(bad)?,1,serde_json::json!({"format":1,"protocol":"ic_mind_custody_v1","transition":transition})).map_err(bad)?.with_references(refs);
            DefinitionRegistryBuilder::new()
                .register(OwnerId::new("tacsim").map_err(bad)?, descriptor)
                .map_err(bad)
        };
        let actions = [action("open")?, action("close")?];
        Ok(Self {
            manifest,
            participants,
            actions,
            bases,
            hangars,
            definitions: Arc::new(OnceLock::new()),
        })
    }
    fn check(
        &self,
        context: &OwnerPrepareContext<'_>,
        index: usize,
    ) -> Result<(), ParticipantError> {
        if self.actions.get(index) != Some(context.action()) {
            return Err(error("action"));
        }
        let definitions = self.definitions.get().ok_or_else(|| error("definitions"))?;
        for reference in std::iter::once(&self.manifest)
            .chain(self.participants.iter())
            .chain(self.actions.iter())
            .chain(self.bases.iter().map(|b| &b.definition))
        {
            definitions.get_exact(reference).map_err(error)?;
        }
        Ok(())
    }
    pub(crate) fn register(&self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        if reg.module() != "sim" {
            return Err(bad("IC exclusion owner differs"));
        }
        let expected = participant_descriptor()?;
        let expected_clone = expected.clone();
        let physical = [
            crate::physical::participant_descriptor_v1()?,
            crate::physical::participant_descriptor()?,
        ];
        let refs: Vec<_> = std::iter::once(self.manifest.clone())
            .chain(self.participants.iter().cloned())
            .chain(self.actions.iter().cloned())
            .chain(self.bases.iter().map(|b| b.definition.clone()))
            .collect();
        reg.definition_validator(
            DefinitionKind::new("owner_participant").map_err(bad)?,
            move |ctx: &DefinitionValidationContext<'_>| {
                let expected = if ctx.definition().reference().key().id == *expected_clone.id() {
                    &expected_clone
                } else if let Some(physical) = physical.iter().find(|descriptor| {
                    ctx.definition().reference().key().id == *descriptor.id()
                        && ctx.definition().reference().version() == descriptor.version()
                }) {
                    physical
                } else {
                    return Err("unknown Sim participant descriptor".into());
                };
                if ctx.definition().reference().version() != expected.version()
                    || ctx.definition().is_current() != expected.is_current()
                    || ctx.definition().payload() != expected.payload()
                    || ctx.definition().references() != expected.references()
                {
                    return Err("Sim participant descriptor differs".into());
                }
                for reference in &refs {
                    ctx.get_exact(reference).map_err(|e| e.to_string())?;
                }
                Ok(())
            },
        )?;
        let definition = reg.definition(expected)?;
        reg.owner_participant(
            definition,
            BTreeSet::from(self.actions.clone()),
            IcExclusionParticipant {
                config: self.clone(),
            },
        )
    }
    pub(crate) fn loaded(&self, ctx: &ModuleCtx<'_>) -> Result<(), ModError> {
        self.definitions
            .set(
                ctx.reg
                    .definitions()
                    .ok_or_else(|| bad("IC definitions missing"))?
                    .clone(),
            )
            .map_err(|_| bad("IC exclusion already initialized"))
    }
}

/// The fixed provider constructs this only after resolving its genuine seal
/// owner and Tactics preparation. It contains no writable placement image.
pub enum IcExclusionRequest {
    Open(IcIdentity),
    Close {
        open: CommitToken,
        event: IcEventRef,
    },
}
pub struct IcExclusionOutput {
    action: DefinitionRef,
    commit: CommitToken,
    identity: IcIdentity,
    digest: DefinitionDigest,
    event: Option<IcEventRef>,
}
impl IcExclusionOutput {
    pub fn action(&self) -> &DefinitionRef {
        &self.action
    }
    pub fn commit(&self) -> CommitToken {
        self.commit
    }
    pub fn identity(&self) -> &IcIdentity {
        &self.identity
    }
    pub fn digest(&self) -> DefinitionDigest {
        self.digest
    }
    pub fn event(&self) -> Option<&IcEventRef> {
        self.event.as_ref()
    }
}
pub struct IcExclusionParticipant {
    config: IcExclusionConfig,
}
pub struct IcExclusionPatch {
    before: u64,
    action: DefinitionRef,
    commit: CommitToken,
    actor: ObjId,
    identity: IcIdentity,
    placement: Option<(crate::data::PlacementRecord, AnchorImage)>,
    post: IcExclusions,
}
pub struct BoundIcExclusion<'a> {
    state: &'a mut IcExclusions,
    post: IcExclusions,
}
impl BoundParticipant for BoundIcExclusion<'_> {
    fn apply(self) {
        *self.state = self.post;
    }
}
fn current_placement(
    world: &World,
    unit: ObjId,
) -> Result<crate::data::PlacementRecord, ParticipantError> {
    let state = crate::state(world).ok_or_else(|| error("spatial unavailable"))?;
    state
        .placements
        .get(&unit)
        .cloned()
        .ok_or_else(|| error("unit unplaced"))
}
fn admit_world(
    world: &World,
    unit: ObjId,
    actor: ObjId,
    config: &IcExclusionConfig,
) -> Result<(crate::data::PlacementRecord, AnchorImage), ParticipantError> {
    if world
        .get(unit)
        .is_none_or(|row| row.kind != ObjKind::Thing || row.flags.contains(Flags::GOING))
        || world
            .get(actor)
            .is_none_or(|row| row.kind != ObjKind::Player || row.flags.contains(Flags::GOING))
    {
        return Err(error("object"));
    }
    if crate::deployed::mark_of(world, unit).is_some() || identity_of(world, unit).is_some() {
        return Err(error("already held"));
    }
    for kind in [
        std::any::TypeId::of::<crate::SpatialState>(),
        std::any::TypeId::of::<crate::JourneyBook>(),
        std::any::TypeId::of::<crate::DeployedUnits>(),
        std::any::TypeId::of::<IcExclusions>(),
    ] {
        if world
            .module_state_owner(kind)
            .is_none_or(|owner| owner.as_str() != "sim")
        {
            return Err(error("foreign owner state"));
        }
    }
    let state = crate::state(world).ok_or_else(|| error("spatial unavailable"))?;
    if state.held_placements.contains_key(&unit) || !state.unbound_placement_docs.is_empty() {
        return Err(error("held placement"));
    }
    let journeys = world
        .module_state::<crate::JourneyBook>()
        .ok_or_else(|| error("journeys unavailable"))?;
    if !journeys.held.is_empty() {
        return Err(error("held journey"));
    }
    let mut subjects = crate::graph::ancestors(state, unit).map_err(error)?;
    subjects.push(unit);
    for subject in subjects {
        if journeys.under_way(subject)
            || crate::journey::journeying_reason(journeys, world, subject).is_some()
        {
            return Err(error("journey"));
        }
    }
    let participant = mush_core::BehaviorId::new("sim.participant").map_err(error)?;
    if !world.has_behavior(unit, &participant) {
        return Err(error("unit binding"));
    }
    let placement = current_placement(world, unit)?;
    let crate::SpatialParent::Interior { space, .. } = &placement.parent else {
        return Err(error("not in a configured base"));
    };
    let Some(configured) = config.bases.iter().find(|b| b.space.object == *space) else {
        return admit_hangar(world, state, &placement, config);
    };
    let interior = mush_core::BehaviorId::new("sim.interior").map_err(error)?;
    if !world.has_behavior(*space, &interior)
        || state.spaces.get(space) != Some(&configured.space)
        || world
            .get(*space)
            .is_none_or(|row| row.kind != ObjKind::Room || row.flags.contains(Flags::GOING))
        || world.get(unit).and_then(|row| row.location) != Some(*space)
    {
        return Err(error("base changed"));
    }
    let domain = state
        .domains
        .get(&configured.domain()?)
        .ok_or_else(|| error("base domain"))?
        .clone();
    if world
        .get(domain.host)
        .is_none_or(|row| row.flags.contains(Flags::GOING))
    {
        return Err(error("domain host"));
    }
    let definitions = config
        .definitions
        .get()
        .ok_or_else(|| error("definitions"))?;
    definitions.get_exact(&domain.driver).map_err(error)?;
    if let crate::SpaceAnchor::FixedDomain {
        anchor_pose: Some(pose),
        ..
    } = &configured.space.anchor
    {
        definitions.get_exact(pose.schema()).map_err(error)?;
    }
    let base = BaseImage {
        definition: configured.definition.clone(),
        space: configured.space.clone(),
        domain,
    };
    base.validate(&placement)?;
    Ok((placement, AnchorImage::Base(Box::new(base))))
}
/// **A hull in a carrier's bay** (doc 23 cut S.4b-1, D-23-11): admitted only where the
/// composition admits hangars, for a hull `Interior` to a live interior anchored `MovingObject
/// { host }` on a live carrier. The carrier's journeys were refused above (the ancestors).
fn admit_hangar(
    world: &World,
    state: &crate::SpatialState,
    placement: &crate::data::PlacementRecord,
    config: &IcExclusionConfig,
) -> Result<(crate::data::PlacementRecord, AnchorImage), ParticipantError> {
    let crate::SpatialParent::Interior { space, .. } = &placement.parent else {
        return Err(error("not in a carrier's bay"));
    };
    if !config.hangars {
        return Err(error("unconfigured base"));
    }
    let record = state
        .spaces
        .get(space)
        .ok_or_else(|| error("bay interior"))?;
    let crate::SpaceAnchor::MovingObject { host } = record.anchor else {
        return Err(error("unconfigured base"));
    };
    let interior = mush_core::BehaviorId::new("sim.interior").map_err(error)?;
    if !world.has_behavior(*space, &interior)
        || world
            .get(*space)
            .is_none_or(|row| row.kind != ObjKind::Room || row.flags.contains(Flags::GOING))
        || world.get(placement.object).and_then(|row| row.location) != Some(*space)
        || world
            .get(host)
            .is_none_or(|row| row.flags.contains(Flags::GOING))
    {
        return Err(error("bay changed"));
    }
    let hangar = HangarImage {
        space: record.clone(),
        carrier: host,
    };
    hangar.validate(placement)?;
    Ok((placement.clone(), AnchorImage::Hangar(Box::new(hangar))))
}
impl OwnerParticipant for IcExclusionParticipant {
    type State = IcExclusions;
    type Request = IcExclusionRequest;
    type Output = IcExclusionOutput;
    type Patch = IcExclusionPatch;
    type Bound<'a> = BoundIcExclusion<'a>;
    fn prepare(
        &self,
        state: &IcExclusions,
        ctx: &OwnerPrepareContext<'_>,
        request: Self::Request,
    ) -> Result<OwnerPreparation<Self::Output, Self::Patch>, ParticipantError> {
        state.ready()?;
        let mut post = state.clone();
        let (record, placement) = match request {
            IcExclusionRequest::Open(identity) => {
                self.config.check(ctx, 0)?;
                identity.validate()?;
                if identity.open != ctx.commit_token()
                    || ctx.invocation().principal()
                        != &(PrincipalId::Object {
                            id: ctx.invocation().actor(),
                        })
                    || state.records.len() >= MAX_IC_EXCLUSION_HISTORY
                    || state.active.len() >= MAX_IC_EXCLUSIONS
                    || state.records.contains_key(&identity.open)
                    || state
                        .records
                        .values()
                        .any(|r| r.open.identity.sortie == identity.sortie)
                    || state.records.values().any(|r| {
                        r.closed
                            .as_ref()
                            .is_some_and(|c| c.operation == identity.open)
                    })
                {
                    return Err(error("open"));
                }
                let (placement, base) = admit_world(
                    ctx.world(),
                    identity.unit,
                    ctx.invocation().actor(),
                    &self.config,
                )?;
                let (base_image, hangar) = match &base {
                    AnchorImage::Base(b) => (Some((**b).clone()), None),
                    AnchorImage::Hangar(h) => (None, Some((**h).clone())),
                };
                let record = Record {
                    format: 1,
                    open: OpenImage {
                        identity,
                        action: ctx.action().clone(),
                        dispatcher: ctx.invocation().actor(),
                        placement: placement.clone(),
                        base: base_image,
                        hangar,
                    },
                    closed: None,
                };
                (record, Some((placement, base)))
            }
            IcExclusionRequest::Close { open, event } => {
                self.config.check(ctx, 1)?;
                event.validate()?;
                let before = state
                    .records
                    .get(&open)
                    .ok_or_else(|| error("missing open"))?;
                if before.closed.is_some()
                    || state.active.get(&before.open.identity.unit) != Some(&open)
                    || ctx.commit_token() == open
                    || state.records.contains_key(&ctx.commit_token())
                    || state.records.values().any(|r| {
                        r.closed
                            .as_ref()
                            .is_some_and(|c| c.operation == ctx.commit_token())
                    })
                {
                    return Err(error("close"));
                }
                let mut record = (**before).clone();
                record.closed = Some(ClosedImage {
                    operation: ctx.commit_token(),
                    action: ctx.action().clone(),
                    event,
                });
                (record, None)
            }
        };
        record.validate()?;
        if record.closed.is_none() {
            // Capacity accounting only: serialize the maximum future closure
            // shape before Open. This projection is never stored or accepted
            // as a genuine event/receipt, and introduces no padding field.
            let closing = self
                .config
                .actions
                .get(1)
                .ok_or_else(|| error("close definition"))?;
            let future = ClosedImage {
                operation: ctx.commit_token(),
                action: closing.clone(),
                event: IcEventRef {
                    sequence: u64::MAX,
                    digest: DefinitionDigest::from_bytes([255; 32]),
                },
            };
            let mut shape = serde_json::to_value(&record).map_err(error)?;
            *shape
                .get_mut("closed")
                .ok_or_else(|| error("close field"))? =
                serde_json::to_value(future).map_err(error)?;
            mush_core::canonical_json_bytes_with_limit(&shape, MAX_IC_EXCLUSION_BYTES)
                .map_err(error)?;
        }
        let identity = record.open.identity.clone();
        if record.closed.is_some() {
            post.active.remove(&identity.unit);
        } else if post.active.insert(identity.unit, identity.open).is_some() {
            return Err(error("duplicate unit"));
        }
        post.records.insert(identity.open, Arc::new(record.clone()));
        post.generation = post
            .generation
            .checked_add(1)
            .ok_or_else(|| error("generation"))?;
        let output = IcExclusionOutput {
            action: ctx.action().clone(),
            commit: ctx.commit_token(),
            identity: identity.clone(),
            digest: record.digest()?,
            event: record.closed.as_ref().map(|v| v.event.clone()),
        };
        let write = ReplayWrite::ModuleJsonSet {
            namespace: OwnerId::new("sim").map_err(error)?,
            key: AuthoritativeKey::parse_canonical(record.key()).map_err(error)?,
            value: serde_json::to_value(record).map_err(error)?,
        };
        Ok(OwnerPreparation::new(
            output,
            IcExclusionPatch {
                before: state.generation,
                action: ctx.action().clone(),
                commit: ctx.commit_token(),
                actor: ctx.invocation().actor(),
                identity,
                placement,
                post,
            },
            vec![write],
        ))
    }
    fn recheck(
        &self,
        state: &IcExclusions,
        ctx: &OwnerPrepareContext<'_>,
        patch: &Self::Patch,
    ) -> Result<(), ParticipantError> {
        state.ready()?;
        self.config
            .check(ctx, if patch.placement.is_some() { 0 } else { 1 })?;
        if state.generation != patch.before
            || ctx.commit_token() != patch.commit
            || ctx.action() != &patch.action
            || ctx.invocation().actor() != patch.actor
        {
            return Err(error("stale preparation"));
        }
        if let Some(expected) = &patch.placement
            && admit_world(ctx.world(), patch.identity.unit, patch.actor, &self.config)?
                != *expected
        {
            return Err(error("placement changed"));
        }
        Ok(())
    }
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut IcExclusions,
        patch: Self::Patch,
    ) -> Result<Self::Bound<'a>, ParticipantError> {
        state.ready()?;
        if state.generation != patch.before {
            return Err(error("stale bind"));
        }
        Ok(BoundIcExclusion {
            state,
            post: patch.post,
        })
    }
    fn accepts_write(&self, write: &ReplayWrite) -> bool {
        let ReplayWrite::ModuleJsonSet {
            namespace,
            key,
            value,
        } = write
        else {
            return false;
        };
        if namespace.as_str() != "sim" {
            return false;
        }
        let Ok(record) = serde_json::from_value::<Record>(value.clone()) else {
            return false;
        };
        record.validate().is_ok()
            && serde_json::to_value(&record).ok().as_ref() == Some(value)
            && key.as_str() == record.key()
            && self
                .config
                .actions
                .get(if record.closed.is_some() { 1 } else { 0 })
                == Some(
                    record
                        .closed
                        .as_ref()
                        .map(|v| &v.action)
                        .unwrap_or(&record.open.action),
                )
    }
}

/// Load only this closed namespace before ordinary spatial boot. Missing all
/// rows is still checked against the other three real owners at final audit.
pub(crate) fn load(
    ctx: &mut ModuleCtx<'_>,
    data: &mut ModuleBootData,
    enabled: bool,
) -> Result<(), ModError> {
    let mut rows = Vec::new();
    let mut retained = Vec::with_capacity(data.kv.len());
    for (key, value) in std::mem::take(&mut data.kv) {
        if !key.starts_with(PREFIX) {
            retained.push((key, value));
            continue;
        }
        if !enabled {
            return Err(bad(
                "retained IC exclusions require their owner configuration",
            ));
        }
        mush_core::canonical_json_bytes_with_limit(&value, MAX_IC_EXCLUSION_BYTES).map_err(bad)?;
        let record: Record = serde_json::from_value(value.clone()).map_err(bad)?;
        if serde_json::to_value(&record).map_err(bad)? != value {
            return Err(bad("IC exclusion is not exact closed data"));
        }
        record.validate().map_err(bad)?;
        if record.key() != key.as_str() || rows.len() >= MAX_IC_EXCLUSION_HISTORY {
            return Err(bad("IC exclusion key or bound differs"));
        }
        rows.push(record);
    }
    data.kv = retained;
    let state = IcExclusions::reconstruct(rows).map_err(bad)?;
    *ctx.w
        .module_state_mut::<IcExclusions>()
        .ok_or_else(|| bad("IC exclusion owner unavailable"))? = state;
    Ok(())
}

/// Called before legacy spatial boot emits any healing/deletion or projects a
/// retained hull. Invalid custody must stay retained for repair, never become
/// an ordinary quarantined/unplaced object that another lane can reuse.
pub(crate) fn check_boot_spatial(
    ctx: &ModuleCtx<'_>,
    spatial: &crate::SpatialState,
) -> Result<(), ModError> {
    let Some(state) = ctx.w.module_state::<IcExclusions>() else {
        return Ok(());
    };
    for operation in state.active.values() {
        let record = state
            .records
            .get(operation)
            .ok_or_else(|| bad("IC exclusion index differs"))?;
        let open = &record.open;
        let interior = open
            .space()
            .ok_or_else(|| bad("IC admission anchor missing"))?;
        // Doc 23 cut S.4b-2 (D-23-15): a hold in a hangar is moved by the epoch's hosts, so its
        // hull stands on a domain or in a bay, never at the Open's image; a base's stands there.
        let placed = spatial.placements.get(&open.identity.unit);
        let stands = match (&open.hangar, placed) {
            (Some(_), Some(placement)) => {
                placement.unplaced.is_none()
                    && match &placement.parent {
                        crate::SpatialParent::Direct { .. } => true,
                        crate::SpatialParent::Interior { space, .. } => {
                            spatial.spaces.get(space).is_some_and(|s| {
                                matches!(s.anchor, crate::SpaceAnchor::MovingObject { .. })
                            })
                        }
                        _ => false,
                    }
            }
            (Some(_), None) => false,
            (None, placed) => placed == Some(&open.placement),
        };
        let placement = placed.unwrap_or(&open.placement);
        if !stands
            || spatial.held_placements.contains_key(&open.identity.unit)
            || !spatial.unbound_placement_docs.is_empty()
            || spatial.spaces.get(&interior.object) != Some(interior)
            || crate::boot::validate_live(ctx, spatial, placement).is_some()
            || crate::graph::project(spatial, ctx.w, open.identity.unit)
                != ctx.w.get(open.identity.unit).and_then(|row| row.location)
            || crate::boot::invalid_members(spatial, ctx.w)
                .iter()
                .any(|(space, _)| *space == interior.object)
        {
            return Err(bad("held IC hull placement requires explicit recovery"));
        }
        // Doc 23 cut S.4b-1: a hangar's carrier must stand; a base's domain must too.
        if let Some(hangar) = &open.hangar
            && ctx
                .w
                .get(hangar.carrier)
                .is_none_or(|o| o.flags.contains(Flags::GOING))
        {
            return Err(bad("held IC hangar requires explicit recovery"));
        }
        let Some(base) = &open.base else {
            continue;
        };
        let domain = spatial
            .domains
            .get(&base.domain.id)
            .ok_or_else(|| bad("IC base domain missing"))?;
        if domain.status == crate::DomainStatus::Retired
            || domain.driver != base.domain.driver
            || ctx
                .w
                .get(domain.host)
                .is_none_or(|o| o.flags.contains(Flags::GOING))
        {
            return Err(bad("held IC base requires explicit recovery"));
        }
    }
    Ok(())
}
pub(crate) fn check_boot_journeys(
    world: &World,
    journeys: &crate::journey::HydratedJourneys,
) -> Result<(), ModError> {
    let state = world
        .module_state::<IcExclusions>()
        .ok_or_else(|| bad("IC exclusion owner missing"))?;
    // A held hull, its interior and that interior's host (a fixed base's domain host, or,
    // doc 23 cut S.4b-1, a hangar's carrier) carry no journey while the claim is held in it.
    // Unknown journey keys may name a held hull, so no absence conclusion can be drawn.
    if !state.active.is_empty() && !journeys.held.is_empty() {
        return Err(bad("held IC hull and unreadable journey"));
    }
    if journeys.journeys.iter().any(|j| {
        state.active.values().any(|open| {
            state.records.get(open).is_none_or(|r| {
                j.root() == r.open.identity.unit
                    || (r.open.hangar.is_none()
                        && (r.open.space().is_none_or(|s| j.root() == s.object)
                            || r.open.host().is_none_or(|h| j.root() == h)))
            })
        })
    }) {
        return Err(bad("held IC hull carries a legacy journey"));
    }
    Ok(())
}
/// **A held hull's move its hangar admits** (doc 23 cut S.4b-2, D-23-15: "moved only by the
/// epoch's hosts"): a pose on the domain it stands on; from its hangar, the ramp onto the domain
/// its carrier is landed on (placed `Direct` there, under no journey); a re-parent `Interior` into its own hangar or into a carrier's bay (a
/// recoverer's, S.3; a salvage load's, Q-23-11). Every other change of parent — a staff teleport
/// to another domain, a mount, an unplacing, a fixed room — is refused.
fn hangar_move_admitted(
    state: &crate::SpatialState,
    world: &World,
    unit: ObjId,
    hangar: &HangarImage,
    next: &crate::data::PlacementRecord,
) -> bool {
    // The planner's own spatial state: inside a plan the world's slot is not readable.
    if next.unplaced.is_some() {
        return false;
    }
    let current = state.placements.get(&unit).map(|p| &p.parent);
    match &next.parent {
        crate::SpatialParent::Direct { domain, .. } => match current {
            Some(crate::SpatialParent::Direct { domain: now, .. }) => now == domain,
            // The launch: the ramp from the hangar down onto the very domain the carrier stands
            // on, the carrier landed there — placed `Direct` on it and under no journey — and
            // the domain active. Any other domain is a teleport, refused.
            Some(crate::SpatialParent::Interior { space, .. }) if *space == hangar.space.object => {
                let landed = matches!(
                    state.placements.get(&hangar.carrier),
                    Some(p) if p.unplaced.is_none()
                        && matches!(&p.parent, crate::SpatialParent::Direct { domain: carrier, .. } if carrier == domain)
                );
                let under_way = world
                    .module_state::<crate::JourneyBook>()
                    .is_none_or(|journeys| journeys.under_way(hangar.carrier));
                landed
                    && !under_way
                    && state
                        .domains
                        .get(domain)
                        .is_some_and(|d| d.status == crate::DomainStatus::Active)
            }
            _ => false,
        },
        crate::SpatialParent::Interior { space, .. } => {
            *space == hangar.space.object
                || state
                    .spaces
                    .get(space)
                    .is_some_and(|s| matches!(s.anchor, crate::SpaceAnchor::MovingObject { .. }))
        }
        _ => false,
    }
}

/// Common physical transition guard. Base presentation stays visible, but a
/// transition cannot move/re-anchor a retained hull through an ancestor edit.
pub(crate) fn check_plan(
    spatial: &crate::SpatialState,
    world: &World,
    plan: &crate::transition::Plan,
) -> Result<(), crate::transition::SpatialError> {
    let Some(state) = world.module_state::<IcExclusions>() else {
        return Ok(());
    };
    for (&unit, operation) in &state.active {
        let row = state
            .records
            .get(operation)
            .ok_or(crate::transition::SpatialError::IcCustody(unit))?;
        if row.open.identity.unit != unit || row.closed.is_some() {
            return Err(crate::transition::SpatialError::IcCustody(unit));
        }
        // Doc 23 cut S.4b-2 (D-23-15): a hold in a hangar freezes re-anchoring only.
        if let Some(hangar) = &row.open.hangar {
            let placed = plan
                .set_placements
                .iter()
                .filter(|p| p.object == unit)
                .collect::<Vec<_>>();
            let admitted = placed
                .iter()
                .all(|p| hangar_move_admitted(spatial, world, unit, hangar, p));
            let projected_alone =
                placed.is_empty() && plan.projections.iter().any(|(object, _)| *object == unit);
            let bay_rewritten = plan
                .set_spaces
                .iter()
                .any(|s| s.object == hangar.space.object);
            if !admitted || projected_alone || bay_rewritten {
                tracing::debug!(
                    target: "ic_claim",
                    unit = unit.0,
                    admitted,
                    projected_alone,
                    bay_rewritten,
                    placements = ?placed.iter().map(|p| &p.parent).collect::<Vec<_>>(),
                    current = ?spatial.placements.get(&unit).map(|p| &p.parent),
                    "sim: a hangar hold's freeze refused a plan"
                );
                return Err(crate::transition::SpatialError::IcCustody(unit));
            }
            continue;
        }
        if row
            .open
            .base
            .as_ref()
            .is_some_and(|b| plan.retire.contains(&b.domain.id))
            || plan.set_placements.iter().any(|p| p.object == unit)
            || plan.projections.iter().any(|(object, _)| *object == unit)
            || plan
                .set_spaces
                .iter()
                .any(|s| row.open.space().is_none_or(|i| s.object == i.object))
        {
            return Err(crate::transition::SpatialError::IcCustody(unit));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
