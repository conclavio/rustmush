//! One bounded authoritative registry. IDs are never reused, including after revoke.
use std::collections::BTreeMap;

use mush_core::{
    CanonicalId, ModError, Registrar, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId,
};
use serde::{Deserialize, Serialize};

use super::record::{MAX_UNIT_GRANTS, UnitGrant, UnitGrantId, UnitGrantParent, UnitGrantStatus};

pub const FAMILY: &str = "unit_grants";
pub const MAX_REGISTRY_BYTES: usize = 256 * 1024;
/// New grant issuance stops below [`MAX_REGISTRY_BYTES`] so revoking active
/// grants can never be locked out by the serialized storage cap.
pub const MAX_ISSUE_REGISTRY_BYTES: usize = MAX_REGISTRY_BYTES - 4 * 1024;

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
        if self.revision == 0 || self.next_id == 0 || self.grants.len() > MAX_UNIT_GRANTS {
            return Err("invalid unit-grant registry bounds".into());
        }
        for (key, grant) in &self.grants {
            if key != &grant.id || key.0 == 0 || key.0 >= self.next_id {
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

    /// Return a copy with terminal (revoked or expired) grants and any
    /// descendant grants whose parent is no longer live removed. `next_id` is
    /// preserved so pruned identities are never reused.
    #[must_use]
    pub fn pruned(&self, now: i64) -> Self {
        let mut kept = BTreeMap::new();
        for (id, grant) in &self.grants {
            if grant.status != UnitGrantStatus::Active {
                continue;
            }
            if grant.valid_until.is_some_and(|end| now >= end) {
                continue;
            }
            if let UnitGrantParent::UnitGrant { id: parent_id } = grant.parent
                && !kept.contains_key(&parent_id)
            {
                continue;
            }
            kept.insert(*id, grant.clone());
        }
        Self {
            revision: self.revision,
            next_id: self.next_id,
            grants: kept,
        }
    }

    fn prune_revoked(&mut self) {
        let mut kept = BTreeMap::new();
        for (id, grant) in &self.grants {
            if grant.status != UnitGrantStatus::Active {
                continue;
            }
            if let UnitGrantParent::UnitGrant { id: parent_id } = grant.parent
                && !kept.contains_key(&parent_id)
            {
                continue;
            }
            kept.insert(*id, grant.clone());
        }
        self.grants = kept;
    }

    pub fn with_grant(&self, grant: UnitGrant) -> Result<Self, String> {
        let mut next = self.clone();
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or("unit-grant registry revision exhausted")?;
        let is_new = grant.id.0 == next.next_id;
        let is_revoke = !is_new && grant.status == UnitGrantStatus::Revoked;
        if is_new {
            next.next_id = next
                .next_id
                .checked_add(1)
                .ok_or("unit-grant identity exhausted")?;
        } else if !next.grants.contains_key(&grant.id) {
            return Err("unit-grant identity unavailable".into());
        }
        next.grants.insert(grant.id, grant);
        if is_revoke {
            let bytes = serde_json::to_vec(&next).map_err(|e| e.to_string())?.len();
            if bytes > MAX_REGISTRY_BYTES {
                // Exempt revocation from MAX_REGISTRY_BYTES lockout even on a
                // legacy registry filled up to the 256 KiB family ceiling.
                next.prune_revoked();
            }
        }
        next.validate()?;
        if is_new
            && serde_json::to_vec(&next).map_err(|e| e.to_string())?.len()
                > MAX_ISSUE_REGISTRY_BYTES
        {
            return Err("unit-grant registry storage limit reached".into());
        }
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
