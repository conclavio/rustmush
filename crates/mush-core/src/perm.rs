//! Centralized permission checks.
//!
//! Every command handler routes its authority questions through this module
//! — nobody inlines flag math. The core primitive is [`controls`]:
//!
//! > `who` controls `what` iff `who` is `what` itself, **or** `who` is
//! > Wizard-or-better at least matching `what`'s owner/object role (`Wizard`
//! > cannot control `#1` God or God-owned objects), **or** `who` (or `who`'s
//! > owner) owns `what`, **or** `what`'s Zone Master Object has a `Zone` lock
//! > that `who` passes (delegated zone administration, design doc §2.6; never
//! > grants control over Wizard/God-owned objects to lower-role actors).
//!
//! Granular Rhost-style [`Powers`] refine the picture: [`has_power`] is
//! automatically satisfied for Wizard+ so staff never need explicit grants.

use crate::flags::{Flags, Powers, Privileges, Role};
use crate::lock::{LockType, MAX_LOCK_DEPTH, eval_lock};
use crate::obj::{ObjId, ObjKind};
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

/// Whether `who` holds the given privilege. Unlike [`has_power`] there is
/// **no role short-circuit**: a privilege is an explicit, audited grant, and
/// God and Wizards are refused exactly like anyone else until one is made
/// (D-D-14). Never route a privilege check through `has_power`.
#[must_use]
pub fn has_privilege(w: &World, who: ObjId, privilege: Privileges) -> bool {
    w.get(who).is_some_and(|o| o.privileges.contains(privilege))
}

/// The classic "does `who` control `what`" check: self ∨ God ∨ Wizard (for
/// non-God targets) ∨ owner ∨ zone delegation (the ZMO's `Zone` lock, which
/// never confers control over a Wizard/God-owned object to a lower-role actor).
///
/// Enforces the `Wizard < God` authority boundary (P2-CORE-21): targeting `#1`
/// (`GOD`) or any God-owned object requires `Role::God` (`who == GOD`) unless
/// `who` itself is owned by `GOD` (e.g. a God-owned puppet acting on God's
/// objects).
#[must_use]
pub fn controls(w: &World, who: ObjId, what: ObjId) -> bool {
    let Some(a) = w.get(who) else { return false };
    let Some(b) = w.get(what) else { return false };
    if who == what {
        return true;
    }
    let target_owner_role = role_of(w, b.owner).max(b.role());
    if a.role() >= Role::Wizard {
        return a.role() >= target_owner_role;
    }
    // Ownership: `what` owned by `who` directly, or by `who`'s owner (so a
    // player's puppets/objects speak with the player's authority).
    if b.owner == who || b.owner == a.owner {
        return true;
    }
    // Zone delegation: the ZMO's Zone lock grants control over members, except
    // that zone delegation never confers control over an object owned by (or
    // carrying) `Role::Wizard` or `Role::God` unless `who` already holds at
    // least that role (EXT-P1-1).
    if (target_owner_role < Role::Wizard || a.role() >= target_owner_role)
        && let Some(z) = b.zone
        && let Some(zo) = w.get(z)
        && zo.flags.contains(Flags::ZONE_MASTER)
        && let Some(expr) = zo.locks.get(&LockType::Zone)
        && eval_lock(w, expr, who, MAX_LOCK_DEPTH)
    {
        return true;
    }
    false
}

/// Whether `name` belongs to the executable `script:` attribute namespace
/// (case-insensitive).
#[must_use]
pub fn is_script_attr(name: &str) -> bool {
    name.split_once(':')
        .is_some_and(|(ns, _)| ns.eq_ignore_ascii_case("script"))
}

