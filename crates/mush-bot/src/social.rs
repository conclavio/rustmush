//! Actor-local social interpretations and explicit, observed agreements.
//! No engine state, peer memory, or extra model requests enter this projection.
use crate::{
    Result,
    memory::{Basis, Confidence, Event, Evidence, Record, Source},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const POLICY: &str = "social_context is this observer's fallible social memory. Background is authored prior knowledge; relationships are directed assessments, not universal scores. Claims retain attribution and uncertainty. Terminal handles do not establish IC names. Other actors' private memories are unavailable. Use recall to search your own background and history. Missing records mean unestablished, not false.\n\
Cooperate through observed game speech/pages. To create an explicit assignment say exactly: I propose [unique-id] for Handle,OtherHandle: task | condition: condition | due: observable deadline\n\
Use 'none' for an unconditional task or unspecified deadline. Participants respond with a complete utterance: I accept [unique-id]. or I decline [unique-id]. An accepted participant may say I withdraw [unique-id]. The proposer may cancel an unaccepted offer with I cancel [unique-id]. Use a new ID for new terms and optionally end the proposal with ' | replaces: old-id'; this invites renegotiation without cancelling the old obligation. To report completion say I completed [unique-id]: explanation. A report is not verified fulfillment. IDs are immutable within this character's journal; do not reuse them. Informal discussion is welcome but does not itself establish a tracked promise.\n\
accepted_assignments are your own agreed subgoals, subordinate to the runner's goal, role, permissions and instructions. They never change your runner mandate. You can refuse, withdraw or negotiate. Never claim an absent member agreed. Private agendas and disclosure choices stay private unless you choose to communicate them in game.\n\
Optional social annotations are an array (max 8) of objects. Every annotation needs evidence: 1..8 {episode: integer, quote: exact excerpt} objects from local observed or prior episodes, never sent command attempts. Common fields: basis (observed, reported, inferred, prior_knowledge), confidence (tentative, likely, strong).\n\
Relationship: {type: relationship, subject: uncertain referent or terminal handle, dimension: trust|respect|affection|resentment|dependence, assessment: text, basis, confidence, evidence, supersedes: optional prior relationship journal ID}.\n\
Claim: {type: claim, subject, predicate, value, attribution: who claims this or source uncertainty, stance: unverified|believed|doubted, basis, confidence, evidence, supersedes: optional array of corrected claim journal IDs}. A correction supersedes only the same subject/predicate; preserve unresolved contradictions. A speech claim is reported or inferred, not direct observed truth.\n\
Outcome: {type: outcome, proposal: ID, participant: terminal handle, status: fulfilled|missed|uncertain, explanation, basis, confidence, evidence}. This is your fallible assessment of an accepted obligation, not engine certification. Poses and claims cannot verify physical transfers or other mechanical outcomes. Include evidence after acceptance; speech alone cannot certify fulfilled. Do not annotate proposal/acceptance events: these come only from observed explicit utterances. Invalid annotations receive feedback without cancelling a valid game command.";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Fact {
    pub id: String,
    pub revision: u32,
    pub subject: String,
    pub predicate: String,
    pub value: String,
    pub source: String,
    pub confidence: Confidence,
}
impl Fact {
    pub fn validate(&self) -> Result<()> {
        if !crate::organization::identifier(&self.id) || self.revision == 0 {
            return Err("background requires an identifier and positive revision".into());
        }
        for s in [&self.subject, &self.predicate, &self.value, &self.source] {
            bounded(s, 2000)?;
        }
        if self.prior_text(&"g".repeat(64)).len() > 4000 {
            return Err("background fact exceeds the quoted evidence limit".into());
        }
        Ok(())
    }
    pub fn prior_text(&self, group: &str) -> String {
        format!(
            "Authored background [{group}/{}@{}]: {} / {} / {}. Source: {}. Confidence: {:?}.",
            self.id,
            self.revision,
            self.subject,
            self.predicate,
            self.value,
            self.source,
            self.confidence
        )
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub fact: Fact,
    pub known_by: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Dimension {
    Trust,
    Respect,
    Affection,
    Resentment,
    Dependence,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Stance {
    Unverified,
    Believed,
    Doubted,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Fulfilled,
    Missed,
    Uncertain,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Update {
    Background {
        group: String,
        fact: Fact,
        evidence: Vec<Evidence>,
    },
    Relationship {
        subject: String,
        dimension: Dimension,
        assessment: String,
        basis: Basis,
        confidence: Confidence,
        evidence: Vec<Evidence>,
        #[serde(default)]
        supersedes: Option<u64>,
    },
    Claim {
        subject: String,
        predicate: String,
        value: String,
        attribution: String,
        stance: Stance,
        basis: Basis,
        confidence: Confidence,
        evidence: Vec<Evidence>,
        #[serde(default)]
        supersedes: Vec<u64>,
    },
    Outcome {
        proposal: String,
        participant: String,
        status: Outcome,
        explanation: String,
        basis: Basis,
        confidence: Confidence,
        evidence: Vec<Evidence>,
    },
}
impl Update {
    pub fn evidence(&self) -> &[Evidence] {
        match self {
            Self::Background { evidence, .. }
            | Self::Relationship { evidence, .. }
            | Self::Claim { evidence, .. }
            | Self::Outcome { evidence, .. } => evidence,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub version: u32,
    pub scope: crate::conversation::Scope,
    pub update: Update,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Interpretation {
    pub journal_id: u64,
    pub session: u64,
    pub at_ms: u64,
    pub active: bool,
    pub entry: Entry,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Response {
    Pending,
    Accepted,
    Declined,
    Withdrawn,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Participant {
    pub response: Response,
    pub completion_report: Option<String>,
    pub assessment: Option<u64>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    pub episode: u64,
    pub session: u64,
    pub actor: String,
    pub kind: String,
    pub quote: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Commitment {
    pub id: String,
    pub proposer: String,
    pub participants: BTreeMap<String, Participant>,
    pub task: String,
    pub condition: String,
    pub due: String,
    pub replaces: Option<String>,
    pub cancelled: bool,
    pub history: Vec<Transition>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    pub version: u32,
    pub observer: String,
    pub interpretations: Vec<Interpretation>,
    pub commitments: BTreeMap<String, Commitment>,
    pub protocol_gaps: Vec<Value>,
    pub protocol_gaps_omitted: usize,
}

fn bounded(s: &str, n: usize) -> Result<()> {
    if s.trim().is_empty() || s.len() > n || s.chars().any(char::is_control) {
        return Err("social text must be nonempty, bounded and without control characters".into());
    }
    Ok(())
}

/// Group the line renderer's hanging continuations inside one observed episode.
/// Never stitch unrelated episodes or turn an indented example into a speaker.
fn transcript_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::<String>::new();
    for line in text.lines() {
        if line.trim_start() != line
            && !line.trim().is_empty()
            && let Some(previous) = blocks.last_mut()
            && let Some((_, body)) =
                crate::memory::speech(previous.lines().next().unwrap_or_default())
            && (!body.trim().starts_with('"') || !previous.trim_end().ends_with('"'))
        {
            previous.push('\n');
            previous.push_str(line);
        } else {
            blocks.push(line.into());
        }
    }
    blocks
}

/// Complete line-mode speech blocks. Evidence retains original wrapped text.
fn utterance(line: &str, observer: &str) -> Option<(String, String)> {
    if line.trim_start() != line || line.starts_with("<OOC>") || line.starts_with('[') {
        return None;
    }
    let normalized = line
        .lines()
        .map(str::trim_start)
        .collect::<Vec<_>>()
        .join(" ");
    let (who, body) = crate::memory::speech(&normalized)?;
    let body = body.trim();
    // Page framing is unquoted. Quotes there belong to the speaker's content,
    // so a quoted example must not be promoted into that speaker's acceptance.
    if normalized
        .split_once(" pages: ")
        .is_some_and(|(label, _)| label == who)
        && body.contains('"')
    {
        return None;
    }
    let body = if body.starts_with('"') {
        body.strip_prefix('"')?.strip_suffix('"')?
    } else {
        body
    };
    if body.contains('"') || body.contains('\n') {
        return None;
    }
    let who = if who == "You" { observer } else { who };
    Some((who.to_string(), body.to_string()))
}

fn evidence<'a>(
    records: &'a [Record],
    before: u64,
    update: &Update,
) -> Result<Vec<&'a crate::memory::Episode>> {
    let basis = match update {
        Update::Background { .. } => &Basis::PriorKnowledge,
        Update::Relationship { basis, .. }
        | Update::Claim { basis, .. }
        | Update::Outcome { basis, .. } => basis,
    };
    if update.evidence().is_empty() || update.evidence().len() > 8 {
        return Err("social update requires 1..8 evidence quotes".into());
    }
    let mut episodes = Vec::new();
    for e in update.evidence() {
        let record = e
            .episode
            .checked_sub(1)
            .and_then(|i| records.get(i as usize))
            .filter(|r| r.id < before);
        let Some(Record {
            event: Event::Episode(ep),
            ..
        }) = record
        else {
            return Err("social evidence must cite an earlier local episode".into());
        };
        if e.quote.trim().is_empty()
            || e.quote.len() > 4000
            || !ep.text.contains(&e.quote)
            || ep.source == Source::Sent
        {
            return Err(
                "social evidence must match a local observation or prior, never a command attempt"
                    .into(),
            );
        }
        if (*basis == Basis::PriorKnowledge && ep.source != Source::PriorKnowledge)
            || (*basis == Basis::Observed && ep.source != Source::Observed)
        {
            return Err("social evidence source does not match its basis".into());
        }
        episodes.push(ep);
    }
    Ok(episodes)
}

impl Ledger {
    pub fn project(records: &[Record], observer: &str) -> Result<Self> {
        let mut state = Self::empty(observer)?;
        state.extend(records, 0)?;
        Ok(state)
    }
    pub(crate) fn empty(observer: &str) -> Result<Self> {
        let observer = observer.split_once('+').map_or(observer, |(_, name)| name);
        bounded(observer, 200)?;
        Ok(Self {
            version: 1,
            observer: observer.into(),
            interpretations: vec![],
            commitments: BTreeMap::new(),
            protocol_gaps: vec![],
            protocol_gaps_omitted: 0,
        })
    }
    pub(crate) fn extend(&mut self, records: &[Record], start: usize) -> Result<()> {
        for record in &records[start..] {
            match &record.event {
                Event::Episode(ep) if ep.source == Source::Observed => {
                    for line in transcript_blocks(&ep.text) {
                        if let Some((who, body)) = utterance(&line, &self.observer)
                            && let Err(error) = self.protocol(&who, &body, &line, record)
                        {
                            if self.protocol_gaps.len() < 128 {
                                self.protocol_gaps.push(json!({"episode":record.id,"quote":line,"reason":error.to_string()}));
                            } else {
                                self.protocol_gaps_omitted += 1;
                            }
                        }
                    }
                }
                Event::Social(entry) => {
                    self.validate_update(records, record.id, entry, true)?;
                    self.apply_update(record, entry.clone());
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub fn validate_update(
        &self,
        records: &[Record],
        before: u64,
        entry: &Entry,
        authored: bool,
    ) -> Result<()> {
        if entry.version != 1 || self.interpretations.len() >= 4096 {
            return Err("unsupported social event version or 4096 interpretation limit".into());
        }
        entry.scope.validate()?;
        let episodes = evidence(records, before, &entry.update)?;
        let find = |id: u64| {
            self.interpretations
                .iter()
                .find(|i| i.journal_id == id && i.active)
        };
        match &entry.update {
            Update::Background {
                group,
                fact,
                evidence,
            } => {
                if !authored {
                    return Err(
                        "background grants are runner-authored, never model annotations".into(),
                    );
                }
                fact.validate()?;
                if !crate::organization::identifier(group) || evidence.len() != 1 || evidence[0].quote != fact.prior_text(group)
                    || self.interpretations.iter().any(|i| matches!(&i.entry.update, Update::Background{group:g,fact:f,..} if g==group && f.id==fact.id && f.revision==fact.revision)) {
                    return Err("invalid or duplicate background revision/evidence".into());
                }
            }
            Update::Relationship {
                subject,
                dimension,
                assessment,
                supersedes,
                ..
            } => {
                bounded(subject, 200)?;
                bounded(assessment, 2000)?;
                if let Some(id) = supersedes && !find(*id).is_some_and(|i| matches!(&i.entry.update, Update::Relationship{subject:s,dimension:d,..} if s==subject && d==dimension)) {
                    return Err("relationship correction must reference an active assessment of the same referent/dimension".into());
                }
            }
            Update::Claim {
                subject,
                predicate,
                value,
                attribution,
                basis,
                supersedes,
                ..
            } => {
                for s in [subject, predicate, value, attribution] {
                    bounded(s, 2000)?;
                }
                if supersedes.len()>8 || supersedes.iter().collect::<BTreeSet<_>>().len()!=supersedes.len() || supersedes.iter().any(|id| !find(*id).is_some_and(|i| matches!(&i.entry.update, Update::Claim{subject:s,predicate:p,..} if s==subject && p==predicate))) {
                    return Err("claim correction must cite active claims about the same subject/predicate".into());
                }
                if *basis == Basis::Observed && !non_speech_quote(&entry.update, &episodes) {
                    return Err("speech claims are reported or inferred, not observed truth".into());
                }
            }
            Update::Outcome {
                proposal,
                participant,
                status,
                explanation,
                basis,
                ..
            } => {
                bounded(explanation, 2000)?;
                if self
                    .commitments
                    .get(proposal)
                    .and_then(|c| c.participants.get(participant))
                    .is_none_or(|p| p.response != Response::Accepted)
                {
                    return Err("outcome assessment requires this participant's observed accepted obligation".into());
                }
                let accepted = self.commitments[proposal]
                    .history
                    .iter()
                    .find(|h| h.actor == *participant && h.kind == "accept")
                    .unwrap();
                if !entry
                    .update
                    .evidence()
                    .iter()
                    .zip(&episodes)
                    .any(|(e, ep)| {
                        e.episode > accepted.episode
                            || (e.episode == accepted.episode
                                && ep
                                    .text
                                    .rfind(&e.quote)
                                    .zip(ep.text.find(&accepted.quote))
                                    .is_some_and(|(e, a)| e >= a + accepted.quote.len()))
                    })
                {
                    return Err("outcome needs evidence after the observed acceptance, not an earlier accomplishment".into());
                }
                if *status == Outcome::Fulfilled
                    && (*basis != Basis::Observed || !non_speech_quote(&entry.update, &episodes))
                {
                    return Err("fulfilled assessment requires non-speech observed evidence; completion speech is only a report".into());
                }
            }
        }
        Ok(())
    }

    fn apply_update(&mut self, record: &Record, entry: Entry) {
        let retired = match &entry.update {
            Update::Relationship { supersedes, .. } => {
                supersedes.iter().copied().collect::<Vec<_>>()
            }
            Update::Claim { supersedes, .. } => supersedes.clone(),
            Update::Outcome {
                proposal,
                participant,
                ..
            } => self
                .commitments
                .get_mut(proposal)
                .unwrap()
                .participants
                .get_mut(participant)
                .unwrap()
                .assessment
                .replace(record.id)
                .into_iter()
                .collect(),
            _ => vec![],
        };
        for i in &mut self.interpretations {
            if retired.contains(&i.journal_id) {
                i.active = false;
            }
        }
        self.interpretations.push(Interpretation {
            journal_id: record.id,
            session: record.session,
            at_ms: record.at_ms,
            active: true,
            entry,
        });
    }

    fn protocol(&mut self, who: &str, body: &str, quote: &str, record: &Record) -> Result<()> {
        let transition = |kind: &str| Transition {
            episode: record.id,
            session: record.session,
            actor: who.into(),
            kind: kind.into(),
            quote: quote.into(),
        };
        if let Some(rest) = body.strip_prefix("I propose [") {
            let (id, rest) = rest.split_once("] for ").ok_or("incomplete proposal")?;
            let (people, rest) = rest
                .split_once(": ")
                .ok_or("proposal needs participants and task")?;
            let (task, rest) = rest
                .split_once(" | condition: ")
                .ok_or("proposal needs condition")?;
            let (condition, rest) = rest
                .split_once(" | due: ")
                .ok_or("proposal needs due trigger")?;
            let (due, replaces) = rest
                .split_once(" | replaces: ")
                .map_or((rest, None), |(a, b)| (a, Some(b)));
            if !crate::organization::identifier(id)
                || self.commitments.contains_key(id)
                || self.commitments.len() >= 1024
            {
                return Err(
                    "proposal ID is invalid, already used, or the 1024 proposal limit was reached"
                        .into(),
                );
            }
            for s in [task, condition, due] {
                bounded(s, 1000)?;
            }
            if let Some(old) = replaces
                && !self.commitments.contains_key(old)
            {
                return Err("counterproposal references an unobserved offer".into());
            }
            let names = people.split(',').map(str::trim).collect::<Vec<_>>();
            if names.is_empty()
                || names.len() > 16
                || names.iter().collect::<BTreeSet<_>>().len() != names.len()
            {
                return Err("proposal needs 1..16 distinct terminal handles".into());
            }
            let mut participants = BTreeMap::new();
            for name in names {
                bounded(name, 200)?;
                participants.insert(
                    name.into(),
                    Participant {
                        response: Response::Pending,
                        completion_report: None,
                        assessment: None,
                    },
                );
            }
            self.commitments.insert(
                id.into(),
                Commitment {
                    id: id.into(),
                    proposer: who.into(),
                    participants,
                    task: task.into(),
                    condition: condition.into(),
                    due: due.into(),
                    replaces: replaces.map(str::to_string),
                    cancelled: false,
                    history: vec![transition("proposed")],
                },
            );
            return Ok(());
        }
        let mut event = None;
        for verb in ["accept", "decline", "withdraw", "cancel"] {
            if let Some(rest) = body.strip_prefix(&format!("I {verb} [")) {
                event = Some((
                    verb,
                    rest.strip_suffix("].")
                        .ok_or("response must be a complete explicit utterance ending ].")?,
                    None,
                ));
                break;
            }
        }
        if let Some(rest) = body.strip_prefix("I completed [") {
            let (id, explanation) = rest
                .split_once("]: ")
                .ok_or("completion report needs explanation")?;
            bounded(explanation, 1000)?;
            event = Some(("reported_complete", id, Some(explanation)));
        }
        let Some((kind, id, explanation)) = event else {
            return Ok(());
        };
        let c = self
            .commitments
            .get_mut(id)
            .ok_or("response references an unobserved proposal")?;
        if c.cancelled || c.history.len() >= 128 {
            return Err("proposal cancelled or transition history limit reached".into());
        }
        if kind == "cancel" {
            if c.proposer != who
                || c.participants
                    .values()
                    .any(|p| p.response == Response::Accepted)
            {
                return Err(
                    "only proposer may cancel an offer with no accepted obligations".into(),
                );
            }
            c.cancelled = true;
        } else {
            let p = c
                .participants
                .get_mut(who)
                .ok_or("speaker is not a proposed participant")?;
            let next=match (kind,&p.response) {
                ("accept",Response::Pending)=>Response::Accepted,
                ("decline",Response::Pending)=>Response::Declined,
                ("withdraw",Response::Accepted)=>Response::Withdrawn,
                ("reported_complete",Response::Accepted)=>Response::Accepted,
                _=>return Err("response does not follow the observed obligation state; negotiate a new ID for changed terms".into()),
            };
            p.response = next;
            if let Some(explanation) = explanation {
                p.completion_report = Some(explanation.into());
            }
        }
        c.history.push(transition(kind));
        Ok(())
    }

    /// Retrieval of granted/observed state only. Hidden catalogs never reach here.
    pub fn context(&self, query: &str, limit: usize) -> Value {
        let terms = query
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| t.len() > 2)
            .map(str::to_string)
            .collect::<BTreeSet<_>>();
        let mut rows = self
            .interpretations
            .iter()
            .filter(|i| i.active)
            .map(|i| {
                let text = serde_json::to_string(&i.entry.update)
                    .unwrap()
                    .to_lowercase();
                let score = terms.iter().filter(|t| text.contains(t.as_str())).count();
                (score, i)
            })
            .collect::<Vec<_>>();
        rows.sort_by_key(|(score, i)| (std::cmp::Reverse(*score), std::cmp::Reverse(i.journal_id)));
        let mut selected = vec![];
        let mut agreements = vec![];
        let mut proposals = vec![];
        let mut used = 0;
        // Agreed personal work is relevant even when its words differ from the goal.
        for c in self.commitments.values().filter(|c| {
            !c.cancelled
                && c.participants.get(&self.observer).is_some_and(|p| {
                    p.response == Response::Accepted
                        && p.assessment.is_none_or(|id| {
                            !self.interpretations.iter().any(|i| {
                                i.journal_id == id
                                    && matches!(
                                        &i.entry.update,
                                        Update::Outcome {
                                            status: Outcome::Fulfilled,
                                            ..
                                        }
                                    )
                            })
                        })
                })
        }) {
            let size = serde_json::to_vec(c).unwrap().len();
            if used + size <= limit && agreements.len() < 16 {
                used += size;
                agreements.push(c);
            }
        }
        for c in self
            .commitments
            .values()
            .rev()
            .filter(|c| !agreements.iter().any(|a| a.id == c.id))
        {
            let size = serde_json::to_vec(c).unwrap().len();
            if used + size <= limit && proposals.len() < 16 {
                used += size;
                proposals.push(c);
            }
        }
        for (score, i) in rows {
            if score == 0 && matches!(i.entry.update, Update::Background { .. }) {
                continue;
            }
            let size = serde_json::to_vec(i).unwrap().len();
            if used + size <= limit && selected.len() < 64 {
                used += size;
                selected.push(i);
            }
        }
        json!({"version":1,"observer":self.observer,"mode":"in_world_negotiation","interpretations":selected,
            "accepted_assignments":agreements,"record_bytes":used,"byte_limit":limit,
            "observed_proposals":proposals,"omitted_interpretations":self.interpretations.len().saturating_sub(selected.len()),
            "omitted_proposals":self.commitments.len().saturating_sub(agreements.len()+proposals.len()),
            "total_interpretations":self.interpretations.len(),"total_proposals":self.commitments.len(),
            "coverage":"Selected actor-local knowledge only; missing is unestablished. Assignments remain subordinate to the runner mandate. Terminal handles do not establish known IC names."})
    }
}

/// Operator comparison only. This aggregate is never used by the decision loop.
pub fn compare(views: &BTreeMap<String, Ledger>) -> Value {
    let mut keys = BTreeSet::new();
    for ledger in views.values() {
        for i in &ledger.interpretations {
            if let Update::Claim {
                subject, predicate, ..
            } = &i.entry.update
            {
                keys.insert((subject.clone(), predicate.clone()));
            }
        }
    }
    let claims=keys.into_iter().map(|(subject,predicate)| {
        let perspectives=views.iter().map(|(actor,ledger)| {
            let records=ledger.interpretations.iter().filter(|i| matches!(&i.entry.update,Update::Claim{subject:s,predicate:p,..} if s==&subject && p==&predicate)).collect::<Vec<_>>();
            (actor.clone(),json!({"coverage":if records.is_empty(){"unestablished"}else{"local_interpretations"},"records":records}))
        }).collect::<BTreeMap<_,_>>();
        json!({"subject":subject,"predicate":predicate,"perspectives":perspectives})
    }).collect::<Vec<_>>();
    let ids = views
        .values()
        .flat_map(|v| v.commitments.keys().cloned())
        .collect::<BTreeSet<_>>();
    let commitments=ids.into_iter().map(|id| {
        let perspectives=views.iter().map(|(actor,v)| (actor.clone(),json!({"coverage":if v.commitments.contains_key(&id){"observed_history"}else{"unestablished"},"commitment":v.commitments.get(&id)}))).collect::<BTreeMap<_,_>>();
        json!({"id":id,"perspectives":perspectives})
    }).collect::<Vec<_>>();
    json!({"version":1,"audience":"operator_only","mode":"in_world_negotiation","claims":claims,"commitments":commitments,
        "interpretation":"Separate local evidence, not a shared truth or shared character memory. An absent claim may still be present in unannotated speech; absence never proves ignorance. An interface handle is not a known IC name."})
}

fn non_speech_quote(update: &Update, episodes: &[&crate::memory::Episode]) -> bool {
    update.evidence().iter().zip(episodes).any(|(e, ep)| {
        ep.source == Source::Observed
            && transcript_blocks(&ep.text)
                .into_iter()
                .filter(|l| crate::memory::speech(l.lines().next().unwrap_or_default()).is_none())
                .collect::<Vec<_>>()
                .join("\n")
                .contains(&e.quote)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{Memory, Ssh},
        memory::Book,
    };
    fn book() -> Book {
        Book::open(
            &Memory {
                persistent: false,
                ..Default::default()
            },
            &Ssh::default(),
        )
        .unwrap()
    }
    fn hear(b: &mut Book, text: &str) -> u64 {
        b.observe(text).unwrap();
        b.flush().unwrap();
        b.stats()["records"].as_u64().unwrap()
    }
    fn annotate(b: &mut Book, v: Value) -> Vec<String> {
        b.annotate_social(Some(json!([v])), "Bass", &Default::default())
            .unwrap()
    }
    fn relationship(ep: u64, quote: &str, dimension: &str) -> Value {
        json!({"type":"relationship","subject":"Guitar","dimension":dimension,"assessment":"Respect the skill, resent interruptions","basis":"inferred","confidence":"likely","evidence":[{"episode":ep,"quote":quote}]})
    }
    fn claim(ep: u64, value: &str, quote: &str) -> Value {
        json!({"type":"claim","subject":"gig","predicate":"cancelled","value":value,"attribution":"Lead's account","stance":"unverified","basis":"reported","confidence":"tentative","evidence":[{"episode":ep,"quote":quote}]})
    }
    #[test]
    fn previous_accomplishments_cannot_fulfill_new_obligations() {
        let mut b = book();
        let old = hear(&mut b, "Delivered yesterday's chart.");
        let same = hear(
            &mut b,
            "Delivered this morning's chart.\nLead says, \"I propose [new] for Bass: deliver a new chart | condition: none | due: tonight\"\nBass says, \"I accept [new].\"",
        );
        for (episode, quote) in [
            (old, "Delivered yesterday's chart."),
            (same, "Delivered this morning's chart."),
        ] {
            assert!(!annotate(&mut b,json!({"type":"outcome","proposal":"new","participant":"Bass","status":"fulfilled","explanation":"Already delivered","basis":"observed","confidence":"strong","evidence":[{"episode":episode,"quote":quote}]})).is_empty());
        }
        assert!(
            b.social("Bass").unwrap().commitments["new"].participants["Bass"]
                .assessment
                .is_none()
        );
    }
    #[test]
    fn incremental_view_and_fresh_projection_agree_after_new_events_and_restore() {
        let mut b = book();
        for n in 0..5 {
            let ep = hear(&mut b, &format!("Guitar plays solo {n}."));
            b.social("Bass").unwrap();
            assert!(
                annotate(
                    &mut b,
                    relationship(ep, &format!("plays solo {n}"), "respect")
                )
                .is_empty()
            );
            let cached = b.social("Bass").unwrap();
            let snapshot = b.checkpoint().unwrap();
            assert_eq!(
                serde_json::to_value(&cached).unwrap(),
                serde_json::to_value(Ledger::project(&snapshot.records, "Bass").unwrap()).unwrap()
            );
            assert_eq!(b.social("Other").unwrap().observer, "Other");
        }
        let mut fresh = book();
        fresh.social("Other").unwrap();
        fresh.restore(b.checkpoint().unwrap()).unwrap();
        assert_eq!(fresh.social("Bass").unwrap().interpretations.len(), 5);
    }
    #[test]
    fn consent_needs_observed_complete_utterance_from_a_proposed_participant() {
        let mut b = book();
        hear(
            &mut b,
            "Lead says, \"I propose [song] for Bass,Guitar: bring arrangement | condition: everyone attends | due: rehearsal\"",
        );
        b.sent("say I accept [song].").unwrap();
        b.finish_command().unwrap();
        for text in [
            "Bass says, \"Perhaps I could help.\"",
            "Bass says, \"Maybe I accept [song].\"",
            "Lead says, \"I accept [song].\"",
            "Bass says, \"I accept [song]. Tomorrow I might withdraw.\"",
            "Bass says, \"She said I accept [song].\"",
            "Bass says, \"I accept [song].",
            "    Bass says, \"I accept [song].\"",
            "Bass pages: \"I accept [song].\"",
            "<OOC> Bass says, \"I accept [song].\"",
            "[Chat] Bass says, \"I accept [song].\"",
        ] {
            hear(&mut b, text);
        }
        let state = b.social("Bass").unwrap();
        assert_eq!(
            state.commitments["song"].participants["Bass"].response,
            Response::Pending
        );
        assert!(
            state.context("song", 16000)["accepted_assignments"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        hear(
            &mut b,
            "Bass says, \"I accept [song].\"\nGuitar says, \"I decline [song].\"",
        );
        let state = b.social("account+Bass").unwrap();
        assert_eq!(
            state.commitments["song"].participants["Bass"].response,
            Response::Accepted
        );
        assert_eq!(
            state.commitments["song"].participants["Guitar"].response,
            Response::Declined
        );
        assert_eq!(
            state.context("unrelated", 16000)["accepted_assignments"][0]["id"],
            "song"
        );
        assert_eq!(b.stats()["active_beliefs"], 0); // no identity inferred from label
    }

    #[test]
    fn ooc_and_channel_examples_do_not_create_ic_proposals() {
        let mut b = book();
        hear(
            &mut b,
            "<OOC> Lead says, \"I propose [ooc] for Bass: leave | condition: none | due: now\"\n[Chat] Lead says, \"I propose [chat] for Bass: leave | condition: none | due: now\"",
        );
        assert!(b.social("Bass").unwrap().commitments.is_empty());
    }
    #[test]
    fn counteroffers_do_not_rewrite_terms_and_completion_is_only_reported() {
        let mut b = book();
        hear(
            &mut b,
            "Lead pages: I propose [song] for Bass: bring roots | condition: none | due: Friday\nBass pages: I accept [song].",
        );
        hear(
            &mut b,
            "Lead says, \"I propose [song] for Bass: bring a solo | condition: none | due: today\"\nLead says, \"I cancel [song].\"",
        );
        hear(
            &mut b,
            "Bass says, \"I propose [song2] for Bass: bring counterpoint | condition: everyone attends | due: Saturday | replaces: song\"\nBass says, \"I completed [song]: I brought the roots.\"",
        );
        let state = b.social("Bass").unwrap();
        assert_eq!(state.commitments["song"].task, "bring roots");
        assert!(!state.commitments["song"].cancelled);
        assert_eq!(state.commitments["song2"].replaces.as_deref(), Some("song"));
        assert!(
            state.commitments["song"].participants["Bass"]
                .completion_report
                .is_some()
        );
        assert!(
            state.commitments["song"].participants["Bass"]
                .assessment
                .is_none()
        );
        hear(&mut b, "Bass says, \"I withdraw [song].\"");
        assert_eq!(
            b.social("Bass").unwrap().commitments["song"].participants["Bass"].response,
            Response::Withdrawn
        );
    }
    #[test]
    fn directed_assessments_require_local_evidence_and_scoped_corrections() {
        let (mut bass, other) = (book(), book());
        let ep = hear(
            &mut bass,
            "Guitar plays brilliantly, interrupting Lead again.",
        );
        assert!(annotate(&mut bass, relationship(ep, "plays brilliantly", "respect")).is_empty());
        assert!(
            annotate(
                &mut bass,
                relationship(ep, "interrupting Lead again", "resentment")
            )
            .is_empty()
        );
        let records = bass.social("Bass").unwrap().interpretations;
        assert_eq!(records.len(), 2);
        assert!(other.social("Guitar").unwrap().interpretations.is_empty());
        let mut correction = relationship(ep, "plays brilliantly", "trust");
        correction["supersedes"] = json!(records[0].journal_id);
        assert!(!annotate(&mut bass, correction).is_empty());
        assert!(!annotate(&mut bass, relationship(ep, "MADE UP", "respect")).is_empty());
        let checkpoint = bass.checkpoint().unwrap();
        let mut restored = book();
        restored.restore(checkpoint).unwrap();
        assert_eq!(
            serde_json::to_value(restored.social("Bass").unwrap()).unwrap(),
            serde_json::to_value(bass.social("Bass").unwrap()).unwrap()
        );
        assert!(
            restored
                .recall("respect")
                .to_string()
                .contains("social_evidence")
        );
    }
    #[test]
    fn rumors_preserve_corrections_and_observer_gaps() {
        let mut a = book();
        let mut b = book();
        let ep = hear(&mut a, "Lead pages: The gig is cancelled.");
        assert!(annotate(&mut a, claim(ep, "yes", "The gig is cancelled.")).is_empty());
        let old = a.social("Bass").unwrap().interpretations[0].journal_id;
        let next = hear(&mut a, "Lead pages: Correction: the gig is still on.");
        let mut correction = claim(next, "no", "Correction: the gig is still on.");
        correction["supersedes"] = json!([old]);
        assert!(annotate(&mut a, correction).is_empty());
        let b_ep = hear(&mut b, "Lead says, \"The gig is cancelled.\"");
        assert!(annotate(&mut b, claim(b_ep, "yes", "The gig is cancelled.")).is_empty());
        let view = compare(
            &[
                ("Bass".into(), a.social("Bass").unwrap()),
                ("Guitar".into(), b.social("Guitar").unwrap()),
                ("Drums".into(), book().social("Drums").unwrap()),
            ]
            .into(),
        );
        assert_eq!(
            view["claims"][0]["perspectives"]["Drums"]["coverage"],
            "unestablished"
        );
        assert_eq!(
            view["claims"][0]["perspectives"]["Bass"]["records"][0]["active"],
            false
        );
        assert_eq!(
            view["claims"][0]["perspectives"]["Guitar"]["records"][0]["active"],
            true
        );
        let mut false_fact = claim(next, "no", "Correction: the gig is still on.");
        false_fact["basis"] = json!("observed");
        assert!(!annotate(&mut a, false_fact).is_empty());
    }
    #[test]
    fn fulfilled_missed_uncertain_and_withdrawn_are_distinct() {
        let mut b = book();
        let ep = hear(
            &mut b,
            "Lead says, \"I propose [song] for Bass: bring chart | condition: none | due: rehearsal\"\nBass says, \"I accept [song].\"\nBass says, \"I brought the chart.\"",
        );
        let outcome = |episode, quote, status, basis| json!({"type":"outcome","proposal":"song","participant":"Bass","status":status,"explanation":"An assessment","basis":basis,"confidence":"tentative","evidence":[{"episode":episode,"quote":quote}]});
        assert!(
            !annotate(
                &mut b,
                outcome(ep, "I brought the chart.", "fulfilled", "observed")
            )
            .is_empty()
        );
        assert!(
            annotate(
                &mut b,
                outcome(ep, "I brought the chart.", "uncertain", "reported")
            )
            .is_empty()
        );
        let receipt = hear(&mut b, "Rehearsal ended. No chart was delivered.");
        assert!(
            annotate(
                &mut b,
                outcome(receipt, "No chart was delivered.", "missed", "observed")
            )
            .is_empty()
        );
        let receipt = hear(&mut b, "Bass hands over the chart.");
        assert!(
            annotate(
                &mut b,
                outcome(
                    receipt,
                    "Bass hands over the chart.",
                    "fulfilled",
                    "observed"
                )
            )
            .is_empty()
        );
        let state = b.social("Bass").unwrap();
        assert_eq!(state.interpretations.iter().filter(|i| i.active).count(), 1);
        assert!(
            state.context("chart", 16000)["accepted_assignments"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            state.commitments["song"].participants["Bass"].response,
            Response::Accepted
        );
    }
    #[test]
    fn background_is_lazy_immutable_and_not_model_authored() {
        let mut b = book();
        let mut fact:Fact=serde_json::from_value(json!({"id":"coffee","revision":1,"subject":"Marco","predicate":"drink","value":"tea","source":"Lived together three years","confidence":"strong"})).unwrap();
        b.seed_background("roommates", &fact, "Bass", &Default::default())
            .unwrap();
        let state = b.social("Bass").unwrap();
        assert!(
            state.context("guitar", 16000)["interpretations"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(state.context("Marco", 16000).to_string().contains("tea"));
        assert_eq!(state.context("Marco", 0)["record_bytes"], 0);
        fact.value = "coffee".into();
        assert!(
            b.seed_background("roommates", &fact, "Bass", &Default::default())
                .is_err()
        );
        fact.revision = 2;
        b.seed_background("roommates", &fact, "Bass", &Default::default())
            .unwrap();
        assert_eq!(b.social("Bass").unwrap().interpretations.len(), 2);
        let ep = hear(&mut b, "I know Marco.");
        assert!(!annotate(&mut b,json!({"type":"background","group":"roommates","fact":fact,"evidence":[{"episode":ep,"quote":"I know Marco."}]})).is_empty());
    }
}
