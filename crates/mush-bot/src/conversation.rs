//! Local conversation eligibility and pacing. No engine or other actor's state.
use std::{collections::VecDeque, path::Path};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{Result, attention::Speech, experiment::load_json};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Independent,
    Spectator,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Scope {
    pub campaign: String,
    pub branch: String,
    pub scene: String,
}
impl Default for Scope {
    fn default() -> Self {
        Self {
            campaign: "standalone".into(),
            branch: "main".into(),
            scene: "session".into(),
        }
    }
}
impl Scope {
    pub fn validate(&self) -> Result<()> {
        if [&self.campaign, &self.branch, &self.scene]
            .iter()
            .any(|v| !crate::organization::identifier(v))
        {
            return Err("scene, branch and campaign must be identifiers".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub mode: Mode,
    pub scope: Scope,
    pub seed: u64,
    pub debounce_ms: u64,
    pub reply_min_ms: u64,
    pub reply_jitter_ms: u64,
    pub request_cooldown_ms: u64,
    pub window_ms: u64,
    pub max_social_requests: usize,
    pub max_utterances: usize,
    pub max_utterance_chars: usize,
    pub thread_idle_ms: u64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            mode: Mode::Independent,
            scope: Scope::default(),
            seed: 1,
            debounce_ms: 600,
            reply_min_ms: 700,
            reply_jitter_ms: 1300,
            request_cooldown_ms: 1500,
            window_ms: 60_000,
            max_social_requests: 12,
            max_utterances: 8,
            max_utterance_chars: 1200,
            thread_idle_ms: 120_000,
        }
    }
}
impl Settings {
    pub fn load(path: &Path) -> Result<Self> {
        let value: Self = load_json(path, 16_000)?;
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        self.scope.validate()?;
        if self.version != 1
            || self.debounce_ms > 30_000
            || self.reply_min_ms > 30_000
            || self.reply_jitter_ms > 30_000
            || self.request_cooldown_ms > 60_000
            || !(1000..=3_600_000).contains(&self.window_ms)
            || !(1..=1000).contains(&self.max_social_requests)
            || !(1..=1000).contains(&self.max_utterances)
            || !(1..=8000).contains(&self.max_utterance_chars)
            || !(1000..=3_600_000).contains(&self.thread_idle_ms)
        {
            return Err("invalid conversation policy version or limits".into());
        }
        Ok(())
    }
}

/// Punctuation and temporal grouping are cues, not semantic adjudication.
#[derive(Clone, Debug, Serialize)]
pub struct Exchange {
    pub id: u64,
    pub last_elapsed_ms: u64,
    pub participant_labels: Vec<String>,
    pub addressed_to_self: Vec<u64>,
    pub question_candidates: Vec<u64>,
    pub handled_through: u64,
}

pub struct Tracker {
    pub settings: Settings,
    seed: u64,
    seen: u64,
    last_speech: Option<u64>,
    requests: VecDeque<u64>,
    utterances: VecDeque<u64>,
    pub exchanges: VecDeque<Exchange>,
    next_exchange: u64,
    pub reason: String,
    pub cancelled_replies: u64,
    pub declined: bool,
}
impl Tracker {
    pub fn configure(&mut self, settings: Settings, actor: &str) {
        self.seed = Self::new(settings.clone(), actor).seed;
        self.settings = settings;
        self.reason = "policy changed; paused".into();
    }
    pub fn new(settings: Settings, actor: &str) -> Self {
        // Fixed algorithm, actor identity and seed: reproducible without correlated casts.
        let seed = actor
            .bytes()
            .fold(settings.seed ^ 0xcbf29ce484222325, |h, b| {
                (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
            });
        Self {
            settings,
            seed,
            seen: 0,
            last_speech: None,
            requests: VecDeque::new(),
            utterances: VecDeque::new(),
            exchanges: VecDeque::new(),
            next_exchange: 1,
            reason: "passive listening; no request needed".into(),
            cancelled_replies: 0,
            declined: false,
        }
    }
    pub fn observe<'a>(&mut self, speech: impl Iterator<Item = &'a Speech>, now: u64) -> bool {
        let mut changed = false;
        for item in speech {
            if item.id <= self.seen {
                continue;
            }
            self.seen = item.id;
            if item.self_echo {
                continue;
            }
            changed = true;
            self.declined = false;
            self.last_speech = Some(now);
            if self.exchanges.back().is_none_or(|t| {
                now.saturating_sub(t.last_elapsed_ms) > self.settings.thread_idle_ms
            }) {
                self.exchanges.push_back(Exchange {
                    id: self.next_exchange,
                    last_elapsed_ms: now,
                    participant_labels: vec![],
                    addressed_to_self: vec![],
                    question_candidates: vec![],
                    handled_through: 0,
                });
                self.next_exchange += 1;
                if self.exchanges.len() > 16 {
                    self.exchanges.pop_front();
                }
            }
            let thread = self.exchanges.back_mut().unwrap();
            thread.last_elapsed_ms = now;
            if !thread.participant_labels.contains(&item.speaker_label)
                && thread.participant_labels.len() < 16
            {
                thread.participant_labels.push(item.speaker_label.clone());
            }
            if item
                .cues
                .iter()
                .any(|c| c == "name_or_alias" || c == "direct_message")
            {
                bounded_push(&mut thread.addressed_to_self, item.id);
            }
            if item.text.contains('?') {
                bounded_push(&mut thread.question_candidates, item.id);
            }
        }
        changed
    }
    pub fn handled(&mut self, through: u64) {
        for thread in &mut self.exchanges {
            thread.handled_through = through;
        }
    }
    fn trim(&mut self, now: u64) {
        for queue in [&mut self.requests, &mut self.utterances] {
            while queue
                .front()
                .is_some_and(|at| now.saturating_sub(*at) >= self.settings.window_ms)
            {
                queue.pop_front();
            }
        }
    }
    /// Idle social decisions only. Explicit questions and goals retain their own budget gates.
    pub fn model_gate(&mut self, now: u64) -> bool {
        self.trim(now);
        self.reason = if self.settings.mode == Mode::Spectator {
            "spectator; no autonomous model requests"
        } else if self
            .last_speech
            .is_some_and(|at| now.saturating_sub(at) < self.settings.debounce_ms)
        {
            "collecting speech burst"
        } else if self
            .requests
            .back()
            .is_some_and(|at| now.saturating_sub(*at) < self.settings.request_cooldown_ms)
        {
            "social request cooldown"
        } else if self.requests.len() >= self.settings.max_social_requests {
            "social request window exhausted; waiting"
        } else {
            "eligible to prepare a reply"
        }
        .into();
        self.reason == "eligible to prepare a reply"
    }
    pub fn burst_ready(&self, now: u64) -> bool {
        self.last_speech
            .is_none_or(|at| now.saturating_sub(at) >= self.settings.debounce_ms)
    }
    pub fn requested(&mut self, now: u64) {
        self.trim(now);
        self.requests.push_back(now);
        self.reason = "preparing reply".into();
        self.declined = false;
    }
    pub fn delay(&self, intent: u64) -> u64 {
        let mut value = self
            .seed
            .wrapping_add(intent.wrapping_mul(0x9e3779b97f4a7c15));
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
        self.settings.reply_min_ms + (value ^ (value >> 31)) % (self.settings.reply_jitter_ms + 1)
    }
    pub fn speech_gate(&mut self, now: u64) -> bool {
        self.trim(now);
        self.utterances.len() < self.settings.max_utterances
            && self
                .last_speech
                .is_none_or(|at| now.saturating_sub(at) >= self.settings.debounce_ms)
    }
    pub fn spoke(&mut self, now: u64) {
        self.trim(now);
        self.utterances.push_back(now);
        self.reason = "reply sent; listening".into();
    }
    pub fn snapshot(&self) -> Value {
        json!({"policy":self.settings,"reason":self.reason,"speech_watermark":self.seen,
            "last_speech_elapsed_ms":self.last_speech,"social_request_times_ms":self.requests,
            "utterance_times_ms":self.utterances,"exchanges":self.exchanges,
            "cancelled_replies":self.cancelled_replies,
            "declined":self.declined,
            "interpretation":"Exchange grouping and question marks are heuristic cues. Labels do not establish identity or acquaintance; handled does not certify answered."})
    }
}
fn bounded_push(values: &mut Vec<u64>, value: u64) {
    values.push(value);
    if values.len() > 32 {
        values.remove(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn speech(id: u64, text: &str) -> Speech {
        Speech {
            id,
            at_ms: 0,
            speaker_label: "Marco".into(),
            kind: "say".into(),
            text: text.into(),
            self_echo: false,
            cues: vec!["name_or_alias".into()],
            incomplete: false,
        }
    }
    #[test]
    fn burst_and_cancelled_request_allowances_are_separate_from_speech() {
        let mut t = Tracker::new(
            Settings {
                max_social_requests: 1,
                ..Default::default()
            },
            "Mira",
        );
        assert!(t.observe([&speech(1, "Mira?")].into_iter(), 100));
        assert!(!t.model_gate(101));
        assert!(!t.burst_ready(101));
        assert!(t.burst_ready(1000));
        assert!(t.model_gate(1000));
        t.requested(1000);
        assert!(!t.model_gate(4000));
        assert!(t.speech_gate(4000));
        assert!(t.model_gate(61_000));
        assert_eq!(t.exchanges[0].question_candidates, vec![1]);
        t.handled(1);
        assert_eq!(t.exchanges[0].handled_through, 1);
    }
    #[test]
    fn pacing_is_repeatable_but_not_correlated_between_actors() {
        let a = Tracker::new(Settings::default(), "Mira");
        let b = Tracker::new(Settings::default(), "Jon");
        assert_eq!(a.delay(8), a.delay(8));
        assert_ne!(a.delay(8), b.delay(8));
        assert!((700..=2000).contains(&a.delay(8)));
    }
    #[test]
    fn spectator_never_becomes_eligible_and_limits_expire() {
        let mut t = Tracker::new(
            Settings {
                mode: Mode::Spectator,
                max_utterances: 1,
                ..Default::default()
            },
            "Observer",
        );
        assert!(!t.model_gate(1_000_000));
        t.spoke(10);
        assert!(!t.speech_gate(11));
        assert!(t.speech_gate(60_010));
        assert!(
            Settings {
                max_social_requests: 0,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(serde_json::from_value::<Settings>(json!({"unknown":true})).is_err());
    }
}
