//! An actor's directed map; reports never silently become traveled routes.
use crate::{
    Result, evidence,
    memory::{Event, Evidence, Place, Record, Source},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Traveled,
    Reported,
    Uncertain,
    Invalidated,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub from: Place,
    pub command: String,
    pub to: Option<Place>,
    pub status: Status,
    pub reason: String,
    pub evidence: Vec<Evidence>,
}
impl Report {
    pub fn validate(&self, records: &[Record], before: u64) -> Result<()> {
        if self.status == Status::Traveled || self.evidence.is_empty() || self.evidence.len() > 8 {
            return Err("map reports require citations; only travel creates traveled edges".into());
        }
        for place in std::iter::once(&self.from).chain(self.to.iter()) {
            evidence::bounded(&place.dbref, 64)?;
            evidence::bounded(&place.name, 200)?;
        }
        crate::transport::validate_command(&self.command)?;
        evidence::bounded(&self.reason, 1000)?;
        for cite in &self.evidence {
            evidence::cite(records, cite, before)?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Edge {
    pub from: Place,
    pub command: String,
    pub to: Option<Place>,
    pub status: Status,
    pub evidence: Vec<u64>,
    pub journal_id: u64,
    pub reason: String,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct Map {
    pub edges: Vec<Edge>,
    pub reports: Vec<Report>,
}
impl Map {
    pub fn project(records: &[Record]) -> Self {
        let mut map: BTreeMap<(String, String), Edge> = BTreeMap::new();
        for r in records {
            match &r.event {
                Event::Journey(j) => {
                    map.insert(
                        (j.from.dbref.clone(), j.command.clone()),
                        Edge {
                            from: j.from.clone(),
                            command: j.command.clone(),
                            to: Some(j.to.clone()),
                            status: Status::Traveled,
                            evidence: j.evidence.clone(),
                            journal_id: r.id,
                            reason: "observed directed journey; accessibility can change".into(),
                        },
                    );
                }
                Event::Navigation(report) => {
                    let key = (report.from.dbref.clone(), report.command.clone());
                    // A report may invalidate a plan, but cannot replace traveled knowledge with hearsay.
                    if report.status == Status::Invalidated || !map.contains_key(&key) {
                        map.insert(
                            key,
                            Edge {
                                from: report.from.clone(),
                                command: report.command.clone(),
                                to: report.to.clone(),
                                status: report.status.clone(),
                                evidence: report.evidence.iter().map(|e| e.episode).collect(),
                                journal_id: r.id,
                                reason: report.reason.clone(),
                            },
                        );
                    }
                }
                Event::Episode(e) if e.source == Source::Observed => {
                    let Some(command_id) = e.after_command else {
                        continue;
                    };
                    let Some(Event::Episode(sent)) = command_id
                        .checked_sub(1)
                        .and_then(|i| records.get(i as usize))
                        .map(|r| &r.event)
                    else {
                        continue;
                    };
                    if sent.source != Source::Sent {
                        continue;
                    }
                    let Some(from) = &sent.place else { continue };
                    let key = (from.dbref.clone(), sent.text.clone());
                    let text = evidence::non_speech(&e.text).to_lowercase();
                    let failed = [
                        "there is no exit that way.",
                        "you can't go that way.",
                        "you cannot go that way.",
                        "the exit is blocked.",
                    ]
                    .iter()
                    .any(|s| text.lines().any(|line| line.trim() == *s));
                    if failed && let Some(edge) = map.get_mut(&key) {
                        edge.status = Status::Invalidated;
                        edge.evidence.push(r.id);
                        edge.journal_id = r.id;
                        edge.reason =
                            "observed travel rejection; replan or obtain new evidence".into();
                    }
                }
                _ => {}
            }
        }
        Self {
            edges: map.into_values().collect(),
            reports: records
                .iter()
                .filter_map(|r| {
                    if let Event::Navigation(report) = &r.event {
                        Some(report.clone())
                    } else {
                        None
                    }
                })
                .collect(),
        }
    }
    pub fn usable(&self, from: &str, command: &str, journey_id: u64) -> bool {
        self.edges.iter().any(|e| {
            e.from.dbref == from
                && e.command == command
                && e.status == Status::Traveled
                && e.journal_id == journey_id
        })
    }
}
