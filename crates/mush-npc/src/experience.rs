//! Bounded personal experiences and beliefs, separate from authored knowledge
//! and the legacy relationship-memory store. Only engine observations establish
//! provenance; a model's interpretation remains an uncertain belief.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// The version of each `mind:<npc>` document.
pub const PERSONAL_MEMORY_SCHEMA: u16 = 1;
/// Maximum retained original observations per NPC.
pub const MAX_EXPERIENCES: usize = 64;
/// Maximum retained interpretations per NPC, including revision history.
pub const MAX_BELIEFS: usize = 32;
/// Maximum UTF-8 bytes in one canonical subject.
pub const MAX_PERSONAL_SUBJECT_BYTES: usize = 128;
/// Maximum UTF-8 bytes in an observation or belief.
pub const MAX_PERSONAL_TEXT_BYTES: usize = 4096;
/// Maximum UTF-8 bytes in a perceived speaker's display name.
pub const MAX_PERSONAL_NAME_BYTES: usize = 256;
/// The complete encoded document ceiling, including copied evidence.
pub const MAX_PERSONAL_MEMORY_BYTES: usize = 256 * 1024;
/// The independent ceiling on recalled observations and beliefs.
pub const MAX_PERSONAL_RECALL_ENTRIES: usize = 16;
/// Maximum rendered UTF-8 bytes, including subjects and line separators.
pub const MAX_PERSONAL_RECALL_BYTES: usize = 8 * 1024;
/// Maximum bytes in one complete rendered recollection, including its labels.
pub const MAX_PERSONAL_RECALL_NOTE_BYTES: usize = 2 * 1024;
/// Recollection becomes explicitly fading after seven real days.
pub const FADING_AFTER_SECS: i64 = 7 * 86_400;
/// Old recollections require an explicit matching conversational cue.
pub const OLD_AFTER_SECS: i64 = 30 * 86_400;

/// An engine-observed event, never a model-selected truth category.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperienceKind {
    /// The NPC perceived someone saying or posing something; the claim itself
    /// is not verified by having been spoken.
    Speech,
    /// An actual public action result observed by the engine.
    Outcome,
}

/// The original public observation. A belief copies this whole value so an
/// evicted observation cannot silently lose or acquire different provenance.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Experience {
    /// The observing NPC, also checked in copied belief evidence at boot.
    pub npc: ObjId,
    /// Positive ID from this NPC's shared monotonic sequence.
    pub id: u64,
    /// Canonical topic or person key.
    pub subject: String,
    /// Original observed text; interpretation never rewrites it.
    pub text: String,
    /// Perceived speaker or actor, if known.
    pub speaker: Option<ObjId>,
    /// The display name actually perceived at observation time.
    pub speaker_name: String,
    /// Observed location, retained internally and not rendered into prompts.
    pub room: Option<ObjId>,
    /// How the engine obtained this observation.
    pub kind: ExperienceKind,
    /// Observation time in Unix seconds.
    pub observed_at: i64,
}

/// A subjective interpretation. Even an evidence-backed belief is not an
/// authoritative fact or an instruction to the game engine.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Belief {
    /// Positive ID from the same sequence as experiences.
    pub id: u64,
    /// Canonical topic or person key.
    pub subject: String,
    /// The model's interpretation, kept separate from original evidence.
    pub note: String,
    /// An engine-selected immutable evidence copy, or an explicitly unsupported
    /// inference for old callers that do not cite an experience.
    pub evidence: Option<Experience>,
    /// Earlier belief revised with newer evidence from the same speaker.
    pub replaces: Option<u64>,
    /// An explicitly competing account from another speaker. A shared subject
    /// alone does not imply disagreement or make corroborating reports disputed.
    #[serde(default)]
    pub disputes: Option<u64>,
    /// When the interpretation was recorded, not when its evidence happened.
    pub created_at: i64,
}

/// One versioned durable personal-memory document per NPC.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PersonalMemoryRecord {
    /// Exact supported schema, validated before loading.
    pub schema: u16,
    /// Owner of all observations and interpretations in this document.
    pub npc: ObjId,
    /// Next unused ID; never decreases when old entries are evicted.
    pub next_id: u64,
    /// Original observations, ordered by increasing ID.
    pub experiences: Vec<Experience>,
    /// Interpretations and retained revision history, ordered by increasing ID.
    pub beliefs: Vec<Belief>,
}

/// A safe rendered projection; internal object IDs and room metadata stay out.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PersonalRecall {
    /// Canonical subject associated with these recollections.
    pub subject: String,
    /// Source, uncertainty and age labels plus bounded text.
    pub notes: Vec<String>,
    /// Engine evidence IDs actually exposed by the included lines. Copied
    /// evidence remains usable while its owning belief is retained.
    #[serde(default)]
    pub evidence_ids: Vec<u64>,
    /// Current belief IDs actually exposed by the included lines; historical
    /// revision/dispute targets are context, not active amendment handles.
    #[serde(default)]
    pub belief_ids: Vec<u64>,
}

/// A character the NPC currently perceives. Identity is used only for matching
/// retained evidence; internal object IDs never appear in rendered recall.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecallPerson {
    pub id: ObjId,
    pub name: String,
}

/// Why an observation, interpretation, or persisted document was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersonalMemoryError {
    /// Unknown document version.
    UnsupportedSchema,
    /// Empty, oversized, or noncanonical subject.
    InvalidSubject,
    /// Empty or oversized text/name.
    InvalidText,
    /// Negative or causally inconsistent timestamp.
    InvalidTime,
    /// Wrong NPC, duplicate/conflicting ID, or inconsistent evidence copy.
    InvalidRecord,
    /// Retained entry count or encoded document bytes exceed a hard ceiling.
    TooLarge,
    /// The sequence cannot allocate another positive ID.
    IdExhausted,
    /// The cited observation is not retained by this NPC.
    UnknownEvidence,
    /// A revision names a missing, superseded, or differently scoped belief.
    InvalidReplacement,
    /// A revision lacks strictly newer evidence from the same speaker.
    StaleEvidence,
}

/// The one module-document key for an NPC's personal memory.
#[must_use]
pub fn mind_key(npc: ObjId) -> SmolStr {
    SmolStr::new(format!("mind:{}", npc.0))
}

impl PersonalMemoryRecord {
    fn empty(npc: ObjId) -> Self {
        Self {
            schema: PERSONAL_MEMORY_SCHEMA,
            npc,
            next_id: 1,
            experiences: Vec::new(),
            beliefs: Vec::new(),
        }
    }

