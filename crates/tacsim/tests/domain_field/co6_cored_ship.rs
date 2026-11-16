//! **Doc 47 unit CO.6: a route to a cored ship.**
//!
//! A first captain owns her Dray and has a governed core made and fitted to her by registered
//! commands alone: `+coop/buyout` makes the ship hers, `+core/buy II go` has the fitting shop
//! make a Class II core from a certified die, and `+core/fit <core>=<ship> go`, typed on the
//! berth gallery where the shop stands and the ship is docked, fits it. The shop's counter does
//! the work: the shop's stock agent issues the die and makes both offers, and the counter
//! submits the captain's own acceptances, reservation, completion, delivery and install, each a
//! typed action its provider checks. Nobody in staff touches her ship or her core.
//!
//! The world is the combined M8 world (the production `LaunchComposition`, `m8_world_manned`)
//! with a chartered Dray docked at Ceres. **What staff do**: top her wallet up for the buyout and
//! the core (the M8 world's standing practice: a story cannot play the months of trading that
//! earn 98,000cr); and, as the deployment does, stand the shop at the berth gallery (the rig's
//! `hardware`). **What the story does by hand**: it moves the clock, and it drives the shop's
//! scheduled hours as the server's schedule driver does (`M8World::progress`).

use mush_core::ObjId;
use mush_economy::core_integration::CoreClass;
use mush_org::PrincipalId;
use tacsim::definitions::core_replacement::SHIP_II;
use tacsim::hardware::counter::{
    CORE_ACCEPTED, CORE_DELIVERED, CORE_END_USER, CORE_FITTED, CORE_NOT_AT_SHOP, CORE_NOT_YOURS,
};

use crate::excursions::{docked_dray, economy, run};
use crate::m8_end_user::{credit, fund};
use crate::m8_world::{M8World, m8_world_manned, title_of};

/// The head of a template, before its first `{`.
fn head(template: &str) -> &str {
    template.split('{').next().expect("a head")
}

/// The combined world with the captain's chartered Dray docked at Ceres, the captain aboard.
pub(crate) fn world() -> M8World {
    m8_world_manned(None, |rig| {
        let (captain, row) = docked_dray(rig, ("Captain", 1, "Salt Moth"), (53, 19));
        (captain, row, Vec::new())
    })
}

/// **The counter's pump**: two seconds on, the due ticks fired, every submission they staged
/// landed on the composed path (a refused one is the counter's to try again), until `done`
/// reads true or `fires` have passed. What the captain heard.
pub(crate) fn pump(
    world: &mut M8World,
    who: ObjId,
    fires: usize,
    done: impl Fn(&M8World) -> bool,
) -> String {
    let mut heard = Vec::new();
    for _ in 0..fires {
        if done(world) {
            return heard.join("\n");
        }
        let t = world.rig.tw.world().now() + 2;
        world.rig.tw.set_now(t);
        let events = world.rig.tw.fire_due(t * 1_000);
        heard.push(crate::excursions::said(&world.rig.tw, &events, who));
        while let Some(submit) = world.rig.tw.engine_mut().take_authoritative_submit() {
            let _ = crate::excursions::land(&mut world.rig, &submit);
        }
    }
    assert!(
        done(world),
        "the counter did not get there: {}",
        heard.join("\n")
    );
    heard.join("\n")
}

