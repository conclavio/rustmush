//! Named model selections shared by runner controls and future frontends.
use std::{collections::BTreeMap, io::Read, path::Path};

use crate::{
    Result,
    config::{Config, Model},
};
use serde::{Deserialize, Serialize};

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
            catalog.profiles.insert("configured".into(), model.clone());
            if catalog.initial.is_none() {
                catalog.initial = Some("configured".into());
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
                name == "configured"
                    || name.is_empty()
                    || name.len() > 64
                    || !name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            })
        {
            return Err("invalid model profiles: version 1, 1..64 named profiles and a matching default are required; configured is reserved".into());
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
