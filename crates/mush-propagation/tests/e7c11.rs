//! E7 cut 11 **B0**, the propagation half (10b §3.10 deviation (h)): what a
//! proxy driver may perceive and whom they may order.
//!
//! Cut 10 shipped no driving ingress, so `+contacts` and `+order` from a proxy
//! driver resolved through the driver's OWN character — which holds no billet
//! and stands nowhere near the unit. Cut 10's fold refuted its own K12 finding
//! with the honest inverse: *"a proxy driver today perceives **nothing** of the
//! unit through `+contacts`"*. With the ingress the driver perceives the NPC's
//! picture and only the NPC's (T12), and their orders are the NPC's orders — and
//! deviation (h)'s narrowing is what keeps that from being the unrestricted
//! "reachable through a station" act T11 forbids.
//!
//! The seat itself is constructed here as a `FocusState` claim rather than
//! through `+seat/activate`: `mush-propagation` has no `mush-campaign` edge in
//! either direction and must not gain one. The **commands** under test still run
//! through the real command path, and the seat's own path is driven end to end
//! in `crates/mush-campaign/tests/e7c11.rs`.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::{
    AccountId, AccountIdentity, Audience, BehaviorBinding, BehaviorId, ClockStamp, ContactId,
    ContactLedgerRow, ContactLedgerState, GOD, IdempotencyKeyText, LIMBO, ObjId, ObjKind,
    PrincipalId,
};
use mush_propagation::{PropagationModule, PropagationSensorPolicy, PropagationStore};
use mush_proto::OutputEvent;
use mush_sim::{AccountOracle, ActiveFocus, FocusState, ReachProof, SimulationModule};
use mush_tactics::TacticsModule;
use mush_testkit::{TestWorld, text_of};

// ---- E7c11: B0 (rig) ----

struct MapOracle(std::sync::Mutex<BTreeMap<ObjId, AccountId>>);

impl AccountIdentity for MapOracle {
    fn account_of(&self, character: ObjId) -> Option<AccountId> {
        self.0.lock().ok()?.get(&character).copied()
    }
}

impl std::fmt::Debug for MapOracle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MapOracle")
    }
}

fn boot() -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(SimulationModule::new()),
        Box::new(TacticsModule::new().with_sensor_policy(Arc::new(PropagationSensorPolicy))),
        Box::new(PropagationModule::new()),
    ])
}

fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n")
}

fn run(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    texts(&tw.run(actor, line))
}

fn bind(tw: &mut TestWorld, obj: ObjId, behavior: &str) {
    tw.world_mut()
        .attach_behavior(
            obj,
            BehaviorId::new(behavior).unwrap(),
            BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )
        .unwrap();
}

struct Fx {
    orbit: ObjId,
    bunker: ObjId,
    atlas: ObjId,
    target: ObjId,
    ace: ObjId,
    driver: ObjId,
    neighbour: ObjId,
    account: AccountId,
}

/// A domain holding a crewed unit and a third object, plus a **separate** room
/// where the absent player's character stands with a neighbour. The two places
/// are what make the ingress's locality substitution observable at all.
fn scene() -> (TestWorld, Fx) {
    let mut tw = boot();
    tw.set_now(1_000);
    let orbit = tw
        .world_mut()
        .create(ObjKind::Room, "Orbit", GOD, None)
        .unwrap();
    let bunker = tw
        .world_mut()
        .create(ObjKind::Room, "Bunker", GOD, None)
        .unwrap();
    let atlas = tw
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, Some(LIMBO))
        .unwrap();
    let target = tw
        .world_mut()
        .create(ObjKind::Thing, "Target", GOD, Some(LIMBO))
        .unwrap();
    let ace = tw
        .world_mut()
        .create(ObjKind::Npc, "Ace", GOD, Some(LIMBO))
        .unwrap();
    let driver = tw
        .world_mut()
        .create(ObjKind::Player, "Driver", GOD, Some(LIMBO))
        .unwrap();
    let neighbour = tw
        .world_mut()
        .create(ObjKind::Player, "Neighbour", GOD, Some(LIMBO))
        .unwrap();
    bind(&mut tw, orbit, "sim.domain");
    for participant in [atlas, target, ace] {
        bind(&mut tw, participant, "sim.participant");
    }
    let said = run(
        &mut tw,
        GOD,
        &format!("@domain/create #{}=sim.passive", orbit.0),
    );
    assert!(!said.contains("Usage"), "{said}");
    for participant in [atlas, target, ace] {
        run(
            &mut tw,
            GOD,
            &format!("@spatial/place/domain #{}=0", participant.0),
        );
    }
    run(
        &mut tw,
        GOD,
        &format!(
            "@spatial/station #{}=pilot,pilot,channels:movement",
            atlas.0
        ),
    );
    run(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=pilot,#{}", atlas.0, ace.0),
    );
    // The absent character and their neighbour stand in an ordinary room, far
    // from the hull: T13's "the player's character never moves".
    for who in [driver, neighbour] {
        tw.world_mut().move_to(who, bunker).unwrap();
    }
    let account = AccountId::mint("DriverAcct", 1).unwrap();
    let oracle = Arc::new(MapOracle(std::sync::Mutex::new(BTreeMap::from([(
        driver, account,
    )]))));
    *tw.world_mut()
        .module_state_mut::<AccountOracle>()
        .expect("the oracle slot") = AccountOracle::new(oracle);
    (
        tw,
        Fx {
            orbit,
            bunker,
            atlas,
            target,
            ace,
            driver,
            neighbour,
            account,
        },
    )
}

