//! Closed first-filing records. Decoding data never grants title or execution.
//!
//! Only the registered Roll filing builder constructs new rows, after reading
//! an actual rating and the prepared E4 result. Private fields are deliberate:
//! clients cannot submit a row as a request or manufacture a publication token.

use mush_core::{CommitToken, ObjId, mechanics::DefinitionDigest, reserved_shell::ShellKey};
use mush_economy::{AccountId, Amount, CatalogRef, TransactionId};
use mush_org::{PrincipalId, SystemPrincipalId};
use serde::{Deserialize, Serialize};

use super::dependent_owner::DependentParent;
use super::filed::FiledRevisionContent;
use super::rating::RatingRef;
use super::{
    CharterId, FiledRevisionId, LicenceId, LineId, RevisionIdentity, TermGeneration, TermWindow,
};

pub const PUBLICATION_FORMAT: u16 = 1;
pub const MAX_CHARTER_NAME_BYTES: usize = 80;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PublicationError {
    #[error("commercial records are unavailable")]
    Unavailable,
    #[error("commercial record is missing")]
    Missing,
    #[error("commercial records disagree")]
    Mismatch,
    #[error("invalid commercial record")]
    Invalid,
    #[error("commercial record or root capacity exceeded")]
    Capacity,
    #[error("commercial audit is stale")]
    Stale,
}

/// Uniform private family contract, including key/body equality at audit.
pub(crate) trait Record:
    Clone + Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static
{
    const FAMILY: &'static str;
    const MAX_BYTES: usize;
    fn key(&self) -> String;
    fn validate(&self) -> Result<(), PublicationError>;
}

fn version(format: u16) -> Result<(), PublicationError> {
    if format == PUBLICATION_FORMAT {
        Ok(())
    } else {
        Err(PublicationError::Invalid)
    }
}

fn name(value: &str) -> Result<(), PublicationError> {
    if value.is_empty()
        || value.len() > MAX_CHARTER_NAME_BYTES
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(PublicationError::Invalid);
    }
    Ok(())
}

/// A durable deployment marker is required even when every business family is
/// empty. Its loss cannot be interpreted as a newly initialized installation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommerceDeployment {
    pub(crate) format: u16,
    pub(crate) initialized_by: CommitToken,
    pub(crate) next_line: u64,
    pub(crate) next_filed: u64,
    pub(crate) next_licence: u64,
}

