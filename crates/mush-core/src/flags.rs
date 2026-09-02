//! Core object flags, Rhost-style granular powers, and the derived staff
//! [`Role`] hierarchy.

use serde::{Deserialize, Serialize};

use crate::obj::{GOD, ObjId};

bitflags::bitflags! {
    /// Core object bit flags (design doc §2.4).
    ///
    /// `OUTDOORS` / `SHELTERED` are consumed by the weather module;
    /// `AGENT` marks LLM-driven behavior on an object. All three are
    /// reserved here from M0 so later milestones don't renumber bits.
    #[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
    pub struct Flags: u64 {
        /// Full staff (below only God).
        const WIZARD      = 1 << 0;
        /// Senior staff below Wizard.
        const ROYALTY     = 1 << 1;
        /// May build (@dig, @create, ...).
        const BUILDER     = 1 << 2;
        /// Hidden from contents listings and WHO.
        const DARK        = 1 << 3;
        /// Refuses pages.
        const HAVEN       = 1 << 4;
        /// Suppresses set/success spam to the owner.
        const QUIET       = 1 << 5;
        /// Stays where dropped; not swept home.
        const STICKY      = 1 << 6;
        /// Anyone may enter (things acting as vehicles/containers).
        const ENTER_OK    = 1 << 7;
        /// Anyone may teleport in.
        const JUMP_OK     = 1 << 8;
        /// Contents are not visible from outside.
        const OPAQUE      = 1 << 9;
        /// Marked for destruction; removed by the next sweep.
        const GOING       = 1 << 10;
        /// Execution halted (scripts/hooks disabled).
        const HALTED      = 1 << 11;
        /// May not be `@destroy`ed without `/override`.
        const SAFE        = 1 << 12;
        /// This object is a Zone Master Object.
        const ZONE_MASTER = 1 << 13;
        /// `$`-style command hooks on this object are disabled.
        const NO_COMMAND  = 1 << 14;
        /// Room is exposed to the sky (weather applies fully).
        const OUTDOORS    = 1 << 15;
        /// Room is outdoors but shielded (weather text is muted).
        const SHELTERED   = 1 << 16;
        /// Object has LLM agent behavior attached.
        const AGENT       = 1 << 17;
    }
}

bitflags::bitflags! {
    /// Rhost-style granular powers, grantable by Wizard+ independently of
    /// the role hierarchy.
    #[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
    pub struct Powers: u32 {
        /// May `@boot` connections.
        const BOOT         = 1 << 0;
        /// May examine any object regardless of ownership.
        const EXAMINE_ALL  = 1 << 1;
        /// May teleport anywhere.
        const TEL_ANYWHERE = 1 << 2;
        /// May teleport other objects.
        const TEL_ANYTHING = 1 << 3;
        /// May `@chown` any object.
        const CHOWN_ANY    = 1 << 4;
        /// May administer comsys channels.
        const COMSYS_ADMIN = 1 << 5;
        /// May spend LLM budget on `@gen` worldgen.
        const BUILDER_LLM  = 1 << 6;
        /// Sees DARK/hidden objects and players.
        const SEE_HIDDEN   = 1 << 7;
        /// May administer factions (`@faction`, `@region`).
        const FACTION_ADMIN = 1 << 8;
        /// May administer the spatial simulation (`@domain`, `@spatial`).
        const SIM_ADMIN = 1 << 9;
    }
}

/// Serialize bitflags as their raw bits (compact, tolerant of bits that
/// future versions add or retire — unknown bits are retained round-trip).
macro_rules! impl_bits_serde {
    ($ty:ty, $repr:ty) => {
        impl Serialize for $ty {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                self.bits().serialize(s)
            }
        }
        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                Ok(<$ty>::from_bits_retain(<$repr>::deserialize(d)?))
            }
        }
    };
}
pub(crate) use impl_bits_serde;

impl_bits_serde!(Flags, u64);
impl_bits_serde!(Powers, u32);

/// Staff hierarchy derived from identity and flags — never stored.
///
/// `Ord` follows privilege: `Player < Builder < Royalty < Wizard < God`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Role {
    /// No staff bits.
    Player,
    /// `BUILDER` flag.
    Builder,
    /// `ROYALTY` flag.
    Royalty,
    /// `WIZARD` flag.
    Wizard,
    /// Object `#1`, regardless of flags.
    God,
}

impl Role {
    /// Derive the role for an object: `#1` is God; otherwise the highest of
    /// `WIZARD` > `ROYALTY` > `BUILDER`; else `Player`.
    #[must_use]
    pub fn of(id: ObjId, flags: Flags) -> Role {
        if id == GOD {
            Role::God
        } else if flags.contains(Flags::WIZARD) {
            Role::Wizard
        } else if flags.contains(Flags::ROYALTY) {
            Role::Royalty
        } else if flags.contains(Flags::BUILDER) {
            Role::Builder
        } else {
            Role::Player
        }
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Role::Player => "Player",
            Role::Builder => "Builder",
            Role::Royalty => "Royalty",
            Role::Wizard => "Wizard",
            Role::God => "God",
        };
        f.write_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_ordering_is_privilege_order() {
        assert!(Role::Player < Role::Builder);
        assert!(Role::Builder < Role::Royalty);
        assert!(Role::Royalty < Role::Wizard);
        assert!(Role::Wizard < Role::God);
        let mut v = [
            Role::Wizard,
            Role::Player,
            Role::God,
            Role::Builder,
            Role::Royalty,
        ];
        v.sort();
        assert_eq!(
            v,
            [
                Role::Player,
                Role::Builder,
                Role::Royalty,
                Role::Wizard,
                Role::God
            ]
        );
    }

    #[test]
    fn role_derivation() {
        // #1 is God even with no flags at all.
        assert_eq!(Role::of(GOD, Flags::empty()), Role::God);
        // ...and stays God whatever the flags say.
        assert_eq!(Role::of(GOD, Flags::BUILDER), Role::God);

        let id = ObjId(42);
        assert_eq!(Role::of(id, Flags::empty()), Role::Player);
        assert_eq!(Role::of(id, Flags::BUILDER), Role::Builder);
        assert_eq!(Role::of(id, Flags::ROYALTY), Role::Royalty);
        assert_eq!(Role::of(id, Flags::WIZARD), Role::Wizard);
        // Highest wins when several hierarchy bits are set.
        assert_eq!(
            Role::of(id, Flags::WIZARD | Flags::BUILDER | Flags::DARK),
            Role::Wizard
        );
        assert_eq!(Role::of(id, Flags::ROYALTY | Flags::BUILDER), Role::Royalty);
    }

    #[test]
    fn flags_from_name_matches_bit_names() {
        assert_eq!(Flags::from_name("WIZARD"), Some(Flags::WIZARD));
        assert_eq!(Flags::from_name("ZONE_MASTER"), Some(Flags::ZONE_MASTER));
        assert_eq!(Flags::from_name("NOPE"), None);
    }

    #[test]
    fn flags_serde_round_trip() {
        let f = Flags::WIZARD | Flags::DARK | Flags::AGENT;
        let json = serde_json::to_string(&f).unwrap();
        let back: Flags = serde_json::from_str(&json).unwrap();
        assert_eq!(back, f);
        let p = Powers::BOOT | Powers::BUILDER_LLM;
        let back: Powers = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
    }
}
