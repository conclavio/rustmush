//! Registered identifiers for `mush-org` (E1a).
//!
//! Two id families mirror the shipped conventions:
//!
//! - **Integer allocator ids** ([`OrgId`], [`OrgScopeId`], [`RoleId`], [`GrantId`],
//!   [`DelegationId`], [`PolicyId`]) — monotonic `u64`, **never reused**, minted by
//!   a per-module allocator (the `FactionId`/`DomainId` pattern). The allocator +
//!   `meta` resume live in the E1b module.
//! - **Canonical string ids** ([`CapabilityId`], [`SystemPrincipalId`]) — a
//!   validated, canonicalized `SmolStr` (the `ChannelId`/`mobility_id!` pattern):
//!   trim + lowercase over `[a-z0-9_.-]`, so `Org.Role.Manage` and
//!   `org.role.manage` never mismatch. The engine only compares ids; the NAMES
//!   (`org.role.manage`, `economy.account.transfer`) are doctrine, and the *set*
//!   of valid capabilities is a vocabulary frozen at wiring time (E1b), not here.
//!
//! [`PrincipalId`] is deliberately **clean** — `Object(ObjId)` carries no embedded
//! generation. `ObjId` is never recycled (a destroyed object leaves a permanent
//! tombstone), so the "reused id" threat cannot occur; a generation guard, if ever
//! needed for a binding that outlives its object, is a side-table (the F3
//! `control_gen` shape), never part of the id's identity/equality/hash.

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// Define a monotonic integer id newtype (allocator-minted, never reused).
macro_rules! int_id {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub u64);

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

int_id! {
    /// Monotonic organization id; never reused (a closed org's id is retired,
    /// not recycled).
    OrgId
}
int_id! {
    /// Monotonic scope id. A scope is a named node in an org's scope tree that
    /// accounts, vaults, and grants bind to (never a hard-coded "wallet division").
    OrgScopeId
}
int_id! {
    /// Monotonic role-template id.
    RoleId
}
int_id! {
    /// Monotonic capability-grant id.
    GrantId
}
int_id! {
    /// Monotonic delegation id.
    DelegationId
}
int_id! {
    /// Monotonic access-policy id.
    PolicyId
}

/// Define a canonical, validated string id newtype (the `ChannelId` pattern).
macro_rules! canon_id {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "SmolStr", into = "SmolStr")]
        pub struct $name(SmolStr);

        impl $name {
            /// Validate + canonicalize (trim + lowercase); `None` if empty or
            /// containing a character outside `[a-z0-9_.-]`.
            #[must_use]
            pub fn new(s: impl AsRef<str>) -> Option<$name> {
                let t = s.as_ref().trim().to_ascii_lowercase();
                if t.is_empty()
                    || !t.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.' || b == b'-'
                    })
                {
                    return None;
                }
                Some($name(SmolStr::new(t)))
            }
            /// The canonical id text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl TryFrom<SmolStr> for $name {
            type Error = String;
            fn try_from(s: SmolStr) -> Result<$name, String> {
                $name::new(&s).ok_or_else(|| format!("invalid {} {s:?}", stringify!($name)))
            }
        }
        impl From<$name> for SmolStr {
            fn from(v: $name) -> SmolStr {
                v.0
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}({:?})", stringify!($name), self.0)
            }
        }
    };
}

canon_id! {
    /// A semantic capability the authorization engine checks (e.g. `org.role.manage`,
    /// `economy.account.transfer`). The set of valid capabilities is a vocabulary
    /// frozen at wiring time; this newtype only guarantees canonical form.
    CapabilityId
}
canon_id! {
    /// A registered system principal (a faucet, sink, escrow authority, or clearing
    /// account owner). Registered at boot by a module (E2); an arbitrary string from
    /// a command is never accepted as one.
    SystemPrincipalId
}

/// An economic principal — the answer to "who owns / who acts".
///
/// `Object` covers characters and NPCs (and staff-approved estate/trust objects);
/// `Organization` is the normal shared owner; `System` represents explicit
/// faucets, sinks, escrow, and clearing accounts. `Object` is **clean** — no
/// embedded generation (see the module note).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PrincipalId {
    /// A character/NPC/object principal.
    Object(ObjId),
    /// An organization principal.
    Organization(OrgId),
    /// A registered system principal (faucet/sink/escrow/clearing).
    System(SystemPrincipalId),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_ids_are_transparent_and_ordered() {
        let a = OrgId(1);
        let b = OrgId(2);
        assert!(a < b);
        assert_eq!(serde_json::to_string(&a).unwrap(), "1");
        assert_eq!(serde_json::from_str::<OrgId>("7").unwrap(), OrgId(7));
        assert_eq!(a.to_string(), "1");
    }

    #[test]
    fn capability_id_canonicalizes_and_round_trips() {
        let c = CapabilityId::new("  Org.Role.Manage ").unwrap();
        assert_eq!(c.as_str(), "org.role.manage");
        // canonical round-trip through serde
        let j = serde_json::to_string(&c).unwrap();
        assert_eq!(j, "\"org.role.manage\"");
        assert_eq!(serde_json::from_str::<CapabilityId>(&j).unwrap(), c);
        // rejects out-of-charset + empty
        assert!(CapabilityId::new("bad space").is_none());
        assert!(CapabilityId::new("").is_none());
        // a corrupt serialized id fails deserialization (try_from)
        assert!(serde_json::from_str::<CapabilityId>("\"bad space\"").is_err());
    }

    #[test]
    fn principal_id_object_is_clean_and_hashable() {
        use std::collections::HashSet;
        let mut s = HashSet::new();
        s.insert(PrincipalId::Object(ObjId(5)));
        s.insert(PrincipalId::Organization(OrgId(5)));
        s.insert(PrincipalId::System(SystemPrincipalId::new("shadowrun.contracts").unwrap()));
        assert_eq!(s.len(), 3, "the three principal kinds are distinct even at the same numeric id");
        // Object equality is by id alone (no generation) — round-trips through serde.
        let p = PrincipalId::Object(ObjId(5));
        assert_eq!(serde_json::from_str::<PrincipalId>(&serde_json::to_string(&p).unwrap()).unwrap(), p);
    }
}