    /// Refuse malformed persisted data without repairing its provenance.
    pub fn check(&self) -> Result<(), PersonalMemoryError> {
        if self.schema != PERSONAL_MEMORY_SCHEMA {
            return Err(PersonalMemoryError::UnsupportedSchema);
        }
        if self.next_id == 0 {
            return Err(PersonalMemoryError::InvalidRecord);
        }
        if self.experiences.len() > MAX_EXPERIENCES || self.beliefs.len() > MAX_BELIEFS {
            return Err(PersonalMemoryError::TooLarge);
        }
        let mut seen = BTreeMap::new();
        let mut previous = 0;
        for experience in &self.experiences {
            self.check_experience(experience)?;
            if experience.id <= previous {
                return Err(PersonalMemoryError::InvalidRecord);
            }
            previous = experience.id;
            seen.insert(experience.id, experience);
        }
        let mut previous = 0;
        let mut revisions = BTreeSet::new();
        for belief in &self.beliefs {
            check_subject(&belief.subject)?;
            check_text(&belief.note, MAX_PERSONAL_TEXT_BYTES)?;
            if belief.created_at < 0 {
                return Err(PersonalMemoryError::InvalidTime);
            }
            if belief.id <= previous || belief.id >= self.next_id {
                return Err(PersonalMemoryError::InvalidRecord);
            }
            previous = belief.id;
            if let Some(evidence) = &belief.evidence {
                self.check_experience(evidence)?;
                if evidence.id >= belief.id || evidence.observed_at > belief.created_at {
                    return Err(PersonalMemoryError::InvalidRecord);
                }
                if let Some(original) = seen.insert(evidence.id, evidence)
                    && original != evidence
                {
                    return Err(PersonalMemoryError::InvalidRecord);
                }
            }
            if let Some(replaced) = belief.replaces {
                if replaced == 0 || replaced >= belief.id || !revisions.insert(replaced) {
                    return Err(PersonalMemoryError::InvalidReplacement);
                }
                let Some(evidence) = &belief.evidence else {
                    return Err(PersonalMemoryError::InvalidReplacement);
                };
                if let Some(old) = self.beliefs.iter().find(|old| old.id == replaced)
                    && (old.subject != belief.subject
                        || !can_correct(evidence, old)
                        || belief.created_at < old.created_at)
                {
                    return Err(PersonalMemoryError::InvalidReplacement);
                }
            }
            if let Some(disputed) = belief.disputes {
                if belief.replaces.is_some() || disputed == 0 || disputed >= belief.id {
                    return Err(PersonalMemoryError::InvalidReplacement);
                }
                let Some(evidence) = &belief.evidence else {
                    return Err(PersonalMemoryError::InvalidReplacement);
                };
                if let Some(old) = self.beliefs.iter().find(|old| old.id == disputed)
                    && (old.subject != belief.subject
                        || belief.created_at < old.created_at
                        || old
                            .evidence
                            .as_ref()
                            .is_none_or(|other| same_speaker(evidence, other)))
                {
                    return Err(PersonalMemoryError::InvalidReplacement);
                }
            }
        }
        // Evidence IDs and belief IDs share one sequence, including observations
        // retained only as copies after the main experience ring evicts them.
        if self
            .beliefs
            .iter()
            .any(|belief| seen.contains_key(&belief.id))
            || self
                .beliefs
                .iter()
                .flat_map(|belief| [belief.replaces, belief.disputes])
                .flatten()
                .any(|id| seen.contains_key(&id))
        {
            return Err(PersonalMemoryError::InvalidRecord);
        }
        if encoded_bytes(self)? > MAX_PERSONAL_MEMORY_BYTES {
            return Err(PersonalMemoryError::TooLarge);
        }
        Ok(())
    }

    fn check_experience(&self, experience: &Experience) -> Result<(), PersonalMemoryError> {
        if experience.npc != self.npc || experience.id == 0 || experience.id >= self.next_id {
            return Err(PersonalMemoryError::InvalidRecord);
        }
        check_subject(&experience.subject)?;
        check_text(&experience.text, MAX_PERSONAL_TEXT_BYTES)?;
        if experience.speaker_name.len() > MAX_PERSONAL_NAME_BYTES {
            return Err(PersonalMemoryError::InvalidText);
        }
        if experience.observed_at < 0 {
            return Err(PersonalMemoryError::InvalidTime);
        }
        Ok(())
    }

    fn allocate(&mut self) -> Result<u64, PersonalMemoryError> {
        let id = self.next_id;
        self.next_id = id.checked_add(1).ok_or(PersonalMemoryError::IdExhausted)?;
        Ok(id)
    }

    fn trim(&mut self) -> Result<(), PersonalMemoryError> {
        if self.experiences.len() > MAX_EXPERIENCES {
            let count = self.experiences.len() - MAX_EXPERIENCES;
            self.experiences.drain(..count);
        }
        if self.beliefs.len() > MAX_BELIEFS {
            let count = self.beliefs.len() - MAX_BELIEFS;
            self.beliefs.drain(..count);
        }
        while encoded_bytes(self)? > MAX_PERSONAL_MEMORY_BYTES {
            match (self.experiences.first(), self.beliefs.first()) {
                (Some(experience), Some(belief)) if experience.id < belief.id => {
                    self.experiences.remove(0);
                }
                (Some(_), None) => {
                    self.experiences.remove(0);
                }
                (_, Some(_)) => {
                    self.beliefs.remove(0);
                }
                (None, None) => return Err(PersonalMemoryError::TooLarge),
            }
        }
        self.check()
    }
}

/// World-loop-owned personal memory, rebuilt exclusively from `mind:*` docs.
#[derive(Clone, Debug, Default)]
pub struct PersonalMemoryStore {
    records: BTreeMap<ObjId, PersonalMemoryRecord>,
}

impl PersonalMemoryStore {
    /// Record an already authorized public observation. The caller owns
    /// perception and audience checks; this API never reads hidden world state.
    #[allow(clippy::too_many_arguments)]
    pub fn observe(
        &mut self,
        npc: ObjId,
        subject: &str,
        text: &str,
        speaker: Option<ObjId>,
        speaker_name: &str,
        room: Option<ObjId>,
        kind: ExperienceKind,
        now: i64,
    ) -> Result<PersonalMemoryRecord, PersonalMemoryError> {
        let subject = canonical_subject(subject)?;
        check_text(text, MAX_PERSONAL_TEXT_BYTES)?;
        if speaker_name.len() > MAX_PERSONAL_NAME_BYTES {
            return Err(PersonalMemoryError::InvalidText);
        }
        if now < 0 {
            return Err(PersonalMemoryError::InvalidTime);
        }
        let mut record = self
            .records
            .get(&npc)
            .cloned()
            .unwrap_or_else(|| PersonalMemoryRecord::empty(npc));
        let id = record.allocate()?;
        record.experiences.push(Experience {
            npc,
            id,
            subject,
            text: text.to_owned(),
            speaker,
            speaker_name: speaker_name.to_owned(),
            room,
            kind,
            observed_at: now,
        });
        record.trim()?;
        self.records.insert(npc, record.clone());
        Ok(record)
    }

    /// Store an explicitly subjective belief. Only a retained observation or
    /// immutable evidence copy from this NPC may be cited. A competing speaker adds a disputed account;
    /// it cannot erase the earlier speaker's account.
    #[allow(clippy::too_many_arguments)]
    pub fn believe(
        &mut self,
        npc: ObjId,
        subject: &str,
        note: &str,
        evidence: Option<u64>,
        replaces: Option<u64>,
        now: i64,
    ) -> Result<PersonalMemoryRecord, PersonalMemoryError> {
        let subject = canonical_subject(subject)?;
        check_text(note, MAX_PERSONAL_TEXT_BYTES)?;
        if now < 0 {
            return Err(PersonalMemoryError::InvalidTime);
        }
        let mut record = self
            .records
            .get(&npc)
            .cloned()
            .unwrap_or_else(|| PersonalMemoryRecord::empty(npc));
        let evidence = evidence
            .map(|id| {
                record
                    .experiences
                    .iter()
                    .chain(
                        record
                            .beliefs
                            .iter()
                            .filter_map(|belief| belief.evidence.as_ref()),
                    )
                    .find(|event| event.id == id)
                    .cloned()
                    .ok_or(PersonalMemoryError::UnknownEvidence)
            })
            .transpose()?;
        if evidence
            .as_ref()
            .is_some_and(|event| event.observed_at > now)
        {
            return Err(PersonalMemoryError::InvalidTime);
        }
        let (replaces, disputes) = if let Some(id) = replaces {
            let old = record
                .beliefs
                .iter()
                .find(|belief| belief.id == id)
                .ok_or(PersonalMemoryError::InvalidReplacement)?;
            if old.subject != subject
                || old.created_at > now
                || record
                    .beliefs
                    .iter()
                    .any(|belief| belief.replaces == Some(id))
            {
                return Err(PersonalMemoryError::InvalidReplacement);
            }
            let new_evidence = evidence
                .as_ref()
                .ok_or(PersonalMemoryError::StaleEvidence)?;
            if old
                .evidence
                .as_ref()
                .is_some_and(|old_evidence| !same_speaker(new_evidence, old_evidence))
            {
                (None, Some(id))
            } else {
                if !can_correct(new_evidence, old) {
                    return Err(PersonalMemoryError::StaleEvidence);
                }
                (Some(id), None)
            }
        } else {
            (None, None)
        };
        let id = record.allocate()?;
        record.beliefs.push(Belief {
            id,
            subject,
            note: note.to_owned(),
            evidence,
            replaces,
            disputes,
            created_at: now,
        });
        record.trim()?;
        self.records.insert(npc, record.clone());
        Ok(record)
    }

