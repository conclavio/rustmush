//! **What an author's own sensors earned in the fiction** (doc 17h cut M8g G3; D-UM-55: a quirk
//! claim rests only on what the author's own sensors earned in the field, the observer rule).
//!
//! **The producer** (doc 23 cut S.4b-T; §17.8, D-23-19): a claim is IC testimony, so only IC
//! contacts count. Every arena mode (`skirmish`, `hold`, `range`) is a SIM match — OOC training
//! whose contacts are not earned in the fiction — and writes nothing here. An IC field does:
//! `crate::field::testimony` reads each chunk of the field's own log as it lands and
//! [`EarnedContacts::admit`](crate::minds::earned_contacts::EarnedContacts::admit)s every hull a unit told apart by its own sensors (the `Classified`
//! line carrying its `classify:` draw), for the unit's admission pilot, else its party's leader
//! (the seal's warrant principal). A mesh classification and a `Detected` blip earn nothing.
//!
//! The index is a per-character ring of [`EARNED_PER_AUTHOR`](crate::minds::earned_contacts::EARNED_PER_AUTHOR) hulls, oldest out, persisted
//! beside its RAM mirror: one [`EarnedRecord`](crate::minds::earned_contacts::EarnedRecord) per author at `tacsim:earned:<author>`, written in
//! the chunk's bundle and read back at boot (`read`).

use std::collections::{BTreeMap, VecDeque};

use mush_core::{Ctx, ObjId, World};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// The most hulls the index keeps for one author, oldest out.
pub const EARNED_PER_AUTHOR: usize = 256;

/// `tacsim:earned:<author>`.
pub const EARNED_PREFIX: &str = "tacsim:earned:";

/// [`EarnedRecord::format`].
pub const EARNED_FORMAT: u16 = 1;

/// One author's ring's key.
#[must_use]
pub fn earned_key(author: ObjId) -> SmolStr {
    SmolStr::new(format!("{EARNED_PREFIX}{}", author.0))
}

/// **One author's ring, as stored** (`tacsim:earned:<author>`): the hulls oldest first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EarnedRecord {
    /// [`EARNED_FORMAT`].
    pub format: u16,
    /// The author (the identity rule: the key's character).
    pub author: ObjId,
    /// The hulls, oldest first, at most [`EARNED_PER_AUTHOR`], each once.
    pub hulls: Vec<ObjId>,
}

/// **The earned-contacts index** (RAM): per author, the hulls their own sensors earned, oldest
/// first. Registered with the field host, which feeds it; read by the quirk claim.
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
    /// it to the newest end; past [`EARNED_PER_AUTHOR`] the oldest goes). Whether the ring
    /// changed: a repeat of the newest hull leaves it as it was.
    pub fn admit(&mut self, author: ObjId, hull: ObjId) -> bool {
        let hulls = self.by_author.entry(author).or_default();
        if hulls.back() == Some(&hull) {
            return false;
        }
        hulls.retain(|held| *held != hull);
        hulls.push_back(hull);
        while hulls.len() > EARNED_PER_AUTHOR {
            hulls.pop_front();
        }
        true
    }

    /// `author`'s ring as its document.
    #[must_use]
    pub fn record(&self, author: ObjId) -> EarnedRecord {
        EarnedRecord {
            format: EARNED_FORMAT,
            author,
            hulls: self
                .by_author
                .get(&author)
                .map(|hulls| hulls.iter().copied().collect())
                .unwrap_or_default(),
        }
    }

    /// A stored ring back into RAM, oldest first: a repeat keeps its newest place and past the
    /// bound the oldest goes, as [`Self::admit`] would have left them.
    fn restore(&mut self, record: &EarnedRecord) {
        for hull in &record.hulls {
            self.admit(record.author, *hull);
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

/// **Read every author's ring back at boot** (`on_world_loaded`). A document that does not
/// decode, or names another character than its key, is skipped loudly: that author's earned
/// contacts are lost, and nothing else.
pub(crate) fn read(ctx: &mut Ctx<'_>, kv: &[(SmolStr, serde_json::Value)]) {
    let Some(index) = ctx.w.module_state_mut::<EarnedContacts>() else {
        return;
    };
    for (key, value) in kv {
        let Some(author) = key
            .as_str()
            .strip_prefix(EARNED_PREFIX)
            .and_then(|n| n.parse::<u32>().ok())
            .map(ObjId)
        else {
            continue;
        };
        match serde_json::from_value::<EarnedRecord>(value.clone()) {
            Ok(record) if record.author == author && record.format == EARNED_FORMAT => {
                index.restore(&record);
            }
            _ => tracing::warn!(
                author = author.0,
                "tacsim: an earned-contacts ring did not decode; that author's earned contacts are lost"
            ),
        }
    }
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
            assert!(index.admit(author, ObjId(hull)));
        }
        assert!(index.has(author, ObjId(1)) && !index.has(other, ObjId(1)));
        assert!(index.admit(author, ObjId(1)));
        assert!(
            !index.admit(author, ObjId(1)),
            "the newest again changes nothing"
        );
        assert!(index.admit(author, ObjId(9_999)));
        assert!(index.has(author, ObjId(1)), "the repeat is the newest");
        assert!(!index.has(author, ObjId(2)), "the oldest went");
        assert!(index.has(author, ObjId(9_999)));
    }

    /// **S.4b-T**: a ring's document restores to the same ring, and its key names its author.
    #[test]
    fn s4bt_a_ring_round_trips_through_its_document() {
        let mut index = EarnedContacts::default();
        let author = ObjId(5);
        for hull in [7, 9, 7, 11] {
            index.admit(author, ObjId(hull));
        }
        let record = index.record(author);
        assert_eq!(record.hulls, vec![ObjId(9), ObjId(7), ObjId(11)]);
        let value = serde_json::to_value(&record).expect("encodes");
        let back: EarnedRecord = serde_json::from_value(value).expect("decodes");
        let mut restored = EarnedContacts::default();
        restored.restore(&back);
        assert_eq!(restored, index);
        assert_eq!(earned_key(author).as_str(), "tacsim:earned:5");
        assert!(index.record(ObjId(6)).hulls.is_empty());
    }
}
