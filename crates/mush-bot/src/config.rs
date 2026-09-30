use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Result;

#[derive(Clone, Default, Deserialize, Serialize)]
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
    pub conversation: Option<PathBuf>,
    pub activities: Option<PathBuf>,
    pub mission: Option<PathBuf>,
    pub playtest_feedback: bool,
    pub model_tape: Option<PathBuf>,
    pub costs: crate::costs::Settings,
    pub attention: crate::attention::Settings,
    pub ssh: Ssh,
    pub model: Option<Model>,
    pub agent: Agent,
    pub memory: Memory,
    pub knowledge: Vec<PathBuf>,
    /// The simulator pilot's table (cut AJ.2): `--arena` and `/arena`.
    pub arena: crate::arena::settings::Settings,
}

#[derive(Clone, Deserialize, Serialize)]
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

#[derive(Clone, Deserialize, Serialize)]
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

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
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
    /// Passed through verbatim as the request's `reasoning` object (cut AJ.3):
    /// OpenRouter's `{ enabled = false }` keeps a reasoning model from spending
    /// its whole token budget thinking. Bounded by [`Model::validate`] and part
    /// of every request's recorded revision. Skipped when unset, so every
    /// earlier ledger line, profile, capture and report reads unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<serde_json::Map<String, serde_json::Value>>,
    /// Passed through verbatim as the request's `provider` object (cut AJ.3):
    /// OpenRouter's routing, e.g. `{ order = ["Together"], allow_fallbacks =
    /// false }`. Bounded and recorded as `reasoning` is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<serde_json::Map<String, serde_json::Value>>,
}

/// The most a `reasoning` or `provider` object may serialise to.
pub const EXTRA_BODY_MAX_BYTES: usize = 2_048;
/// The deepest a `reasoning` or `provider` object may nest (objects and
/// arrays; the object itself is level 1).
pub const EXTRA_BODY_MAX_DEPTH: usize = 4;

/// How deep `value` nests: 0 for a scalar, 1 for an object or array of
/// scalars, one more for each level inside.
fn nesting(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Object(map) => 1 + map.values().map(nesting).max().unwrap_or(0),
        serde_json::Value::Array(items) => 1 + items.iter().map(nesting).max().unwrap_or(0),
        _ => 0,
    }
}

/// A `reasoning` or `provider` value a request may carry: a JSON object of at
/// most [`EXTRA_BODY_MAX_BYTES`] serialised, nested at most
/// [`EXTRA_BODY_MAX_DEPTH`] deep. [`Model::validate`] and
/// `model::Capture::validate` both hold a request to it.
pub fn extra_body_ok(value: &serde_json::Value) -> bool {
    value.is_object()
        && serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= EXTRA_BODY_MAX_BYTES)
        && nesting(value) <= EXTRA_BODY_MAX_DEPTH
}

impl Model {
    /// Insert the pass-through `reasoning` and `provider` objects into a
    /// request payload, when set. Every request the bot builds calls it after
    /// the temperature: the goal loop's (`Brain::prepare`), the studio's and
    /// the arena commander's; `Capture::with_model` re-inserts them.
    pub fn extra_body(&self, payload: &mut serde_json::Value) {
        if let Some(reasoning) = &self.reasoning {
            payload["reasoning"] = serde_json::Value::Object(reasoning.clone());
        }
        if let Some(provider) = &self.provider {
            payload["provider"] = serde_json::Value::Object(provider.clone());
        }
    }

    pub fn validate(&self) -> Result<()> {
        if [&self.reasoning, &self.provider].into_iter().any(|extra| {
            extra
                .as_ref()
                .is_some_and(|map| !extra_body_ok(&serde_json::Value::Object(map.clone())))
        }) {
            return Err("invalid model reasoning/provider: each must be a JSON object of at most 2048 bytes, nested at most 4 deep".into());
        }
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
            .chain(config.conversation.iter_mut())
            .chain(config.activities.iter_mut())
            .chain(config.mission.iter_mut())
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
        self.arena.validate()?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    const ROUTED: &str = "endpoint = \"https://openrouter.ai/api/v1/chat/completions\"\n\
        name = \"deepseek/deepseek-v4.1-flash\"\n\
        reasoning = { enabled = false }\n\
        provider = { order = [\"Together\"], allow_fallbacks = false }\n";

    fn with(key: &str, value: Value) -> Model {
        serde_json::from_value(json!({"endpoint":"http://127.0.0.1","name":"m", key: value}))
            .expect("a model")
    }

    #[test]
    fn the_pass_through_objects_read_from_toml_inline_tables() {
        let model: Model = toml::from_str(ROUTED).expect("a model");
        model.validate().expect("valid");
        assert_eq!(
            Value::Object(model.reasoning.clone().expect("reasoning")),
            json!({"enabled": false})
        );
        assert_eq!(
            Value::Object(model.provider.clone().expect("provider")),
            json!({"order": ["Together"], "allow_fallbacks": false})
        );
        let mut payload = json!({"model": "m"});
        model.extra_body(&mut payload);
        assert_eq!(payload["reasoning"], json!({"enabled": false}));
        assert_eq!(payload["provider"]["order"], json!(["Together"]));
        // Unset, a payload carries neither.
        let bare: Model =
            toml::from_str("endpoint = \"http://127.0.0.1\"\nname = \"m\"\n").expect("a model");
        let mut payload = json!({"model": "m"});
        bare.extra_body(&mut payload);
        assert_eq!(payload, json!({"model": "m"}));
    }

    #[test]
    fn a_pass_through_object_is_bounded_in_bytes_and_depth() {
        const REFUSAL: &str = "invalid model reasoning/provider: each must be a JSON object of at most 2048 bytes, nested at most 4 deep";
        for key in ["reasoning", "provider"] {
            // `{"k":"…"}` is eight bytes around the string.
            let fits = with(key, json!({"k": "x".repeat(2_040)}));
            assert_eq!(
                serde_json::to_vec(&fits.reasoning.clone().or(fits.provider.clone()))
                    .unwrap()
                    .len(),
                2_048
            );
            fits.validate().expect("2048 bytes");
            let over = with(key, json!({"k": "x".repeat(2_041)}));
            assert_eq!(over.validate().expect_err("2049").to_string(), REFUSAL);
            with(key, json!({"a": {"b": {"c": {"d": 1}}}}))
                .validate()
                .expect("depth 4");
            with(key, json!({"a": [[[1]]]}))
                .validate()
                .expect("depth 4 by arrays");
            for deep in [
                json!({"a": {"b": {"c": {"d": {}}}}}),
                json!({"a": [[[[1]]]]}),
            ] {
                assert_eq!(
                    with(key, deep).validate().expect_err("depth 5").to_string(),
                    REFUSAL
                );
            }
            // Not an object: refused as the value is read.
            for bad in [json!("off"), json!([1]), json!(true)] {
                assert!(
                    serde_json::from_value::<Model>(
                        json!({"endpoint":"http://127.0.0.1","name":"m", key: bad})
                    )
                    .is_err()
                );
            }
            assert!(
                toml::from_str::<Model>(&format!(
                    "endpoint = \"http://127.0.0.1\"\nname = \"m\"\n{key} = \"off\"\n"
                ))
                .is_err()
            );
        }
        assert!(!extra_body_ok(&json!(["not", "an", "object"])));
        assert!(extra_body_ok(&json!({})));
    }
}
