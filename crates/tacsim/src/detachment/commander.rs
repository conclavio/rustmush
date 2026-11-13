//! **The compiled NPC commander's rules** (doc 44 units MW.14a and MW.14c; Q-44-20, Q-44-44,
//! Q-44-49).
//!
//! A detachment's plan may name an `npc_commander` (doc 26 D-26-1, doc 27 FO-15). Until MW.14a
//! nothing decided for one: a roster with no player in its units flew the doctrine with no
//! orders at all. The owner ruled a deterministic commander (Q-44-20) and split the work
//! (Q-44-44): MW.14a built this compiled one, and MW.14b was to lift these rules into a Rhai
//! doctrine. **The owner deferred MW.14b on 2026-11-13 (Q-44-49)**: the compiled commander is
//! kept, and MW.14c gives it what it lacked: a dwell, a work point the plan names, and one
//! contact for the roster to concentrate on.
//!
//! **Everything the commander decides is in this file**, and it is small on purpose:
//!
//! - [`View`] is all it decides on: what its own frame holds ([`Sight`]), the state of its
//!   roster ([`Strength`]), where its work is ([`View::work`]), and what the roster's order
//!   history records it last ordered and how long ago ([`Held`], [`Named`]). It reads no world, no clock
//!   and no dice, so the same view gives the same decision on every host and every replay;
//! - [`decide`] is the whole rule. The posture, first match wins: **withdraw** once the roster
//!   has lost a hull, **hold** while a Hostile stands in sight, **advance** to the work point
//!   otherwise; and a posture it ordered less than [`DWELL_S`] ago is kept, unless the roster
//!   has lost a hull (a withdrawal is never delayed). While it holds, it names **one contact for
//!   the roster to concentrate on** ([`Decision::concentrate`]) and keeps to it until that
//!   contact is destroyed or out of its sight;
//! - [`Posture`] and the named contact are what it may ask for: the three roster orders a
//!   player leader can give that move a detachment, and the `CONCENTRATE` a player leader can
//!   give. It gives no ENGAGE, SCREEN, SUPPORT or MINE, marks no party and sets no player's
//!   POSTURE.
//!
//! The host half ([`crate::detachment::sortie`]'s `commander`) builds the view from the
//! commander's own frame, the roster's last reconciliation and the roster's order book, and
//! issues the decision as roster orders through the delivery a player leader's take: the
//! leader's authority and term, the IC transport that can be jammed, the recipient's own
//! executor and contacts.
//!
//! **The shape is the one a doctrine takes**: a plain view in, a plain decision out, no handle
//! on anything. A commander doctrine, if one is ever ruled (Q-44-49), and a House's policy
//! doctrine (Q-44-32, deferred) can be written against the same types.

use serde::{Deserialize, Serialize};

/// **How far out from its carrier's ramp a detachment's default work point lies**, site cells,
/// straight away from the carrier (the host computes the cell). Used when the plan names no
/// work point, or names one that is not on the site (doc 44 unit MW.14c). Provisional.
pub const WORK_POINT_CELLS: u16 = 6;

/// **How many hulls a roster may lose before its commander orders it home.** Provisional: the
/// first loss.
pub const WITHDRAW_AT_LOSSES: u16 = 1;

/// **How long a posture stands before the commander may change it**, wall seconds (doc 44 unit
/// MW.14c). A Hostile that drifts in and out of the commander's sight would otherwise flip the
/// roster between HOLD and ADVANCE as often as once a world second. The same dwell holds the
/// contact the roster concentrates on while that contact is merely out of the commander's
/// sight. A withdrawal after a loss is exempt, and so is a new name once the named contact is
/// destroyed.
///
/// **Wall time, not IC.** The game runs at 24:1 and a principal acts once a world second, so
/// an IC-second dwell would be a twenty-fourth of this and no damping at all (MW.14a's review
/// found the same of its resend pacing). Thirty seconds is some thirty of the commander's own
/// turns: long enough for a posture to reach every hull of a full roster and be answered, and
/// short enough that a roster is not left advancing into a Hostile, or standing after one has
/// gone, for longer than a player would leave it. Provisional.
pub const DWELL_S: u32 = 30;

