//! A bounded speech window and attention cues over the public terminal stream.
//! Cues invite a model decision; they never directly issue a game command.
use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{Result, character::Character};

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub enabled: bool,
    pub listen_when_idle: bool,
    pub recent_limit: usize,
    pub recent_bytes: usize,
    pub cooldown_ms: u64,
    pub conversation_window_secs: u64,
    pub address_cues: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            listen_when_idle: true,
            recent_limit: 64,
            recent_bytes: 24_000,
            cooldown_ms: 1000,
            conversation_window_secs: 120,
            address_cues: vec![
                "excuse me".into(),
                "hey you".into(),
                "hello there".into(),
                "can you help".into(),
                "a question for you".into(),
            ],
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        if !(4..=256).contains(&self.recent_limit)
            || !(4096..=128_000).contains(&self.recent_bytes)
            || !(100..=30_000).contains(&self.cooldown_ms)
            || self.conversation_window_secs > 3600
            || self.address_cues.len() > 32
            || self
                .address_cues
                .iter()
                .any(|s| s.trim().is_empty() || s.len() > 200)
        {
            return Err("invalid attention window, cooldown or address cues".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Speech {
    pub id: u64,
    pub at_ms: u64,
    pub speaker_label: String,
    pub kind: String,
    pub text: String,
    pub self_echo: bool,
    pub cues: Vec<String>,
    pub incomplete: bool,
}

#[derive(Clone, Deserialize, Serialize)]
struct Fragment {
    speaker: String,
    kind: String,
    text: String,
    quoted: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    settings: Settings,
    names: Vec<String>,
    self_labels: Vec<String>,
    records: VecDeque<Speech>,
    bytes: usize,
    next_id: u64,
    handled: u64,
    dropped_unhandled: usize,
    buffer: String,
    fragment: Option<Fragment>,
    last_input_ago_ms: u64,
    partners_elapsed_ms: std::collections::BTreeMap<String, u64>,
}

pub struct Attention {
    settings: Settings,
    names: Vec<String>,
    self_labels: Vec<String>,
    records: VecDeque<Speech>,
    bytes: usize,
    next_id: u64,
    handled: u64,
    dropped_unhandled: usize,
    partners: HashMap<String, Instant>,
    buffer: String,
    fragment: Option<Fragment>,
    last_input: Instant,
}

impl Attention {
    pub fn checkpoint(&self) -> crate::Result<Checkpoint> {
        Ok(serde_json::from_value(self.bookmark())?)
    }
    pub fn restore(value: Checkpoint) -> crate::Result<Self> {
        value.settings.validate()?;
        if value.next_id == 0
            || value.handled >= value.next_id
            || value.records.len() > value.settings.recent_limit
            || value.records.iter().any(|r| r.id >= value.next_id)
            || value
                .records
                .iter()
                .zip(value.records.iter().skip(1))
                .any(|(a, b)| a.id >= b.id)
            || value.partners_elapsed_ms.len() > 100_000
            || value.bytes > value.settings.recent_bytes
        {
            return Err("invalid checkpoint attention state".into());
        }
        let now = Instant::now();
        let ago = |ms| {
            now.checked_sub(std::time::Duration::from_millis(ms))
                .ok_or_else(|| -> crate::Error { "checkpoint attention timer out of range".into() })
        };
        Ok(Self {
            settings: value.settings,
            names: value.names,
            self_labels: value.self_labels,
            records: value.records,
            bytes: value.bytes,
            next_id: value.next_id,
            handled: value.handled,
            dropped_unhandled: value.dropped_unhandled,
            buffer: value.buffer,
            fragment: value.fragment,
            last_input: ago(value.last_input_ago_ms)?,
            partners: value
                .partners_elapsed_ms
                .into_iter()
                .map(|(n, ms)| Ok((n, ago(ms)?)))
                .collect::<crate::Result<_>>()?,
        })
    }
    pub fn bookmark(&self) -> Value {
        json!({"settings":self.settings,"names":self.names,"self_labels":self.self_labels,
            "records":self.records,"next_id":self.next_id,"handled":self.handled,"bytes":self.bytes,
            "dropped_unhandled":self.dropped_unhandled,"buffer":self.buffer,"fragment":self.fragment,
            "last_input_ago_ms":self.last_input.elapsed().as_millis(),
            "partners_elapsed_ms":self.partners.iter().map(|(name, at)| (name.clone(),at.elapsed().as_millis())).collect::<std::collections::BTreeMap<_,_>>()})
    }

    pub fn records(&self) -> impl Iterator<Item = &Speech> {
        self.records.iter()
    }
    pub fn handled_through(&self) -> u64 {
        self.handled
    }

    pub fn new(settings: Settings, character: &Character, username: &str) -> Self {
        let login_name = username
            .split_once('+')
            .map_or(username, |(_, name)| name)
            .to_owned();
        let mut names = character.aliases.clone();
        names.extend([character.name.clone(), login_name.clone()]);
        let self_labels = if username.contains('+') {
            vec![login_name]
        } else {
            vec![login_name, character.name.clone()]
        };
        Self {
            settings,
            names,
            self_labels,
            records: VecDeque::new(),
            bytes: 0,
            next_id: 1,
            handled: 0,
            dropped_unhandled: 0,
            partners: HashMap::new(),
            buffer: String::new(),
            fragment: None,
            last_input: Instant::now(),
        }
    }

    pub fn feed(&mut self, text: &str) {
        self.buffer.push_str(text);
        self.last_input = Instant::now();
        while let Some(end) = self.buffer.find('\n') {
            let line = self.buffer[..end].trim_end_matches('\r').to_owned();
            self.buffer.drain(..=end);
            self.line(&line);
        }
        if self.buffer.len() > 8192 {
            let text = std::mem::take(&mut self.buffer);
            self.line(&text);
            self.finish(true);
        }
    }

    pub fn flush_if_quiet(&mut self, millis: u64) {
        if self.last_input.elapsed() >= Duration::from_millis(millis) {
            if !self.buffer.is_empty() {
                let line = std::mem::take(&mut self.buffer);
                self.line(&line);
            }
            self.finish(true);
        }
    }

    fn line(&mut self, line: &str) {
        if let Some(fragment) = parse_start(line) {
            self.finish(true);
            let complete = !fragment.quoted || fragment.text.ends_with('"');
            self.fragment = Some(fragment);
            if complete {
                self.finish(false);
            }
        } else if let Some(fragment) = &mut self.fragment {
            if !line.trim().is_empty() {
                fragment.text.push(' ');
                fragment.text.push_str(line.trim());
            }
            if line.ends_with('"') {
                self.finish(false);
            } else if self.fragment.as_ref().is_some_and(|f| f.text.len() > 8192) {
                self.finish(true);
            }
        }
    }

    fn finish(&mut self, mut incomplete: bool) {
        let Some(mut fragment) = self.fragment.take() else {
            return;
        };
        if fragment.quoted {
            fragment.text = fragment
                .text
                .strip_prefix('"')
                .unwrap_or(&fragment.text)
                .to_owned();
            if !incomplete {
                fragment.text = fragment
                    .text
                    .strip_suffix('"')
                    .unwrap_or(&fragment.text)
                    .to_owned();
            }
        }
        let self_echo = self
            .self_labels
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&fragment.speaker));
        let mut cues = Vec::new();
        if !self_echo && matches!(fragment.kind.as_str(), "say" | "whisper" | "page") {
            if self.names.iter().any(|name| mentions(&fragment.text, name)) {
                cues.push("name_or_alias".into());
            }
            if matches!(fragment.kind.as_str(), "whisper" | "page") {
                cues.push("direct_message".into());
            }
            if self
                .settings
                .address_cues
                .iter()
                .any(|cue| mentions(&fragment.text, cue))
            {
                cues.push("possible_address".into());
            }
            if self
                .partners
                .get(&fragment.speaker.to_lowercase())
                .is_some_and(|at| {
                    at.elapsed() < Duration::from_secs(self.settings.conversation_window_secs)
                })
            {
                cues.push("conversation_follow_up".into());
            }
        }
        // Even the smallest configured window must retain the latest utterance,
        // with an explicit truncation marker instead of evicting it outright.
        let text_limit = 8192.min(
            self.settings
                .recent_bytes
                .saturating_sub(fragment.speaker.len() + 128),
        );
        if fragment.text.len() > text_limit {
            let mut end = text_limit;
            while !fragment.text.is_char_boundary(end) {
                end -= 1;
            }
            fragment.text.truncate(end);
            incomplete = true;
        }
        let speech = Speech {
            id: self.next_id,
            at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            speaker_label: fragment.speaker,
            kind: fragment.kind,
            text: fragment.text,
            self_echo,
            cues,
            incomplete,
        };
        self.next_id += 1;
        self.bytes += size(&speech);
        self.records.push_back(speech);
        while self.records.len() > self.settings.recent_limit
            || self.bytes > self.settings.recent_bytes
        {
            if let Some(old) = self.records.pop_front() {
                self.bytes -= size(&old);
                if old.id > self.handled && !old.cues.is_empty() {
                    self.dropped_unhandled += 1;
                }
            }
        }
    }

    pub fn snapshot(&self) -> u64 {
        self.next_id - 1
    }

    pub fn revision(&self) -> u64 {
        self.records
            .iter()
            .rev()
            .find(|r| !r.cues.is_empty())
            .map_or(0, |r| r.id)
    }

    pub fn has_pending(&self) -> bool {
        self.revision() > self.handled
    }

    pub fn acknowledge(&mut self, through: u64, replied: bool) {
        if replied {
            for record in &self.records {
                if record.id > self.handled && record.id <= through && !record.cues.is_empty() {
                    self.partners
                        .insert(record.speaker_label.to_lowercase(), Instant::now());
                }
            }
        }
        self.partners.retain(|_, at| {
            at.elapsed() < Duration::from_secs(self.settings.conversation_window_secs)
        });
        self.handled = self.handled.max(through);
    }

    pub fn query(&self) -> String {
        self.records
            .iter()
            .rev()
            .filter(|r| r.id > self.handled && !r.cues.is_empty())
            .take(3)
            .map(|r| format!("{} {}", r.speaker_label, r.text))
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(2000)
            .collect()
    }

    pub fn context(&self) -> Value {
        json!({"utterances":self.records, "handled_through":self.handled, "snapshot":self.snapshot(),
            "pending_attention":self.has_pending(), "dropped_unhandled":self.dropped_unhandled,
            "interpretation":"Cues suggest engagement; decide from context. Speaker labels are not proof of known names. OOC/channel speech is not an IC attention trigger."})
    }
}

