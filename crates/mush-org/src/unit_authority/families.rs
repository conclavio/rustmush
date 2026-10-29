//! One bounded authoritative registry. IDs are never reused, including after revoke.
use std::collections::BTreeMap;

use mush_core::{
    CanonicalId, ModError, Registrar, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId,
};
use serde::{Deserialize, Serialize};

use super::record::{MAX_UNIT_GRANTS, UnitGrant, UnitGrantId, UnitGrantParent};

pub const FAMILY: &str = "unit_grants";
pub const MAX_REGISTRY_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RegistryWire", into = "RegistryWire")]
pub struct UnitGrantRegistry {
    pub revision: u64,
    pub next_id: u64,
    pub grants: BTreeMap<UnitGrantId, UnitGrant>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryWire {
    format: u16,
    revision: u64,
    next_id: u64,
    grants: Vec<UnitGrant>,
}

impl Default for UnitGrantRegistry {
    fn default() -> Self {
        Self {
            revision: 1,
            next_id: 1,
            grants: BTreeMap::new(),
        }
    }
}

impl UnitGrantRegistry {
    pub fn validate(&self) -> Result<(), String> {
        if self.revision == 0
            || self.grants.len() > MAX_UNIT_GRANTS
            || usize::try_from(self.next_id).ok() != self.grants.len().checked_add(1)
        {
            return Err("invalid unit-grant registry bounds".into());
        }
        for (ordinal, (key, grant)) in self.grants.iter().enumerate() {
            if key != &grant.id || usize::try_from(key.0).ok() != ordinal.checked_add(1) {
                return Err("invalid unit-grant registry identity".into());
            }
            grant.validate().map_err(|e| e.to_string())?;
            if let UnitGrantParent::UnitGrant { id } = grant.parent
                && (id >= grant.id || !self.grants.contains_key(&id))
            {
                return Err("invalid unit-grant parent identity".into());
            }
        }
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > MAX_REGISTRY_BYTES {
            return Err("unit-grant registry storage limit reached".into());
        }
        Ok(())
    }

    pub fn with_grant(&self, grant: UnitGrant) -> Result<Self, String> {
        let mut next = self.clone();
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or("unit-grant registry revision exhausted")?;
        if grant.id.0 == next.next_id {
            next.next_id = next
                .next_id
                .checked_add(1)
                .ok_or("unit-grant identity exhausted")?;
        } else if !next.grants.contains_key(&grant.id) {
            return Err("unit-grant identity unavailable".into());
        }
        next.grants.insert(grant.id, grant);
        next.validate()?;
        Ok(next)
    }
}

impl TryFrom<RegistryWire> for UnitGrantRegistry {
    type Error = String;
    fn try_from(wire: RegistryWire) -> Result<Self, String> {
        if wire.format != 1
            || wire.grants.len() > MAX_UNIT_GRANTS
            || wire
                .grants
                .windows(2)
                .any(|pair| matches!(pair, [a, b] if a.id >= b.id))
        {
            return Err("invalid unit-grant registry encoding".into());
        }
        let value = Self {
            revision: wire.revision,
            next_id: wire.next_id,
            grants: wire.grants.into_iter().map(|g| (g.id, g)).collect(),
        };
        value.validate()?;
        Ok(value)
    }
}

impl From<UnitGrantRegistry> for RegistryWire {
    fn from(value: UnitGrantRegistry) -> Self {
        Self {
            format: 1,
            revision: value.revision,
            next_id: value.next_id,
            grants: value.grants.into_values().collect(),
        }
    }
}

pub fn register(
    reg: &mut Registrar<'_>,
) -> Result<StateFamilyHandle<(), UnitGrantRegistry>, ModError> {
    let id = |s: &str| CanonicalId::new(s).map_err(|e| ModError::msg(e.to_string()));
    let descriptor = StateFamilyDescriptor::new(
        StateFamilyId::new(FAMILY).map_err(|e| ModError::msg(e.to_string()))?,
        id("unit_grants.key")?,
        id("unit_grants.value")?,
        1,
        id("unit_grants.decode")?,
        id("unit_grants.check")?,
        id("unit_grants.encode")?,
    )
    .and_then(|d| d.with_limits(8, MAX_REGISTRY_BYTES))
    .map_err(|e| ModError::msg(e.to_string()))?;
    reg.state_family(
        descriptor,
        |_: &()| Ok("registry".to_owned()),
        |v: &serde_json::Value| serde_json::from_value(v.clone()).map_err(|e| e.to_string()),
        UnitGrantRegistry::validate,
        |v: &UnitGrantRegistry| serde_json::to_value(v).map_err(|e| e.to_string()),
    )
}