/// **One contact the commander's own frame holds Hostile** (doc 44 unit MW.14c).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hostile {
    /// Its row key in the commander's own scope: what an order names it by.
    pub key: u64,
    /// How far off its plot is, centimetres.
    pub range_cm: u64,
    /// Whether the commander's hull reads it destroyed. A hull that carries nothing to fire
    /// reads no contact destroyed, and goes on holding it until the plot is dropped.
    pub destroyed: bool,
}

/// **What the commander's own frame holds** (doc 16 §3.6's observer rule): nothing the kernel
/// knows and the commander's hull has not seen.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sight {
    /// The contacts its own party's book holds Hostile, in row-key order.
    pub hostiles: Vec<Hostile>,
}

impl Sight {
    /// The Hostile contacts it does not read destroyed.
    fn live(&self) -> impl Iterator<Item = &Hostile> {
        self.hostiles.iter().filter(|hostile| !hostile.destroyed)
    }
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

/// **The posture the roster's order history records its NPC leader last ordered** (doc 44
/// unit MW.14c), and for how long. That history is the commander's only memory, so this is
/// read from the orders it keeps and from nothing in RAM: a restart reads the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Held {
    /// The posture.
    pub posture: Posture,
    /// Wall seconds since it was first ordered.
    pub for_s: u32,
}

/// **The contact the roster's order history records the commander last named** for the roster to
/// concentrate on (doc 44 unit MW.14c), and for how long.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Named {
    /// The contact's row key in the commander's own scope.
    pub key: u64,
    /// Wall seconds since it was first named.
    pub for_s: u32,
}

/// **Everything a compiled commander decides on** (this module's head).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    /// What its own frame holds.
    pub sight: Sight,
    /// The state of its roster.
    pub strength: Strength,
    /// The detachment's work point: the cell its plan names, or by default the site cell
    /// [`WORK_POINT_CELLS`] out from its carrier's ramp.
    pub work: (u16, u16),
    /// The posture it last ordered, when the book holds one.
    pub held: Option<Held>,
    /// The contact it last named, when the book holds one.
    pub named: Option<Named>,
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

/// **What a compiled commander decides** (doc 44 unit MW.14c): the roster's posture, and while
/// it holds, the one contact every hull is to concentrate on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    /// The posture.
    pub posture: Posture,
    /// The contact to concentrate on, by its row key in the commander's own scope. Only ever
    /// one of [`Sight::hostiles`] or the contact the book already names: the commander names no
    /// contact its party's book does not hold Hostile.
    pub concentrate: Option<u64>,
}

/// **The posture the rule wants, before the dwell**, first match wins:
///
/// 1. the roster has lost [`WITHDRAW_AT_LOSSES`] hulls or more: [`Posture::Withdraw`];
/// 2. a Hostile it does not read destroyed stands in the commander's own sight:
///    [`Posture::Hold`];
/// 3. otherwise: [`Posture::Advance`] to the work point.
fn wanted(view: &View) -> Posture {
    if view.strength.lost >= WITHDRAW_AT_LOSSES {
        Posture::Withdraw
    } else if view.sight.live().next().is_some() {
        Posture::Hold
    } else {
        Posture::Advance { to: view.work }
    }
}

/// **The contact the roster concentrates on while it holds** (doc 44 unit MW.14c), first match
/// wins:
///
/// 1. the contact already named, while it stands in the commander's sight and is not read
///    destroyed: the commander keeps to it;
/// 2. the contact already named, out of the commander's sight for less than the dwell
///    ([`DWELL_S`] since it was named) and not read destroyed: the name stands, so a contact
///    that drifts in and out of sight does not turn the roster from one hull to another;
/// 3. **the nearest Hostile in the commander's sight** that it does not read destroyed, the
///    lower row key between two at one range. Provisional: the nearest is the one most of the
///    roster is likeliest to hold as well. The commander's own frame does not say which contact
///    fired, so the rule cannot prefer the one that last fired on the roster; on an NPC-led
///    roster every Hostile is of a party that fired on it or on a friend (the commander marks
///    nobody).
fn target(view: &View) -> Option<u64> {
    let live = |key: u64| view.sight.live().any(|hostile| hostile.key == key);
    let destroyed = |key: u64| {
        view.sight
            .hostiles
            .iter()
            .any(|hostile| hostile.key == key && hostile.destroyed)
    };
    if let Some(named) = view.named
        && (live(named.key) || (named.for_s < DWELL_S && !destroyed(named.key)))
    {
        return Some(named.key);
    }
    view.sight
        .live()
        .min_by_key(|hostile| (hostile.range_cm, hostile.key))
        .map(|hostile| hostile.key)
}