impl Record for CommerceDeployment {
    const FAMILY: &'static str = "roll_mind_commerce";
    const MAX_BYTES: usize = 4 * 1024;
    fn key(&self) -> String {
        "registry".into()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        version(self.format)?;
        if self.next_line == 0 || self.next_filed == 0 || self.next_licence == 0 {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
}

/// Charter ownership always comes from E4. This row never caches a holder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharterRecord {
    pub(crate) format: u16,
    pub(crate) id: CharterId,
    pub(crate) name: String,
    pub(crate) shell_receipt: CommitToken,
    pub(crate) operation: CommitToken,
    pub(crate) line: LineId,
}

impl CharterRecord {
    pub const fn id(&self) -> CharterId {
        self.id
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub const fn line(&self) -> LineId {
        self.line
    }
}
impl Record for CharterRecord {
    const FAMILY: &'static str = "roll_mind_charter";
    const MAX_BYTES: usize = 4 * 1024;
    fn key(&self) -> String {
        self.id.key()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        version(self.format)?;
        name(&self.name)?;
        if self.shell_receipt == self.operation {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineRecord {
    pub(crate) format: u16,
    pub(crate) id: LineId,
    pub(crate) charter: CharterId,
    /// Exact parameter schema of the initial combat line. This is independent
    /// of the execution manifest to which a Registry rating applies.
    pub(crate) schema_digest: DefinitionDigest,
    pub(crate) head: FiledRevisionId,
    pub(crate) next_sequence: u64,
    pub(crate) operation: CommitToken,
}

/// Initial combat lines use exact canonical schema equality. Source, saved
/// values, presets, rules, declaration and interpreter ABI belong to revision
/// or rating identities. Later compatible-update admission must deliberately
/// define its permitted schema changes; this initial-only codec does not.
pub fn schema_line_digest(
    envelope: &crate::Envelope,
) -> Result<DefinitionDigest, PublicationError> {
    envelope.validate().map_err(|_| PublicationError::Invalid)?;
    DefinitionDigest::for_canonical_value_with_limit(
        "tacsim.mind.combat_schema_line.v1",
        &envelope.schema,
        crate::MAX_ENVELOPE_BYTES,
    )
    .map_err(|_| PublicationError::Invalid)
}

/// ABI-independent normalized duplicate identity. The immutable source SHA is
/// over the normalized source retained by the real source owner; save and boot
/// source audits verify it. This pure helper proves no source access or rights.
/// Preserve every executable authoring input except the runtime ABI: changing
/// engine compatibility cannot erase an earlier charter's duplicate entry.
pub fn canonical_duplicate_digest(
    revision: &crate::Revision,
) -> Result<DefinitionDigest, PublicationError> {
    revision.validate().map_err(|_| PublicationError::Invalid)?;
    let envelope = revision.envelope();
    DefinitionDigest::for_canonical_value_with_limit(
        "tacsim.mind.duplicate.v1",
        &serde_json::json!({
            "normalized_source_sha256":revision.source(),
            "schema":envelope.schema,
            "presets":envelope.presets,
            "profiles":envelope.profiles,
            "rules":envelope.rules,
            "declared_class":envelope.declared_class,
        }),
        crate::MAX_ENVELOPE_BYTES + 512,
    )
    .map_err(|_| PublicationError::Invalid)
}
impl LineRecord {
    pub const fn id(&self) -> LineId {
        self.id
    }
    pub const fn charter(&self) -> CharterId {
        self.charter
    }
    pub const fn head(&self) -> FiledRevisionId {
        self.head
    }
}
impl Record for LineRecord {
    const FAMILY: &'static str = "roll_mind_line";
    const MAX_BYTES: usize = 4 * 1024;
    fn key(&self) -> String {
        self.id.key()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        version(self.format)?;
        // This registered schema is the initial original-filing cut. A later
        // revision producer must extend the audited schema deliberately.
        if self.next_sequence != 2 {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
}

/// Complete filed wrapper: a structural component alone is not a filed right.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "FiledRecordWire")]
pub struct FiledRecord {
    pub(crate) format: u16,
    pub(crate) content: FiledRevisionContent,
    pub(crate) definition: DefinitionDigest,
    pub(crate) duplicate: DefinitionDigest,
    pub(crate) rating: RatingRef,
    pub(crate) operation: CommitToken,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FiledRecordWire {
    format: u16,
    content: FiledRevisionContent,
    definition: DefinitionDigest,
    duplicate: DefinitionDigest,
    rating: RatingRef,
    operation: CommitToken,
}
impl TryFrom<FiledRecordWire> for FiledRecord {
    type Error = PublicationError;
    fn try_from(w: FiledRecordWire) -> Result<Self, Self::Error> {
        let row = Self {
            format: w.format,
            content: w.content,
            definition: w.definition,
            duplicate: w.duplicate,
            rating: w.rating,
            operation: w.operation,
        };
        row.validate()?;
        Ok(row)
    }
}
impl FiledRecord {
    pub fn content(&self) -> &FiledRevisionContent {
        &self.content
    }
    pub fn rating(&self) -> &RatingRef {
        &self.rating
    }
    pub const fn definition(&self) -> DefinitionDigest {
        self.definition
    }
    pub const fn duplicate(&self) -> DefinitionDigest {
        self.duplicate
    }
}
impl Record for FiledRecord {
    const FAMILY: &'static str = "roll_mind_filed";
    const MAX_BYTES: usize = 8 * 1024;
    fn key(&self) -> String {
        self.content.key()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        self.content
            .encode()
            .map_err(|_| PublicationError::Invalid)?;
        if !matches!(
            (self.format, self.content.parent().is_some()),
            (1, false) | (2, true)
        ) || self.content.position().sequence.get() != 1
        {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
}

/// Status is separate from immutable content. The first producer initializes
/// both facts clear; later registered transitions retain their committed cause.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionStatus {
    pub(crate) format: u16,
    pub(crate) filed: FiledRevisionId,
    pub(crate) withdrawn_by: Option<CommitToken>,
    pub(crate) recalled_by: Option<CommitToken>,
}
impl Record for RevisionStatus {
    const FAMILY: &'static str = "roll_mind_status";
    const MAX_BYTES: usize = 4 * 1024;
    fn key(&self) -> String {
        self.filed.key()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        version(self.format)
    }
}

/// A title-derived, nontransferable owner's licence. There is intentionally no
/// paid seat count, independent holder or source-bearing payload in this type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerLicence {
    pub(crate) format: u16,
    pub(crate) id: LicenceId,
    pub(crate) charter: CharterId,
    pub(crate) line: LineId,
    pub(crate) term: TermGeneration,
    pub(crate) operation: CommitToken,
}
impl OwnerLicence {
    pub const fn id(&self) -> LicenceId {
        self.id
    }
    pub const fn charter(&self) -> CharterId {
        self.charter
    }
    pub const fn line(&self) -> LineId {
        self.line
    }
}
impl Record for OwnerLicence {
    const FAMILY: &'static str = "roll_mind_owner_licence";
    const MAX_BYTES: usize = 4 * 1024;
    fn key(&self) -> String {
        self.id.key()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        version(self.format)?;
        if self.term.get() != 1 {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RevisionPolicy {
    Pin {
        filed: FiledRevisionId,
        revision: RevisionIdentity,
    },
    Follow {
        line: LineId,
    },
}

/// Immutable first owner term. Format one is original-only; format two must
/// carry the exact owner-specific dependent qualification. Paid terms and
/// renewals remain separate protocols.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "OwnerTermWire")]
pub struct OwnerTerm {
    pub(crate) format: u16,
    pub(crate) licence: LicenceId,
    pub(crate) generation: TermGeneration,
    pub(crate) window: TermWindow,
    pub(crate) revision: RevisionPolicy,
    pub(crate) operation: CommitToken,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) parent: Option<DependentParent>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnerTermWire {
    format: u16,
    licence: LicenceId,
    generation: TermGeneration,
    window: TermWindow,
    revision: RevisionPolicy,
    operation: CommitToken,
    #[serde(default, deserialize_with = "present_parent")]
    parent: Option<DependentParent>,
}
impl TryFrom<OwnerTermWire> for OwnerTerm {
    type Error = PublicationError;
    fn try_from(w: OwnerTermWire) -> Result<Self, Self::Error> {
        let row = Self {
            format: w.format,
            licence: w.licence,
            generation: w.generation,
            window: w.window,
            revision: w.revision,
            operation: w.operation,
            parent: w.parent,
        };
        row.validate()?;
        Ok(row)
    }
}
impl OwnerTerm {
    pub const fn licence(&self) -> LicenceId {
        self.licence
    }
    pub const fn generation(&self) -> TermGeneration {
        self.generation
    }
    pub const fn window(&self) -> TermWindow {
        self.window
    }
    pub fn policy(&self) -> &RevisionPolicy {
        &self.revision
    }
    pub(crate) fn encoded_key(licence: LicenceId, generation: TermGeneration) -> String {
        format!("{}/{}", licence.key(), generation.key())
    }
}
impl Record for OwnerTerm {
    const FAMILY: &'static str = "roll_mind_owner_term";
    const MAX_BYTES: usize = 8 * 1024;
    fn key(&self) -> String {
        Self::encoded_key(self.licence, self.generation)
    }
    fn validate(&self) -> Result<(), PublicationError> {
        if !matches!((self.format, self.parent.is_some()), (1, false) | (2, true))
            || self.generation.get() != 1
        {
            return Err(PublicationError::Invalid);
        }
        if let Some(parent) = &self.parent {
            parent.validate()?;
            if self.licence == parent.licence
                || self.window != parent.child_window(self.window.issued_at())?
            {
                return Err(PublicationError::Mismatch);
            }
        }
        Ok(())
    }
}

fn present_parent<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<DependentParent>, D::Error> {
    DependentParent::deserialize(deserializer).map(Some)
}

/// Historical economic facts, not an independent current-title projection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilingEconomicReceipt {
    pub(crate) transaction: TransactionId,
    pub(crate) journal_seq: u64,
    pub(crate) title: ObjId,
    pub(crate) issued_owner: PrincipalId,
    pub(crate) title_revision: u64,
    pub(crate) payer: AccountId,
    pub(crate) sink: AccountId,
    pub(crate) fee: Amount,
    pub(crate) issuer: SystemPrincipalId,
    #[serde(deserialize_with = "strict_catalog")]
    pub(crate) catalog: CatalogRef,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_house"
    )]
    pub(crate) house: Option<mush_economy::registry_filing::HouseFilingAttribution>,
}

fn present_house<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<mush_economy::registry_filing::HouseFilingAttribution>, D::Error> {
    mush_economy::registry_filing::HouseFilingAttribution::deserialize(deserializer).map(Some)
}

// E4's general-purpose CatalogRef predates strict commerce wire records. Keep
// its actual value type while refusing ignored nested fields at this boundary.
fn strict_catalog<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<CatalogRef, D::Error> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Wire {
        ruleset: smol_str::SmolStr,
        code: smol_str::SmolStr,
    }
    let value = Wire::deserialize(deserializer)?;
    Ok(CatalogRef {
        ruleset: value.ruleset,
        code: value.code,
    })
}
impl FilingEconomicReceipt {
    pub(crate) fn from_prepared(
        value: &mush_economy::registry_filing::InitialFilingOutput,
    ) -> Result<Self, PublicationError> {
        let receipt = Self {
            house: value.house().cloned(),
            transaction: value.transaction(),
            journal_seq: value.journal_seq(),
            title: value.title().object,
            issued_owner: value.title().title_owner.clone(),
            title_revision: value.title().revision,
            payer: value.payer(),
            sink: value.sink(),
            fee: value.fee(),
            issuer: value.issuer().clone(),
            catalog: value
                .title()
                .catalog
                .clone()
                .ok_or(PublicationError::Invalid)?,
        };
        receipt.validate()?;
        if value.title().scope != receipt.house.as_ref().map(|h| h.scope()) {
            return Err(PublicationError::Mismatch);
        }
        Ok(receipt)
    }
    pub(crate) fn owner_matches(&self, actor: ObjId) -> bool {
        match &self.house {
            None => self.issued_owner == PrincipalId::Object(actor),
            Some(house) => {
                house.curator() == actor
                    && self.issued_owner == PrincipalId::Organization(house.organization())
            }
        }
    }
    fn validate(&self) -> Result<(), PublicationError> {
        if self.title_revision != 1
            || self.title.0 == 0
            || self.fee.is_negative()
            || self.payer == self.sink
            || match &self.house {
                None => !matches!(self.issued_owner, PrincipalId::Object(_)),
                Some(house) => self.issued_owner != PrincipalId::Organization(house.organization()),
            }
        {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
}

/// The permanent terminal initial-filing receipt. Its key is the actual core
/// commit token; embedding its own eventual bundle hash would be circular.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "InitialFilingOperationWire")]
pub struct InitialFilingOperation {
    pub(crate) format: u16,
    pub(crate) commit: CommitToken,
    pub(crate) requester: ObjId,
    pub(crate) shell: ShellKey,
    pub(crate) shell_receipt: CommitToken,
    pub(crate) charter: CharterId,
    pub(crate) line: LineId,
    pub(crate) filed: FiledRevisionId,
    pub(crate) licence: LicenceId,
    pub(crate) term: TermGeneration,
    pub(crate) revision: RevisionIdentity,
    pub(crate) rating: RatingRef,
    pub(crate) economic: FilingEconomicReceipt,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) parent: Option<DependentParent>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InitialFilingOperationWire {
    format: u16,
    commit: CommitToken,
    requester: ObjId,
    shell: ShellKey,
    shell_receipt: CommitToken,
    charter: CharterId,
    line: LineId,
    filed: FiledRevisionId,
    licence: LicenceId,
    term: TermGeneration,
    revision: RevisionIdentity,
    rating: RatingRef,
    economic: FilingEconomicReceipt,
    #[serde(default, deserialize_with = "present_parent")]
    parent: Option<DependentParent>,
}
impl TryFrom<InitialFilingOperationWire> for InitialFilingOperation {
    type Error = PublicationError;
    fn try_from(w: InitialFilingOperationWire) -> Result<Self, Self::Error> {
        let row = Self {
            format: w.format,
            commit: w.commit,
            requester: w.requester,
            shell: w.shell,
            shell_receipt: w.shell_receipt,
            charter: w.charter,
            line: w.line,
            filed: w.filed,
            licence: w.licence,
            term: w.term,
            revision: w.revision,
            rating: w.rating,
            economic: w.economic,
            parent: w.parent,
        };
        row.validate()?;
        Ok(row)
    }
}

impl InitialFilingOperation {
    pub const fn commit(&self) -> CommitToken {
        self.commit
    }
    pub const fn revision(&self) -> RevisionIdentity {
        self.revision
    }
    pub(crate) fn audit_subject(
        &self,
    ) -> Result<mush_economy::registry_filing::InitialFilingAuditSubject, PublicationError> {
        self.validate()?;
        let receipt = &self.economic;
        if let Some(house) = &receipt.house {
            return mush_economy::registry_filing::InitialFilingAuditSubject::new_house(
                self.commit,
                receipt.transaction,
                receipt.journal_seq,
                receipt.title,
                receipt.title_revision,
                receipt.payer,
                receipt.sink,
                receipt.fee,
                receipt.issuer.clone(),
                receipt.catalog.clone(),
                house.clone(),
            )
            .map_err(|_| PublicationError::Invalid);
        }
        mush_economy::registry_filing::InitialFilingAuditSubject::new(
            self.commit,
            receipt.transaction,
            receipt.journal_seq,
            receipt.title,
            receipt.issued_owner.clone(),
            receipt.title_revision,
            receipt.payer,
            receipt.sink,
            receipt.fee,
            receipt.issuer.clone(),
            receipt.catalog.clone(),
        )
        .map_err(|_| PublicationError::Invalid)
    }
}
impl Record for InitialFilingOperation {
    const FAMILY: &'static str = "roll_mind_initial_filing";
    const MAX_BYTES: usize = 16 * 1024;
    fn key(&self) -> String {
        self.commit.to_string()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        if !matches!(
            (
                self.format,
                self.economic.house.is_some(),
                self.parent.is_some()
            ),
            (1, false, false) | (2, true, false) | (3, false, true)
        ) {
            return Err(PublicationError::Invalid);
        }
        self.economic.validate()?;
        if let Some(parent) = &self.parent {
            parent.validate()?;
            if parent.owner != self.requester
                || parent.licence == self.licence
                || parent.charter == self.charter
                || parent.parent.filed == self.filed
            {
                return Err(PublicationError::Mismatch);
            }
        }
        if self.requester.0 == 0
            || self.shell.requester() != self.requester
            || self.commit == self.shell_receipt
            || self.term.get() != 1
            || self.charter.object() != self.economic.title
            || !self.economic.owner_matches(self.requester)
            || self.revision.revision().key.character != self.requester
        {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
}

/// Two separate bounded indexes; neither substitutes for its full filed row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FiledIdentityIndex {
    pub(crate) format: u16,
    pub(crate) duplicate: DefinitionDigest,
    pub(crate) revision: RevisionIdentity,
    pub(crate) filed: FiledRevisionId,
}
impl Record for FiledIdentityIndex {
    const FAMILY: &'static str = "roll_mind_filed_identity";
    const MAX_BYTES: usize = 2 * 1024;
    fn key(&self) -> String {
        self.duplicate.to_string()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        version(self.format)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FiledPositionIndex {
    pub(crate) format: u16,
    pub(crate) line: LineId,
    pub(crate) filed: FiledRevisionId,
}
impl Record for FiledPositionIndex {
    const FAMILY: &'static str = "roll_mind_filed_position";
    const MAX_BYTES: usize = 2 * 1024;
    fn key(&self) -> String {
        format!("{}/0000000000000001", self.line.key())
    }
    fn validate(&self) -> Result<(), PublicationError> {
        version(self.format)
    }
}
