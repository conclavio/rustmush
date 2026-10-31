//! **The OOC range's two parties** (doc 32 cut CF.0; Q-CF-1, Q-CF-3, Q-CF-11).
//!
//! The kernel knows parties, not teams. The simulator centre is a host that
//! happens to run with two, which it names red and blue and letters `R` and
//! `B`, so every OOC callsign keeps its first letter and every OOC wire word
//! (`red`, `blue`) is unchanged. These names live here, in the match wrapper,
//! and the kernel never reads them.

use crate::arena::party::PartyId;

impl PartyId {
    /// The red side of a match.
    pub const RED: PartyId = PartyId::known(b'R');
    /// The blue side of a match.
    pub const BLUE: PartyId = PartyId::known(b'B');
    /// **The OOC host's two parties**, in its code order (red, then blue) —
    /// what the lobby, the centre and the record walk.
    pub const OOC: [PartyId; 2] = [PartyId::RED, PartyId::BLUE];

    /// **The other side of a two-party match.** OOC only: an IC field has no
    /// "other" party. A party that is neither red nor blue answers red.
    #[must_use]
    pub const fn ooc_other(&self) -> PartyId {
        if self.code() == b'R' {
            PartyId::BLUE
        } else {
            PartyId::RED
        }
    }
}

/// **A centre pod's label** (16b cut Q.1's shape, kept for the pods by doc 32
/// cut CF.0): the side's letter and the slot in bijective base 26 (`RA` is red
/// pod 1, `BAA` blue pod 27). Since CF.0 it is **not** a unit designator — a
/// unit letter is drawn at check-out — but the name staff verbs and the lobby
/// give a pod before any match has checked its tank out.
pub(crate) fn pod_label(team: PartyId, slot: u8) -> String {
    let mut left = u32::from(slot);
    let mut letters: Vec<char> = Vec::new();
    while left > 0 {
        let index = u8::try_from(left.saturating_sub(1) % 26).unwrap_or(0);
        letters.push(char::from(b'A'.saturating_add(index)));
        left = left.saturating_sub(1) / 26;
    }
    letters.push(team.letter());
    letters.reverse();
    letters.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ooc_parties_keep_their_letters_and_words() {
        assert_eq!(PartyId::RED.letter(), 'R');
        assert_eq!(PartyId::BLUE.letter(), 'B');
        assert_eq!(PartyId::RED.as_str(), "red");
        assert_eq!(PartyId::BLUE.as_str(), "blue");
        assert_eq!(PartyId::RED.ooc_other(), PartyId::BLUE);
        assert_eq!(PartyId::BLUE.ooc_other(), PartyId::RED);
    }
}

#[cfg(test)]
mod slot_order_tests {
    use crate::match_::Fielded;
    use mush_core::ObjId;

    /// Doc 32 cut CF.0: the roster's order is the pods' — explicit slots in
    /// slot order, unslotted units at the lowest slot left in unit-id order.
    #[test]
    fn a_roster_is_in_pod_slot_order_whatever_the_callsigns_say() {
        let at = |unit: u32, slot: u8| Fielded {
            slot,
            ..Fielded::unslotted(ObjId(unit))
        };
        let order = super::super::slot_order(&[at(30, 2), at(9, 0), at(12, 4), at(5, 0)]);
        assert_eq!(order, vec![ObjId(5), ObjId(30), ObjId(9), ObjId(12)]);
    }
}