    /// Load a checked document. The caller also checks its `mind:<npc>` key.
    pub fn insert(&mut self, record: PersonalMemoryRecord) -> Result<(), PersonalMemoryError> {
        record.check()?;
        if self
            .records
            .get(&record.npc)
            .is_some_and(|old| old.next_id > record.next_id)
        {
            return Err(PersonalMemoryError::InvalidRecord);
        }
        self.records.insert(record.npc, record);
        Ok(())
    }

    /// Inspect this NPC's durable record without creating an empty record.
    #[must_use]
    pub fn record_for(&self, npc: ObjId) -> Option<&PersonalMemoryRecord> {
        self.records.get(&npc)
    }

    /// Number of retained original observations, excluding copied evidence.
    #[must_use]
    pub fn experience_count(&self, npc: ObjId) -> usize {
        self.record_for(npc)
            .map_or(0, |record| record.experiences.len())
    }

    /// Number of retained beliefs, including superseded revision history.
    #[must_use]
    pub fn belief_count(&self, npc: ObjId) -> usize {
        self.record_for(npc)
            .map_or(0, |record| record.beliefs.len())
    }

    /// Legacy name-based recall for callers without perceived identities.
    /// Live dialogue should use [`Self::recall_for_people`] so names cannot
    /// associate a different character with an earlier speaker's evidence.
    #[must_use]
    pub fn recall(
        &self,
        npc: ObjId,
        present_names: &[String],
        cue: &str,
        now: i64,
        top_n: usize,
    ) -> Vec<PersonalRecall> {
        let names = present_names
            .iter()
            .take(64)
            .map(|name| crate::memory::slug(name))
            .collect();
        self.recall_with_audience(npc, RecallAudience::Names(names), cue, now, top_n)
    }

    /// Deterministic, bounded recall for currently perceived identities. Topic
    /// cues rank first, matching evidence speakers second, then recent context.
    /// Names alone never grant a present-person boost. Age and uncertainty stay
    /// attached to original evidence, including evidence copied into beliefs.
    #[must_use]
    pub fn recall_for_people(
        &self,
        npc: ObjId,
        people: &[RecallPerson],
        cue: &str,
        now: i64,
        top_n: usize,
    ) -> Vec<PersonalRecall> {
        let people: Vec<_> = people
            .iter()
            .take(64)
            .filter(|person| {
                !person.name.trim().is_empty() && person.name.len() <= MAX_PERSONAL_NAME_BYTES
            })
            .cloned()
            .collect();
        self.recall_with_audience(npc, RecallAudience::People(&people), cue, now, top_n)
    }

    fn recall_with_audience(
        &self,
        npc: ObjId,
        audience: RecallAudience<'_>,
        cue: &str,
        now: i64,
        top_n: usize,
    ) -> Vec<PersonalRecall> {
        let Some(record) = self.record_for(npc) else {
            return Vec::new();
        };
        let limit = top_n.min(MAX_PERSONAL_RECALL_ENTRIES);
        if limit == 0 {
            return Vec::new();
        }
        let cue = cue_tokens(cue);
        let superseded: BTreeSet<u64> = record
            .beliefs
            .iter()
            .filter_map(|belief| belief.replaces)
            .collect();
        let active: Vec<&Belief> = record
            .beliefs
            .iter()
            .filter(|belief| !superseded.contains(&belief.id))
            .collect();
        let mut candidates = Vec::new();
        for event in &record.experiences {
            let explicit = matches_cue(&event.subject, &event.text, &cue);
            let age = age_label(now, event.observed_at);
            if age == Age::Old && !explicit {
                continue;
            }
            let present = audience.is_present(&event.subject, Some(event));
            candidates.push(RecallCandidate {
                priority: u8::from(explicit) * 2 + u8::from(present),
                id: event.id,
                evidence_id: Some(event.id),
                belief_id: None,
                subject: &event.subject,
                text: render_experience(event, age, &audience),
            });
        }
        for belief in &active {
            let evidence_text = belief
                .evidence
                .as_ref()
                .map_or("", |event| event.text.as_str());
            let explicit = matches_cue(&belief.subject, &belief.note, &cue)
                || matches_cue(&belief.subject, evidence_text, &cue);
            // Reinterpreting old evidence does not make the evidence fresh.
            let observed_at = belief
                .evidence
                .as_ref()
                .map_or(belief.created_at, |event| event.observed_at);
            let age = age_label(now, observed_at);
            if age == Age::Old && !explicit {
                continue;
            }
            let present = audience.is_present(&belief.subject, belief.evidence.as_ref());
            let disputed = belief.disputes.is_some()
                || active.iter().any(|other| other.disputes == Some(belief.id));
            candidates.push(RecallCandidate {
                priority: u8::from(explicit) * 2 + u8::from(present),
                id: belief.id,
                evidence_id: belief.evidence.as_ref().map(|event| event.id),
                belief_id: Some(belief.id),
                subject: &belief.subject,
                text: render_belief(belief, age, disputed, &audience),
            });
        }
        candidates.sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| right.id.cmp(&left.id))
        });
        let mut result: Vec<PersonalRecall> = Vec::new();
        let mut bytes = 0usize;
        let mut entries = 0usize;
        for candidate in candidates {
            if entries >= limit {
                break;
            }
            let position = result
                .iter()
                .position(|item| item.subject == candidate.subject);
            let cost =
                candidate
                    .text
                    .len()
                    .saturating_add(1)
                    .saturating_add(if position.is_none() {
                        candidate.subject.len() + 1
                    } else {
                        0
                    });
            if bytes.saturating_add(cost) > MAX_PERSONAL_RECALL_BYTES {
                continue;
            }
            bytes += cost;
            entries += 1;
            if let Some(position) = position {
                if let Some(item) = result.get_mut(position) {
                    item.notes.push(candidate.text);
                    include_id(&mut item.evidence_ids, candidate.evidence_id);
                    include_id(&mut item.belief_ids, candidate.belief_id);
                }
            } else {
                result.push(PersonalRecall {
                    subject: candidate.subject.to_owned(),
                    notes: vec![candidate.text],
                    evidence_ids: candidate.evidence_id.into_iter().collect(),
                    belief_ids: candidate.belief_id.into_iter().collect(),
                });
            }
        }
        result
    }
}

