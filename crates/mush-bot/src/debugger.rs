//! Runner-side breakpoints; never sends commands or mutates character memories.
use crate::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub after_model: bool,
    pub before_commands: Vec<String>,
    pub observations: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            after_model: false,
            before_commands: Vec::new(),
            observations: Vec::new(),
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Result<Self> {
        let settings: Self = crate::experiment::load_json(path, 16_000)?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || self.before_commands.len() + self.observations.len() > 64
            || self
                .before_commands
                .iter()
                .chain(&self.observations)
                .any(|s| s.is_empty() || s.len() > 1000 || s.chars().any(char::is_control))
        {
            return Err("debug settings require version 1 and at most 64 nonempty single-line patterns, each <= 1000 bytes".into());
        }
        Ok(())
    }

    pub fn command_break(&self, command: Option<&str>) -> bool {
        self.after_model
            || command.is_some_and(|c| self.before_commands.iter().any(|p| c.contains(p)))
    }

    pub fn observation_match<'a>(&'a self, previous: &str, incoming: &str) -> Option<&'a str> {
        self.observations
            .iter()
            .find(|pattern| {
                let overlap = crate::tail(previous, pattern.len().saturating_sub(1));
                format!("{overlap}{incoming}").contains(pattern.as_str())
            })
            .map(String::as_str)
    }
}
