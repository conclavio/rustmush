//! **What an author's own sensors earned in the fiction** (doc 17h cut M8g G3; D-UM-55: a quirk
//! claim rests only on what the author's own sensors earned in the field, the observer rule).
//!
//! **No producer yet** (the lead's G3 ruling, branch (B)): a claim is IC testimony, so only IC
//! contacts count, and no IC detection exists until the fight (16c H1). Every arena mode
//! today (`skirmish`, `hold`, `range`) is a SIM match — OOC training whose contacts are not
//! earned in the fiction — so nothing writes this index, and every hull an author names is
//! refused with the observer rule's one sentence. The fight adds the producer here: a
//! per-character ring of `EARNED_PER_AUTHOR` hulls, oldest out, fed from the author's own
//! `Detected { viewer == me }` in an IC fight, persisted beside its RAM mirror. The same
//! deferral as testimony (Q-M8-2).

use std::collections::{BTreeMap, VecDeque};

use mush_core::{ObjId, World};

/// The most hulls the index keeps for one author, oldest out.
pub const EARNED_PER_AUTHOR: usize = 256;

/// **The earned-contacts index** (RAM): per author, the hulls their own sensors earned, oldest
/// first. Registered with the quirk claim; empty until the IC fight's producer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EarnedContacts {
    by_author: BTreeMap<ObjId, VecDeque<ObjId>>,
}

impl EarnedContacts {
    /// Whether `author` earned `hull`.
    #[must_use]
    pub fn has(&self, author: ObjId, hull: ObjId) -> bool {
        self.by_author
            .get(&author)
            .is_some_and(|hulls| hulls.contains(&hull))
    }

    /// **The producer's one entry point**: `author`'s own sensors earned `hull` (a repeat moves
    /// it to the newest end; past [`EARNED_PER_AUTHOR`] the oldest goes). Nothing calls it in
    /// production until the IC fight (16c H1).
    pub fn admit(&mut self, author: ObjId, hull: ObjId) {
        let hulls = self.by_author.entry(author).or_default();
        hulls.retain(|held| *held != hull);
        hulls.push_back(hull);
        while hulls.len() > EARNED_PER_AUTHOR {
            hulls.pop_front();
        }
    }
}

/// Whether `author`'s own sensors earned `hull`, owner-checked: `false` with no index composed.
#[must_use]
pub fn has_earned(w: &World, author: ObjId, hull: ObjId) -> bool {
    w.module_state_owner(std::any::TypeId::of::<EarnedContacts>())
        .is_some_and(|owner| owner.as_str() == crate::NS)
        && w.module_state::<EarnedContacts>()
            .is_some_and(|index| index.has(author, hull))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ring: a repeat moves a hull to the newest end, and past the bound the oldest goes.
    #[test]
    fn m8g_the_index_keeps_the_newest_hulls_per_author() {
        let mut index = EarnedContacts::default();
        let (author, other) = (ObjId(5), ObjId(6));
        for hull in 1..=u32::try_from(EARNED_PER_AUTHOR).unwrap_or(u32::MAX) {
            index.admit(author, ObjId(hull));
        }
        assert!(index.has(author, ObjId(1)) && !index.has(other, ObjId(1)));
        index.admit(author, ObjId(1));
        index.admit(author, ObjId(9_999));
        assert!(index.has(author, ObjId(1)), "the repeat is the newest");
        assert!(!index.has(author, ObjId(2)), "the oldest went");
        assert!(index.has(author, ObjId(9_999)));
    }
}