enum RecallAudience<'a> {
    Names(BTreeSet<String>),
    People(&'a [RecallPerson]),
}

impl RecallAudience<'_> {
    fn is_present(&self, subject: &str, evidence: Option<&Experience>) -> bool {
        match self {
            Self::Names(names) => {
                names.contains(subject)
                    || evidence.is_some_and(|event| {
                        names.contains(&crate::memory::slug(&event.speaker_name))
                    })
            }
            Self::People(people) => evidence
                .and_then(|event| event.speaker)
                .is_some_and(|id| people.iter().any(|person| person.id == id)),
        }
    }

    fn source(&self, event: &Experience) -> String {
        let historical = event.speaker_name.trim();
        let speaker = quote_excerpt(
            if historical.is_empty() {
                "unknown speaker"
            } else {
                historical
            },
            256,
        );
        let Self::People(people) = self else {
            return speaker;
        };
        let Some(id) = event.speaker else {
            return format!("{speaker} (historical name; identity unverified)");
        };
        if let Some(person) = people.iter().find(|person| person.id == id) {
            if people.iter().any(|other| {
                other.id != id && other.name.trim().eq_ignore_ascii_case(person.name.trim())
            }) {
                return format!(
                    "{speaker} (one of the present characters named {}; name alone does not distinguish them)",
                    quote_excerpt(person.name.trim(), 256)
                );
            }
            if person.name.trim().eq_ignore_ascii_case(historical) {
                return format!("{speaker} (same observed character now present)");
            }
            return format!(
                "{speaker} (same observed character now known here as {})",
                quote_excerpt(person.name.trim(), 256)
            );
        }
        if !historical.is_empty()
            && people
                .iter()
                .any(|person| person.name.trim().eq_ignore_ascii_case(historical))
        {
            return format!(
                "{speaker} (a different character from the person now present with this name)"
            );
        }
        speaker
    }
}

struct RecallCandidate<'a> {
    priority: u8,
    id: u64,
    evidence_id: Option<u64>,
    belief_id: Option<u64>,
    subject: &'a str,
    text: String,
}