/// Install the proxy focus cut 10's `+seat/activate` installs — the same
/// `ActiveFocus`, with `actor = npc` and `proxy = true`.
fn take_proxy_seat(tw: &mut TestWorld, fx: &Fx) {
    let acquired_at = tw.world().world_ic_now();
    let focus = ActiveFocus {
        account: fx.account,
        actor: fx.ace,
        // The independent verify fold's F4/B4: the drive is bound to the
        // character that took the seat, so the seed must name it too.
        driver: fx.driver,
        unit: fx.atlas,
        station: mush_sim::StationId::new("pilot").unwrap(),
        acquired_at,
        epoch: 1,
        reach: ReachProof::Console { room: fx.orbit },
        proxy: true,
    };
    // F4/B5: `FocusState::claim` is `pub(crate)` now; the seeding door is the
    // `testkit`-fenced shim, reached through this crate's dev-dependency on
    // `mush-sim`'s `testkit` feature (not a new edge).
    let released = tw
        .world_mut()
        .module_state_mut::<FocusState>()
        .expect("the focus slot")
        .claim_for_test(focus);
    assert!(released.is_empty());
}

fn ledger_row(tw: &TestWorld, holder: ObjId, contact: [u8; 16]) -> ContactLedgerRow {
    let ClockStamp { clock, at } = tw.world().world_ic_now();
    ContactLedgerRow {
        holder: PrincipalId::Object { id: holder },
        contact: ContactId::from_bytes(contact),
        generation: 1,
        revision: 1,
        active: true,
        observed_at: ClockStamp { clock, at },
        phantom: None,
    }
}

fn install_ledger(tw: &mut TestWorld, rows: Vec<ContactLedgerRow>) {
    let at = tw.world().world_ic_now();
    tw.world_mut()
        .module_state_mut::<ContactLedgerState>()
        .expect("the tactics ledger slot")
        .install(rows, at);
}

fn envelopes(tw: &TestWorld) -> Vec<(ObjId, ObjId)> {
    tw.world()
        .module_state::<PropagationStore>()
        .expect("the propagation store")
        .envelopes()
        .map(|e| (e.issuer(), e.recipient()))
        .collect()
}

// ===========================================================================
// B0 — perception and orders through the ingress
// ===========================================================================

/// **B0-P1. A proxy driver perceives the NPC's picture and only the NPC's.**
///
/// Before the seat, `+contacts` shows the driver's own (empty) roster; after it,
/// the unit's contact — and the driver's own character's contact disappears from
/// the render, because T12 exposes *that NPC station's* observer projection and
/// not a union of the two.
#[test]
fn e7c11_b0_a_proxy_driver_reads_the_units_picture_and_not_their_own() {
    let (mut tw, fx) = scene();
    let rows = vec![
        ledger_row(&tw, fx.atlas, [0x11; 16]),
        ledger_row(&tw, fx.driver, [0x22; 16]),
    ];
    install_ledger(&mut tw, rows);
    let before = run(&mut tw, fx.driver, "+contacts");
    assert!(
        before.contains("22222222"),
        "the driver's own row: {before}"
    );
    assert!(
        !before.contains("11111111"),
        "the unit's row is not theirs yet: {before}"
    );
    take_proxy_seat(&mut tw, &fx);
    let after = run(&mut tw, fx.driver, "+contacts");
    assert!(
        after.contains("11111111"),
        "the seat exposes the unit's picture: {after}"
    );
    assert!(
        !after.contains("22222222"),
        "…and ONLY the unit's: the absent character's own row is out of the \
         projection while the seat is live: {after}"
    );
}

