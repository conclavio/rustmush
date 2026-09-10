use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Result;

#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub character: Option<PathBuf>,
    pub affiliations: Option<PathBuf>,
    pub model_profiles: Option<PathBuf>,
    pub model_profile: Option<String>,
    pub prompts: Option<PathBuf>,
    pub context: Option<PathBuf>,
    pub debug: Option<PathBuf>,
    pub budget: Option<PathBuf>,
    pub model_tape: Option<PathBuf>,
    pub costs: crate::costs::Settings,
    pub attention: crate::attention::Settings,
    pub ssh: Ssh,
    pub model: Option<Model>,
    pub agent: Agent,
    pub memory: Memory,
    pub knowledge: Vec<PathBuf>,
}

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Memory {
    pub persistent: bool,
    pub directory: PathBuf,
    pub file: Option<PathBuf>,
    /// Separate campaigns/world resets even when the SSH character is reused.
    pub namespace: String,
    pub recall_bytes: usize,
    pub max_bytes: u64,
}

impl Default for Memory {
    fn default() -> Self {
        Self {
            persistent: true,
            directory: ".mush-bot/memory".into(),
            file: None,
            namespace: "default".into(),
            recall_bytes: 24_000,
            max_bytes: 64_000_000,
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Ssh {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password_env: Option<String>,
    pub identity_file: Option<PathBuf>,
    pub key_password_env: Option<String>,
    pub known_hosts: Option<PathBuf>,
    pub host_key_sha256: Option<String>,
    pub timeout_secs: u64,
}

impl Default for Ssh {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 2222,
            username: "play".into(),
            password_env: None,
            identity_file: None,
            key_password_env: None,
            known_hosts: None,
            host_key_sha256: None,
            timeout_secs: 15,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    /// Full chat-completions endpoint, including /v1/chat/completions.
    pub endpoint: String,
    pub name: String,
    pub api_key_env: Option<String>,
    #[serde(default = "model_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "output_tokens")]
    pub max_tokens: u32,
    pub temperature: Option<f64>,
    /// Currency of usage.cost for endpoints other than OpenRouter.
    pub cost_currency: Option<String>,
}

impl Model {
    pub fn validate(&self) -> Result<()> {
        let url = reqwest::Url::parse(&self.endpoint)?;
        if !matches!(url.scheme(), "http" | "https")
            || self.name.trim().is_empty()
            || self.name.len() > 200
            || self.name.chars().any(char::is_control)
            || self.timeout_secs == 0
            || self.timeout_secs > 600
            || self.max_tokens == 0
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || self
                .temperature
                .is_some_and(|t| !t.is_finite() || !(0.0..=2.0).contains(&t))
            || self
                .cost_currency
                .as_ref()
                .is_some_and(|c| c.len() != 3 || !c.bytes().all(|b| b.is_ascii_uppercase()))
        {
            return Err("invalid model endpoint/settings; use api_key_env for credentials".into());
        }
        Ok(())
    }

    pub fn currency(&self) -> Option<String> {
        self.cost_currency.clone().or_else(|| {
            let url = reqwest::Url::parse(&self.endpoint).ok()?;
            (url.host_str() == Some("openrouter.ai")).then(|| "USD".into())
        })
    }
}

fn model_timeout() -> u64 {
    90
}
fn output_tokens() -> u32 {
    2048
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Agent {
    pub role: String,
    pub goal: String,
    pub max_steps: usize,
    pub max_seconds: u64,
    pub context_bytes: usize,
    pub settle_ms: u64,
    pub observation_ms: u64,
    pub bootstrap: Vec<String>,
}

impl Default for Agent {
    fn default() -> Self {
        Self {
            role: "An ordinary player exploring the world with curiosity and courtesy.".into(),
            goal: String::new(),
            max_steps: 100,
            max_seconds: 900,
            context_bytes: 32_000,
            settle_ms: 500,
            observation_ms: 3000,
            bootstrap: vec!["help".into(), "look".into()],
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let mut config: Self = toml::from_str(&std::fs::read_to_string(path)?)?;
        let base = path.parent().unwrap_or(Path::new("."));
        for path in config
            .knowledge
            .iter_mut()
            .chain(config.ssh.identity_file.iter_mut())
            .chain(config.ssh.known_hosts.iter_mut())
            .chain(std::iter::once(&mut config.memory.directory))
            .chain(config.memory.file.iter_mut())
            .chain(config.character.iter_mut())
            .chain(config.affiliations.iter_mut())
            .chain(config.model_profiles.iter_mut())
            .chain(config.prompts.iter_mut())
            .chain(config.context.iter_mut())
            .chain(config.debug.iter_mut())
            .chain(config.budget.iter_mut())
            .chain(config.model_tape.iter_mut())
            .chain(std::iter::once(&mut config.costs.directory))
        {
            if path.is_relative() {
                *path = base.join(&*path);
            }
        }
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        self.attention.validate()?;
        self.costs.validate()?;
        let a = &self.agent;
        if self.memory.namespace.trim().is_empty()
            || self.memory.namespace.len() > 200
            || !(4096..=128_000).contains(&self.memory.recall_bytes)
            || !(1_000_000..=1_000_000_000).contains(&self.memory.max_bytes)
        {
            return Err("invalid memory namespace or limits (see mush-bot.toml.example)".into());
        }
        if self.ssh.host.is_empty()
            || self.ssh.username.is_empty()
            || self.ssh.username.len() > 255
            || self.ssh.username.chars().any(char::is_control)
            || self.ssh.port == 0
            || self.ssh.timeout_secs == 0
            || a.max_steps == 0
            || a.max_seconds == 0
            || !(4096..=1_000_000).contains(&a.context_bytes)
            || a.settle_ms == 0
            || a.observation_ms < a.settle_ms
            || a.observation_ms > 60_000
        {
            return Err("invalid SSH settings or agent limits (see mush-bot.toml.example)".into());
        }
        if let Some(m) = &self.model {
            m.validate()?;
        }
        for line in &a.bootstrap {
            crate::transport::validate_command(line)?;
        }
        Ok(())
    }
}

pub fn secret(name: &Option<String>) -> Result<Option<String>> {
    name.as_ref()
        .map(|name| {
            std::env::var(name)
                .map_err(|_| format!("required environment variable {name} is unset").into())
        })
        .transpose()
}