fn include_id(ids: &mut Vec<u64>, id: Option<u64>) {
    if let Some(id) = id
        && !ids.contains(&id)
    {
        ids.push(id);
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Age {
    Fresh,
    Fading,
    Old,
}

impl Age {
    fn label(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Fading => "fading; uncertain recall",
            Self::Old => "old; uncertain recall",
        }
    }
}

fn age_label(now: i64, then: i64) -> Age {
    match now.saturating_sub(then).max(0) {
        elapsed if elapsed >= OLD_AFTER_SECS => Age::Old,
        elapsed if elapsed >= FADING_AFTER_SECS => Age::Fading,
        _ => Age::Fresh,
    }
}

fn same_speaker(left: &Experience, right: &Experience) -> bool {
    match (left.speaker, right.speaker) {
        (Some(left), Some(right)) => left == right,
        (None, None) => {
            !left.speaker_name.trim().is_empty()
                && left
                    .speaker_name
                    .trim()
                    .eq_ignore_ascii_case(right.speaker_name.trim())
        }
        _ => false,
    }
}

fn newer_evidence(new: &Experience, old: &Experience) -> bool {
    new.id > old.id && new.observed_at >= old.observed_at
}

fn can_correct(evidence: &Experience, old: &Belief) -> bool {
    match &old.evidence {
        Some(previous) => same_speaker(evidence, previous) && newer_evidence(evidence, previous),
        None => evidence.id > old.id && evidence.observed_at >= old.created_at,
    }
}

fn render_experience(event: &Experience, age: Age, audience: &RecallAudience<'_>) -> String {
    let source = match event.kind {
        ExperienceKind::Speech => "reported speech (not verified truth)",
        ExperienceKind::Outcome => "observed outcome",
    };
    let prefix = format!(
        "experience #{} [{}; {source}; source {}]: ",
        event.id,
        age.label(),
        audience.source(event)
    );
    let text = quote_excerpt(
        &event.text,
        MAX_PERSONAL_RECALL_NOTE_BYTES.saturating_sub(prefix.len()),
    );
    format!("{prefix}{text}")
}

fn render_belief(
    belief: &Belief,
    age: Age,
    disputed: bool,
    audience: &RecallAudience<'_>,
) -> String {
    let status = if disputed { "disputed" } else { "current" };
    let revision = belief.replaces.map_or_else(String::new, |id| {
        format!("; revises belief #{id} (prior interpretation superseded)")
    });
    let dispute = belief
        .disputes
        .map_or_else(String::new, |id| format!("; disputes belief #{id}"));
    match &belief.evidence {
        Some(event) => {
            let source = match event.kind {
                ExperienceKind::Speech => "reported speech, not verified truth",
                ExperienceKind::Outcome => "observed outcome",
            };
            let prefix = format!(
                "belief #{} [uncertain belief; {status}; {}{revision}{dispute}; from experience #{} ({source}; source {})]: ",
                belief.id,
                age.label(),
                event.id,
                audience.source(event)
            );
            let separator = " — original observation: ";
            let budget =
                MAX_PERSONAL_RECALL_NOTE_BYTES.saturating_sub(prefix.len() + separator.len());
            let note_budget = budget / 2;
            format!(
                "{prefix}{}{separator}{}",
                quote_excerpt(&belief.note, note_budget),
                quote_excerpt(&event.text, budget - note_budget)
            )
        }
        None => {
            let prefix = format!(
                "belief #{} [unsupported inference; {status}; {}; no observed evidence]: ",
                belief.id,
                age.label()
            );
            let note = quote_excerpt(
                &belief.note,
                MAX_PERSONAL_RECALL_NOTE_BYTES.saturating_sub(prefix.len()),
            );
            format!("{prefix}{note}")
        }
    }
}

fn quote(text: &str) -> String {
    // Strings always serialize; the fallback still cannot create instructions.
    serde_json::to_string(text).unwrap_or_else(|_| "\"[unavailable text]\"".to_owned())
}

fn quote_excerpt(text: &str, limit: usize) -> String {
    let full = quote(text);
    if full.len() <= limit {
        return full;
    }
    let suffix = "… [excerpt]";
    let mut bytes = suffix.len() + 2;
    let mut end = 0;
    for (index, ch) in text.char_indices() {
        let cost = match ch {
            '"' | '\\' | '\n' | '\r' | '\t' | '\u{08}' | '\u{0c}' => 2,
            ch if ch <= '\u{1f}' => 6,
            ch => ch.len_utf8(),
        };
        if bytes + cost > limit {
            break;
        }
        bytes += cost;
        end = index + ch.len_utf8();
    }
    quote(&format!("{}{suffix}", text.get(..end).unwrap_or_default()))
}

fn canonical_subject(subject: &str) -> Result<String, PersonalMemoryError> {
    // Bound input before normalization, not just its potentially tiny slug.
    if subject.len() > MAX_PERSONAL_SUBJECT_BYTES * 4 {
        return Err(PersonalMemoryError::InvalidSubject);
    }
    let subject = crate::memory::slug(subject);
    if subject.is_empty() || subject.len() > MAX_PERSONAL_SUBJECT_BYTES {
        return Err(PersonalMemoryError::InvalidSubject);
    }
    Ok(subject)
}

fn check_subject(subject: &str) -> Result<(), PersonalMemoryError> {
    if canonical_subject(subject)? != subject {
        return Err(PersonalMemoryError::InvalidSubject);
    }
    Ok(())
}

fn check_text(text: &str, limit: usize) -> Result<(), PersonalMemoryError> {
    if text.trim().is_empty() || text.len() > limit {
        return Err(PersonalMemoryError::InvalidText);
    }
    Ok(())
}

fn encoded_bytes(record: &PersonalMemoryRecord) -> Result<usize, PersonalMemoryError> {
    serde_json::to_vec(record)
        .map(|encoded| encoded.len())
        .map_err(|_| PersonalMemoryError::TooLarge)
}

fn tokens(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn meaningful(word: &str) -> bool {
    word.chars().count() >= 3
        && !matches!(
            word,
            "the"
                | "and"
                | "but"
                | "that"
                | "this"
                | "those"
                | "these"
                | "there"
                | "here"
                | "what"
                | "where"
                | "when"
                | "which"
                | "who"
                | "why"
                | "how"
                | "about"
                | "with"
                | "from"
                | "have"
                | "has"
                | "had"
                | "was"
                | "were"
                | "are"
                | "you"
                | "your"
                | "they"
                | "their"
                | "them"
                | "she"
                | "her"
                | "his"
                | "can"
                | "could"
                | "would"
                | "should"
                | "will"
                | "did"
                | "does"
                | "for"
                | "please"
                | "tell"
                | "said"
                | "says"
                | "say"
                | "heard"
                | "remember"
                | "know"
                | "think"
                | "again"
                | "some"
                | "anything"
                | "something"
        )
}

fn cue_tokens(cue: &str) -> Vec<String> {
    // The world builds small cues, but keep this public API independently bounded.
    let bounded: String = cue.chars().take(1024).collect();
    tokens(&bounded)
        .into_iter()
        .filter(|word| meaningful(word))
        .take(64)
        .collect()
}

fn matches_cue(subject: &str, text: &str, cue: &[String]) -> bool {
    if cue.is_empty() {
        return false;
    }
    let subject_words: Vec<String> = tokens(subject)
        .into_iter()
        .filter(|word| meaningful(word))
        .collect();
    if !subject_words.is_empty() && subject_words.iter().all(|word| cue.contains(word)) {
        return true;
    }
    let content: BTreeSet<String> = tokens(text)
        .into_iter()
        .filter(|word| meaningful(word))
        .collect();
    let hits = cue
        .iter()
        .filter(|word| content.contains(*word))
        .collect::<BTreeSet<_>>()
        .len();
    // One meaningful concrete term is useful for short questions; longer cues
    // need two matching terms so ordinary conversational filler cannot revive
    // every old memory. Multiword subjects/slugs above require all their words.
    hits >= 2
        || (cue.len() == 1
            && cue.first().is_some_and(|word| word.chars().count() >= 4)
            && hits == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NPC: ObjId = ObjId(10);
    const ALICE: ObjId = ObjId(20);
    const BOB: ObjId = ObjId(30);

    fn observed(store: &mut PersonalMemoryStore, speaker: ObjId, text: &str, at: i64) -> u64 {
        store
            .observe(
                NPC,
                "fuel depot",
                text,
                Some(speaker),
                if speaker == ALICE { "Alice" } else { "Bob" },
                Some(ObjId(99)),
                ExperienceKind::Speech,
                at,
            )
            .expect("observation")
            .experiences
            .last()
            .expect("new experience")
            .id
    }

    fn believed(
        store: &mut PersonalMemoryStore,
        note: &str,
        evidence: u64,
        replaces: Option<u64>,
        at: i64,
    ) -> u64 {
        store
            .believe(NPC, "fuel depot", note, Some(evidence), replaces, at)
            .expect("belief")
            .beliefs
            .last()
            .expect("new belief")
            .id
    }

    fn rendered(store: &PersonalMemoryStore, cue: &str, now: i64) -> String {
        store
            .recall(NPC, &[], cue, now, usize::MAX)
            .into_iter()
            .flat_map(|record| record.notes)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn evidence_is_local_existing_and_immutable() {
        let mut store = PersonalMemoryStore::default();
        let event = observed(&mut store, ALICE, "Alice reports the depot is closed.", 10);
        let before = store.record_for(NPC).cloned();
        assert_eq!(
            store.believe(NPC, "depot", "Guess", Some(99), None, 11),
            Err(PersonalMemoryError::UnknownEvidence)
        );
        assert_eq!(store.record_for(NPC), before.as_ref());
        assert_eq!(
            store.believe(ObjId(11), "depot", "Stolen", Some(event), None, 11),
            Err(PersonalMemoryError::UnknownEvidence)
        );
        believed(&mut store, "The depot may be closed.", event, None, 11);
        let copy = store
            .record_for(NPC)
            .expect("record")
            .beliefs
            .last()
            .expect("belief")
            .evidence
            .clone()
            .expect("copy");
        for n in 0..MAX_EXPERIENCES {
            store
                .observe(
                    NPC,
                    "gardens",
                    "Unrelated public speech.",
                    Some(BOB),
                    "Bob",
                    None,
                    ExperienceKind::Speech,
                    12 + n as i64,
                )
                .expect("other topic");
        }
        let record = store.record_for(NPC).expect("record");
        assert!(!record.experiences.iter().any(|item| item.id == event));
        assert_eq!(
            record.beliefs.first().expect("belief").evidence.as_ref(),
            Some(&copy)
        );
        assert!(rendered(&store, "fuel depot", 100).contains("Alice reports the depot is closed."));
        let copied = store
            .believe(
                NPC,
                "depot",
                "A later interpretation of retained evidence.",
                Some(event),
                None,
                100,
            )
            .expect("retained immutable source");
        assert_eq!(
            copied.beliefs.last().expect("new belief").evidence.as_ref(),
            Some(&copy)
        );
    }

    #[test]
    fn same_speaker_new_evidence_revises_without_rewriting_experiences() {
        let mut store = PersonalMemoryStore::default();
        let first = observed(&mut store, ALICE, "The depot is closed.", 10);
        let old = believed(&mut store, "It might be closed.", first, None, 11);
        let correction = observed(&mut store, ALICE, "Correction: the depot is open.", 12);
        believed(
            &mut store,
            "It might be open now.",
            correction,
            Some(old),
            13,
        );
        let record = store.record_for(NPC).expect("record");
        assert_eq!(
            record.experiences.first().expect("first").text,
            "The depot is closed."
        );
        assert_eq!(record.beliefs.len(), 2);
        assert_eq!(record.beliefs.last().expect("new").replaces, Some(old));
        let recall = rendered(&store, "depot", 13);
        assert!(!recall.contains("It might be closed."));
        assert!(recall.contains(&format!("revises belief #{old}")));
        assert!(recall.contains("It might be open now."));
        assert!(!recall.contains("disputed"));
        assert_eq!(
            store.believe(
                NPC,
                "fuel depot",
                "Revise stale branch",
                Some(correction),
                Some(old),
                14
            ),
            Err(PersonalMemoryError::InvalidReplacement)
        );
    }

    #[test]
    fn cross_speaker_account_is_disputed_not_a_correction() {
        let mut store = PersonalMemoryStore::default();
        let first = observed(&mut store, ALICE, "The depot is closed.", 10);
        let old = believed(&mut store, "Alice's account says closed.", first, None, 11);
        let other = observed(&mut store, BOB, "The depot is open.", 12);
        believed(&mut store, "Bob's account says open.", other, Some(old), 13);
        let record = store.record_for(NPC).expect("record");
        assert_eq!(record.beliefs.len(), 2);
        assert_eq!(record.beliefs.last().expect("new").replaces, None);
        assert_eq!(record.beliefs.last().expect("new").disputes, Some(old));
        let recall = rendered(&store, "depot", 13);
        assert!(recall.contains("Alice's account says closed."));
        assert!(recall.contains("Bob's account says open."));
        assert_eq!(recall.matches("uncertain belief; disputed").count(), 2);
    }

    #[test]
    fn corroborating_or_different_facet_accounts_are_not_invented_disputes() {
        let mut store = PersonalMemoryStore::default();
        let first = observed(&mut store, ALICE, "The depot is open.", 10);
        believed(&mut store, "The depot may be open.", first, None, 11);
        let second = observed(
            &mut store,
            BOB,
            "The depot is open; the attendant is friendly.",
            12,
        );
        believed(
            &mut store,
            "The depot may be open and welcoming.",
            second,
            None,
            13,
        );
        let recall = rendered(&store, "fuel depot", 13);
        assert!(!recall.contains("disputed"));
        assert_eq!(recall.matches("uncertain belief; current").count(), 2);
    }

    #[test]
    fn new_evidence_can_correct_an_unsupported_initial_guess() {
        let mut store = PersonalMemoryStore::default();
        let record = store
            .believe(NPC, "fuel depot", "Perhaps it is closed.", None, None, 10)
            .expect("guess");
        let old = record.beliefs.first().expect("guess").id;
        let event = observed(&mut store, ALICE, "The depot is open.", 11);
        believed(
            &mut store,
            "It may be open after all.",
            event,
            Some(old),
            12,
        );
        let record = store.record_for(NPC).expect("record");
        assert!(record.check().is_ok());
        assert_eq!(
            record.beliefs.last().expect("correction").replaces,
            Some(old)
        );
        let recall = rendered(&store, "fuel depot", 12);
        assert!(!recall.contains("Perhaps it is closed."));
        assert!(recall.contains(&format!("revises belief #{old}")));
        let mut rebooted = PersonalMemoryStore::default();
        rebooted
            .insert(record.clone())
            .expect("valid corrected guess");
        assert_eq!(rendered(&rebooted, "fuel depot", 12), recall);
    }

    #[test]
    fn replacement_rejects_stale_missing_and_wrong_subject_evidence_atomically() {
        let mut store = PersonalMemoryStore::default();
        let event = observed(&mut store, ALICE, "The depot is closed.", 10);
        let old = believed(&mut store, "It might be closed.", event, None, 11);
        let before = store.record_for(NPC).cloned();
        assert_eq!(
            store.believe(
                NPC,
                "fuel depot",
                "Unsupported rewrite",
                None,
                Some(old),
                12
            ),
            Err(PersonalMemoryError::StaleEvidence)
        );
        assert_eq!(
            store.believe(
                NPC,
                "fuel depot",
                "Same evidence",
                Some(event),
                Some(old),
                12
            ),
            Err(PersonalMemoryError::StaleEvidence)
        );
        assert_eq!(
            store.believe(
                NPC,
                "another topic",
                "Wrong topic",
                Some(event),
                Some(old),
                12
            ),
            Err(PersonalMemoryError::InvalidReplacement)
        );
        assert_eq!(store.record_for(NPC), before.as_ref());
    }

    #[test]
    fn missing_evidence_is_explicitly_an_unsupported_inference() {
        let mut store = PersonalMemoryStore::default();
        store
            .believe(
                NPC,
                "Alice",
                "Perhaps Alice likes quiet rooms.",
                None,
                None,
                10,
            )
            .expect("legacy inference");
        let recall = rendered(&store, "Alice", 10);
        assert!(recall.contains("unsupported inference"));
        assert!(recall.contains("no observed evidence"));
        assert!(!recall.contains("observed outcome"));
    }

    #[test]
    fn aging_changes_availability_not_content_and_cues_recover_old_topics() {
        let mut store = PersonalMemoryStore::default();
        observed(
            &mut store,
            ALICE,
            "The fuel depot is beside the northern landing pad.",
            100,
        );
        assert!(rendered(&store, "", 100 + FADING_AFTER_SECS - 1).contains("fresh"));
        assert!(rendered(&store, "", 100 + FADING_AFTER_SECS).contains("fading; uncertain recall"));
        assert!(
            store
                .recall(NPC, &["Alice".into()], "", 100 + OLD_AFTER_SECS, 10)
                .is_empty()
        );
        for cue in [
            "What about the fuel depot?",
            "fuel_depot",
            "Where was the northern landing pad?",
        ] {
            let recall = rendered(&store, cue, 100 + OLD_AFTER_SECS);
            assert!(recall.contains("old; uncertain recall"), "{cue}: {recall}");
            assert!(recall.contains("The fuel depot is beside the northern landing pad."));
        }
        assert!(rendered(&store, "What did you say?", 100 + OLD_AFTER_SECS).is_empty());
    }

    #[test]
    fn recent_interpretation_does_not_refresh_old_evidence() {
        let mut store = PersonalMemoryStore::default();
        let event = observed(&mut store, ALICE, "Depot closed.", 1);
        believed(
            &mut store,
            "It may still be closed.",
            event,
            None,
            OLD_AFTER_SECS + 1,
        );
        assert!(rendered(&store, "", OLD_AFTER_SECS + 1).is_empty());
        assert!(
            rendered(&store, "fuel depot", OLD_AFTER_SECS + 1).contains("old; uncertain recall")
        );
    }

    #[test]
    fn recall_prioritizes_cues_then_present_speakers_with_stable_ids() {
        let mut store = PersonalMemoryStore::default();
        observed(&mut store, ALICE, "Fuel depot report.", 10);
        store
            .observe(
                NPC,
                "Bob",
                "Bob discussed the gardens.",
                Some(BOB),
                "Bob",
                None,
                ExperienceKind::Speech,
                11,
            )
            .expect("Bob");
        store
            .observe(
                NPC,
                "news",
                "The newest unrelated public report.",
                None,
                "",
                None,
                ExperienceKind::Outcome,
                12,
            )
            .expect("news");
        let recall = store.recall(NPC, &["Bob".into()], "fuel_depot", 13, 2);
        assert_eq!(recall.first().expect("first").subject, "fuel_depot");
        assert_eq!(recall.get(1).expect("second").subject, "bob");
        assert_eq!(
            recall,
            store.recall(NPC, &["Bob".into()], "fuel_depot", 13, 2)
        );
        assert!(store.recall(NPC, &[], "", 13, 0).is_empty());
    }

    #[test]
    fn identity_recall_follows_a_rename_without_rewriting_the_observation() {
        let mut store = PersonalMemoryStore::default();
        let alice = observed(&mut store, ALICE, "Alice says the fuel depot is open.", 10);
        observed(&mut store, BOB, "Bob has a newer unrelated report.", 11);
        let before = store.record_for(NPC).cloned();
        let people = [RecallPerson {
            id: ALICE,
            name: "Beatrice".into(),
        }];
        let recall = store.recall_for_people(NPC, &people, "", 12, 1);
        let item = recall.first().expect("renamed speaker prioritized");
        assert_eq!(item.evidence_ids, vec![alice]);
        let note = &item.notes[0];
        assert!(
            note.contains(
                "source \"Alice\" (same observed character now known here as \"Beatrice\")"
            )
        );
        assert!(note.contains("Alice says the fuel depot is open."));
        assert!(note.contains("reported speech (not verified truth)"));
        assert_eq!(store.record_for(NPC), before.as_ref());
    }

    #[test]
    fn same_name_different_identity_does_not_gain_presence_priority() {
        let mut store = PersonalMemoryStore::default();
        let original = observed(&mut store, ALICE, "The original manifest was delayed.", 10);
        let newer = observed(&mut store, BOB, "The gardens are quiet.", 11);
        let people = [RecallPerson {
            id: ObjId(987654),
            name: "Alice".into(),
        }];
        let ordinary = store.recall_for_people(NPC, &people, "", 12, 1);
        assert_eq!(ordinary[0].evidence_ids, vec![newer]);
        let cued = store.recall_for_people(NPC, &people, "original manifest", 12, 1);
        assert_eq!(cued[0].evidence_ids, vec![original]);
        assert!(
            cued[0].notes[0]
                .contains("a different character from the person now present with this name")
        );
        assert!(!cued[0].notes[0].contains("987654"));
        assert!(cued[0].notes[0].contains("not verified truth"));
    }

    #[test]
    fn grounded_beliefs_use_copied_evidence_identity_after_source_eviction() {
        let mut store = PersonalMemoryStore::default();
        let event = observed(&mut store, ALICE, "The original fuel depot report.", 10);
        let belief = believed(&mut store, "The depot may be open.", event, None, 11);
        for index in 0..MAX_EXPERIENCES {
            observed(
                &mut store,
                BOB,
                "A newer garden observation.",
                12 + index as i64,
            );
        }
        let before = store.record_for(NPC).cloned();
        assert!(
            !before
                .as_ref()
                .expect("record")
                .experiences
                .iter()
                .any(|row| row.id == event)
        );
        let people = [RecallPerson {
            id: ALICE,
            name: "Beatrice".into(),
        }];
        let recall = store.recall_for_people(NPC, &people, "", 100, 1);
        assert_eq!(recall[0].evidence_ids, vec![event]);
        assert_eq!(recall[0].belief_ids, vec![belief]);
        assert!(recall[0].notes[0].contains("uncertain belief"));
        assert!(
            recall[0].notes[0].contains("same observed character now known here as \"Beatrice\"")
        );
        assert!(recall[0].notes[0].contains("The original fuel depot report."));
        assert_eq!(store.record_for(NPC), before.as_ref());

        let imposter = [RecallPerson {
            id: BOB,
            name: "Alice".into(),
        }];
        let recall = store.recall_for_people(NPC, &imposter, "", 100, 1);
        assert!(
            recall[0].belief_ids.is_empty(),
            "a belief's old speaker name cannot override a mismatched identity"
        );
    }

    #[test]
    fn unknown_speakers_and_unsupported_beliefs_cannot_claim_a_present_identity() {
        let mut store = PersonalMemoryStore::default();
        store
            .observe(
                NPC,
                "Alice",
                "The unauthenticated report.",
                None,
                "Alice",
                None,
                ExperienceKind::Speech,
                10,
            )
            .expect("unknown speaker");
        store
            .believe(
                NPC,
                "Alice",
                "An unsupported interpretation.",
                None,
                None,
                11,
            )
            .expect("legacy-compatible inference");
        let bob = observed(&mut store, BOB, "The latest gardens report.", 12);
        let people = [RecallPerson {
            id: ALICE,
            name: "Alice".into(),
        }];
        let recall = store.recall_for_people(NPC, &people, "", 13, 1);
        assert_eq!(recall[0].evidence_ids, vec![bob]);
        let recall = store.recall_for_people(NPC, &people, "Alice", 13, 2);
        let text = recall
            .iter()
            .flat_map(|item| &item.notes)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("identity unverified"));
        assert!(text.contains("unsupported inference"));
        assert!(text.contains("no observed evidence"));
        assert!(!text.contains("same observed character"));
    }

    #[test]
    fn identity_recall_preserves_age_and_budgets_with_long_current_names() {
        let mut store = PersonalMemoryStore::default();
        let body = format!("The fuel depot report {}", "水".repeat(1200));
        for index in 0..MAX_EXPERIENCES {
            observed(&mut store, ALICE, &body, 100 + index as i64);
        }
        let people = [RecallPerson {
            id: ALICE,
            name: "新".repeat(80),
        }];
        let now = 200 + FADING_AFTER_SECS;
        let recall = store.recall_for_people(NPC, &people, "fuel depot", now, usize::MAX);
        assert_eq!(
            recall,
            store.recall_for_people(NPC, &people, "fuel depot", now, usize::MAX)
        );
        let count: usize = recall.iter().map(|item| item.notes.len()).sum();
        let bytes: usize = recall
            .iter()
            .map(|item| {
                item.subject.len() + 1 + item.notes.iter().map(|note| note.len() + 1).sum::<usize>()
            })
            .sum();
        assert!(count > 0 && count <= MAX_PERSONAL_RECALL_ENTRIES);
        assert!(bytes <= MAX_PERSONAL_RECALL_BYTES);
        assert!(
            recall
                .iter()
                .flat_map(|item| &item.notes)
                .all(|note| note.len() <= MAX_PERSONAL_RECALL_NOTE_BYTES
                    && note.contains("fading; uncertain recall"))
        );
        assert!(
            store
                .recall_for_people(NPC, &people, "", 200 + OLD_AFTER_SECS, 16)
                .is_empty()
        );
        let old = store.recall_for_people(NPC, &people, "fuel depot", 200 + OLD_AFTER_SECS, 1);
        assert!(old[0].notes[0].contains("old; uncertain recall"));
        assert!(
            store
                .recall_for_people(NPC, &people, "fuel depot", now, 0)
                .is_empty()
        );
    }

    #[test]
    fn recall_handles_follow_only_included_entries_and_deduplicate_sources() {
        let mut store = PersonalMemoryStore::default();
        let event = observed(&mut store, ALICE, "The fuel depot is open.", 10);
        let belief = believed(&mut store, "It may be open.", event, None, 11);
        let one = store.recall(NPC, &[], "fuel depot", 12, 1);
        let projected = one.first().expect("one belief");
        assert_eq!(projected.notes.len(), 1);
        assert_eq!(projected.evidence_ids, vec![event]);
        assert_eq!(projected.belief_ids, vec![belief]);
        let both = store.recall(NPC, &[], "fuel depot", 12, 2);
        let projected = both.first().expect("both entries");
        assert_eq!(projected.notes.len(), 2);
        assert_eq!(projected.evidence_ids, vec![event]);
        assert_eq!(projected.belief_ids, vec![belief]);
        let correction = observed(&mut store, ALICE, "The fuel depot closed again.", 13);
        let newer = believed(
            &mut store,
            "It may now be closed.",
            correction,
            Some(belief),
            14,
        );
        let one = store.recall(NPC, &[], "fuel depot", 15, 1);
        let projected = one.first().expect("current belief only");
        assert_eq!(projected.belief_ids, vec![newer]);
        assert_eq!(projected.evidence_ids, vec![correction]);
        assert!(
            projected
                .notes
                .first()
                .expect("line")
                .contains(&format!("revises belief #{belief}"))
        );
        // Historical revision text must not reopen the old amendment handle.
        assert!(!projected.belief_ids.contains(&belief));
        assert!(!projected.evidence_ids.contains(&event));
    }

    #[test]
    fn evicted_evidence_without_a_retained_copy_is_unknown() {
        let mut store = PersonalMemoryStore::default();
        let old = observed(&mut store, ALICE, "An uncopied old report.", 1);
        for n in 0..MAX_EXPERIENCES {
            observed(&mut store, BOB, "A later report.", 2 + n as i64);
        }
        assert_eq!(
            store.believe(NPC, "topic", "Guess an evicted ID", Some(old), None, 100),
            Err(PersonalMemoryError::UnknownEvidence)
        );
    }

    #[test]
    fn reusing_copied_evidence_never_refreshes_its_age() {
        let mut store = PersonalMemoryStore::default();
        let old = observed(&mut store, ALICE, "The fuel depot is closed.", 1);
        believed(&mut store, "It may be closed.", old, None, 2);
        for n in 0..MAX_EXPERIENCES {
            store
                .observe(
                    NPC,
                    "other",
                    "An unrelated report.",
                    Some(BOB),
                    "Bob",
                    None,
                    ExperienceKind::Speech,
                    3 + n as i64,
                )
                .expect("other report");
        }
        let now = OLD_AFTER_SECS + 1;
        let record = store
            .believe(
                NPC,
                "fuel depot",
                "Still only an old report.",
                Some(old),
                None,
                now,
            )
            .expect("copied source");
        assert_eq!(
            record
                .beliefs
                .last()
                .expect("new belief")
                .evidence
                .as_ref()
                .expect("source")
                .observed_at,
            1
        );
        assert!(!rendered(&store, "", now).contains("Still only an old report."));
        assert!(rendered(&store, "fuel depot", now).contains("old; uncertain recall"));
        let mut rebooted = PersonalMemoryStore::default();
        rebooted.insert(record).expect("boot copied source");
        assert_eq!(
            rendered(&store, "fuel depot", now),
            rendered(&rebooted, "fuel depot", now)
        );
    }

    #[test]
    fn bounds_preserve_newest_entries_and_never_reuse_ids() {
        let mut store = PersonalMemoryStore::default();
        for n in 0..MAX_EXPERIENCES + 10 {
            observed(&mut store, ALICE, "A public report.", n as i64);
        }
        assert_eq!(store.experience_count(NPC), MAX_EXPERIENCES);
        for n in 0..MAX_BELIEFS + 10 {
            store
                .believe(
                    NPC,
                    "topic",
                    "An unsupported reflection.",
                    None,
                    None,
                    100 + n as i64,
                )
                .expect("belief");
        }
        assert_eq!(store.belief_count(NPC), MAX_BELIEFS);
        let record = store.record_for(NPC).expect("record");
        assert_eq!(record.next_id, (MAX_EXPERIENCES + MAX_BELIEFS + 21) as u64);
        assert!(record.check().is_ok());
        let recalled = store.recall(NPC, &[], "", 200, usize::MAX);
        assert!(
            recalled.iter().map(|item| item.notes.len()).sum::<usize>()
                <= MAX_PERSONAL_RECALL_ENTRIES
        );
    }

    #[test]
    fn encoded_document_and_rendered_recall_are_byte_bounded() {
        let mut store = PersonalMemoryStore::default();
        // Multibyte and JSON-escaped text exercise both independent ceilings.
        let text = format!("{}{}", "記憶".repeat(400), "\"".repeat(1000));
        for n in 0..90 {
            let event = observed(&mut store, ALICE, &text, n * 2);
            believed(&mut store, &text, event, None, n * 2 + 1);
        }
        let record = store.record_for(NPC).expect("record");
        assert!(encoded_bytes(record).expect("encoding") <= MAX_PERSONAL_MEMORY_BYTES);
        assert!(record.check().is_ok());
        let recalled = store.recall(NPC, &[], "", 200, usize::MAX);
        let bytes: usize = recalled
            .iter()
            .map(|item| {
                item.subject.len() + 1 + item.notes.iter().map(|note| note.len() + 1).sum::<usize>()
            })
            .sum();
        assert!(bytes <= MAX_PERSONAL_RECALL_BYTES);
        assert!(!recalled.is_empty());
        assert!(
            recalled
                .iter()
                .flat_map(|item| &item.notes)
                .all(|note| note.len() <= MAX_PERSONAL_RECALL_NOTE_BYTES)
        );
        assert!(
            recalled
                .iter()
                .flat_map(|item| &item.notes)
                .any(|note| note.contains("[excerpt]"))
        );
    }

    #[test]
    fn boot_round_trip_preserves_sources_revisions_and_next_id() {
        let mut store = PersonalMemoryStore::default();
        let first = observed(&mut store, ALICE, "Closed.", 10);
        let old = believed(&mut store, "Probably closed.", first, None, 11);
        let corrected = observed(&mut store, ALICE, "Open now.", 12);
        believed(&mut store, "Probably open.", corrected, Some(old), 13);
        let record = store.record_for(NPC).expect("record");
        let expected = record.next_id;
        let bytes = serde_json::to_vec(record).expect("serialize");
        let mut rebooted = PersonalMemoryStore::default();
        rebooted
            .insert(serde_json::from_slice(&bytes).expect("deserialize"))
            .expect("load");
        assert_eq!(
            store.recall(NPC, &[], "depot", 13, 16),
            rebooted.recall(NPC, &[], "depot", 13, 16)
        );
        assert_eq!(observed(&mut rebooted, BOB, "After reboot.", 14), expected);
        assert_eq!(mind_key(NPC), "mind:10");
    }

    #[test]
    fn malformed_documents_fail_closed() {
        let mut store = PersonalMemoryStore::default();
        let event = observed(&mut store, ALICE, "Original evidence.", 10);
        believed(&mut store, "An interpretation.", event, None, 11);
        let valid = store.record_for(NPC).expect("record").clone();
        let mut wrong_schema = valid.clone();
        wrong_schema.schema = 99;
        let mut foreign = valid.clone();
        foreign
            .beliefs
            .first_mut()
            .expect("belief")
            .evidence
            .as_mut()
            .expect("evidence")
            .npc = ObjId(999);
        let mut forged = valid.clone();
        forged
            .beliefs
            .first_mut()
            .expect("belief")
            .evidence
            .as_mut()
            .expect("evidence")
            .text = "Forged replacement evidence.".into();
        let mut duplicate = valid.clone();
        duplicate
            .experiences
            .push(duplicate.experiences.first().expect("experience").clone());
        let mut sequence = valid.clone();
        sequence.next_id = 1;
        let mut subject = valid.clone();
        subject.experiences.first_mut().expect("experience").subject = "NOT canonical".into();
        let mut empty = valid.clone();
        empty.beliefs.first_mut().expect("belief").note.clear();
        let mut future = valid.clone();
        future.beliefs.first_mut().expect("belief").created_at = 9;
        for malformed in [
            wrong_schema,
            foreign,
            forged,
            duplicate,
            sequence,
            subject,
            empty,
            future,
        ] {
            assert!(PersonalMemoryStore::default().insert(malformed).is_err());
        }
        let before = store.record_for(NPC).cloned();
        assert_eq!(
            store.observe(NPC, "bad", "", None, "", None, ExperienceKind::Outcome, 20),
            Err(PersonalMemoryError::InvalidText)
        );
        assert_eq!(store.record_for(NPC), before.as_ref());
    }

    #[test]
    fn exhausted_ids_fail_without_mutation() {
        let mut store = PersonalMemoryStore::default();
        let mut record = PersonalMemoryRecord::empty(NPC);
        record.next_id = u64::MAX;
        store
            .insert(record.clone())
            .expect("valid exhausted record");
        assert_eq!(
            store.observe(
                NPC,
                "topic",
                "A report.",
                None,
                "",
                None,
                ExperienceKind::Outcome,
                1
            ),
            Err(PersonalMemoryError::IdExhausted)
        );
        assert_eq!(store.record_for(NPC), Some(&record));
    }
}