fn size(speech: &Speech) -> usize {
    speech.text.len() + speech.speaker_label.len() + 128
}

fn mentions(text: &str, phrase: &str) -> bool {
    let words = |s: &str| {
        s.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let text = words(text);
    let phrase = words(phrase);
    !phrase.is_empty() && text.windows(phrase.len()).any(|window| window == phrase)
}

fn parse_start(line: &str) -> Option<Fragment> {
    // Player input echoed by the PTY can itself contain text like "Marco says".
    if line.starts_with('@')
        || ["say ", "pose ", "page ", "whisper "]
            .iter()
            .any(|p| line.starts_with(p))
    {
        return None;
    }
    for (marker, mut kind, quoted) in [
        (" says, ", "say", true),
        (" whispers, ", "whisper", true),
        (" pages: ", "page", false),
    ] {
        if let Some((speaker, text)) = line.split_once(marker) {
            if speaker.is_empty() || speaker.len() > 200 {
                return None;
            }
            if speaker.starts_with("<OOC> ") {
                kind = "ooc";
            } else if speaker.starts_with('[') {
                kind = "channel";
            }
            return Some(Fragment {
                speaker: speaker.into(),
                kind: kind.into(),
                text: text.into(),
                quoted,
            });
        }
    }
    None
}

pub fn is_communication(command: &str) -> bool {
    let verb = command
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_lowercase();
    matches!(
        verb.as_str(),
        "say" | "pose" | "semipose" | "whisper" | "page" | "@emit"
    ) || command.starts_with(['"', ':', ';'])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attention() -> Attention {
        Attention::new(
            Settings::default(),
            &Character {
                name: "Rosebot".into(),
                aliases: vec!["Rosie".into()],
                ..Character::default()
            },
            "account+Rosebot",
        )
    }

    #[test]
    fn speech_survives_room_noise_fragmentation_and_uses_whole_name_tokens() {
        let mut a = attention();
        a.feed("Marco says, \"An annual festival.\"\nMar");
        assert!(!a.has_pending());
        a.feed("co says, \"Rosie, where did\n");
        a.feed("you grow up?\"\n");
        for _ in 0..100 {
            a.feed("A very long room description with no speech.\n");
        }
        assert!(a.has_pending());
        assert_eq!(a.records.len(), 2);
        assert_eq!(a.records[1].text, "Rosie, where did you grow up?");
        assert!(!mentions("annual", "Ann"));
        assert!(mentions("Hey, Rose-bot!", "Rose bot"));
    }

    #[test]
    fn self_echo_ooc_channels_and_acknowledged_mentions_do_not_retrigger() {
        let mut a = attention();
        a.feed("say Marco says, \"Rosebot\"\nRosebot says, \"Rosebot is me.\"\n<OOC> Marco says, \"Rosebot?\"\n[Public] Marco says, \"Rosebot?\"\n");
        assert!(!a.has_pending());
        a.feed("Marco whispers, \"Hello.\"\n");
        let snapshot = a.snapshot();
        assert!(a.has_pending());
        a.acknowledge(snapshot, true);
        assert!(!a.has_pending());
        a.feed("Marco says, \"And your favourite team?\"\n");
        assert!(a.has_pending());
        assert!(
            a.records
                .back()
                .unwrap()
                .cues
                .contains(&"conversation_follow_up".into())
        );
        a.acknowledge(snapshot, false);
        assert!(
            a.has_pending(),
            "an old request cannot consume a later utterance"
        );
    }

    #[test]
    fn bounded_window_retains_latest_long_speech_and_ignoring_a_mention_does_not_start_a_conversation()
     {
        let mut a = Attention::new(
            Settings {
                recent_limit: 4,
                recent_bytes: 4096,
                ..Settings::default()
            },
            &Character::for_login("account+Rosebot"),
            "account+Rosebot",
        );
        a.feed("Marco says, \"I wonder where Rosebot grew up.\"\n");
        a.acknowledge(a.snapshot(), false);
        a.feed("Marco says, \"Ada, do you know?\"\n");
        assert!(
            !a.has_pending(),
            "ignoring overheard conversation must not enlist that speaker as a partner"
        );
        for _ in 0..5 {
            a.feed("Marco says, \"Rosebot?\"\n");
        }
        a.feed(&format!("Marco says, \"Rosebot, {}\"\n", "é".repeat(4096)));
        assert!(a.has_pending());
        assert!(a.bytes <= 4096);
        assert!(a.dropped_unhandled > 0);
        assert!(a.records.back().unwrap().incomplete);
        assert!(a.records.back().unwrap().text.starts_with("Rosebot, "));
    }
}
