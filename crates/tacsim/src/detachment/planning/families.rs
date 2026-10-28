//! Bounded private books and exact invitation lookup, without a World mirror.
use std::collections::BTreeSet;

use mush_core::{ModError, ObjId, Registrar, StateFamilyHandle};
use mush_tactics::detachment::planning::{MAX_PLANNING_BOOK_BYTES, PlanId, PlanningBook};
use serde::{Deserialize, Serialize};

pub const BOOK_FAMILY: &str = "tacsim.detachment_plans";
pub const INVITE_FAMILY: &str = "tacsim.detachment_invitations";
pub const MAX_INVITATIONS: usize = 32;
const MAX_INDEX_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftRef {
    pub organizer: ObjId,
    pub id: PlanId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "InvitationWire", into = "InvitationWire")]
pub struct InvitationIndex {
    pub revision: u64,
    pub drafts: BTreeSet<DraftRef>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InvitationWire {
    schema: u16,
    revision: u64,
    drafts: Vec<DraftRef>,
}
impl Default for InvitationIndex {
    fn default() -> Self {
        Self {
            revision: 1,
            drafts: BTreeSet::new(),
        }
    }
}
impl InvitationIndex {
    pub fn validate(&self) -> Result<(), String> {
        if self.revision == 0
            || self.drafts.len() > MAX_INVITATIONS
            || self
                .drafts
                .iter()
                .any(|d| d.organizer.0 == 0 || d.id.0 == 0)
        {
            return Err("invalid detachment invitation index".into());
        }
        Ok(())
    }
}
impl TryFrom<InvitationWire> for InvitationIndex {
    type Error = String;
    fn try_from(w: InvitationWire) -> Result<Self, String> {
        if w.schema != 1
            || w.drafts.len() > MAX_INVITATIONS
            || w.drafts.windows(2).any(|p| matches!(p, [a,b] if a >= b))
        {
            return Err("invalid detachment invitation encoding".into());
        }
        let index = Self {
            revision: w.revision,
            drafts: w.drafts.into_iter().collect(),
        };
        index.validate()?;
        Ok(index)
    }
}
impl From<InvitationIndex> for InvitationWire {
    fn from(index: InvitationIndex) -> Self {
        Self {
            schema: 1,
            revision: index.revision,
            drafts: index.drafts.into_iter().collect(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Families {
    pub books: StateFamilyHandle<ObjId, PlanningBook>,
    pub invitations: StateFamilyHandle<ObjId, InvitationIndex>,
}
pub fn register(reg: &mut Registrar<'_>) -> Result<Families, ModError> {
    let descriptor = crate::port::families::family_descriptor(
        BOOK_FAMILY,
        "tacsim.detachment_plans.key.v1",
        "tacsim.detachment_plans",
    )?
    .with_limits(16, MAX_PLANNING_BOOK_BYTES)
    .map_err(|e| ModError::msg(e.to_string()))?;
    let books = reg.state_family(
        descriptor,
        |key: &ObjId| Ok(key.0.to_string()),
        |v: &serde_json::Value| serde_json::from_value(v.clone()).map_err(|e| e.to_string()),
        |book: &PlanningBook| book.validate().map_err(|e| e.to_string()),
        |book: &PlanningBook| serde_json::to_value(book).map_err(|e| e.to_string()),
    )?;
    let descriptor = crate::port::families::family_descriptor(
        INVITE_FAMILY,
        "tacsim.detachment_invitations.key.v1",
        "tacsim.detachment_invitations",
    )?
    .with_limits(16, MAX_INDEX_BYTES)
    .map_err(|e| ModError::msg(e.to_string()))?;
    let invitations = reg.state_family(
        descriptor,
        |key: &ObjId| Ok(key.0.to_string()),
        |v: &serde_json::Value| serde_json::from_value(v.clone()).map_err(|e| e.to_string()),
        InvitationIndex::validate,
        |index: &InvitationIndex| serde_json::to_value(index).map_err(|e| e.to_string()),
    )?;
    Ok(Families { books, invitations })
}
