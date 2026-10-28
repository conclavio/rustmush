//! The inspection record and the words it is made of. Decoding a record grants nothing.
use super::*;
use mush_core::ClockStamp;
use mush_org::OrgId;
use smol_str::SmolStr;

/// **The class of core a hull carries**, in tacsim's words (`0`, `I`, `II`, `III`): `0` is
/// the drive ROM, a hull with no core component at all. Defined here because the record
/// names it and the Roll cannot name tacsim's `CoreClass`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HullClass {
    /// No core: the drive ROM.
    #[serde(rename = "0")]
    Zero,
    /// The doctrine core.
    #[serde(rename = "I")]
    I,
    /// The line core.
    #[serde(rename = "II")]
    II,
    /// The command core.
    #[serde(rename = "III")]
    III,
}

/// **Where a core came from** (doc 17h §2.3, A6): `certified` when its title was born of a
/// completed core integration whose die lot carries the industry's certificate as its origin,
/// naming the importer whose stock the die came from and the class ordered (the die's grade);
/// `unknown` otherwise. `heirloom` and `grey` are named for the later cuts that will read them
/// (M8c's org holder, M8e's holding); M8a's read never returns them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Provenance {
    /// A certified die, integrated and titled through the economy.
    Certified {
        /// The importer whose stock the die came from.
        importer: OrgId,
        /// The class ordered.
        grade: mush_economy::core_integration::CoreClass,
    },
    /// A core titled before the certificate chain existed.
    Heirloom,
    /// A core whose title has no birth the chain can vouch for.
    Grey,
    /// Nothing the chain can say.
    Unknown,
}

/// **The seal against the fitted hardware** (doc 17h §2.3): the open trip's seal names a
/// core, class and governor; `matches` when the hull reads the same now, `differs` when a cell
/// moved after the seal (the IC audit's own check, as a read), `no_seal` when no trip is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SealCheck {
    Matches,
    Differs,
    NoSeal,
}

/// **What tacsim read of the hull** (the facts half of an [`InspectionRecord`]): the fields
/// the operator's reply prints and the record retains, in the record's own words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InspectionFacts {
    /// The hull inspected.
    pub hull: ObjId,
    /// The core component's asset object, when the component names one.
    pub core: Option<ObjId>,
    /// The class the core's cell reads now (`Zero` with no core).
    pub class: HullClass,
    /// Whether the core runs under the governor; `None` with no core.
    pub governed: Option<bool>,
    /// The core's provenance.
    pub provenance: Provenance,
    /// The open trip's seal digest (all sixty-four characters), when a trip is open.
    pub seal_digest: Option<SmolStr>,
    /// The seal against the fitted hardware.
    pub seal_check: SealCheck,
    /// The licence state the open trip's seal was read under; `None` with no trip or on a
    /// seal recorded before M8a.
    pub licence_state: Option<super::super::SealLicenceState>,
}

/// **One inspection** (doc 17h cut M8a, A4): who asked, under what authority and
/// authorization, what the hull read, when. Keyed by the inspection's commit; immutable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RecordWire")]
pub struct InspectionRecord {
    pub format: u16,
    /// The inspection's commit: the record's key.
    pub inspection: mush_core::CommitToken,
    pub action: DefinitionRef,
    pub authority: RegistryInspectionPolicy,
    /// The Registry operator who asked.
    pub operator: ObjId,
    pub authorization: Authorization,
    pub hull: ObjId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core: Option<ObjId>,
    pub class: HullClass,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub governed: Option<bool>,
    pub provenance: Provenance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seal_digest: Option<SmolStr>,
    pub seal_check: SealCheck,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub licence_state: Option<super::super::SealLicenceState>,
    pub at_ic: ClockStamp,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordWire {
    format: u16,
    inspection: mush_core::CommitToken,
    action: DefinitionRef,
    authority: RegistryInspectionPolicy,
    operator: ObjId,
    authorization: Authorization,
    hull: ObjId,
    #[serde(default)]
    core: Option<ObjId>,
    class: HullClass,
    #[serde(default)]
    governed: Option<bool>,
    provenance: Provenance,
    #[serde(default)]
    seal_digest: Option<SmolStr>,
    seal_check: SealCheck,
    #[serde(default)]
    licence_state: Option<super::super::SealLicenceState>,
    at_ic: ClockStamp,
}
impl TryFrom<RecordWire> for InspectionRecord {
    type Error = PublicationError;
    fn try_from(w: RecordWire) -> Result<Self, Self::Error> {
        let v = Self {
            format: w.format,
            inspection: w.inspection,
            action: w.action,
            authority: w.authority,
            operator: w.operator,
            authorization: w.authorization,
            hull: w.hull,
            core: w.core,
            class: w.class,
            governed: w.governed,
            provenance: w.provenance,
            seal_digest: w.seal_digest,
            seal_check: w.seal_check,
            licence_state: w.licence_state,
            at_ic: w.at_ic,
        };
        v.validate()?;
        Ok(v)
    }
}
impl InspectionRecord {
    /// The facts half, as tacsim handed them in.
    #[must_use]
    pub fn facts(&self) -> InspectionFacts {
        InspectionFacts {
            hull: self.hull,
            core: self.core,
            class: self.class,
            governed: self.governed,
            provenance: self.provenance.clone(),
            seal_digest: self.seal_digest.clone(),
            seal_check: self.seal_check,
            licence_state: self.licence_state,
        }
    }
}
fn seal_digest_ok(digest: &SmolStr) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
impl Record for InspectionRecord {
    const FAMILY: &'static str = INSPECTION_FAMILY;
    const MAX_BYTES: usize = 8192;
    fn key(&self) -> String {
        self.inspection.to_hex()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        self.authority.validate()?;
        self.authorization.validate()?;
        super::super::WorldIcSeconds::from_stamp(&self.at_ic).map_err(bad)?;
        let no_core = self.class == HullClass::Zero;
        if self.format != 1
            || self.action != self.authority.action().map_err(bad)?
            || self
                .authority
                .operators
                .binary_search(&self.operator)
                .is_err()
            || (no_core && (self.core.is_some() || self.governed.is_some()))
            || (!no_core && self.governed.is_none())
            || (self.seal_check == SealCheck::NoSeal) != self.seal_digest.is_none()
            || (self.seal_digest.is_none() && self.licence_state.is_some())
            || self
                .seal_digest
                .as_ref()
                .is_some_and(|d| !seal_digest_ok(d))
            || (no_core && matches!(self.provenance, Provenance::Certified { .. }))
        {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
}
