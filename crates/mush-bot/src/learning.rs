//! Cited, fallible learning and a separate learned-character overlay.
use crate::{
    Result, evidence,
    memory::{Event, Evidence, Record},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const POLICY: &str = "Activity and learning state is character-local and fallible. Never claim work or skill awards from timers, sent commands, lesson prose or a model's success summary. Follow the current lesson instruction and corrections; old practice remains history. Optional learning annotations are an array of lesson or growth objects with exact observed episode citations. A lesson has type=lesson, id, teacher and student ({name,kind: pc|external_bot|game_npc}), topic, phase (assess|demonstrate|practice|feedback|apply), revision, content and evidence. Start with assess at revision 1, then demonstrate, practice, feedback, and apply in another observed place. Feedback after practice may increment the revision to correct the technique; practice again before application. Cite the acting participant's complete public speech or pose, not your sent command. Keep identities as observed labels unless known independently. Use normal say/pose commands for teaching and practice, observe them, then annotate. These records never modify a PC or game NPC's hidden state. Optional growth annotations have type=growth, subject ({name,kind}), key, value (integer -100..100), rationale and evidence. They propose bounded confidence/opinion/ambition tendencies, not new biography. The runner's development policy controls whether they apply; never treat a pending suggestion as an applied change.";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Pc,
    ExternalBot,
    GameNpc,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Participant {
    pub name: String,
    pub kind: Kind,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Assess,
    Demonstrate,
    Practice,
    Feedback,
    Apply,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Lesson {
    pub id: String,
    pub teacher: Participant,
    pub student: Participant,
    pub topic: String,
    pub phase: Phase,
    pub revision: u32,
    pub content: String,
    pub evidence: Vec<Evidence>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Growth {
    pub subject: Participant,
    pub key: String,
    pub value: i16,
    pub rationale: String,
    pub evidence: Vec<Evidence>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Frozen,
    Suggest,
    Automatic,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    pub version: u32,
    pub mode: Mode,
    pub allowed_keys: Vec<String>,
    pub max_changes: usize,
    pub max_delta: i16,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            version: 1,
            mode: Mode::Frozen,
            allowed_keys: vec![],
            max_changes: 8,
            max_delta: 10,
        }
    }
}
impl Policy {
    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || self.allowed_keys.len() > 32
            || self.max_changes > 64
            || !(1..=20).contains(&self.max_delta)
        {
            return Err("invalid development policy bounds/version".into());
        }
        for key in &self.allowed_keys {
            validate_key(key)?;
        }
        Ok(())
    }
}
fn validate_key(key: &str) -> Result<()> {
    evidence::bounded(key, 100)?;
    if !["confidence.", "opinion.", "ambition."]
        .iter()
        .any(|p| key.starts_with(p) && key.len() > p.len())
    {
        return Err("development keys use confidence.*, opinion.* or ambition.*".into());
    }
    Ok(())
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Update {
    Lesson(Lesson),
    Growth(Growth),
    Policy(Policy),
}
impl Update {
    pub fn evidence(&self) -> &[Evidence] {
        match self {
            Self::Lesson(l) => &l.evidence,
            Self::Growth(g) => &g.evidence,
            Self::Policy(_) => &[],
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub version: u32,
    pub update: Update,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LessonRecord {
    pub history: Vec<Lesson>,
    pub journal_ids: Vec<u64>,
    pub current_instruction: String,
    pub revision: u32,
    pub practice_place: Option<String>,
    pub application_places: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub journal_id: u64,
    pub change: Growth,
    pub status: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub version: u32,
    pub lessons: BTreeMap<String, LessonRecord>,
    pub policy: Policy,
    pub proposals: Vec<Proposal>,
    pub overlay: BTreeMap<String, i16>,
    pub applied_changes: usize,
}
impl Default for State {
    fn default() -> Self {
        Self {
            version: 1,
            lessons: BTreeMap::new(),
            policy: Policy::default(),
            proposals: vec![],
            overlay: BTreeMap::new(),
            applied_changes: 0,
        }
    }
}
impl State {
    pub fn project(records: &[Record], observer: &str) -> Result<Self> {
        let mut state = Self::default();
        for r in records {
            if let Event::Learning(entry) = &r.event {
                state.apply(entry, records, r.id, observer)?;
            }
        }
        Ok(state)
    }
    pub fn apply(
        &mut self,
        entry: &Entry,
        records: &[Record],
        id: u64,
        observer: &str,
    ) -> Result<()> {
        if entry.version != 1 {
            return Err("unsupported learning contribution version".into());
        }
        match &entry.update {
            Update::Policy(policy) => {
                policy.validate()?;
                self.policy = policy.clone();
            }
            Update::Lesson(lesson) => {
                evidence::bounded(&lesson.id, 100)?;
                evidence::bounded(&lesson.topic, 200)?;
                evidence::bounded(&lesson.content, 2000)?;
                for p in [&lesson.teacher, &lesson.student] {
                    evidence::bounded(&p.name, 200)?;
                }
                if lesson
                    .teacher
                    .name
                    .eq_ignore_ascii_case(&lesson.student.name)
                    || lesson.revision == 0
                    || lesson.revision > 10000
                {
                    return Err("lesson needs distinct participants and a bounded revision".into());
                }
                citations(&lesson.evidence, records, id)?;
                let actor = if matches!(lesson.phase, Phase::Practice | Phase::Apply) {
                    &lesson.student
                } else {
                    &lesson.teacher
                };
                let attributed = lesson.evidence.iter().any(|cite| {
                    cite.quote.lines().any(|line| {
                        crate::memory::speech(line)
                            .is_some_and(|(label, _)| label.eq_ignore_ascii_case(&actor.name))
                            || line.starts_with(&format!("{} ", actor.name))
                                && crate::memory::speech(line).is_none()
                    })
                });
                if !attributed {
                    return Err(
                        "lesson needs observed speech/pose attributed to the acting participant"
                            .into(),
                    );
                }
                let mut next = self.lessons.get(&lesson.id).cloned().unwrap_or_default();
                if next.history.len() >= 128
                    || (!self.lessons.contains_key(&lesson.id) && self.lessons.len() >= 128)
                {
                    return Err("lesson history capacity reached".into());
                }
                if let Some(first) = next.history.first() {
                    if first.teacher != lesson.teacher
                        || first.student != lesson.student
                        || first.topic != lesson.topic
                    {
                        return Err("lesson identity/participants cannot change".into());
                    }
                } else if lesson.phase != Phase::Assess || lesson.revision != 1 {
                    return Err("a lesson starts with assessment at revision 1".into());
                }
                let latest = next.history.last();
                if let Some(previous) = latest {
                    let correction = lesson.phase == Phase::Feedback
                        && lesson.revision == next.revision + 1
                        && previous.phase == Phase::Practice;
                    if !correction && lesson.revision != next.revision {
                        return Err(
                            "only feedback after practice can advance the lesson revision".into(),
                        );
                    }
                    let allowed = match lesson.phase {
                        Phase::Assess => false,
                        Phase::Demonstrate => {
                            matches!(previous.phase, Phase::Assess | Phase::Feedback)
                        }
                        Phase::Practice => matches!(
                            previous.phase,
                            Phase::Demonstrate | Phase::Feedback | Phase::Practice | Phase::Apply
                        ),
                        Phase::Feedback => previous.phase == Phase::Practice,
                        Phase::Apply => {
                            previous.phase == Phase::Feedback
                                && previous.revision == lesson.revision
                                && next.history.iter().any(|s| {
                                    s.phase == Phase::Practice && s.revision == lesson.revision
                                })
                        }
                    };
                    if !allowed {
                        return Err("lesson phase requires assessment, demonstration, practice and feedback before transfer".into());
                    }
                    let last_evidence = previous
                        .evidence
                        .iter()
                        .map(|e| {
                            (
                                e.episode,
                                evidence::cite(records, e, id)
                                    .unwrap()
                                    .text
                                    .find(&e.quote)
                                    .unwrap()
                                    + e.quote.len(),
                            )
                        })
                        .max()
                        .unwrap_or((0, 0));
                    if lesson.evidence.iter().any(|e| {
                        (
                            e.episode,
                            evidence::cite(records, e, id)
                                .unwrap()
                                .text
                                .find(&e.quote)
                                .unwrap(),
                        ) < last_evidence
                    }) {
                        return Err(
                            "lesson phase needs new observations after its predecessor".into()
                        );
                    }
                }
                if matches!(lesson.phase, Phase::Practice | Phase::Apply) {
                    let place = lesson
                        .evidence
                        .iter()
                        .filter_map(|e| {
                            evidence::cite(records, e, id)
                                .ok()?
                                .place
                                .as_ref()
                                .map(|p| p.dbref.clone())
                        })
                        .next();
                    if lesson.phase == Phase::Apply {
                        let place = place
                            .filter(|p| Some(p) != next.practice_place.as_ref())
                            .ok_or("transfer requires a different observed place")?;
                        if next.practice_place.is_none() {
                            return Err("practice location was not established".into());
                        }
                        next.application_places.push(place);
                    } else {
                        next.practice_place = place;
                    }
                }
                if lesson.phase == Phase::Demonstrate
                    || (lesson.phase == Phase::Feedback && lesson.revision > next.revision)
                {
                    next.current_instruction = lesson.content.clone();
                }
                next.revision = lesson.revision;
                next.history.push(lesson.clone());
                next.journal_ids.push(id);
                self.lessons.insert(lesson.id.clone(), next);
            }
            Update::Growth(growth) => {
                validate_key(&growth.key)?;
                evidence::bounded(&growth.subject.name, 200)?;
                evidence::bounded(&growth.rationale, 2000)?;
                citations(&growth.evidence, records, id)?;
                if !(-100..=100).contains(&growth.value) || self.proposals.len() >= 512 {
                    return Err("development bounds/capacity exceeded".into());
                }
                let observer = observer.split_once('+').map_or(observer, |(_, name)| name);
                let own = growth.subject.kind == Kind::ExternalBot
                    && growth.subject.name.eq_ignore_ascii_case(observer);
                let previous = self.overlay.get(&growth.key).copied().unwrap_or(0);
                let status = if self.policy.mode == Mode::Frozen {
                    "frozen"
                } else if own
                    && self.policy.mode == Mode::Automatic
                    && self.policy.allowed_keys.contains(&growth.key)
                    && self.applied_changes < self.policy.max_changes
                    && (growth.value - previous).abs() <= self.policy.max_delta
                {
                    self.overlay.insert(growth.key.clone(), growth.value);
                    self.applied_changes += 1;
                    "applied"
                } else {
                    "suggested"
                };
                self.proposals.push(Proposal {
                    journal_id: id,
                    change: growth.clone(),
                    status: status.into(),
                });
            }
        }
        Ok(())
    }
    pub fn context(&self, query: &str, limit: usize) -> Value {
        let terms: Vec<_> = query.split_whitespace().map(str::to_lowercase).collect();
        let mut selected = Vec::new();
        let mut used = 0;
        let mut candidates: Vec<_> = self.lessons.iter().collect();
        candidates.sort_by_key(|(id, lesson)| {
            let search = format!("{id} {}", lesson.history[0].topic).to_lowercase();
            std::cmp::Reverse((
                terms.iter().filter(|s| search.contains(s.as_str())).count(),
                lesson.journal_ids.last().copied().unwrap_or(0),
            ))
        });
        for (index, (id, lesson)) in candidates.into_iter().enumerate() {
            let first = &lesson.history[0];
            let search = format!(
                "{id} {} {} {}",
                first.topic, first.teacher.name, first.student.name
            )
            .to_lowercase();
            if index >= 4 && !terms.is_empty() && !terms.iter().any(|s| search.contains(s)) {
                continue;
            }
            let value = json!({"id":id,"teacher":first.teacher,"student":first.student,"topic":first.topic,
                "revision":lesson.revision,"current_instruction":lesson.current_instruction,"latest":lesson.history.last(),
                "source_records":lesson.journal_ids,"practice_place":lesson.practice_place,"application_places":lesson.application_places,
                "status":"fallible learning record, not a game skill award"});
            let size = value.to_string().len();
            if used + size > limit {
                continue;
            }
            used += size;
            selected.push(value);
        }
        json!({"lessons":selected,"total_lessons":self.lessons.len(),"learned_tendencies":self.overlay,
            "policy":self.policy,"pending_proposals":self.proposals.iter().filter(|p|p.status=="suggested").count(),
            "boundary":"PC/game-NPC state is not modified; temporary mood and authored profile are separate"})
    }
}
fn citations(evidence: &[Evidence], records: &[Record], before: u64) -> Result<()> {
    if evidence.is_empty() || evidence.len() > 8 {
        return Err("learning requires 1..8 observed citations".into());
    }
    for cite in evidence {
        evidence::cite(records, cite, before)?;
    }
    Ok(())
}
