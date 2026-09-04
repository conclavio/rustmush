//! Housing identifiers (E5a).
//!
//! Two allocator ids in the shipped `OrgId`/`ObligationId` shape — monotonic `u64`,
//! **never reused**, minted by the E5b module's own allocator — plus one deliberately
//! opaque reference.
//!
//! [`SceneRef`] is *not* `mush_core::SceneId`, and the difference is the point.
//! Core's scene ids are transient: "scenes are transient world state and scene ids
//! restart at 0 every boot" (`mush-core/src/module.rs`). A `SceneId` persisted inside
//! a [`crate::StoryWindow`] would therefore name a **different** scene after the next
//! restart — the story window's scene would silently become somebody else's. So the
//! pure library stores an opaque `u64` it never interprets, and E5b owns the mapping
//! (and the staleness rule: a `SceneRef` minted before the current boot resolves to
//! nothing, it does not resolve to scene #0). Keeping the newtype here also keeps
//! `mush-housing` free of a `mush-scenes` edge.

use serde::{Deserialize, Serialize};

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
    /// Monotonic property id; never reused. A property is a staff/builder-built
    /// ordinary room or room group (owner answer 5, D-E5-12), not a `@dig`
    /// permission.
    PropertyId
}
int_id! {
    /// Monotonic tenancy id; never reused. A tenancy is the exact, durable record of
    /// who lives somewhere; a retired one is closed, never recycled.
    TenancyId
}
int_id! {
    /// An opaque handle on the scene a story window optionally opened.
    ///
    /// Deliberately **not** `mush_core::SceneId`: core scene ids restart at 0 at every
    /// boot, so a persisted one would name a different scene after a restart. E5b maps
    /// this to whatever it can still resolve, and to nothing when it cannot.
    SceneRef
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_ids_transparent_and_ordered() {
        assert!(PropertyId(1) < PropertyId(2));
        assert_eq!(serde_json::to_string(&TenancyId(7)).unwrap(), "7");
        assert_eq!(serde_json::from_str::<SceneRef>("9").unwrap(), SceneRef(9));
        assert_eq!(PropertyId(3).to_string(), "3");
    }
}