/// **`who` walks to `to`** by the exits a player types: the shortest way through the rooms'
/// exits, each typed by its first name. The lines they heard.
pub(crate) fn walk(world: &mut M8World, who: ObjId, to: ObjId) -> String {
    let w = world.rig.tw.world();
    let from = w.get(who).and_then(|o| o.location).expect("somewhere");
    let mut came: std::collections::BTreeMap<ObjId, (ObjId, ObjId)> =
        std::collections::BTreeMap::new();
    let mut queue = std::collections::VecDeque::from([from]);
    while let Some(room) = queue.pop_front() {
        if room == to {
            break;
        }
        for exit in w.get(room).map(|o| o.exits.clone()).unwrap_or_default() {
            let Some(next) = w.get(exit).and_then(|e| e.destination) else {
                continue;
            };
            if next != from && !came.contains_key(&next) {
                came.insert(next, (room, exit));
                queue.push_back(next);
            }
        }
    }
    let mut steps = Vec::new();
    let mut at = to;
    while at != from {
        let (back, exit) = *came.get(&at).expect("a way there");
        steps.push(exit);
        at = back;
    }
    steps.reverse();
    let mut heard = Vec::new();
    for exit in steps {
        let name = world
            .rig
            .tw
            .world()
            .get(exit)
            .map(|e| e.name.split(';').next().unwrap_or_default().to_owned())
            .expect("the exit's name");
        heard.push(run(&mut world.rig.tw, who, &name));
    }
    assert_eq!(
        place_of(world, who),
        Some(to),
        "walked there: {}",
        heard.join(
            "
"
        )
    );
    heard.join(
        "
",
    )
}

/// Where `thing` is.
fn place_of(world: &M8World, thing: ObjId) -> Option<ObjId> {
    world.rig.tw.world().get(thing).and_then(|o| o.location)
}

/// The governed cores in `who`'s hands, titled to them.
fn cores_in_hand(world: &M8World, who: ObjId) -> Vec<ObjId> {
    let w = world.rig.tw.world();
    w.get(who)
        .map(|o| o.contents.clone())
        .unwrap_or_default()
        .into_iter()
        .filter(|thing| {
            economy(&world.rig.tw)
                .core_title_birth(*thing)
                .ok()
                .flatten()
                .is_some()
        })
        .collect()
}

/// The core `hull` holds, as the arena reads it.
fn core_of(world: &M8World, hull: ObjId) -> Option<(tacsim::arena::core::CoreClass, bool)> {
    mush_tactics::module::unit_of(world.rig.tw.world(), hull)
        .and_then(tacsim::arena::core::read_core)
        .map(|hw| (hw.class, hw.governed))
}

/// The root `hull`'s unit is bound to.
fn root_of(world: &M8World, hull: ObjId) -> String {
    mush_tactics::module::unit_of(world.rig.tw.world(), hull)
        .map(|u| u.schema().key().id.as_str().to_owned())
        .expect("the hull's unit")
}

