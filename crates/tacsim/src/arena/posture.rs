//! **The arena's copy of a unit's posture** (doc 45 unit AP.3; Q-45-4, Q-45-23).
//!
//! The posture is the unit's own (`crate::posture::record`); a host that flies the unit
//! tells the arena what it is, and the arena carries the word on the unit
//! ([`UnitState::posture`](crate::arena::state::UnitState::posture)), on its frame and so
//! in the view its mind reads. Nothing here acts on it: the arena's gates, its stock mind and
//! every order stand as they were. What a mind does with `GUARD` and `HOLD` is doc 45 unit
//! AP.6's.

use mush_core::ObjId;

use crate::arena::state::Arena;
use crate::posture::Posture;

impl Arena {
    /// **The posture the arena holds for `unit`**: `RETURN` for a unit it does not hold.
    #[must_use]
    pub fn posture(&self, unit: ObjId) -> Posture {
        self.unit(unit)
            .map_or(Posture::Return, |state| state.posture)
    }

    /// **The host says what posture `unit` holds** (this module's head;
    /// `crate::arena_task::ArenaMsg::Posture`, applied at a pulse's head, and a resume's
    /// reconciliation with the unit's record). Answers whether the arena holds the unit.
    pub fn set_posture(&mut self, unit: ObjId, posture: Posture) -> bool {
        let Some(state) = self.unit_mut(unit) else {
            return false;
        };
        state.posture = posture;
        true
    }
}

#[cfg(test)]
mod tests {
    use mush_core::ObjId;

    use crate::arena::PartyId;
    use crate::arena::snapshot::Snapshot;
    use crate::arena::test_support::{arena_of, pose, tank};
    use crate::posture::Posture;

    /// **A posture off the default rides the snapshot and comes back; the default spends no
    /// key**, so `SNAPSHOT_FORMAT` stands and a snapshot taken before this field decodes.
    #[test]
    fn ap3_a_posture_is_carried_by_the_snapshot_only_when_it_is_not_return() {
        let units = || {
            vec![
                tank(ObjId(4), PartyId::RED, pose(1_500, 1_500)),
                tank(ObjId(9), PartyId::BLUE, pose(6_500, 1_500)),
            ]
        };
        let plain = arena_of(units());
        let before = Snapshot::encode(&plain).expect("an arena between pulses encodes");
        // (A pose carries a driver's `posture` blob of its own, so the key is read with its
        // word.)
        let plain_text = String::from_utf8_lossy(before.bytes()).into_owned();
        for posture in Posture::ALL {
            assert!(
                !plain_text.contains(&format!("\"posture\":\"{}\"", posture.as_str())),
                "a unit on RETURN writes no key"
            );
        }
        for posture in [Posture::Guard, Posture::Hold] {
            let mut arena = arena_of(units());
            assert!(arena.set_posture(ObjId(9), posture));
            let snapshot = Snapshot::encode(&arena).expect("it encodes");
            assert_ne!(snapshot.state_hash, before.state_hash, "the hash folds it");
            let text = String::from_utf8_lossy(snapshot.bytes()).into_owned();
            assert_eq!(
                text.matches(&format!("\"posture\":\"{}\"", posture.as_str()))
                    .count(),
                1,
                "one unit carries the word"
            );
            let back = Snapshot::decode(&snapshot.document, (*arena.definition).clone())
                .expect("it decodes");
            assert_eq!(back.posture(ObjId(9)), posture);
            assert_eq!(back.posture(ObjId(4)), Posture::Return);
            // Set back to the default it is the snapshot it was before.
            let mut again = back;
            assert!(again.set_posture(ObjId(9), Posture::Return));
            assert_eq!(
                Snapshot::encode(&again).expect("it encodes").state_hash,
                before.state_hash
            );
        }
    }

    /// **The frame, and so the mind's view, carries the unit's posture**, and nothing else
    /// about the frame moves with it: a changed posture is a changed frame, sent at once.
    #[test]
    fn ap3_the_frame_and_the_view_carry_the_posture() {
        use crate::arena::Controller;
        let mut red = tank(ObjId(4), PartyId::RED, pose(1_500, 1_500));
        red.controller = Controller::Mind;
        let mut arena = arena_of(vec![red, tank(ObjId(9), PartyId::BLUE, pose(6_500, 1_500))]);
        let tempo = arena.clock.tempo();
        let before = arena.frame_for(ObjId(4), tempo).expect("a frame");
        assert_eq!(before.posture, Posture::Return);
        assert_eq!(
            arena.mind_view(ObjId(4)).map(|view| view.posture),
            Some(Posture::Return)
        );
        assert!(arena.set_posture(ObjId(4), Posture::Guard));
        let after = arena.frame_for(ObjId(4), tempo).expect("a frame");
        assert_eq!(after.posture, Posture::Guard);
        assert!(
            before.same_as(&before.clone(), 0),
            "a frame is its own twin"
        );
        let mut twin = before.clone();
        twin.posture = Posture::Guard;
        assert!(!twin.same_as(&before, 0), "a posture change alone is sent");
        assert!(!after.same_as(&before, 0));
        assert_eq!(
            arena.mind_view(ObjId(4)).map(|view| view.posture),
            Some(Posture::Guard)
        );
        // The other hull's frame is its own.
        assert_eq!(
            arena.frame_for(ObjId(9), tempo).map(|frame| frame.posture),
            Some(Posture::Return)
        );
    }
}