/// **B0-P2. An order from a seat is the NPC's order** — the recipient is
/// addressed through the NPC's reach, and the envelope names the NPC as issuer.
/// The account never appears (T12/T13); its attribution is cut 10's `SeatAudit`.
#[test]
fn e7c11_b0_an_order_from_a_seat_is_issued_by_the_npc() {
    let (mut tw, fx) = scene();
    // Out of the seat the driver cannot reach the hull's neighbourhood at all.
    let refused = run(&mut tw, fx.driver, &format!("+order #{}=hold", fx.target.0));
    assert!(refused.contains("No such contact."), "{refused}");
    assert!(envelopes(&tw).is_empty(), "nothing was queued");
    take_proxy_seat(&mut tw, &fx);
    let queued = run(&mut tw, fx.driver, &format!("+order #{}=hold", fx.target.0));
    assert!(queued.contains("Queued revision 1"), "{queued}");
    assert_eq!(
        envelopes(&tw),
        vec![(fx.ace, fx.target)],
        "the envelope is issued BY the NPC, never by the absent character"
    );
}

/// **B0-P3. Deviation (h): the seat narrows whom a driver may address.**
///
/// The driver's own room-mate is addressable *as themselves* and stops being so
/// the moment the seat is live: locality is the NPC's, and the roster is the
/// seat's own `[npc, unit]`. Without this narrowing the ingress would hand a
/// proxy the unrestricted "reachable through a station" act T11 names — and the
/// refusal is the byte-identical existence-hiding one, so it discloses nothing
/// about who is standing where.
#[test]
fn e7c11_b0_a_seat_narrows_who_a_driver_may_address() {
    let (mut tw, fx) = scene();
    let own = run(
        &mut tw,
        fx.driver,
        &format!("+order #{}=stand by", fx.neighbour.0),
    );
    assert!(own.contains("Queued revision 1"), "{own}");
    take_proxy_seat(&mut tw, &fx);
    let narrowed = run(
        &mut tw,
        fx.driver,
        &format!("+order #{}=stand by", fx.neighbour.0),
    );
    assert!(narrowed.contains("No such contact."), "{narrowed}");
    // Byte-identical to an object that does not exist at all.
    let absent = run(&mut tw, fx.driver, "+order #99999=stand by");
    assert_eq!(narrowed, absent, "the refusals must not be distinguishable");
    assert_eq!(
        envelopes(&tw),
        vec![(fx.driver, fx.neighbour)],
        "only the pre-seat order exists"
    );
    // And the driver is still where they were: the seat moved nobody (T13).
    assert_eq!(
        tw.world().get(fx.driver).and_then(|o| o.location),
        Some(fx.bunker)
    );
}

/// **B0-P4. The ingress is inert for everybody else.** A player with no seat, a
/// character the oracle does not map and the NPC itself all resolve to
/// themselves, so nothing about cut 7's ordering surface moved.
#[test]
fn e7c11_b0_the_ingress_is_inert_without_a_live_seat() {
    let (mut tw, fx) = scene();
    for who in [fx.driver, fx.neighbour, fx.ace, GOD] {
        assert_eq!(mush_sim::driving_actor(tw.world(), who), who);
        assert!(mush_sim::proxy_drive_of(tw.world(), who).is_none());
    }
    take_proxy_seat(&mut tw, &fx);
    // Only the mapped driver resolves; the NPC is a fixed point.
    assert_eq!(mush_sim::driving_actor(tw.world(), fx.driver), fx.ace);
    assert_eq!(mush_sim::driving_actor(tw.world(), fx.ace), fx.ace);
    assert_eq!(
        mush_sim::driving_actor(tw.world(), fx.neighbour),
        fx.neighbour
    );
    let _ = IdempotencyKeyText::parse_canonical("e7c11.b0.p4").expect("a canonical key");
}

