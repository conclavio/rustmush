//! Finite public legal-offer transport. These values select no provider or
//! mechanics definition; only core's regenerated offer can authorize execution.
use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, de};

use crate::{ActionStatusId, ProjectedCommitToken};

/// Same public-input ceilings as the authoritative action kernel.
pub const MAX_LEGAL_ACTION_PARAMETERS: usize = 32;
/// Maximum encoded public parameter object before conversion to kernel input.
pub const MAX_LEGAL_ACTION_PARAMETER_BYTES: usize = 64 * 1024;
/// Maximum UTF-8 bytes in one text or choice input.
pub const MAX_LEGAL_ACTION_TEXT_BYTES: usize = 4 * 1024;

/// An opaque projected offer, never an action definition or authorization cache.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ProjectedOfferId(String);
impl ProjectedOfferId {
    pub fn new(value: String) -> Result<Self, &'static str> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("offer must be 64 lowercase hexadecimal digits");
        }
        Ok(Self(value))
    }
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl<'de> Deserialize<'de> for ProjectedOfferId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(d)?).map_err(de::Error::custom)
    }
}

/// Exact i64 input transported as decimal text, never rounded by JavaScript.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct LegalActionInteger(String);
impl LegalActionInteger {
    pub fn new(value: String) -> Result<Self, &'static str> {
        if value.len() > 20 {
            return Err("integer is out of range");
        }
        let number = value
            .parse::<i64>()
            .map_err(|_| "integer is out of range")?;
        if number.to_string() != value {
            return Err("integer must be canonical decimal text");
        }
        Ok(Self(value))
    }
    pub fn value(&self) -> Result<i64, &'static str> {
        self.0.parse().map_err(|_| "integer is out of range")
    }
}
impl<'de> Deserialize<'de> for LegalActionInteger {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(d)?).map_err(de::Error::custom)
    }
}

/// Only scalar values supported by the projected parameter vocabulary.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LegalActionScalar {
    Boolean {
        value: bool,
    },
    Integer {
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        value: LegalActionInteger,
    },
    Text {
        value: String,
    },
}
impl LegalActionScalar {
    fn json(&self) -> Result<serde_json::Value, &'static str> {
        match self {
            Self::Boolean { value } => Ok((*value).into()),
            Self::Integer { value } => Ok(value.value()?.into()),
            Self::Text { value }
                if value.len() <= MAX_LEGAL_ACTION_TEXT_BYTES
                    && !value.chars().any(char::is_control) =>
            {
                Ok(value.clone().into())
            }
            Self::Text { .. } => Err("text is too long or contains a control character"),
        }
    }
}

/// Bounded map with duplicate-key rejection before any Value conversion.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(transparent)]
pub struct LegalActionParameters(BTreeMap<String, LegalActionScalar>);
impl LegalActionParameters {
    pub fn new(values: BTreeMap<String, LegalActionScalar>) -> Result<Self, &'static str> {
        let result = Self(values);
        result.to_json()?;
        Ok(result)
    }
    pub fn to_json(&self) -> Result<serde_json::Value, &'static str> {
        if self.0.len() > MAX_LEGAL_ACTION_PARAMETERS {
            return Err("too many parameters");
        }
        let mut values = serde_json::Map::new();
        for (name, value) in &self.0 {
            if name.is_empty()
                || name.len() > 96
                || name.starts_with('.')
                || name.ends_with('.')
                || name.contains("..")
                || !name.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'.')
                })
            {
                return Err("invalid parameter name");
            }
            values.insert(name.clone(), value.json()?);
        }
        // Include the typed framing in the transport cap as well as checking
        // the resulting kernel object. Neither encoding may exceed the bound.
        for bytes in [serde_json::to_vec(self), serde_json::to_vec(&values)] {
            if bytes.map_err(|_| "invalid parameters")?.len() > MAX_LEGAL_ACTION_PARAMETER_BYTES {
                return Err("parameters exceed 64 KiB");
            }
        }
        Ok(values.into())
    }
}
impl<'de> Deserialize<'de> for LegalActionParameters {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Parameters;
        impl<'de> de::Visitor<'de> for Parameters {
            type Value = LegalActionParameters;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a bounded object of distinct public scalar parameters")
            }
            fn visit_map<M: de::MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut values = BTreeMap::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.len() == MAX_LEGAL_ACTION_PARAMETERS || values.contains_key(&key) {
                        return Err(de::Error::custom("too many or duplicate parameters"));
                    }
                    values.insert(key, map.next_value::<LegalActionScalar>()?);
                }
                LegalActionParameters::new(values).map_err(de::Error::custom)
            }
        }
        d.deserialize_map(Parameters)
    }
}

/// Gateway-owned operations. The server receives an ordinary opaque offer
/// submission or authenticated status lookup, never this as a module invoke.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum LegalActionRequest {
    SubmitSelf {
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        offer: ProjectedOfferId,
        parameters: LegalActionParameters,
    },
    Status {
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        reference: ActionStatusId,
    },
}
impl LegalActionRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::SubmitSelf { parameters, .. } => parameters.to_json().map(|_| ()),
            Self::Status { .. } => Ok(()),
        }
    }
}

/// Correlated public result. Queued means transport custody only. Unavailable
/// cannot prove absence and must never cause automatic fresh submission.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum LegalActionResponse {
    Queued {},
    Owned {
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        reference: ActionStatusId,
    },
    Committed {
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        reference: ActionStatusId,
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        commit: ProjectedCommitToken,
    },
    Pending {
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        reference: ActionStatusId,
    },
    Unavailable {
        #[cfg_attr(feature = "ts", ts(type = "string | null"))]
        reference: Option<ActionStatusId>,
    },
    Unknown {},
    Expired {},
    Busy {},
    Rejected {},
    Invalid {},
}

#[cfg(test)]
mod tests;