/// Whether `actor` may write, clear, or flag an executable `script:*`
/// attribute on `target`.
///
/// Requires [`can_modify`] and at least [`Role::Builder`], and refuses
/// mutating `script:*` on an object owned by (or carrying) a strictly higher
/// [`Role`] than `actor` — preventing delegated zone controllers or lower-role
/// staff from planting executable scripts on higher-privilege objects
/// (EXT-P1-1).
#[must_use]
pub fn can_write_script_attr(w: &World, actor: ObjId, target: ObjId) -> bool {
    if !can_modify(w, actor, target) {
        return false;
    }
    let Some(a) = w.get(actor) else { return false };
    let Some(t) = w.get(target) else { return false };
    let actor_role = a.role();
    if actor_role < Role::Builder {
        return false;
    }
    let target_owner_role = role_of(w, t.owner).max(t.role());
    actor_role >= target_owner_role
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
/// Structural & authority requirements (P2-CORE-12):
/// 1. **Object kinds**: rooms may never be teleported; exits may never be
///    destinations; actors (`Player`/`Npc`) and exits may only be moved into
///    `Room` destinations.
/// 2. **The body**: moving something other than yourself needs control over
///    it or the `TEL_ANYTHING` power.
/// 3. **The destination**: relocating an `Exit` always requires
///    `controls(w, who, dest)` (matching `@open`, forbidding non-controlling
///    exit injection into `JUMP_OK` rooms); other targets require control over
///    `dest`, `JUMP_OK`, or `TEL_ANYWHERE`.
/// 4. **The destination's `Teleport` lock**, evaluated against `target`
///    (Wizard+ bypasses a failing lock).
#[must_use]
pub fn can_teleport(w: &World, who: ObjId, target: ObjId, dest: ObjId) -> bool {
    let Some(d) = w.get(dest) else { return false };
    let Some(t) = w.get(target) else { return false };
    if t.kind == ObjKind::Room {
        return false;
    }
    if d.kind == ObjKind::Exit {
        return false;
    }
    if (t.kind == ObjKind::Exit || t.kind.is_actor()) && d.kind != ObjKind::Room {
        return false;
    }
    if target != who && !controls(w, who, target) && !has_power(w, who, Powers::TEL_ANYTHING) {
        return false;
    }
    let dest_open = if t.kind == ObjKind::Exit {
        controls(w, who, dest)
    } else {
        controls(w, who, dest)
            || d.flags.contains(Flags::JUMP_OK)
            || has_power(w, who, Powers::TEL_ANYWHERE)
    };
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
    fn zone_lock_never_delegates_control_over_wizard_or_god_objects() {
        let (mut w, nick, _bob, _ball, rock) = setup();
        let builder = w.create(ObjKind::Player, "Bld", GOD, Some(LIMBO)).unwrap();
        w.set_flag(builder, Flags::BUILDER, true).unwrap();
        let royalty = w.create(ObjKind::Player, "Roy", GOD, Some(LIMBO)).unwrap();
        w.set_flag(royalty, Flags::ROYALTY, true).unwrap();
        let wiz = w.create(ObjKind::Player, "Wiz", GOD, Some(LIMBO)).unwrap();
        w.set_flag(wiz, Flags::WIZARD, true).unwrap();

        let zmo = w.create(ObjKind::Thing, "ZMO", GOD, Some(LIMBO)).unwrap();
        w.set_flag(zmo, Flags::ZONE_MASTER, true).unwrap();
        w.get_mut(zmo)
            .unwrap()
            .locks
            .insert(LockType::Zone, LockExpr::Const(true));

        let god_thing = w
            .create(ObjKind::Thing, "god_thing", GOD, Some(LIMBO))
            .unwrap();
        let wiz_thing = w
            .create(ObjKind::Thing, "wiz_thing", wiz, Some(LIMBO))
            .unwrap();
        let roy_thing = w
            .create(ObjKind::Thing, "roy_thing", royalty, Some(LIMBO))
            .unwrap();
        for obj in [rock, god_thing, wiz_thing, roy_thing] {
            w.set_zone(obj, Some(zmo)).unwrap();
        }

        // Mortal/Builder/Royalty cannot gain control over Wizard- or God-owned
        // objects through a ZMO Zone lock.
        for actor in [nick, builder, royalty] {
            assert!(!controls(&w, actor, god_thing));
            assert!(!can_modify(&w, actor, god_thing));
            assert!(!controls(&w, actor, wiz_thing));
            assert!(!can_modify(&w, actor, wiz_thing));
            // Non-Wizard members (Bob's rock, Royalty's thing) remain delegable.
            assert!(controls(&w, actor, rock));
            assert!(controls(&w, actor, roy_thing));
        }

        // Staff at Wizard+ still control everything.
        assert!(controls(&w, wiz, wiz_thing));
        assert!(controls(&w, GOD, god_thing));
    }

    #[test]
    fn can_write_script_attr_blocks_higher_role_targets() {
        let (mut w, nick, _bob, _ball, rock) = setup();
        let builder_a = w.create(ObjKind::Player, "BldA", GOD, Some(LIMBO)).unwrap();
        w.set_flag(builder_a, Flags::BUILDER, true).unwrap();
        let builder_b = w.create(ObjKind::Player, "BldB", GOD, Some(LIMBO)).unwrap();
        w.set_flag(builder_b, Flags::BUILDER, true).unwrap();
        let royalty = w.create(ObjKind::Player, "Roy", GOD, Some(LIMBO)).unwrap();
        w.set_flag(royalty, Flags::ROYALTY, true).unwrap();
        let wiz = w.create(ObjKind::Player, "Wiz", GOD, Some(LIMBO)).unwrap();
        w.set_flag(wiz, Flags::WIZARD, true).unwrap();

        let zmo = w.create(ObjKind::Thing, "ZMO", GOD, Some(LIMBO)).unwrap();
        w.set_flag(zmo, Flags::ZONE_MASTER, true).unwrap();
        w.get_mut(zmo)
            .unwrap()
            .locks
            .insert(LockType::Zone, LockExpr::Const(true));

        let b_thing = w
            .create(ObjKind::Thing, "b_thing", builder_b, Some(LIMBO))
            .unwrap();
        let roy_thing = w
            .create(ObjKind::Thing, "roy_thing", royalty, Some(LIMBO))
            .unwrap();
        let wiz_thing = w
            .create(ObjKind::Thing, "wiz_thing", wiz, Some(LIMBO))
            .unwrap();
        let god_thing = w
            .create(ObjKind::Thing, "god_thing", GOD, Some(LIMBO))
            .unwrap();
        for obj in [rock, b_thing, roy_thing, wiz_thing, god_thing] {
            w.set_zone(obj, Some(zmo)).unwrap();
        }

        assert!(is_script_attr("script:use"));
        assert!(is_script_attr("SCRIPT:command:hi"));
        assert!(!is_script_attr("desc"));
        assert!(!is_script_attr("scripted:note"));

        // Plain player can never write script:* anywhere.
        assert!(!can_write_script_attr(&w, nick, nick));
        assert!(!can_write_script_attr(&w, nick, rock));

        // Builder may write script:* on equal/lower-role controlled targets,
        // never on Royalty/Wizard/God objects.
        assert!(can_write_script_attr(&w, builder_a, builder_a));
        assert!(can_write_script_attr(&w, builder_a, b_thing));
        assert!(can_write_script_attr(&w, builder_a, rock));
        assert!(!can_write_script_attr(&w, builder_a, roy_thing));
        assert!(!can_write_script_attr(&w, builder_a, wiz_thing));
        assert!(!can_write_script_attr(&w, builder_a, god_thing));

        // Royalty may script Royalty/Builder targets, not Wizard/God.
        assert!(can_write_script_attr(&w, royalty, roy_thing));
        assert!(can_write_script_attr(&w, royalty, b_thing));
        assert!(!can_write_script_attr(&w, royalty, wiz_thing));
        assert!(!can_write_script_attr(&w, royalty, god_thing));

        // Wizard may script Wizard/Royalty/Builder targets, not God's.
        assert!(can_write_script_attr(&w, wiz, wiz_thing));
        assert!(can_write_script_attr(&w, wiz, roy_thing));
        assert!(!can_write_script_attr(&w, wiz, god_thing));

        // God may script anything non-GOING.
        assert!(can_write_script_attr(&w, GOD, god_thing));
        assert!(can_write_script_attr(&w, GOD, b_thing));
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
    fn privilege_checks_have_no_role_short_circuit() {
        let (mut w, nick, _bob, _ball, _rock) = setup();
        // Neither God nor a mortal holds a privilege by default (D-D-14).
        assert!(!has_privilege(&w, GOD, Privileges::DISCORD_IDENTITY));
        assert!(!has_privilege(&w, nick, Privileges::DISCORD_IDENTITY));
        // A Wizard flag changes nothing: only the explicit bit does.
        w.set_flag(nick, Flags::WIZARD, true).unwrap();
        assert!(has_power(&w, nick, Powers::BOOT));
        assert!(!has_privilege(&w, nick, Privileges::DISCORD_IDENTITY));
        w.get_mut(nick).unwrap().privileges = Privileges::DISCORD_IDENTITY;
        assert!(has_privilege(&w, nick, Privileges::DISCORD_IDENTITY));
        // An unknown object holds nothing.
        assert!(!has_privilege(&w, ObjId(99), Privileges::DISCORD_IDENTITY));
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

    #[test]
    fn p2_core_21_wizard_does_not_control_god_or_god_owned_objects() {
        let (mut w, nick, _bob, ball, _rock) = setup();
        let wiz = w.create(ObjKind::Player, "Wiz", GOD, Some(LIMBO)).unwrap();
        w.set_flag(wiz, Flags::WIZARD, true).unwrap();
        let wiz_thing = w
            .create(ObjKind::Thing, "wthing", wiz, Some(LIMBO))
            .unwrap();
        let god_thing = w
            .create(ObjKind::Thing, "gthing", GOD, Some(LIMBO))
            .unwrap();

        // Wizard cannot control #1 (GOD) or any GOD-owned object (incl. Limbo).
        assert!(!controls(&w, wiz, GOD));
        assert!(!controls(&w, wiz, god_thing));
        assert!(!controls(&w, wiz, LIMBO));
        assert!(!can_modify(&w, wiz, GOD));
        assert!(!can_modify(&w, wiz, god_thing));

        // Wizard retains control over self, Wizard-owned, and lower-role objects.
        assert!(controls(&w, wiz, wiz));
        assert!(controls(&w, wiz, wiz_thing));
        assert!(controls(&w, wiz, nick));
        assert!(controls(&w, wiz, ball));

        // GOD controls everything, including #1 and Wizard objects.
        assert!(controls(&w, GOD, GOD));
        assert!(controls(&w, GOD, wiz));
        assert!(controls(&w, GOD, wiz_thing));
        assert!(controls(&w, GOD, god_thing));
    }

    #[test]
    fn p2_core_12_can_teleport_restricts_rooms_exits_and_destinations() {
        let (mut w, nick, _bob, ball, _rock) = setup();
        let own_room = w.create(ObjKind::Room, "OwnRoom", nick, None).unwrap();
        let own_room2 = w.create(ObjKind::Room, "OwnRoom2", nick, None).unwrap();
        let open = w.create(ObjKind::Room, "Open", GOD, None).unwrap();
        w.set_flag(open, Flags::JUMP_OK, true).unwrap();
        let my_exit = w
            .create(ObjKind::Exit, "north", nick, Some(own_room))
            .unwrap();

        // Relocating an exit requires controlling dest — JUMP_OK and
        // TEL_ANYWHERE never suffice.
        assert!(!can_teleport(&w, nick, my_exit, open));
        w.get_mut(nick).unwrap().powers = Powers::TEL_ANYWHERE;
        assert!(!can_teleport(&w, nick, my_exit, open));
        assert!(can_teleport(&w, nick, my_exit, own_room2));

        // Rooms may never be teleported, even by GOD.
        assert!(!can_teleport(&w, nick, own_room, open));
        assert!(!can_teleport(&w, GOD, own_room, open));

        // Exits may never be teleport destinations; actors require Room dest.
        assert!(!can_teleport(&w, nick, ball, my_exit));
        assert!(!can_teleport(&w, GOD, ball, my_exit));
        assert!(!can_teleport(&w, nick, nick, ball));
        assert!(!can_teleport(&w, GOD, nick, ball));
        assert!(!can_teleport(&w, nick, my_exit, ball));
    }
}
