//! **The compiled NPC commander's rules** (doc 44 unit MW.14a; Q-44-20, Q-44-44).
//!
//! A detachment's plan may name an `npc_commander` (doc 26 D-26-1, doc 27 FO-15). Until this
//! unit nothing decided for one: a roster with no player in its units flew the doctrine with no
//! orders at all. The owner ruled a deterministic commander (Q-44-20) and split the work
//! (Q-44-44): this is the compiled one, and MW.14b lifts these rules into a Rhai doctrine once
//! the mind ABI can hold a commander (one hull that may not `order` cannot, D-UM-11).
//!
//! **Everything the commander decides is in this file**, and it is small on purpose:
//!
//! - [`View`] is all it decides on: what its own frame holds ([`Sight`]), the state of its
//!   roster ([`Strength`]) and where its work is ([`View::work`]). It reads no world, no clock
//!   and no dice, so the same view gives the same posture on every host and every replay;
//! - [`decide`] is the whole rule: **withdraw** once the roster has lost a hull, **hold** while a
//!   Hostile stands in sight, **advance** to the work point otherwise;
//! - [`Posture`] is what it may ask for: the three roster orders a player leader can give that
//!   move a detachment. It gives no ENGAGE, SCREEN, SUPPORT or MINE.
//!
//! The host half ([`crate::detachment::sortie`]'s `commander`) builds the view from the
//! commander's own frame and the roster's last reconciliation, and issues the posture as roster
//! orders through the delivery a player leader's take: the leader's authority and term, the IC
//! transport that can be jammed, the recipient's own executor and contacts.
//!
//! **The shape is the one a doctrine takes**: a plain view in, a plain decision out, no handle
//! on anything. MW.14b's commander doctrine and a House's policy doctrine (Q-44-32, deferred)
//! can be written against the same two types.

use serde::{Deserialize, Serialize};

/// **How far out from its carrier's ramp a detachment's work point lies**, site cells, straight
/// away from the carrier (the host computes the cell). Provisional.
pub const WORK_POINT_CELLS: u16 = 6;

/// **How many hulls a roster may lose before its commander orders it home.** Provisional: the
/// first loss.
pub const WITHDRAW_AT_LOSSES: u16 = 1;

/// **What the commander's own frame holds** (doc 16 §3.6's observer rule): nothing the kernel
/// knows and the commander's hull has not seen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sight {
    /// The live contacts its own party's book holds Hostile.
    pub hostiles: u16,
}

/// **The state of the commander's roster**, as its last reconciliation and the field host's
/// record of destroyed units have it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Strength {
    /// The roster's hulls.
    pub hulls: u16,
    /// Those lost: destroyed in this field.
    pub lost: u16,
}

/// **Everything a compiled commander decides on** (this module's head).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    /// What its own frame holds.
    pub sight: Sight,
    /// The state of its roster.
    pub strength: Strength,
    /// The detachment's work point: a site cell [`WORK_POINT_CELLS`] out from its carrier's
    /// ramp.
    pub work: (u16, u16),
}

/// **What a compiled commander asks of its roster**: one posture for every hull it can reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Posture {
    /// Go to the work point.
    Advance {
        /// The cell.
        to: (u16, u16),
    },
    /// Stand where you are.
    Hold,
    /// Go home to the carrier.
    Withdraw,
}

/// **The compiled commander's whole rule** (this module's head), first match wins:
///
/// 1. the roster has lost [`WITHDRAW_AT_LOSSES`] hulls or more: [`Posture::Withdraw`];
/// 2. a Hostile stands in the commander's own sight: [`Posture::Hold`];
/// 3. otherwise: [`Posture::Advance`] to the work point.
#[must_use]
pub const fn decide(view: &View) -> Posture {
    if view.strength.lost >= WITHDRAW_AT_LOSSES {
        Posture::Withdraw
    } else if view.sight.hostiles > 0 {
        Posture::Hold
    } else {
        Posture::Advance { to: view.work }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(hostiles: u16, lost: u16) -> View {
        View {
            sight: Sight { hostiles },
            strength: Strength { hulls: 2, lost },
            work: (128, 118),
        }
    }

    /// **Doc 44 unit MW.14a**: the rule's three arms, and their order.
    #[test]
    fn mw14a_the_commander_advances_holds_and_withdraws_on_what_it_sees_and_its_roster() {
        assert_eq!(
            decide(&view(0, 0)),
            Posture::Advance { to: (128, 118) },
            "nothing hostile in sight and nobody lost: on with the work"
        );
        assert_eq!(decide(&view(1, 0)), Posture::Hold, "a Hostile in sight");
        assert_eq!(decide(&view(3, 0)), Posture::Hold);
        assert_eq!(decide(&view(0, 1)), Posture::Withdraw, "a hull lost");
        assert_eq!(
            decide(&view(2, 1)),
            Posture::Withdraw,
            "a loss outranks a Hostile in sight"
        );
    }

    /// **Doc 44 unit MW.14a**: the same view is the same posture, and a posture round-trips as
    /// data (what MW.14b's doctrine returns).
    #[test]
    fn mw14a_the_same_view_gives_the_same_posture() {
        for hostiles in 0..3 {
            for lost in 0..3 {
                let seen = view(hostiles, lost);
                assert_eq!(decide(&seen), decide(&seen));
                let posture = decide(&seen);
                let wire = serde_json::to_string(&posture).expect("a posture encodes");
                assert_eq!(
                    serde_json::from_str::<Posture>(&wire).expect("and decodes"),
                    posture
                );
            }
        }
    }
}
