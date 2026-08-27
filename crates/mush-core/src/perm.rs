//! Centralized permission checks.
//!
//! Every command handler routes its authority questions through this module
//! — nobody inlines flag math. The core primitive is [`controls`]:
//!
//! > `who` controls `what` iff `who` is Wizard-or-better, **or** `who` is
//! > `what` itself, **or** `who` (or `who`'s owner) owns `what`, **or**
//! > `what`'s Zone Master Object has a `Zone` lock that `who` passes
//! > (delegated zone administration, design doc §2.6).
//!
//! Granular Rhost-style [`Powers`] refine the picture: [`has_power`] is
//! automatically satisfied for Wizard+ so staff never need explicit grants.

use crate::flags::{Flags, Powers, Role};
use crate::lock::{LockType, MAX_LOCK_DEPTH, eval_lock};
use crate::obj::ObjId;
use crate::world::World;

/// The staff role of an object ([`Role::Player`] if it does not exist).
#[must_use]
pub fn role_of(w: &World, who: ObjId) -> Role {
    w.get(who).map_or(Role::Player, |o| o.role())
}

/// Whether `who`'s role is at least `min` (role gating helper).
#[must_use]
pub fn has_role(w: &World, who: ObjId, min: Role) -> bool {
    role_of(w, who) >= min
}

/// Whether `who` holds the given power. Wizard+ implicitly holds **every**
/// power; everyone else needs an explicit grant on their object.
#[must_use]
pub fn has_power(w: &World, who: ObjId, power: Powers) -> bool {
    if has_role(w, who, Role::Wizard) {
        return true;
    }
    w.get(who).is_some_and(|o| o.powers.contains(power))
}

/// The classic "does `who` control `what`" check: God ∨ Wizard ∨ self ∨
/// owner ∨ zone delegation (the ZMO's `Zone` lock).
#[must_use]
pub fn controls(w: &World, who: ObjId, what: ObjId) -> bool {
    let Some(a) = w.get(who) else { return false };
    let Some(b) = w.get(what) else { return false };
    if a.role() >= Role::Wizard {
        return true;
    }
    if who == what {
        return true;
    }
    // Ownership: `what` owned by `who` directly, or by `who`'s owner (so a
    // player's puppets/objects speak with the player's authority).
    if b.owner == who || b.owner == a.owner {
        return true;
    }
    // Zone delegation: the ZMO's Zone lock grants control over members.
    if let Some(z) = b.zone
        && let Some(zo) = w.get(z)
        && zo.flags.contains(Flags::ZONE_MASTER)
        && let Some(expr) = zo.locks.get(&LockType::Zone)
        && eval_lock(w, expr, who, MAX_LOCK_DEPTH)
    {
        return true;
    }
    false
}

/// Whether `who` may see `what`'s full examine output (attributes, locks).
/// Controllers always can; the `EXAMINE_ALL` power grants it globally.
#[must_use]
pub fn can_examine(w: &World, who: ObjId, what: ObjId) -> bool {
    controls(w, who, what) || has_power(w, who, Powers::EXAMINE_ALL)
}

/// Whether `who` may modify `what` (set attributes, flags, locks, name…).
/// Requires control, and refuses objects already marked `GOING`.
#[must_use]
pub fn can_modify(w: &World, who: ObjId, what: ObjId) -> bool {
    if !controls(w, who, what) {
        return false;
    }
    w.get(what).is_some_and(|o| !o.flags.contains(Flags::GOING))
}

