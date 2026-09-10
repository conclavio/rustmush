//! Quiet deterministic activities. This scheduler never advances world time.
use crate::{
    Result, evidence,
    memory::{Book, Event, Evidence, Record, Source},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Clock {
    #[default]
    SessionMonotonic,
    ManualTest,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub decision_ms: u64,
    pub action_ms: u64,
    pub unsolicited_speech_ms: u64,
    pub max_decisions: u64,
    pub max_actions: u64,
    pub max_unsolicited_speech: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            decision_ms: 1000,
            action_ms: 5000,
            unsolicited_speech_ms: 60000,
            max_decisions: 1000,
            max_actions: 100,
            max_unsolicited_speech: 0,
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub command: String,
    /// Exact complete game-output lines, excluding speech. For communication
    /// steps this is the complete public utterance (the act being confirmed).
    pub expect: Vec<String>,
    #[serde(default)]
    pub reject: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Procedure {
    pub id: String,
    pub revision: u32,
    pub applicability: String,
    pub steps: Vec<Step>,
    #[serde(default)]
    pub source: Vec<Evidence>,
    pub failure: Failure,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    Suspend,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Routine {
    pub id: String,
    pub procedure: String,
    pub destination: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub priority: i16,
    /// Other routines completed in this same cycle.
    #[serde(default)]
    pub requires: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub version: u32,
    pub clock: Clock,
    pub period_ms: u64,
    pub limits: Limits,
    pub routines: Vec<Routine>,
    pub procedures: Vec<Procedure>,
}
impl Plan {
    pub fn load(path: &Path) -> Result<Self> {
        let p: Self = crate::experiment::load_json(path, 128000)?;
        p.validate()?;
        Ok(p)
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || self.period_ms == 0
            || self.period_ms > 365 * 86400 * 1000
            || self.routines.is_empty()
            || self.routines.len() > 32
            || self.procedures.is_empty()
            || self.procedures.len() > 32
            || self.limits.decision_ms < 50
            || self.limits.action_ms < 50
            || self.limits.unsolicited_speech_ms < 50
            || self.limits.decision_ms > 86400000
            || self.limits.action_ms > 86400000
            || self.limits.unsolicited_speech_ms > 86400000
            || self.limits.max_actions > 100000
            || self.limits.max_decisions > 1000000
            || self.limits.max_unsolicited_speech > 10000
            || serde_json::to_vec(self)?.len() > 128000
        {
            return Err("invalid activity plan version/bounds".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        for p in &self.procedures {
            evidence::bounded(&p.id, 100)?;
            evidence::bounded(&p.applicability, 1000)?;
            if !ids.insert(&p.id)
                || p.revision == 0
                || p.steps.is_empty()
                || p.steps.len() > 32
                || p.source.len() > 8
            {
                return Err("invalid/duplicate procedure".into());
            }
            for step in &p.steps {
                crate::transport::validate_command(&step.command)?;
                if step.command.starts_with("__campaign_")
                    || step.expect.is_empty()
                    || step.expect.len() > 8
                    || step.reject.len() > 8
                {
                    return Err("procedure steps need bounded explicit receipts".into());
                }
                for s in step.expect.iter().chain(&step.reject) {
                    evidence::bounded(s, 2000)?;
                }
            }
        }
        let mut ids = std::collections::BTreeSet::new();
        for r in &self.routines {
            evidence::bounded(&r.id, 100)?;
            evidence::bounded(&r.destination, 200)?;
            if !ids.insert(&r.id)
                || r.start_ms >= r.end_ms
                || r.end_ms > self.period_ms
                || r.requires.len() > 32
                || !self.procedures.iter().any(|p| p.id == r.procedure)
                || r.requires
                    .iter()
                    .any(|id| id == &r.id || !self.routines.iter().any(|other| &other.id == id))
            {
                return Err("invalid routine/window/prerequisite".into());
            }
        }
        // Reject prerequisite cycles instead of leaving a permanently idle plan.
        let mut resolved = std::collections::BTreeSet::new();
        for _ in 0..self.routines.len() {
            for r in &self.routines {
                if r.requires.iter().all(|id| resolved.contains(id)) {
                    resolved.insert(r.id.clone());
                }
            }
        }
        if resolved.len() != self.routines.len() {
            return Err("routine prerequisites contain a cycle".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Active {
    pub routine: String,
    pub procedure_revision: u32,
    pub cycle: u64,
    pub step: usize,
    pub receipts: Vec<u64>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Pending {
    pub command: String,
    pub sent: Option<u64>,
    pub travel_to: Option<String>,
    pub step: Option<Step>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Completion {
    pub cycle: u64,
    pub procedure_revision: u32,
    pub receipts: Vec<u64>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub version: u32,
    pub plan: Option<Plan>,
    pub enabled: bool,
    pub clock_ms: u64,
    pub active: Option<Active>,
    pub pending: Option<Pending>,
    pub completed: BTreeMap<String, Completion>,
    pub reason: String,
    pub suspended: bool,
    pub decisions: u64,
    pub actions: u64,
    pub unsolicited_speech: u64,
    pub next_decision_ms: u64,
    pub next_action_ms: u64,
    pub next_speech_ms: u64,
    /// Bounded revision history; observations retain earlier receipts independently.
    pub history: Vec<Plan>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            version: 1,
            plan: None,
            enabled: false,
            clock_ms: 0,
            active: None,
            pending: None,
            completed: BTreeMap::new(),
            reason: "no activity plan".into(),
            suspended: false,
            decisions: 0,
            actions: 0,
            unsolicited_speech: 0,
            next_decision_ms: 0,
            next_action_ms: 0,
            next_speech_ms: 0,
            history: vec![],
        }
    }
}
impl State {
    pub fn validate(&self, records: &[Record], before: u64) -> Result<()> {
        if self.version != 1
            || self.clock_ms > 365 * 86400000
            || self.history.len() > 32
            || self.completed.len() > 32
            || self.reason.len() > 2000
            || self.decisions > 1000000
            || self.actions > 100000
            || self.unsolicited_speech > 10000
            || serde_json::to_vec(self)?.len() > 2_000_000
        {
            return Err("invalid activity state version/bounds".into());
        }
        for p in self.plan.iter().chain(&self.history) {
            p.validate()?;
            for procedure in &p.procedures {
                for cite in &procedure.source {
                    evidence::cite(records, cite, before)?;
                }
            }
        }
        if let Some(active) = &self.active {
            let plan = self.plan.as_ref().ok_or("active activity without plan")?;
            let routine = plan
                .routines
                .iter()
                .find(|r| r.id == active.routine)
                .ok_or("unknown active routine")?;
            let procedure = plan
                .procedures
                .iter()
                .find(|p| p.id == routine.procedure)
                .unwrap();
            if active.step >= procedure.steps.len()
                || active.procedure_revision != procedure.revision
                || active.receipts.len() > 128
            {
                return Err("invalid activity cursor/revision".into());
            }
        }
        if let Some(pending) = &self.pending {
            if self.active.is_none() || pending.step.is_some() == pending.travel_to.is_some() {
                return Err("invalid pending activity".into());
            }
            crate::transport::validate_command(&pending.command)?;
            if let Some(id) = pending.sent {
                let Some(Event::Episode(e)) = id
                    .checked_sub(1)
                    .and_then(|n| records.get(n as usize))
                    .map(|r| &r.event)
                else {
                    return Err("pending command evidence missing".into());
                };
                if e.source != Source::Sent || e.text != pending.command || id >= before {
                    return Err("pending command evidence mismatch".into());
                }
            }
        }
        for receipt in self
            .completed
            .values()
            .flat_map(|c| &c.receipts)
            .chain(self.active.iter().flat_map(|a| &a.receipts))
        {
            let Some(Event::Episode(e)) = receipt
                .checked_sub(1)
                .and_then(|n| records.get(n as usize))
                .map(|r| &r.event)
            else {
                return Err("activity receipt missing".into());
            };
            if e.source != Source::Observed || *receipt >= before {
                return Err("activity receipt must be observed".into());
            }
        }
        Ok(())
    }
    pub fn install(&mut self, plan: Plan, records: &[Record]) -> Result<()> {
        plan.validate()?;
        if let Some(old) = &self.plan {
            if serde_json::to_value(old)? == serde_json::to_value(&plan)? {
                return Ok(());
            }
            if old.clock != plan.clock {
                return Err("cannot change an established activity clock domain".into());
            }
            for p in &plan.procedures {
                if let Some(prior) = old.procedures.iter().find(|v| v.id == p.id)
                    && (p.revision < prior.revision
                        || (p.revision == prior.revision
                            && serde_json::to_value(p)? != serde_json::to_value(prior)?))
                {
                    return Err("changed procedures require a newer revision".into());
                }
            }
            if self.history.len() >= 32 {
                return Err("procedure revision history capacity reached".into());
            }
        }
        let mut next = self.clone();
        if let Some(old) = next.plan.replace(plan) {
            next.history.push(old);
        }
        next.active = None;
        next.pending = None;
        next.suspended = false;
        next.enabled = false;
        next.reason = "plan installed; /activity on enables it while listening".into();
        next.validate(records, records.len() as u64 + 1)?;
        *self = next;
        Ok(())
    }
    pub fn reconnect(&mut self) {
        self.enabled = false;
        if self.pending.take().is_some() {
            self.suspended = true;
            self.reason =
                "unresumable unsettled command after reconnect; inspect receipts before retry"
                    .into();
        } else if self.plan.is_some() {
            self.reason = "restored activity paused; resume explicitly".into();
        }
    }
    pub fn advance(&mut self, delta: u64) -> Result<()> {
        self.clock_ms = self
            .clock_ms
            .checked_add(delta)
            .filter(|v| *v <= 365 * 86400000)
            .ok_or("activity clock overflow")?;
        Ok(())
    }
    pub fn cancel_unsent(&mut self) {
        if self.pending.as_ref().is_some_and(|p| p.sent.is_none()) {
            self.pending = None;
            self.reason = "unsent activity interrupted; replan on resume".into();
        }
    }
    pub fn dispatched(&mut self, id: u64) {
        if let Some(pending) = &mut self.pending {
            pending.sent = Some(id);
            self.actions += 1;
            let limits = &self.plan.as_ref().unwrap().limits;
            self.next_action_ms = self.clock_ms.saturating_add(limits.action_ms);
            if crate::attention::is_communication(&pending.command) {
                self.unsolicited_speech += 1;
                self.next_speech_ms = self.clock_ms.saturating_add(limits.unsolicited_speech_ms);
            }
        }
    }
    pub fn reconcile(&mut self, records: &[Record], settled: bool) -> bool {
        let Some(pending) = &self.pending else {
            return false;
        };
        let Some(sent) = pending.sent else {
            return false;
        };
        let observations: Vec<_> = records
            .iter()
            .filter_map(|r| match &r.event {
                Event::Episode(e)
                    if e.source == Source::Observed && e.after_command == Some(sent) =>
                {
                    Some((r, e))
                }
                _ => None,
            })
            .collect();
        let mut receipts = vec![];
        let mut rejection = false;
        let success = if let Some(destination) = &pending.travel_to {
            observations.iter().any(|(r, e)| {
                let found = e.place.as_ref().is_some_and(|p| &p.dbref == destination);
                if found {
                    receipts.push(r.id)
                }
                found
            })
        } else {
            let step = pending.step.as_ref().unwrap();
            let comm = crate::attention::is_communication(&pending.command);
            let output = observations
                .iter()
                .map(|(_, e)| {
                    if comm {
                        e.text.clone()
                    } else {
                        evidence::non_speech(&e.text)
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            rejection = step.reject.iter().any(|s| output.lines().any(|l| l == s));
            let found = step.expect.iter().all(|s| output.lines().any(|l| l == s));
            if found {
                receipts.extend(observations.iter().map(|(r, _)| r.id));
            }
            found
        };
        if !settled {
            return false;
        }
        if rejection || !success {
            // Keep the cursor; no automatic repetition of an ambiguous command.
            self.suspended = true;
            self.reason="step rejected or settled without required observed receipt; inspect and /activity retry".into();
            self.pending = None;
            return true;
        }
        let pending = self.pending.take().unwrap();
        let active = self.active.as_mut().unwrap();
        if active.receipts.len() + receipts.len() > 128 {
            self.suspended = true;
            self.reason = "activity evidence capacity reached".into();
            return true;
        }
        active.receipts.extend(receipts);
        if pending.step.is_some() {
            active.step += 1;
        }
        let plan = self.plan.as_ref().unwrap();
        let routine = plan
            .routines
            .iter()
            .find(|r| r.id == active.routine)
            .unwrap();
        let procedure = plan
            .procedures
            .iter()
            .find(|p| p.id == routine.procedure)
            .unwrap();
        if active.step == procedure.steps.len() {
            let active = self.active.take().unwrap();
            self.completed.insert(
                active.routine,
                Completion {
                    cycle: active.cycle,
                    procedure_revision: active.procedure_revision,
                    receipts: active.receipts,
                },
            );
            self.reason = "activity completed with observed receipts".into();
        } else {
            self.reason = "receipt observed; ready for next activity step".into();
        }
        true
    }
    /// Returns at most one unsent command. No calls, polling commands or backlog.
    pub fn choose(&mut self, book: &Book) -> Option<String> {
        let plan = self.plan.as_ref()?;
        if !self.enabled
            || self.suspended
            || self.pending.is_some()
            || self.clock_ms < self.next_decision_ms
        {
            return None;
        }
        let cycle = self.clock_ms / plan.period_ms;
        let time = self.clock_ms % plan.period_ms;
        if self.active.as_ref().is_some_and(|a| {
            a.cycle != cycle
                || plan
                    .routines
                    .iter()
                    .find(|r| r.id == a.routine)
                    .is_none_or(|r| time >= r.end_ms)
        }) {
            self.active = None;
            self.reason = "missed/expired window coalesced; no offscreen completion".into();
        }
        let candidate = plan
            .routines
            .iter()
            .filter(|r| {
                time >= r.start_ms
                    && time < r.end_ms
                    && self.completed.get(&r.id).is_none_or(|c| c.cycle != cycle)
                    && r.requires
                        .iter()
                        .all(|id| self.completed.get(id).is_some_and(|c| c.cycle == cycle))
            })
            .max_by_key(|r| (r.priority, std::cmp::Reverse(&r.id)));
        let routine = candidate?;
        if self.clock_ms < self.next_action_ms {
            return None;
        }
        if self.decisions >= plan.limits.max_decisions || self.actions >= plan.limits.max_actions {
            self.suspended = true;
            self.reason = "activity decision/action allowance exhausted".into();
            return None;
        }
        self.decisions += 1;
        self.next_decision_ms = self.clock_ms.saturating_add(plan.limits.decision_ms);
        if self.active.is_none() {
            let procedure = plan
                .procedures
                .iter()
                .find(|p| p.id == routine.procedure)
                .unwrap();
            self.active = Some(Active {
                routine: routine.id.clone(),
                procedure_revision: procedure.revision,
                cycle,
                step: 0,
                receipts: vec![],
            });
        }
        let active = self.active.as_ref().unwrap();
        let routine = plan
            .routines
            .iter()
            .find(|r| r.id == active.routine)
            .unwrap();
        let route = book.route(&routine.destination);
        if route["status"] != "remembered_route" {
            self.suspended = true;
            self.reason="destination/route unestablished or invalidated; inspect /map and revise the procedure".into();
            return None;
        }
        let journeys = route["journeys"].as_array()?;
        let pending = if let Some(first) = journeys.first() {
            Pending {
                command: first["event"]["data"]["command"].as_str()?.into(),
                sent: None,
                travel_to: Some(first["event"]["data"]["to"]["dbref"].as_str()?.into()),
                step: None,
            }
        } else {
            let procedure = plan
                .procedures
                .iter()
                .find(|p| p.id == routine.procedure)
                .unwrap();
            let step = procedure.steps[active.step].clone();
            if crate::attention::is_communication(&step.command)
                && (self.unsolicited_speech >= plan.limits.max_unsolicited_speech
                    || self.clock_ms < self.next_speech_ms)
            {
                self.suspended = true;
                self.reason = "unsolicited speech allowance/cooldown prevents this step".into();
                return None;
            }
            Pending {
                command: step.command.clone(),
                sent: None,
                travel_to: None,
                step: Some(step),
            }
        };
        let command = pending.command.clone();
        self.pending = Some(pending);
        self.reason = "awaiting observed activity receipt".into();
        Some(command)
    }
    pub fn context(&self) -> Value {
        serde_json::json!({"clock":self.plan.as_ref().map(|p|&p.clock),"clock_ms":self.clock_ms,"enabled":self.enabled,
            "schedule":self.plan.as_ref().map(|p|&p.routines),
            "active":self.active,"pending":self.pending,"reason":self.reason,"suspended":self.suspended,
            "completed":self.completed,"limits":self.plan.as_ref().map(|p|&p.limits),"actions":self.actions,"decisions":self.decisions,
            "guidance":"Activities yield to engagement and explicit goals. Local scheduler time is not fictional world time; unknown receipts are not completion."})
    }
}