/// **The route, from a chartered ship to a cored one.** Each step is a registered command the
/// captain types; the counter's tick does the shop's half. `(core, hull)` at the end, the core in
/// the hull.
pub(crate) fn core_her(world: &mut M8World) -> (ObjId, ObjId) {
    let (captain, hull) = (world.pilot, ObjId(world.row.hull));
    // Out on the gallery, where the shop stands and the ship is docked.
    let left = run(&mut world.rig.tw, captain, "+ship/leave");
    assert!(left.contains("Visiting Berth Gallery"), "{left}");
    assert_eq!(place_of(world, hull), place_of(world, captain));
    // A Co-op charter is the Co-op's hull: the shop will not fit her.
    let early = run(&mut world.rig.tw, captain, "+core/fit core=Salt Moth");
    assert!(early.contains("You hold no governed core"), "{early}");
    fund(world, captain, 9_810_000);
    // The buyout, by the Co-op's own verb.
    let gallery = place_of(world, captain).expect("the gallery");
    let desk = crate::excursions::station_room(&world.rig.tw, "ceres_port", "room.c05_registry");
    let _ = walk(world, captain, desk);
    let bought = run(&mut world.rig.tw, captain, "+coop/buyout");
    assert!(
        bought.contains("The Registry writes the title transfer"),
        "{bought}"
    );
    let _ = pump(world, captain, 40, |w| {
        title_of(w, hull).title_owner == PrincipalId::Object(w.pilot)
    });
    let _ = walk(world, captain, gallery);
    let title = title_of(world, hull);
    assert!(
        title.scope.is_none() && title.encumbrances.is_empty(),
        "clear title: {title:?}"
    );
    // The core: a preview, then the order.
    let before = credit(world, captain).1;
    let preview = run(&mut world.rig.tw, captain, "+core/buy II");
    assert!(preview.contains("8000.00cr"), "{preview}");
    assert_eq!(credit(world, captain).1, before, "a preview pays nothing");
    let ordered = run(&mut world.rig.tw, captain, "+core/buy II go");
    assert!(
        ordered.contains("The counter takes your order"),
        "{ordered}"
    );
    let busy = run(&mut world.rig.tw, captain, "+core/buy II go");
    assert!(busy.contains("already working for you"), "{busy}");
    let heard = pump(world, captain, 60, |w| {
        tacsim::hardware::counter::working(w.rig.tw.world(), w.pilot)
            .is_some_and(|stage| stage.contains("at work"))
    });
    assert!(heard.contains(head(CORE_ACCEPTED)), "{heard}");
    // The shop's 48 hours, as the server's schedule driver runs them.
    world.progress("advance_core_integration", 48);
    let heard = pump(world, captain, 60, |w| {
        !cores_in_hand(w, w.pilot).is_empty()
    });
    let heard = heard
        + pump(world, captain, 10, |w| {
            tacsim::hardware::counter::working(w.rig.tw.world(), w.pilot).is_none()
        })
        .as_str();
    assert!(heard.contains(head(CORE_DELIVERED)), "{heard}");
    assert_eq!(
        credit(world, captain).1,
        before - CoreClass::II.finished_minor(),
        "the core's price"
    );
    let core = cores_in_hand(world, captain)[0];
    let birth = economy(&world.rig.tw)
        .core_title_birth(core)
        .expect("the history")
        .expect("the birth");
    assert_eq!((birth.class, birth.requester), (CoreClass::II, captain));
    assert_eq!(
        title_of(world, core).title_owner,
        PrincipalId::Object(captain),
        "the core is titled to her"
    );
    // The fitting, signed where the shop stands, with the ship docked there.
    let core_ref = format!("#{}", core.0);
    let aboard = run(&mut world.rig.tw, captain, "+ship/board");
    assert!(!aboard.is_empty(), "{aboard}");
    let away = run(
        &mut world.rig.tw,
        captain,
        &format!("+core/fit {core_ref}=Salt Moth go"),
    );
    assert!(
        away.contains(head(CORE_NOT_AT_SHOP)),
        "aboard her, the captain is not at the shop: {away}"
    );
    let _ = run(&mut world.rig.tw, captain, "+ship/leave");
    let paid = credit(world, captain).1;
    let preview = run(
        &mut world.rig.tw,
        captain,
        &format!("+core/fit {core_ref}=Salt Moth"),
    );
    assert!(preview.contains("100.00cr"), "{preview}");
    let fitting = run(
        &mut world.rig.tw,
        captain,
        &format!("+core/fit {core_ref}=Salt Moth go"),
    );
    assert!(fitting.contains("takes the Salt Moth in hand"), "{fitting}");
    let _ = pump(world, captain, 60, |w| {
        tacsim::hardware::counter::working(w.rig.tw.world(), w.pilot)
            .is_some_and(|stage| stage.contains("at work"))
    });
    world.progress("advance_core_refit", 2);
    let heard = pump(world, captain, 60, |w| {
        tacsim::hardware::counter::working(w.rig.tw.world(), w.pilot).is_none()
    });
    assert!(heard.contains(head(CORE_FITTED)), "{heard}");
    assert_eq!(
        credit(world, captain).1,
        paid - mush_economy::core_refit::FITTING_MINOR,
        "the fitting fee"
    );
    (core, hull)
}

