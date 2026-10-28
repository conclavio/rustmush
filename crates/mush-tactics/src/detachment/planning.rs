//! Private reusable plans and revision-bound declarations for preparation.
//!
//! These records never reserve a unit, grant permission, acquire a station or
//! move anyone. `Ready` records what the authenticated player declared; the
//! owning host must separately prove current seating and all live prerequisites.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::marker::PhantomData;

use mush_core::{CanonicalHash, ObjId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::roster::Slot;

pub const PLANNING_SCHEMA: u16 = 1;
pub const MAX_PRESETS: usize = 8;
pub const MAX_DRAFTS: usize = 8;
pub const MAX_PLAN_SLOTS: usize = 6;
pub const MAX_MIND_PROFILES: usize = 8;
pub const MAX_PLAN_BYTES: usize = 16 * 1024;
pub const MAX_PLANNING_BOOK_BYTES: usize = 128 * 1024;
const MAX_NAME_BYTES: usize = 64;
const MAX_PROFILE_VALUE_BYTES: usize = 128;

/// Local to one organizer's book, shared by presets and drafts, never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlanId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PlanHash(String);

impl PlanHash {
    pub fn new(value: impl Into<String>) -> Result<Self, PlanningError> {
        let value = value.into();
        if value.len() != 64
            || !value
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(PlanningError::Invalid(
                "plan hash must be 64 lowercase hexadecimal digits",
            ));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for PlanHash {
    type Error = PlanningError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<PlanHash> for String {
    fn from(value: PlanHash) -> Self {
        value.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlannedController {
    Unassigned,
    Player { character: ObjId },
    Npc { character: ObjId },
    Automated,
}

impl PlannedController {
    #[must_use]
    pub const fn character(self) -> Option<ObjId> {
        match self {
            Self::Player { character } | Self::Npc { character } => Some(character),
            Self::Unassigned | Self::Automated => None,
        }
    }
}

/// Opaque preferences. This does not prove a licence, core, installed mind or
/// profile is usable, nor does it replace or reseal an installed controller.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "MindChoiceWire", into = "MindChoiceWire")]
pub struct MindChoice {
    pub licence: u64,
    pub profiles: BTreeMap<String, String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MindChoiceWire {
    licence: u64,
    #[serde(deserialize_with = "profile_map")]
    profiles: BTreeMap<String, String>,
}

impl MindChoice {
    pub fn validate(&self) -> Result<(), PlanningError> {
        if self.licence == 0
            || self.profiles.len() > MAX_MIND_PROFILES
            || self.profiles.iter().any(|(key, value)| {
                !bounded_text(key, MAX_NAME_BYTES) || !bounded_text(value, MAX_PROFILE_VALUE_BYTES)
            })
        {
            return Err(PlanningError::Invalid("invalid mind preference"));
        }
        Ok(())
    }
}
impl TryFrom<MindChoiceWire> for MindChoice {
    type Error = PlanningError;
    fn try_from(value: MindChoiceWire) -> Result<Self, Self::Error> {
        let value = Self {
            licence: value.licence,
            profiles: value.profiles,
        };
        value.validate()?;
        Ok(value)
    }
}
impl From<MindChoice> for MindChoiceWire {
    fn from(value: MindChoice) -> Self {
        Self {
            licence: value.licence,
            profiles: value.profiles,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotPlan {
    pub slot: Slot,
    #[serde(deserialize_with = "required_option")]
    pub unit: Option<ObjId>,
    pub controller: PlannedController,
    #[serde(deserialize_with = "required_option")]
    pub role: Option<String>,
    #[serde(deserialize_with = "required_option")]
    pub mind: Option<MindChoice>,
}

/// Material selections only. Missing objects remain references, and incomplete
/// slots remain explicit. Financial terms are a selected digest, not acceptance
/// of a contract or authority to debit a budget.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "PlanWire", into = "PlanWire")]
pub struct Plan {
    pub carrier: ObjId,
    pub slots: Vec<SlotPlan>,
    pub npc_commander: Option<ObjId>,
    pub financial_terms: Option<PlanHash>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanWire {
    carrier: ObjId,
    #[serde(deserialize_with = "slot_vec")]
    slots: Vec<SlotPlan>,
    #[serde(deserialize_with = "required_option")]
    npc_commander: Option<ObjId>,
    #[serde(deserialize_with = "required_option")]
    financial_terms: Option<PlanHash>,
}

impl Plan {
    #[must_use]
    pub fn empty(carrier: ObjId) -> Self {
        Self {
            carrier,
            slots: Vec::new(),
            npc_commander: None,
            financial_terms: None,
        }
    }

    pub fn validate(&self) -> Result<(), PlanningError> {
        if self.carrier.0 == 0
            || self.slots.len() > MAX_PLAN_SLOTS
            || self.npc_commander.is_some_and(|id| id.0 == 0)
        {
            return Err(PlanningError::Invalid(
                "invalid plan identity or slot bound",
            ));
        }
        let mut previous = 0;
        let mut units = BTreeSet::new();
        let mut people = BTreeSet::new();
        for slot in &self.slots {
            if slot.slot.get() <= previous || usize::from(slot.slot.get()) > MAX_PLAN_SLOTS {
                return Err(PlanningError::Invalid(
                    "slots must be unique and ascending within 1..6",
                ));
            }
            previous = slot.slot.get();
            if let Some(unit) = slot.unit
                && (unit.0 == 0 || unit == self.carrier || !units.insert(unit))
            {
                return Err(PlanningError::Invalid(
                    "invalid or duplicate unit preference",
                ));
            }
            if let Some(character) = slot.controller.character()
                && (character.0 == 0 || !people.insert(character))
            {
                return Err(PlanningError::Invalid("invalid or duplicate controller"));
            }
            if slot
                .role
                .as_ref()
                .is_some_and(|role| !bounded_text(role, MAX_NAME_BYTES))
            {
                return Err(PlanningError::Invalid("invalid role label"));
            }
            if let Some(mind) = &slot.mind {
                mind.validate()?;
            }
        }
        encoded_bound(self, MAX_PLAN_BYTES)
    }

    pub fn material_hash(&self) -> Result<PlanHash, PlanningError> {
        self.validate()?;
        let value = serde_json::to_value(self).map_err(|_| PlanningError::Encoding)?;
        let hash = CanonicalHash::for_canonical_value(
            "tactics.detachment.plan.v1",
            1,
            &value,
            MAX_PLAN_BYTES * 2,
        )
        .map_err(|_| PlanningError::Encoding)?;
        PlanHash::new(hash.to_hex())
    }
}
impl TryFrom<PlanWire> for Plan {
    type Error = PlanningError;
    fn try_from(value: PlanWire) -> Result<Self, Self::Error> {
        let value = Self {
            carrier: value.carrier,
            slots: value.slots,
            npc_commander: value.npc_commander,
            financial_terms: value.financial_terms,
        };
        value.validate()?;
        Ok(value)
    }
}
impl From<Plan> for PlanWire {
    fn from(value: Plan) -> Self {
        Self {
            carrier: value.carrier,
            slots: value.slots,
            npc_commander: value.npc_commander,
            financial_terms: value.financial_terms,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub id: PlanId,
    pub name: String,
    pub revision: u64,
    pub plan: Plan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresetSource {
    pub id: PlanId,
    pub revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsentState {
    Accepted,
    Ready,
    Declined,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Consent {
    pub actor: ObjId,
    pub revision: u64,
    pub plan_hash: PlanHash,
    pub state: ConsentState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "DraftWire", into = "DraftWire")]
pub struct Draft {
    pub id: PlanId,
    pub organizer: ObjId,
    pub revision: u64,
    pub plan: Plan,
    pub plan_hash: PlanHash,
    pub source_preset: Option<PresetSource>,
    pub consents: BTreeMap<ObjId, Consent>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftWire {
    id: PlanId,
    organizer: ObjId,
    revision: u64,
    plan: Plan,
    plan_hash: PlanHash,
    #[serde(deserialize_with = "required_option")]
    source_preset: Option<PresetSource>,
    #[serde(deserialize_with = "consent_map")]
    consents: BTreeMap<ObjId, Consent>,
}

impl Draft {
    #[must_use]
    pub fn assigned_player(&self, actor: ObjId) -> Option<&SlotPlan> {
        self.plan
            .slots
            .iter()
            .find(|slot| slot.controller == PlannedController::Player { character: actor })
    }

    #[must_use]
    pub fn visible_to(&self, actor: ObjId) -> bool {
        actor == self.organizer || self.assigned_player(actor).is_some()
    }

    pub fn validate(&self) -> Result<(), PlanningError> {
        if self.id.0 == 0
            || self.organizer.0 == 0
            || self.revision == 0
            || self.consents.len() > MAX_PLAN_SLOTS
            || self.source_preset.is_some_and(|source| {
                source.id.0 == 0 || source.id >= self.id || source.revision == 0
            })
            || self.plan.material_hash()? != self.plan_hash
        {
            return Err(PlanningError::Invalid(
                "invalid draft identity, source or material hash",
            ));
        }
        for (actor, consent) in &self.consents {
            let slot = self
                .assigned_player(*actor)
                .ok_or(PlanningError::Invalid("consent actor is not assigned"))?;
            if consent.actor != *actor
                || consent.revision != self.revision
                || consent.plan_hash != self.plan_hash
                || (consent.state != ConsentState::Declined && slot.unit.is_none())
            {
                return Err(PlanningError::Invalid(
                    "consent does not bind this assignment",
                ));
            }
        }
        Ok(())
    }
}
impl TryFrom<DraftWire> for Draft {
    type Error = PlanningError;
    fn try_from(value: DraftWire) -> Result<Self, Self::Error> {
        let value = Self {
            id: value.id,
            organizer: value.organizer,
            revision: value.revision,
            plan: value.plan,
            plan_hash: value.plan_hash,
            source_preset: value.source_preset,
            consents: value.consents,
        };
        value.validate()?;
        Ok(value)
    }
}
impl From<Draft> for DraftWire {
    fn from(value: Draft) -> Self {
        Self {
            id: value.id,
            organizer: value.organizer,
            revision: value.revision,
            plan: value.plan,
            plan_hash: value.plan_hash,
            source_preset: value.source_preset,
            consents: value.consents,
        }
    }
}

/// One private organizer partition. Hosts use `owner` as the exact typed-family
/// key and expose another player's draft only through `Draft::visible_to`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "PlanningBookWire", into = "PlanningBookWire")]
pub struct PlanningBook {
    pub schema: u16,
    pub owner: ObjId,
    pub revision: u64,
    pub next_id: u64,
    pub presets: BTreeMap<PlanId, Preset>,
    pub drafts: BTreeMap<PlanId, Draft>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanningBookWire {
    schema: u16,
    owner: ObjId,
    revision: u64,
    next_id: u64,
    #[serde(deserialize_with = "preset_map")]
    presets: BTreeMap<PlanId, Preset>,
    #[serde(deserialize_with = "draft_map")]
    drafts: BTreeMap<PlanId, Draft>,
}

impl PlanningBook {
    pub fn new(owner: ObjId) -> Result<Self, PlanningError> {
        if owner.0 == 0 {
            return Err(PlanningError::Invalid("invalid organizer"));
        }
        Ok(Self {
            schema: PLANNING_SCHEMA,
            owner,
            revision: 1,
            next_id: 1,
            presets: BTreeMap::new(),
            drafts: BTreeMap::new(),
        })
    }

    pub fn validate(&self) -> Result<(), PlanningError> {
        if self.schema != PLANNING_SCHEMA
            || self.owner.0 == 0
            || self.revision == 0
            || self.next_id == 0
            || self.presets.len() > MAX_PRESETS
            || self.drafts.len() > MAX_DRAFTS
        {
            return Err(PlanningError::Invalid("invalid planning book or capacity"));
        }
        for (id, preset) in &self.presets {
            if id.0 == 0
                || id.0 >= self.next_id
                || preset.id != *id
                || preset.revision == 0
                || !bounded_text(&preset.name, MAX_NAME_BYTES)
                || self.drafts.contains_key(id)
            {
                return Err(PlanningError::Invalid("invalid preset identity or name"));
            }
            preset.plan.validate()?;
        }
        for (id, draft) in &self.drafts {
            if id.0 == 0 || id.0 >= self.next_id || draft.id != *id || draft.organizer != self.owner
            {
                return Err(PlanningError::Invalid("invalid draft owner or identity"));
            }
            draft.validate()?;
        }
        encoded_bound(self, MAX_PLANNING_BOOK_BYTES)
    }

    fn allocate(&mut self) -> Result<PlanId, PlanningError> {
        let id = PlanId(self.next_id);
        self.next_id = self.next_id.checked_add(1).ok_or(PlanningError::Capacity)?;
        Ok(id)
    }

    fn preset(&self, id: PlanId, revision: u64) -> Result<&Preset, PlanningError> {
        let preset = self.presets.get(&id).ok_or(PlanningError::UnknownPlan)?;
        if preset.revision != revision {
            return Err(PlanningError::Stale);
        }
        Ok(preset)
    }

    fn draft(&self, id: PlanId, revision: u64) -> Result<&Draft, PlanningError> {
        let draft = self.drafts.get(&id).ok_or(PlanningError::UnknownPlan)?;
        if draft.revision != revision {
            return Err(PlanningError::Stale);
        }
        Ok(draft)
    }

    fn insert_preset(&mut self, name: &str, plan: &Plan) -> Result<PlanId, PlanningError> {
        if self.presets.len() >= MAX_PRESETS {
            return Err(PlanningError::Capacity);
        }
        if !bounded_text(name, MAX_NAME_BYTES) {
            return Err(PlanningError::Invalid("invalid preset name"));
        }
        plan.validate()?;
        let id = self.allocate()?;
        self.presets.insert(
            id,
            Preset {
                id,
                name: name.into(),
                revision: 1,
                plan: plan.clone(),
            },
        );
        Ok(id)
    }

    fn insert_draft(
        &mut self,
        plan: &Plan,
        source_preset: Option<PresetSource>,
    ) -> Result<PlanId, PlanningError> {
        if self.drafts.len() >= MAX_DRAFTS {
            return Err(PlanningError::Capacity);
        }
        let hash = plan.material_hash()?;
        let id = self.allocate()?;
        self.drafts.insert(
            id,
            Draft {
                id,
                organizer: self.owner,
                revision: 1,
                plan: plan.clone(),
                plan_hash: hash,
                source_preset,
                consents: BTreeMap::new(),
            },
        );
        Ok(id)
    }

    /// Pure proposed transition. The host still authenticates the actor, checks
    /// current physical facts and commits with an exact book revision fence.
    /// Errors and no-op edits leave every record and allocator unchanged.
    pub fn apply(
        &self,
        actor: ObjId,
        operation: &PlanningOperation,
    ) -> Result<PlanningChange, PlanningError> {
        self.validate()?;
        if actor != self.owner && !matches!(operation, PlanningOperation::Declare { .. }) {
            return Err(PlanningError::Forbidden);
        }
        let mut next = self.clone();
        let affected = match operation {
            PlanningOperation::CreatePreset { name, plan } => next.insert_preset(name, plan)?,
            PlanningOperation::UpdatePreset {
                id,
                expected_revision,
                name,
                plan,
            } => {
                let old = self.preset(*id, *expected_revision)?;
                if old.name == *name && old.plan == *plan {
                    return Err(PlanningError::NoChange);
                }
                if !bounded_text(name, MAX_NAME_BYTES) {
                    return Err(PlanningError::Invalid("invalid preset name"));
                }
                plan.validate()?;
                let revision = old.revision.checked_add(1).ok_or(PlanningError::Capacity)?;
                next.presets.insert(
                    *id,
                    Preset {
                        id: *id,
                        name: name.clone(),
                        revision,
                        plan: plan.clone(),
                    },
                );
                *id
            }
            PlanningOperation::CopyPreset {
                id,
                expected_revision,
                name,
            } => {
                let source = self.preset(*id, *expected_revision)?;
                next.insert_preset(name, &source.plan)?
            }
            PlanningOperation::SaveDraftAsPreset {
                draft,
                expected_revision,
                name,
            } => {
                let source = self.draft(*draft, *expected_revision)?;
                next.insert_preset(name, &source.plan)?
            }
            PlanningOperation::DeletePreset {
                id,
                expected_revision,
            } => {
                self.preset(*id, *expected_revision)?;
                next.presets.remove(id);
                *id
            }
            PlanningOperation::CreateDraft { plan } => next.insert_draft(plan, None)?,
            PlanningOperation::LoadPreset {
                id,
                expected_revision,
            } => {
                let source = self.preset(*id, *expected_revision)?;
                next.insert_draft(
                    &source.plan,
                    Some(PresetSource {
                        id: *id,
                        revision: *expected_revision,
                    }),
                )?
            }
            PlanningOperation::EditDraft {
                id,
                expected_revision,
                plan,
            } => {
                let old = self.draft(*id, *expected_revision)?;
                if old.plan == *plan {
                    return Err(PlanningError::NoChange);
                }
                let hash = plan.material_hash()?;
                let revision = old.revision.checked_add(1).ok_or(PlanningError::Capacity)?;
                next.drafts.insert(
                    *id,
                    Draft {
                        id: *id,
                        organizer: old.organizer,
                        revision,
                        plan: plan.clone(),
                        plan_hash: hash,
                        source_preset: old.source_preset,
                        consents: BTreeMap::new(),
                    },
                );
                *id
            }
            PlanningOperation::DiscardDraft {
                id,
                expected_revision,
            } => {
                self.draft(*id, *expected_revision)?;
                next.drafts.remove(id);
                *id
            }
            PlanningOperation::Declare {
                id,
                expected_revision,
                expected_hash,
                state,
            } => {
                let old = self.draft(*id, *expected_revision)?;
                let slot = old.assigned_player(actor).ok_or(PlanningError::Forbidden)?;
                if old.plan_hash != *expected_hash {
                    return Err(PlanningError::Stale);
                }
                if *state != ConsentState::Declined && slot.unit.is_none() {
                    return Err(PlanningError::UnresolvedAssignment);
                }
                let previous = old.consents.get(&actor);
                if previous.is_some_and(|consent| consent.state == *state) {
                    return Err(PlanningError::NoChange);
                }
                if *state == ConsentState::Ready
                    && previous.is_none_or(|consent| consent.state != ConsentState::Accepted)
                {
                    return Err(PlanningError::AcceptanceRequired);
                }
                let draft = next.drafts.get_mut(id).ok_or(PlanningError::UnknownPlan)?;
                draft.consents.insert(
                    actor,
                    Consent {
                        actor,
                        revision: old.revision,
                        plan_hash: old.plan_hash.clone(),
                        state: *state,
                    },
                );
                *id
            }
        };
        next.revision = self
            .revision
            .checked_add(1)
            .ok_or(PlanningError::Capacity)?;
        next.validate()?;
        Ok(PlanningChange {
            book: next,
            affected,
        })
    }
}
impl TryFrom<PlanningBookWire> for PlanningBook {
    type Error = PlanningError;
    fn try_from(value: PlanningBookWire) -> Result<Self, Self::Error> {
        let value = Self {
            schema: value.schema,
            owner: value.owner,
            revision: value.revision,
            next_id: value.next_id,
            presets: value.presets,
            drafts: value.drafts,
        };
        value.validate()?;
        Ok(value)
    }
}
impl From<PlanningBook> for PlanningBookWire {
    fn from(value: PlanningBook) -> Self {
        Self {
            schema: value.schema,
            owner: value.owner,
            revision: value.revision,
            next_id: value.next_id,
            presets: value.presets,
            drafts: value.drafts,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum PlanningOperation {
    CreatePreset {
        name: String,
        plan: Plan,
    },
    UpdatePreset {
        id: PlanId,
        expected_revision: u64,
        name: String,
        plan: Plan,
    },
    CopyPreset {
        id: PlanId,
        expected_revision: u64,
        name: String,
    },
    SaveDraftAsPreset {
        draft: PlanId,
        expected_revision: u64,
        name: String,
    },
    DeletePreset {
        id: PlanId,
        expected_revision: u64,
    },
    CreateDraft {
        plan: Plan,
    },
    LoadPreset {
        id: PlanId,
        expected_revision: u64,
    },
    EditDraft {
        id: PlanId,
        expected_revision: u64,
        plan: Plan,
    },
    DiscardDraft {
        id: PlanId,
        expected_revision: u64,
    },
    Declare {
        id: PlanId,
        expected_revision: u64,
        expected_hash: PlanHash,
        state: ConsentState,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanningChange {
    pub book: PlanningBook,
    pub affected: PlanId,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum PlanningError {
    #[error("that planning operation is not yours")]
    Forbidden,
    #[error("unknown plan")]
    UnknownPlan,
    #[error("the plan revision or hash changed")]
    Stale,
    #[error("planning capacity reached")]
    Capacity,
    #[error("the operation makes no change")]
    NoChange,
    #[error("accept the current assignment before marking ready")]
    AcceptanceRequired,
    #[error("the assignment has no selected unit")]
    UnresolvedAssignment,
    #[error("invalid planning record: {0}")]
    Invalid(&'static str),
    #[error("planning record could not be encoded")]
    Encoding,
}

fn bounded_text(text: &str, maximum: usize) -> bool {
    !text.is_empty()
        && text.len() <= maximum
        && text.trim() == text
        && !text.chars().any(char::is_control)
}

fn encoded_bound(value: &impl Serialize, maximum: usize) -> Result<(), PlanningError> {
    if serde_json::to_vec(value)
        .map_err(|_| PlanningError::Encoding)?
        .len()
        > maximum
    {
        return Err(PlanningError::Capacity);
    }
    Ok(())
}

fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}

fn bounded_map<'de, D, K, V, const N: usize>(d: D) -> Result<BTreeMap<K, V>, D::Error>
where
    D: serde::Deserializer<'de>,
    K: Deserialize<'de> + Ord,
    V: Deserialize<'de>,
{
    struct Map<K, V, const N: usize>(PhantomData<(K, V)>);
    impl<'de, K: Deserialize<'de> + Ord, V: Deserialize<'de>, const N: usize>
        serde::de::Visitor<'de> for Map<K, V, N>
    {
        type Value = BTreeMap<K, V>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "at most {N} unique entries")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut access: A,
        ) -> Result<Self::Value, A::Error> {
            let mut result = BTreeMap::new();
            while let Some((key, value)) = access.next_entry()? {
                if result.len() >= N || result.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom(
                        "duplicate key or map capacity exceeded",
                    ));
                }
            }
            Ok(result)
        }
    }
    d.deserialize_map(Map::<K, V, N>(PhantomData))
}

fn profile_map<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<String, String>, D::Error> {
    bounded_map::<D, String, String, MAX_MIND_PROFILES>(d)
}
fn consent_map<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<ObjId, Consent>, D::Error> {
    bounded_map::<D, ObjId, Consent, MAX_PLAN_SLOTS>(d)
}
fn preset_map<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<PlanId, Preset>, D::Error> {
    bounded_map::<D, PlanId, Preset, MAX_PRESETS>(d)
}
fn draft_map<'de, D: serde::Deserializer<'de>>(d: D) -> Result<BTreeMap<PlanId, Draft>, D::Error> {
    bounded_map::<D, PlanId, Draft, MAX_DRAFTS>(d)
}

fn slot_vec<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<SlotPlan>, D::Error> {
    struct Slots;
    impl<'de> serde::de::Visitor<'de> for Slots {
        type Value = Vec<SlotPlan>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("at most six command slots")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut access: A,
        ) -> Result<Self::Value, A::Error> {
            let mut result = Vec::new();
            while let Some(slot) = access.next_element()? {
                if result.len() >= MAX_PLAN_SLOTS {
                    return Err(serde::de::Error::custom("slot capacity exceeded"));
                }
                result.push(slot);
            }
            Ok(result)
        }
    }
    d.deserialize_seq(Slots)
}

#[cfg(test)]
#[path = "planning_tests.rs"]
mod tests;
