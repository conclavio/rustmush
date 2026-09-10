//! Explicit testing scope, enforced at the common player-command boundary.
use crate::{
    Result,
    config::Ssh,
    evidence::bounded,
    experiment::{load_json, revision},
    memory::{Event, Record, Source},
    organization::identifier,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Permission {
    OwnedFixture,
    Server {
        host: String,
        port: u16,
        host_key_sha256: String,
    },
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    pub version: u32,
    pub id: String,
    pub goal: String,
    pub permission: Permission,
    pub authorization: String,
    pub max_actions: usize,
    pub commands: Vec<String>,
    #[serde(default)]
    pub expectations: Vec<Expectation>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Expectation {
    pub command: String,
    pub line: String,
    pub basis: crate::playtest::Basis,
}
impl Spec {
    pub fn load(path: &Path) -> Result<Self> {
        let s: Self = load_json(path, 128_000)?;
        s.validate()?;
        Ok(s)
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || !identifier(&self.id)
            || !(1..=1000).contains(&self.max_actions)
            || self.commands.is_empty()
            || self.commands.len() > 256
            || self.expectations.len() > 256
        {
            return Err("invalid mission version, identity or limits".into());
        }
        bounded(&self.goal, 12000)?;
        bounded(&self.authorization, 4000)?;
        let mut commands = BTreeSet::new();
        for c in &self.commands {
            crate::transport::validate_command(c)?;
            if c.starts_with("__campaign_") || !commands.insert(c) {
                return Err("mission commands must be unique ordinary player commands".into());
            }
        }
        for e in &self.expectations {
            bounded(&e.line, 4000)?;
            if !commands.contains(&e.command) {
                return Err("expectation command is not permitted".into());
            }
        }
        if let Permission::Server {
            host,
            port,
            host_key_sha256,
        } = &self.permission
        {
            bounded(host, 253)?;
            bounded(host_key_sha256, 200)?;
            if *port == 0 || !host_key_sha256.starts_with("SHA256:") {
                return Err("server mission requires a pinned endpoint".into());
            }
        }
        Ok(())
    }
    pub fn bind(&self, ssh: &Ssh, owned: bool) -> Result<()> {
        self.validate()?;
        let matches = match &self.permission {
            Permission::OwnedFixture => owned,
            Permission::Server {
                host,
                port,
                host_key_sha256,
            } => {
                *host == ssh.host
                    && *port == ssh.port
                    && ssh.host_key_sha256.as_ref() == Some(host_key_sha256)
            }
        };
        if !matches {
            return Err(
                "mission authorization does not match this pinned server or owned fixture".into(),
            );
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub spec: Option<Spec>,
    pub active: bool,
    pub reservations: Vec<String>,
    pub sent: Vec<u64>,
}
impl State {
    pub fn load(&mut self, spec: Spec) -> Result<()> {
        spec.validate()?;
        if let Some(old) = &self.spec
            && revision(&serde_json::to_vec(old)?) != revision(&serde_json::to_vec(&spec)?)
        {
            return Err("one immutable mission contract per memory namespace; use a new namespace for a different scope".into());
        }
        self.spec = Some(spec);
        Ok(())
    }
    pub fn reserve(&mut self, command: &str) -> Result<bool> {
        if !self.active {
            return Ok(false);
        }
        let spec = self.spec.as_ref().ok_or("active mission has no contract")?;
        if self.reservations.len() != self.sent.len() {
            return Err("previous mission dispatch has an uncertain receipt; use a new namespace after inspection".into());
        }
        if !spec.commands.iter().any(|c| c == command) {
            return Err("mission denied command outside its exact allowlist".into());
        }
        if self.reservations.len() >= spec.max_actions {
            return Err("mission action allowance exhausted".into());
        }
        self.reservations.push(command.into());
        Ok(true)
    }
    pub fn validate(&self, records: &[Record], before: u64) -> Result<()> {
        if let Some(s) = &self.spec {
            s.validate()?;
            if self.reservations.len() > s.max_actions
                || self.reservations.iter().any(|c| !s.commands.contains(c))
            {
                return Err("mission reservations violate contract".into());
            }
        } else if self.active || !self.reservations.is_empty() || !self.sent.is_empty() {
            return Err("mission state without contract".into());
        }
        if self.sent.len() > self.reservations.len() {
            return Err("unreserved mission sends".into());
        }
        let mut last = 0;
        for (i, id) in self.sent.iter().enumerate() {
            if *id <= last || *id >= before || !records.iter().any(|r| r.id == *id && matches!(&r.event,Event::Episode(e) if e.source == Source::Sent && e.text == self.reservations[i])) { return Err("mission send evidence disagrees with journal".into()); }
            last = *id;
        }
        Ok(())
    }
    pub fn project(records: &[Record]) -> Result<Self> {
        let mut state = Self::default();
        for r in records {
            if let Event::Mission(s) = &r.event {
                s.validate(records, r.id)?;
                if let Some(spec) = &state.spec
                    && s.spec.as_ref().is_none_or(|v| {
                        serde_json::to_value(v).ok() != serde_json::to_value(spec).ok()
                    })
                {
                    return Err("mission contract changed in journal".into());
                }
                if !s.reservations.starts_with(&state.reservations)
                    || !s.sent.starts_with(&state.sent)
                {
                    return Err("mission allowance was refunded".into());
                }
                state = (**s).clone();
            }
        }
        Ok(state)
    }
    pub fn report(&self, records: &[Record]) -> Value {
        let mut receipts = vec![];
        let mut places = BTreeSet::new();
        let mut visited_commands = BTreeSet::new();
        for id in &self.sent {
            let command = records
                .iter()
                .find_map(|r| {
                    if r.id == *id {
                        if let Event::Episode(e) = &r.event {
                            Some(e.text.as_str())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .unwrap_or_default();
            visited_commands.insert(command);
            let observations: Vec<_> = records
                .iter()
                .filter_map(|r| {
                    if let Event::Episode(e) = &r.event
                        && e.source == Source::Observed
                        && e.after_command == Some(*id)
                    {
                        if let Some(p) = &e.place {
                            places.insert(format!("{}({})", p.name, p.dbref));
                        }
                        Some((r.id, e))
                    } else {
                        None
                    }
                })
                .collect();
            let outcomes: Vec<_> = self.spec.iter().flat_map(|s| &s.expectations).filter(|e| e.command == command).map(|e| {
                let found = observations.iter().any(|(_,o)| o.text.lines().any(|l| l.trim_end() == e.line));
                json!({"expected_line":e.line,"basis":e.basis,"outcome":if observations.is_empty(){"no_receipt"}else if found{"observed"}else{"expectation_not_observed"},"engine_defect_proven":false})
            }).collect();
            receipts.push(json!({"command_episode":id,"command":command,"observations":observations.iter().map(|(id,_)|id).collect::<Vec<_>>(),"expectations":outcomes}));
        }
        json!({"state":self,"remaining":self.spec.as_ref().map(|s|s.max_actions.saturating_sub(self.reservations.len())),"coverage":{"commands":visited_commands,"places":places,"receipts":receipts},"scope":"actor-local observed coverage, not a complete map; missing expected text is a candidate, not proof of an engine defect"})
    }
}