/// **A captain has a governed core fitted to her own Dray by registered commands alone**, and
/// a restart reads it: her ship's root is the Dray's governed Class II, the core she paid for is
/// in her, the Co-op's yard will not re-dress her, and `+ship/mind` no longer answers that she
/// has no core.
#[test]
fn co6_a_captain_has_a_governed_core_fitted_to_her_ship_by_her_own_commands() {
    let mut world = world();
    let captain = world.pilot;
    let hull = ObjId(world.row.hull);
    assert_eq!(core_of(&world, hull), None, "a shipped Dray is coreless");
    let (core, hull) = core_her(&mut world);
    assert_eq!(root_of(&world, hull), SHIP_II);
    assert_eq!(
        core_of(&world, hull),
        Some((tacsim::arena::core::CoreClass::II, true)),
        "Class II, governed"
    );
    assert_eq!(place_of(&world, core), Some(hull), "the core is in her");
    let mut world = world.reboot();
    assert_eq!(root_of(&world, hull), SHIP_II, "after the restart");
    assert_eq!(place_of(&world, core), Some(hull));
    // The Co-op's yard will not re-dress her: the rebind would drop the core's mount.
    let redress = run(&mut world.rig.tw, captain, "+coop/refit prospector");
    assert!(
        redress.contains(&tacsim::port::refit::REFIT_CORED.replace("{name}", "Salt Moth")),
        "{redress}"
    );
    assert_eq!(root_of(&world, hull), SHIP_II, "still cored");
    let _ = run(&mut world.rig.tw, captain, "+ship/board");
    let mind = run(&mut world.rig.tw, captain, "+ship/mind");
    assert!(
        !mind.contains(head(tacsim::port::ship_mind::MIND_NO_CORE)),
        "{mind}"
    );
}

/// **The counter's refusals**: a Class III core for a captain who is not a registered end user,
/// and a fitting to a ship she does not hold clear title to (her Co-op charter). Nothing is
/// queued or paid for either.
#[test]
fn co6_the_shop_fits_only_her_own_ship_at_the_shop() {
    let mut world = world();
    let captain = world.pilot;
    fund(&mut world, captain, 900_000);
    let _ = run(&mut world.rig.tw, captain, "+ship/leave");
    // Class III is built only for a registered end user; nothing is queued or paid.
    let wallet = credit(&world, captain).1;
    let third = run(&mut world.rig.tw, captain, "+core/buy III go");
    assert!(third.contains(CORE_END_USER), "{third}");
    assert!(tacsim::hardware::counter::working(world.rig.tw.world(), captain).is_none());
    assert_eq!(credit(&world, captain).1, wallet);
    let _ = run(&mut world.rig.tw, captain, "+core/buy I go");
    let _ = pump(&mut world, captain, 60, |w| {
        tacsim::hardware::counter::working(w.rig.tw.world(), w.pilot)
            .is_some_and(|stage| stage.contains("at work"))
    });
    world.progress("advance_core_integration", 24);
    let _ = pump(&mut world, captain, 60, |w| {
        tacsim::hardware::counter::working(w.rig.tw.world(), w.pilot).is_none()
    });
    let core = cores_in_hand(&world, captain)[0];
    let chartered = run(
        &mut world.rig.tw,
        captain,
        &format!("+core/fit #{}=Salt Moth go", core.0),
    );
    assert!(
        chartered.contains(&CORE_NOT_YOURS.replace("{hull}", "Salt Moth")),
        "{chartered}"
    );
    assert!(
        tacsim::hardware::counter::working(world.rig.tw.world(), captain).is_none(),
        "nothing queued"
    );
}

/// **Doc 48 cut EX.2: a hull with a governed core fitted is not listed, for now.** Her core
/// changes hands only through its own sale, which is not open, and the refusal says so; nothing
/// is listed.
#[test]
fn ex2_a_hull_with_a_governed_core_fitted_is_not_listed_yet() {
    let mut world = world();
    let captain = world.pilot;
    let (core, hull) = core_her(&mut world);
    let exchange =
        crate::excursions::station_room(&world.rig.tw, "ceres_port", "room.c07_exchange");
    let _ = walk(&mut world, captain, exchange);
    let out = run(
        &mut world.rig.tw,
        captain,
        &format!("+market/post #{} price=5000000", hull.0),
    );
    assert!(out.contains(tacsim::port::hull_sale::HULL_CORED), "{out}");
    // Nor is the core itself listed: a governed core sells only in its own family.
    let out = run(
        &mut world.rig.tw,
        captain,
        &format!("+market/post #{} price=5000000", core.0),
    );
    assert!(out.contains(mush_market::MANAGED_CORE_SALE), "{out}");
    assert!(
        !mush_market::asset_on_sale(world.rig.tw.world(), hull)
            && !mush_market::asset_on_sale(world.rig.tw.world(), core),
        "nothing was listed"
    );
}