// ===========================================================================
// ---- E7 cut 11 independent verify fold ----
//
// The picture-and-order half. Owner decision taken: F5=(a) (a live seat
// suppresses the staff short-circuit). F3(a)'s half — `held_contact` reading the
// same holder set the render does — went with the retired combat lane (16b cut
// T, D-16-20), which took `+fire` and the only two positive contact-naming
// proofs in the tree with it.
// This crate has no `mush-campaign` edge and must not gain one, so the seat is
// still seeded through `FocusState`'s `testkit`-fenced shim; the seat's own
// end-to-end path stays in `crates/mush-campaign/tests/e7c11.rs`.
// ===========================================================================

/// The proxy seat, on a named station of the hull.
fn take_proxy_seat_on(tw: &mut TestWorld, fx: &Fx, station: &str) {
    let acquired_at = tw.world().world_ic_now();
    let focus = ActiveFocus {
        account: fx.account,
        actor: fx.ace,
        driver: fx.driver,
        unit: fx.atlas,
        station: mush_sim::StationId::new(station).unwrap(),
        acquired_at,
        epoch: 1,
        reach: ReachProof::Console { room: fx.orbit },
        proxy: true,
    };
    let released = tw
        .world_mut()
        .module_state_mut::<FocusState>()
        .expect("the focus slot")
        .claim_for_test(focus);
    assert!(released.is_empty());
}

/// **D7 / F5. A staff driver is narrowed by the seat like anyone else.**
///
/// `may_address` computed `is_admin` on the TYPING actor and short-circuited on
/// it before the seat roster and the NPC's locality, while `cmd_order` still set
/// `issuer = driving_actor`: a staff account holding a proxy seat issued
/// envelopes AS THE NPC to any object anywhere in the world, laundered onto a
/// character that could neither perceive nor reach the recipient and recoverable
/// only through `@seat/audit`. §3.11's own rule is that T11 forbids the act from
/// the SEAT, not from the person — so a live seat suppresses the short-circuit
/// and staff who want the wider reach stand down first.
#[test]
fn vf_d7_a_staff_driver_is_narrowed_by_the_seat_like_anyone_else() {
    let (mut tw, fx) = scene();
    // The same person, with staff powers, addressing their own room-mate.
    tw.world_mut()
        .get_mut(fx.driver)
        .expect("the driver")
        .flags
        .insert(mush_core::Flags::ROYALTY);
    let own = run(
        &mut tw,
        fx.driver,
        &format!("+order #{}=stand by", fx.neighbour.0),
    );
    assert!(own.contains("Queued revision 1"), "{own}");

    take_proxy_seat_on(&mut tw, &fx, "pilot");
    let narrowed = run(
        &mut tw,
        fx.driver,
        &format!("+order #{}=stand by", fx.neighbour.0),
    );
    assert!(
        narrowed.contains("No such contact."),
        "a staff account holding a seat is narrowed by it: {narrowed}"
    );
    // Byte-identical to an object that does not exist at all: the existence
    // hiding is unchanged.
    let missing = run(&mut tw, fx.driver, "+order #99999=stand by");
    assert_eq!(narrowed, missing);
    assert_eq!(
        envelopes(&tw),
        vec![(fx.driver, fx.neighbour)],
        "no envelope was issued from the seat, and none was attributed to the NPC"
    );
    // The staff gate is untouched for everything the SEAT reaches: the NPC's own
    // neighbourhood is still addressable, as issuer.
    let reached = run(&mut tw, fx.driver, &format!("+order #{}=hold", fx.target.0));
    assert!(reached.contains("Queued revision 1"), "{reached}");
    assert_eq!(
        envelopes(&tw),
        vec![(fx.driver, fx.neighbour), (fx.ace, fx.target)]
    );

    // Stand down, and the same staff member addresses their room-mate again —
    // as themselves. The narrowing was the seat's, never the person's.
    tw.world_mut()
        .module_state_mut::<FocusState>()
        .expect("the focus slot")
        .relinquish(fx.account);
    let again = run(
        &mut tw,
        fx.driver,
        &format!("+order #{}=stand by", fx.neighbour.0),
    );
    assert!(again.contains("Queued revision 2"), "{again}");

    // …and a NON-staff driver is byte-identically narrowed, so the fix changed
    // nothing for anybody who was not staff.
    let plain = {
        let (mut tw2, fx2) = scene();
        take_proxy_seat_on(&mut tw2, &fx2, "pilot");
        run(
            &mut tw2,
            fx2.driver,
            &format!("+order #{}=stand by", fx2.neighbour.0),
        )
    };
    assert_eq!(plain, narrowed);
}
