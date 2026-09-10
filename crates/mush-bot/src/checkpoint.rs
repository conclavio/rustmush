//! Versioned character-local restore state. World certification is separate.
use crate::{
    Result,
    experiment::{load_json, revision},
};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// During quiescence, retain ordinary output in order but keep the operator's
/// fence protocol outside character observations. Only a complete marker line
/// can certify the boundary; fragments and stale epochs cannot acknowledge it.
pub(crate) fn fenced_output(
    buffer: &mut String,
    received: &mut bool,
    text: &str,
    epoch: &str,
) -> Result<String> {
    if *received {
        return if text.trim().is_empty() {
            Ok(String::new())
        } else {
            Err("unexpected game output after frozen-world watermark".into())
        };
    }
    buffer.push_str(text);
    if buffer.len() > 1_000_000 {
        return Err("checkpoint boundary output overflow".into());
    }
    let marker = format!("CAMPAIGN FROZEN {epoch}");
    let echo = format!("__campaign_fence {epoch}");
    let lines: Vec<_> = buffer.split_inclusive('\n').collect();
    for line in lines.iter().filter(|line| line.ends_with('\n')) {
        if line.trim() == "CAMPAIGN INVALID"
            || (line.trim().starts_with("CAMPAIGN FROZEN ") && line.trim() != marker)
        {
            return Err("fixture invalidated checkpoint or returned a stale epoch".into());
        }
    }
    let Some(index) = lines
        .iter()
        .position(|line| line.ends_with('\n') && line.trim() == marker)
    else {
        return Ok(String::new());
    };
    if lines[index + 1..]
        .iter()
        .any(|line| !line.trim().is_empty())
    {
        return Err("unexpected game output after frozen-world watermark".into());
    }
    let output = lines[..index]
        .iter()
        .filter(|line| line.trim() != echo)
        .copied()
        .collect::<String>();
    *received = true;
    buffer.clear();
    Ok(output)
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub version: u32,
    pub actor: String,
    pub epoch: String,
    pub watermark: crate::scene::Watermark,
    pub state: crate::runner::State,
    pub resume_status: String,
    pub character: crate::character::Character,
    pub affiliations: Option<crate::organization::Affiliations>,
    pub condition: crate::character::Condition,
    pub memory: crate::memory::Checkpoint,
    pub attention: crate::attention::Checkpoint,
    pub knowledge: crate::knowledge::Knowledge,
    pub conversation: crate::conversation::Tracker,
    pub models: crate::models::Catalog,
    pub model: Option<crate::models::Selection>,
    pub tape: Option<crate::tape::Source>,
    pub prompts: crate::experiment::Prompts,
    pub context: crate::experiment::Context,
    pub debug: crate::debugger::Settings,
    pub budget: crate::budget::Settings,
    pub usage: crate::budget::Usage,
    pub agent: crate::config::Agent,
    pub recall_query: Option<String>,
    pub belief_feedback: Vec<String>,
    pub attention_enabled: bool,
    pub noticed_attention: u64,
    pub social_followup: bool,
    pub social_elapsed_ms: Option<u64>,
    pub goal_elapsed_ms: u64,
    pub next_turn_after_ms: u64,
    pub next_intent: u64,
    pub last_capture: Option<crate::model::Capture>,
    pub probe_results: std::collections::VecDeque<crate::probe::Grade>,
    pub probe_results_omitted: usize,
}

impl Snapshot {
    pub fn load(path: &Path) -> Result<Self> {
        let value: Self = load_json(path, 32_000_000)?;
        value.validate()?;
        Ok(value)
    }
    pub fn digest(&self) -> Result<String> {
        Ok(revision(&serde_json::to_vec(self)?))
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || !crate::organization::identifier(&self.epoch)
            || self.actor.is_empty()
            || self.actor.len() > 200
            || self.state.status != "paused"
            || !["paused", "running", "listening", "done", "blocked"]
                .contains(&self.resume_status.as_str())
            || self.next_intent == 0
            || self.goal_elapsed_ms > 365 * 86400 * 1000
            || self.watermark.elapsed_ms > 365 * 86400 * 1000
            || self.next_turn_after_ms > 600_000
            || self.usage.requests > 1_000_000
            || self.usage.unknown_cost_requests > self.usage.requests
            || serde_json::to_vec(self)?.len() > 32_000_000
        {
            return Err("invalid bot checkpoint version, identity, boundary or limits".into());
        }
        self.watermark.scope.validate()?;
        self.memory.validate()?;
        crate::attention::Attention::restore(self.attention.clone())?;
        self.knowledge.validate_snapshot()?;
        self.conversation
            .validate_snapshot(self.watermark.elapsed_ms)?;
        crate::character::Character::parse(&serde_json::to_string(&self.character)?)?;
        self.prompts.validate()?;
        self.context.validate()?;
        self.debug.validate()?;
        self.budget.validate()?;
        self.condition.validate()?;
        if let Some(affiliations) = &self.affiliations {
            affiliations.validate()?;
        }
        if let Some(capture) = &self.last_capture {
            capture.validate()?;
        }
        crate::config::Config {
            agent: self.agent.clone(),
            ..Default::default()
        }
        .validate()?;
        if let Some(model) = &self.model {
            model.settings.validate()?;
        }
        for model in self.models.list() {
            model.settings.validate()?;
        }
        if let Some(tape) = &self.tape {
            tape.validate(true)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fence_fragments_do_not_acknowledge_early_or_enter_character_knowledge() {
        let (mut buffer, mut received) = (String::new(), false);
        assert!(
            fenced_output(
                &mut buffer,
                &mut received,
                "__campaign_fence e1\nMarco says, \"wait\"\nCAMPAIGN FRO",
                "e1"
            )
            .unwrap()
            .is_empty()
        );
        assert!(
            fenced_output(&mut buffer, &mut received, "ZEN e1", "e1")
                .unwrap()
                .is_empty()
        );
        assert!(!received);
        assert_eq!(
            fenced_output(&mut buffer, &mut received, "\n", "e1").unwrap(),
            "Marco says, \"wait\"\n"
        );
        assert!(received);
        assert!(fenced_output(&mut buffer, &mut received, "new world output\n", "e1").is_err());
    }
    #[test]
    fn stale_invalid_or_nonterminal_fences_refuse_certification() {
        for text in [
            "CAMPAIGN FROZEN old\n",
            "CAMPAIGN INVALID\n",
            "CAMPAIGN FROZEN e1\nlate say\n",
        ] {
            let (mut buffer, mut received) = (String::new(), false);
            assert!(fenced_output(&mut buffer, &mut received, text, "e1").is_err());
            assert!(!received);
        }
    }
}