/// Whether `who` may teleport `target` to `dest`.
///
/// Three questions, all of which must pass:
/// 1. **The body**: moving something other than yourself needs control over
///    it or the `TEL_ANYTHING` power.
/// 2. **The destination**: must be controlled by `who`, be `JUMP_OK`, or
///    `who` holds `TEL_ANYWHERE`.
/// 3. **The destination's `Teleport` lock**, evaluated against `target`
///    (Wizard+ bypasses a failing lock).
#[must_use]
pub fn can_teleport(w: &World, who: ObjId, target: ObjId, dest: ObjId) -> bool {
    let Some(d) = w.get(dest) else { return false };
    if w.get(target).is_none() {
        return false;
    }
    if target != who && !controls(w, who, target) && !has_power(w, who, Powers::TEL_ANYTHING) {
        return false;
    }
    let dest_open = controls(w, who, dest)
        || d.flags.contains(Flags::JUMP_OK)
        || has_power(w, who, Powers::TEL_ANYWHERE);
    if !dest_open {
        return false;
    }
    if let Some(lock) = d.locks.get(&LockType::Teleport)
        && !eval_lock(w, lock, target, MAX_LOCK_DEPTH)
        && !has_role(w, who, Role::Wizard)
    {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::LockExpr;
    use crate::obj::{GOD, LIMBO, ObjKind};

    /// God (#1, wizard), a plain player Nick, Nick's ball, Bob, and Bob's
    /// rock — plus a zone whose Zone lock admits Nick.
    fn setup() -> (World, ObjId, ObjId, ObjId, ObjId) {
        let mut w = World::new_minimal();
        let nick = w.create(ObjKind::Player, "Nick", GOD, Some(LIMBO)).unwrap();
        let bob = w.create(ObjKind::Player, "Bob", GOD, Some(LIMBO)).unwrap();
        let ball = w.create(ObjKind::Thing, "ball", nick, Some(LIMBO)).unwrap();
        let rock = w.create(ObjKind::Thing, "rock", bob, Some(LIMBO)).unwrap();
        (w, nick, bob, ball, rock)
    }

    #[test]
    fn controls_owner_self_wizard() {
        let (w, nick, bob, ball, rock) = setup();
        // Self.
        assert!(controls(&w, nick, nick));
        // Owner.
        assert!(controls(&w, nick, ball));
        assert!(!controls(&w, nick, rock));
        assert!(!controls(&w, nick, bob));
        // Wizard controls everything.
        assert!(controls(&w, GOD, ball));
        assert!(controls(&w, GOD, nick));
        // Missing objects control nothing / are controlled by nobody.
        assert!(!controls(&w, ObjId(99), ball));
        assert!(!controls(&w, nick, ObjId(99)));
    }

    #[test]
    fn controls_via_zone_lock() {
        let (mut w, nick, _bob, _ball, rock) = setup();
        let zmo = w.create(ObjKind::Thing, "ZMO", GOD, Some(LIMBO)).unwrap();
        w.set_flag(zmo, Flags::ZONE_MASTER, true).unwrap();
        w.get_mut(zmo)
            .unwrap()
            .locks
            .insert(LockType::Zone, LockExpr::Is(nick));
        w.set_zone(rock, Some(zmo)).unwrap();
        // Nick passes the zone lock -> controls Bob's rock.
        assert!(controls(&w, nick, rock));
        // Without ZONE_MASTER the delegation is void.
        w.set_flag(zmo, Flags::ZONE_MASTER, false).unwrap();
        assert!(!controls(&w, nick, rock));
    }

    #[test]
    fn power_checks_and_wizard_implication() {
        let (mut w, nick, _bob, _ball, rock) = setup();
        assert!(!has_power(&w, nick, Powers::EXAMINE_ALL));
        assert!(has_power(&w, GOD, Powers::EXAMINE_ALL)); // wizard implies all
        w.get_mut(nick).unwrap().powers = Powers::EXAMINE_ALL;
        assert!(has_power(&w, nick, Powers::EXAMINE_ALL));
        assert!(!has_power(&w, nick, Powers::BOOT));
        // EXAMINE_ALL grants examine without control.
        assert!(can_examine(&w, nick, rock));
        // ...but not modify.
        assert!(!can_modify(&w, nick, rock));
    }

    #[test]
    fn can_modify_refuses_going() {
        let (mut w, nick, _bob, ball, _rock) = setup();
        assert!(can_modify(&w, nick, ball));
        w.destroy(ball).unwrap();
        assert!(!can_modify(&w, nick, ball));
    }

    #[test]
    fn teleport_rules() {
        let (mut w, nick, bob, ball, _rock) = setup();
        let open = w.create(ObjKind::Room, "Open", GOD, None).unwrap();
        let closed = w.create(ObjKind::Room, "Closed", GOD, None).unwrap();
        w.set_flag(open, Flags::JUMP_OK, true).unwrap();

        // Self to a JUMP_OK room: fine. To a closed room: no.
        assert!(can_teleport(&w, nick, nick, open));
        assert!(!can_teleport(&w, nick, nick, closed));
        // Own object anywhere open: fine. Someone else's body: no.
        assert!(can_teleport(&w, nick, ball, open));
        assert!(!can_teleport(&w, nick, bob, open));
        // TEL_ANYTHING lets you move others; TEL_ANYWHERE opens closed rooms.
        w.get_mut(nick).unwrap().powers = Powers::TEL_ANYTHING;
        assert!(can_teleport(&w, nick, bob, open));
        assert!(!can_teleport(&w, nick, nick, closed));
        w.get_mut(nick).unwrap().powers = Powers::TEL_ANYTHING | Powers::TEL_ANYWHERE;
        assert!(can_teleport(&w, nick, nick, closed));
        // Wizards do whatever they like.
        assert!(can_teleport(&w, GOD, bob, closed));

        // Teleport lock on the destination gates non-staff.
        w.get_mut(open)
            .unwrap()
            .locks
            .insert(LockType::Teleport, LockExpr::Is(bob));
        assert!(!can_teleport(&w, nick, nick, open));
        assert!(can_teleport(&w, GOD, GOD, open)); // wizard bypass
    }

    #[test]
    fn role_helpers() {
        let (mut w, nick, _bob, _ball, _rock) = setup();
        assert_eq!(role_of(&w, nick), Role::Player);
        assert_eq!(role_of(&w, GOD), Role::God);
        assert_eq!(role_of(&w, ObjId(99)), Role::Player);
        assert!(!has_role(&w, nick, Role::Builder));
        w.set_flag(nick, Flags::BUILDER, true).unwrap();
        assert!(has_role(&w, nick, Role::Builder));
        assert!(!has_role(&w, nick, Role::Wizard));
    }
}