/// **The compiled commander's whole rule** (this module's head).
///
/// The posture is the one `wanted` gives, damped: a posture the history records it ordered less than
/// [`DWELL_S`] ago is kept. A withdrawal is never delayed, and a roster already ordered home is
/// never turned round by the dwell. While the posture is [`Posture::Hold`] the roster
/// concentrates on the contact `target` gives; under any other posture it names none.
#[must_use]
pub fn decide(view: &View) -> Decision {
    let wanted = wanted(view);
    let posture = match view.held {
        Some(held)
            if wanted != Posture::Withdraw && held.posture != wanted && held.for_s < DWELL_S =>
        {
            held.posture
        }
        _ => wanted,
    };
    Decision {
        posture,
        concentrate: (posture == Posture::Hold).then(|| target(view)).flatten(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: (u16, u16) = (128, 118);

    fn hostile(key: u64, range_cm: u64) -> Hostile {
        Hostile {
            key,
            range_cm,
            destroyed: false,
        }
    }

    fn view(hostiles: &[Hostile], lost: u16) -> View {
        View {
            sight: Sight {
                hostiles: hostiles.to_vec(),
            },
            strength: Strength { hulls: 2, lost },
            work: WORK,
            held: None,
            named: None,
        }
    }

    /// **Doc 44 unit MW.14a**: the rule's three arms, and their order.
    #[test]
    fn mw14a_the_commander_advances_holds_and_withdraws_on_what_it_sees_and_its_roster() {
        let posture = |hostiles: &[Hostile], lost| decide(&view(hostiles, lost)).posture;
        assert_eq!(
            posture(&[], 0),
            Posture::Advance { to: WORK },
            "nothing hostile in sight and nobody lost: on with the work"
        );
        assert_eq!(posture(&[hostile(7, 900)], 0), Posture::Hold);
        assert_eq!(
            posture(&[hostile(7, 900), hostile(8, 400), hostile(9, 100)], 0),
            Posture::Hold
        );
        assert_eq!(posture(&[], 1), Posture::Withdraw, "a hull lost");
        assert_eq!(
            posture(&[hostile(7, 900), hostile(8, 400)], 1),
            Posture::Withdraw,
            "a loss outranks a Hostile in sight"
        );
    }

    /// **Doc 44 unit MW.14a**: the same view is the same decision, and a decision round-trips
    /// as data.
    #[test]
    fn mw14a_the_same_view_gives_the_same_posture() {
        for hostiles in 0..3u64 {
            for lost in 0..3 {
                let rows: Vec<Hostile> = (0..hostiles).map(|n| hostile(n + 1, 500)).collect();
                let seen = view(&rows, lost);
                assert_eq!(decide(&seen), decide(&seen));
                let decision = decide(&seen);
                let wire = serde_json::to_string(&decision).expect("a decision encodes");
                assert_eq!(
                    serde_json::from_str::<Decision>(&wire).expect("and decodes"),
                    decision
                );
            }
        }
    }

    /// **Doc 44 unit MW.14c**: a posture stands for the dwell. A Hostile that comes into sight
    /// just after an ADVANCE does not turn it into a HOLD until the dwell is out, and one that
    /// leaves just after a HOLD does not turn that into an ADVANCE; a loss withdraws the roster
    /// at once, inside any dwell; and a roster ordered home stays ordered home.
    #[test]
    fn mw14c_a_posture_stands_for_the_dwell_and_a_loss_withdraws_at_once() {
        let advance = Posture::Advance { to: WORK };
        let held = |posture, for_s| Some(Held { posture, for_s });
        let mut seen = view(&[hostile(7, 900)], 0);
        for (for_s, expect) in [
            (0, advance),
            (DWELL_S - 1, advance),
            (DWELL_S, Posture::Hold),
            (DWELL_S + 600, Posture::Hold),
        ] {
            seen.held = held(advance, for_s);
            assert_eq!(decide(&seen).posture, expect, "an ADVANCE {for_s} s old");
        }
        let mut clear = view(&[], 0);
        for (for_s, expect) in [
            (0, Posture::Hold),
            (DWELL_S - 1, Posture::Hold),
            (DWELL_S, advance),
        ] {
            clear.held = held(Posture::Hold, for_s);
            assert_eq!(decide(&clear).posture, expect, "a HOLD {for_s} s old");
        }
        // A loss: no dwell delays the way home, whatever was ordered a second ago.
        for posture in [advance, Posture::Hold] {
            let mut lost = view(&[hostile(7, 900)], 1);
            lost.held = held(posture, 0);
            assert_eq!(decide(&lost).posture, Posture::Withdraw);
        }
        // The same posture again is no change, at any age.
        seen.held = held(Posture::Hold, 0);
        assert_eq!(decide(&seen).posture, Posture::Hold);
        // With no order of its own in the book it decides undamped.
        seen.held = None;
        assert_eq!(decide(&seen).posture, Posture::Hold);
    }

    /// **Doc 44 unit MW.14c**: while it holds, the commander names the nearest Hostile in its
    /// sight, keeps to the one it named while that one is in sight, keeps the name for the
    /// dwell while it is merely out of sight, and names the next at once when it is destroyed.
    /// It names nobody under another posture, and never a contact it does not hold Hostile.
    #[test]
    fn mw14c_the_commander_names_one_contact_and_keeps_to_it() {
        let near = hostile(9, 400);
        let far = hostile(7, 900);
        let both = [far, near];
        assert_eq!(decide(&view(&both, 0)).concentrate, Some(9), "the nearest");
        assert_eq!(
            decide(&view(&[hostile(8, 400), hostile(3, 400)], 0)).concentrate,
            Some(3),
            "the lower row key between two at one range"
        );
        // It keeps to the one it named, though another is nearer now.
        let named = |key, for_s| Some(Named { key, for_s });
        let mut seen = view(&both, 0);
        seen.named = named(7, DWELL_S + 600);
        assert_eq!(decide(&seen).concentrate, Some(7));
        // Out of its sight: the name stands for the dwell, then the next is named.
        let mut gone = view(&[near], 0);
        gone.named = named(7, DWELL_S - 1);
        assert_eq!(decide(&gone).concentrate, Some(7));
        gone.named = named(7, DWELL_S);
        assert_eq!(decide(&gone).concentrate, Some(9));
        // Destroyed: the next at once, inside the dwell.
        let mut dead = view(
            &[
                Hostile {
                    destroyed: true,
                    ..far
                },
                near,
            ],
            0,
        );
        dead.named = named(7, 1);
        assert_eq!(decide(&dead).concentrate, Some(9));
        // Only a destroyed Hostile in sight is nothing to hold for, or to name.
        let mut only = view(
            &[Hostile {
                destroyed: true,
                ..far
            }],
            0,
        );
        only.named = named(7, 1);
        assert_eq!(
            decide(&only),
            Decision {
                posture: Posture::Advance { to: WORK },
                concentrate: None
            }
        );
        // No contact is named while it advances or withdraws, and none it does not hold.
        assert_eq!(decide(&view(&[], 0)).concentrate, None);
        assert_eq!(decide(&view(&both, 1)).concentrate, None);
        let mut advancing = view(&both, 0);
        advancing.held = Some(Held {
            posture: Posture::Advance { to: WORK },
            for_s: 0,
        });
        assert_eq!(
            decide(&advancing).concentrate,
            None,
            "inside an ADVANCE's dwell it names nobody"
        );
    }
}
