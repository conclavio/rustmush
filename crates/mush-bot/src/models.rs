//! Named model selections shared by runner controls and future frontends.
use std::{collections::BTreeMap, io::Read, path::Path};

use crate::{
    Result,
    config::{Config, Model},
};
use serde::{Deserialize, Serialize};

/// The profile name the TOML `[model]` is recorded under.
pub const CONFIGURED: &str = "configured";
/// The profile name `[arena.commander_model]` is recorded under in the cost
/// ledger and the transcript (cut AJ.3).
pub const ARENA_COMMANDER: &str = "arena_commander";
/// Names a profile file may not take: each is the recorded name of a model
/// that comes from the TOML, and a profile sharing it would merge its cost
/// rows with that model's.
pub const RESERVED: [&str; 2] = [CONFIGURED, ARENA_COMMANDER];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub profile: String,
    pub settings: Model,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    version: u32,
    default: String,
    profiles: BTreeMap<String, Model>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    profiles: BTreeMap<String, Model>,
    initial: Option<String>,
}

impl Catalog {
    pub fn from_config(config: &Config) -> Result<Self> {
        let mut catalog = Self {
            profiles: BTreeMap::new(),
            initial: None,
        };
        if let Some(path) = &config.model_profiles {
            let source = Self::read(path)?;
            catalog.initial = Some(source.default);
            catalog.profiles = source.profiles;
        }
        if let Some(model) = &config.model {
            catalog.profiles.insert(CONFIGURED.into(), model.clone());
            if catalog.initial.is_none() {
                catalog.initial = Some(CONFIGURED.into());
            }
        }
        if let Some(profile) = &config.model_profile {
            catalog.initial = Some(profile.clone());
        }
        if let Some(profile) = &catalog.initial {
            catalog.select(profile)?;
        }
        Ok(catalog)
    }

    fn read(path: &Path) -> Result<Source> {
        if !path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("json"))
        {
            return Err("model profiles extension must be .json".into());
        }
        let mut text = String::new();
        std::fs::File::open(path)?
            .take(128_001)
            .read_to_string(&mut text)?;
        if text.len() > 128_000 {
            return Err("model profiles exceed 128 KB".into());
        }
        // Parse errors can include an accidental inline secret. Do not echo values.
        let source: Source = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|_| "invalid model profiles JSON; check field names and types")?;
        if source.version != 1
            || source.profiles.is_empty()
            || source.profiles.len() > 64
            || !source.profiles.contains_key(&source.default)
            || source.profiles.keys().any(|name| {
                RESERVED.contains(&name.as_str())
                    || name.is_empty()
                    || name.len() > 64
                    || !name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            })
        {
            return Err("invalid model profiles: version 1, 1..64 named profiles and a matching default are required; configured and arena_commander are reserved".into());
        }
        for model in source.profiles.values() {
            model.validate()?;
        }
        Ok(source)
    }

    pub fn initial(&self) -> Result<Option<Selection>> {
        self.initial
            .as_deref()
            .map(|name| self.select(name))
            .transpose()
    }

    pub fn select(&self, name: &str) -> Result<Selection> {
        let settings = self
            .profiles
            .get(name)
            .ok_or("unknown model profile; use /model list")?;
        Ok(Selection {
            profile: name.into(),
            settings: settings.clone(),
        })
    }

    pub fn list(&self) -> Vec<Selection> {
        self.profiles
            .iter()
            .map(|(profile, settings)| Selection {
                profile: profile.clone(),
                settings: settings.clone(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A profiles file in its own directory.
    fn profiles(name: &str, body: serde_json::Value) -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let directory = std::env::temp_dir().join(format!(
            "mush-bot-models-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).expect("directory");
        let path = directory.join(format!("{name}.json"));
        std::fs::write(&path, body.to_string()).expect("profiles");
        path
    }

    fn catalog(path: std::path::PathBuf) -> Result<Catalog> {
        let config = Config {
            model_profiles: Some(path),
            ..Config::default()
        };
        Catalog::from_config(&config)
    }

    #[test]
    fn a_json_profile_carries_the_pass_through_objects() {
        let path = profiles(
            "routed",
            json!({"version":1,"default":"flash","profiles":{"flash":{
                "endpoint":"https://openrouter.ai/api/v1/chat/completions",
                "name":"deepseek/deepseek-v4.1-flash","max_tokens":200,
                "reasoning":{"enabled":false},
                "provider":{"order":["Together"],"allow_fallbacks":false}}}}),
        );
        let selection = catalog(path)
            .expect("catalog")
            .initial()
            .expect("select")
            .expect("a selection");
        assert_eq!(
            serde_json::to_value(&selection.settings).expect("json")["provider"],
            json!({"order":["Together"],"allow_fallbacks":false})
        );
        assert_eq!(
            serde_json::to_value(&selection.settings).expect("json")["reasoning"],
            json!({"enabled":false})
        );
        let deep = profiles(
            "deep",
            json!({"version":1,"default":"deep","profiles":{"deep":{
                "endpoint":"http://127.0.0.1","name":"m",
                "reasoning":{"a":{"b":{"c":{"d":{}}}}}}}}),
        );
        assert!(catalog(deep).is_err());
    }

    #[test]
    fn a_profile_may_not_take_a_reserved_name() {
        for reserved in RESERVED {
            let path = profiles(
                reserved,
                json!({"version":1,"default":reserved,"profiles":{reserved:{
                    "endpoint":"http://127.0.0.1","name":"m"}}}),
            );
            assert_eq!(
                catalog(path).err().expect("refused").to_string(),
                "invalid model profiles: version 1, 1..64 named profiles and a matching default are required; configured and arena_commander are reserved",
                "{reserved}"
            );
        }
        let fine = profiles(
            "fine",
            json!({"version":1,"default":"arena","profiles":{"arena":{
                "endpoint":"http://127.0.0.1","name":"m"}}}),
        );
        assert!(catalog(fine).is_ok());
    }
}
