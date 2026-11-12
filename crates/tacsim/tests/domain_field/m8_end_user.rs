//! **Doc 44 unit MW.23 (Q-44-40): the launch's route issues the Class III end-user entitlement.**
//!
//! The combined M8 world is the production `LaunchComposition` booted as the bring-up boots it
//! (`m8_world`): the plain world, the `Initialize` boot with its four setup actions, the
//! Registry's grants, the `Active` boot. Until this unit no launch world could hold a Class III
//! core: the economy refuses its integration and its delivery to a requester without the
//! end-user entitlement, and only a test fixture issued one.
//!
//! Here nothing mints it. The pilot types `+enduser/apply go`, pays the Registry its fee, and
//! the same integration that was refused them a moment before is accepted, worked, completed
//! and delivered. Staff commands in these stories do only what a bring-up or a wallet top-up
//! does (credits into a wallet, a wallet opened for a second character); no staff command
//! issues the entitlement, the core or the certified die. The heirloom story's die is the one
//! exception, and it is doc 17h's: heirloom dies are seeded by staff (17h §4, the Tether's back
//! room as a vendor is deferred), and the story is about what the entitlement then does.

use mush_core::{GOD, ObjId};
use mush_economy::core_integration::{
    CORE3_ENTITLEMENT, CoreClass, DieSource, EndUserStanding, end_user_standing,
    require_core_delivery,
};
use mush_economy::{AccountId, AccountStatus};
use mush_industry::core_production::DieClass;
use mush_org::{PrincipalId, SystemPrincipalId};
use serde_json::json;
use tacsim::launch::end_user::{
    END_USER_DONE, END_USER_FEE_MINOR, END_USER_LISTED, END_USER_PREVIEW, END_USER_UNLISTED,
};

use crate::excursions::{economy, god, run};
use crate::m8_world::{M8World, m8_world};

/// The integration's uniform refusal (doc 17h cut M8b-2 fix round 1, L3): what a requester
/// without the entitlement has always been told.
const MANUFACTURE_REFUSAL: &str = "Core manufacture is unavailable or changed.";

/// `owner`'s unscoped account in `instrument`: its id, posted balance and status.
fn holding(
    world: &M8World,
    owner: &PrincipalId,
    instrument: &str,
) -> Option<(AccountId, i64, AccountStatus)> {
    economy(&world.rig.tw)
        .snapshot()
        .accounts_of(owner)
        .filter(|a| a.instrument.as_str() == instrument && a.scope.is_none())
        .min_by_key(|a| a.id)
        .map(|a| (a.id, a.posted_balance.minor(), a.status))
}

fn credit(world: &M8World, who: ObjId) -> (AccountId, i64) {
    let (id, balance, _) = holding(world, &PrincipalId::Object(who), tacsim::economy::CREDIT)
        .expect("a credit account");
    (id, balance)
}

fn balance(world: &M8World, id: AccountId) -> i64 {
    economy(&world.rig.tw)
        .snapshot()
        .account(id)
        .expect("the account")
        .posted_balance
        .minor()
}

fn system(name: &str) -> PrincipalId {
    PrincipalId::System(SystemPrincipalId::new(name).expect("a system principal"))
}

/// Credits into `who`'s wallet from the world's faucet: a top-up, and the only thing minted.
fn fund(world: &mut M8World, who: ObjId, minor: i64) {
    let (payer, _) = credit(world, who);
    let (faucet, _, _) = holding(world, &system("hardware.fixture"), tacsim::economy::CREDIT)
        .expect("the world's faucet");
    god(
        &mut world.rig,
        &format!("@econ/mint {}={} {minor}", faucet.0, payer.0),
    );
}

/// The stock agent issues a certified Class III die and offers its integration to
/// `requester`; `requester` accepts, paying from `payer`. The acceptance's answer.
fn order_class_three(
    world: &mut M8World,
    requester: ObjId,
    payer: AccountId,
) -> Result<(mush_core::CommitToken, serde_json::Value), String> {
    let agent = world.deployment.hardware.agents[1].actor;
    let (_, die) = world.perform(
        "industry",
        "issue_core_die",
        agent,
        json!({"class":DieClass::III}),
    );
    let lot: mush_inventory::LotId =
        serde_json::from_value(die["lot"].clone()).expect("the die's lot");
    let (offer, _) = world.perform(
        "industry",
        "offer_core_integration",
        agent,
        json!({"requester":requester.0,"class":DieClass::III,"supplier_lot":lot.to_string()}),
    );
    world.try_perform(
        "industry",
        "accept_core_integration",
        requester,
        json!({"offer":offer.to_hex(),"payer":payer.0.to_string()}),
    )
}

/// **The acceptance** (doc 44 §7.1, MW.23): with no staff minting, an end user registered by
/// the launch's own route has a Class III core integrated and delivered; one who is not is
/// refused with the refusal they always were; a restart between the registration and the
/// integration keeps it; and it is issued once.
#[test]
fn mw23_a_registered_end_user_has_a_class_three_core_built_and_delivered() {
    let mut world = m8_world();
    let pilot = world.pilot;
    let registry = world.deployment.registry.account;
    // The wallet: the fee and the core's 25,000cr, and a margin.
    fund(&mut world, pilot, 4_000_000);
    let (payer, funded) = credit(&world, pilot);
    let fee = END_USER_FEE_MINOR;

    // 1. Not registered. The verb says so, and the world has no entitlement instrument at
    //    all: nothing in the bring-up made one.
    let me = PrincipalId::Object(pilot);
    assert_eq!(
        end_user_standing(world.rig.tw.world(), pilot),
        EndUserStanding::Unlisted
    );
    assert!(holding(&world, &me, CORE3_ENTITLEMENT).is_none());
    let stands = run(&mut world.rig.tw, pilot, "+enduser");
    assert!(
        stands.contains(END_USER_UNLISTED.split("{fee}").next().expect("a head")),
        "{stands}"
    );

    // 2. Today's refusal: the Class III integration is offered and its acceptance refused,
    //    with nothing charged.
    let refused = order_class_three(&mut world, pilot, payer)
        .expect_err("a Class III core is refused a requester with no entitlement");
    assert!(refused.contains(MANUFACTURE_REFUSAL), "{refused}");
    assert_eq!(credit(&world, pilot).1, funded);

    // 3. The preview writes nothing; the registration pays the Registry and issues one unit.
    let took = balance(&world, registry);
    let preview = run(&mut world.rig.tw, pilot, "+enduser/apply");
    assert!(
        preview.contains(END_USER_PREVIEW.split("{fee}").next().expect("a head")),
        "{preview}"
    );
    assert!(preview.contains("2500.00cr"), "{preview}");
    assert_eq!(credit(&world, pilot).1, funded);
    assert!(holding(&world, &me, CORE3_ENTITLEMENT).is_none());
    let done = run(&mut world.rig.tw, pilot, "+enduser/apply go");
    assert!(
        done.contains(END_USER_DONE.split("{fee}").next().expect("a head")),
        "{done}"
    );
    let (own, units, status) = holding(&world, &me, CORE3_ENTITLEMENT).expect("the registration");
    assert_eq!((units, status), (1, AccountStatus::Active));
    assert_eq!(credit(&world, pilot).1, funded - fee);
    assert_eq!(balance(&world, registry), took + fee);
    assert_eq!(
        end_user_standing(world.rig.tw.world(), pilot),
        EndUserStanding::Registered { account: own }
    );
    // The issuer is the launch's own: the principal that issues its charter and core titles.
    let issuer = PrincipalId::System(world.deployment.registry_issuer.clone());
    let (_, issued, _) = holding(&world, &issuer, CORE3_ENTITLEMENT).expect("the issuer's source");
    assert_eq!(issued, -1, "one unit issued");

    // 4. Once. A second registration, and its preview, are refused with the standing; no fee
    //    is taken and no unit issued.
    for line in ["+enduser/apply go", "+enduser/apply"] {
        let again = run(&mut world.rig.tw, pilot, line);
        assert!(
            again.contains(END_USER_LISTED.split("{account}").next().expect("a head")),
            "{line}: {again}"
        );
    }
    assert_eq!(
        holding(&world, &me, CORE3_ENTITLEMENT),
        Some((own, 1, status))
    );
    assert_eq!(
        holding(&world, &issuer, CORE3_ENTITLEMENT).map(|h| h.1),
        Some(-1)
    );
    assert_eq!(credit(&world, pilot).1, funded - fee);
    assert_eq!(balance(&world, registry), took + fee);

    // 5. A restart between the issue and the integration keeps the registration.
    let mut world = world.reboot();
    assert_eq!(
        end_user_standing(world.rig.tw.world(), pilot),
        EndUserStanding::Registered { account: own }
    );
    assert_eq!(
        holding(&world, &me, CORE3_ENTITLEMENT),
        Some((own, 1, status))
    );
    let stands = run(&mut world.rig.tw, pilot, "+enduser");
    assert!(
        stands.contains(END_USER_LISTED.split("{account}").next().expect("a head")),
        "{stands}"
    );

    // 6. The integration: the same steps that were refused are now accepted, worked the
    //    shipped 96 hours, completed and delivered into the pilot's hands.
    let paid = credit(&world, pilot).1;
    let (accept, core) = world.integrate(pilot, payer, DieClass::III);
    assert_eq!(
        credit(&world, pilot).1,
        paid - CoreClass::III.finished_minor(),
        "the core's price"
    );
    assert_eq!(
        world.rig.tw.world().get(core).and_then(|o| o.location),
        Some(pilot),
        "delivered"
    );
    let history = require_core_delivery(world.rig.tw.world(), accept)
        .expect("the delivery's facts hold for an entitled requester");
    assert_eq!(history.subject().requester, pilot);
    assert_eq!(history.subject().class, CoreClass::III);
    let birth = economy(&world.rig.tw)
        .core_title_birth(core)
        .expect("the title's history")
        .expect("the core's birth");
    assert_eq!(
        (birth.class, birth.source),
        (CoreClass::III, DieSource::Certified)
    );

    // 7. The registration is the holder's alone: another character, with a wallet as full, is
    //    refused as the pilot was.
    let other = world.rig.tw.spawn_player("Unlisted buyer");
    crate::excursions::with_account(&mut world.rig, other, "UnlistedBuyer", 9_023);
    god(
        &mut world.rig,
        &format!("@account/open #{}={}", other.0, tacsim::economy::CREDIT),
    );
    fund(&mut world, other, 4_000_000);
    let (wallet, full) = credit(&world, other);
    let refused = order_class_three(&mut world, other, wallet)
        .expect_err("a Class III core is refused a character who is not registered");
    assert!(refused.contains(MANUFACTURE_REFUSAL), "{refused}");
    assert_eq!(credit(&world, other).1, full);
    crate::excursions::no_refused_writes(&world.rig.tw);
}

/// **Doc 44 Q-44-12 over the launch world** (doc 17h §7 M8g, "still owed: the heirloom over
/// the launch world"): a heirloom in the Tether back room is under the entitlement like any
/// other Class III. The pilot's own heirloom die is refused integration until they register;
/// then the back room integrates it as an ungoverned Class III and delivers it.
#[test]
fn mw23_a_heirloom_in_the_back_room_is_under_the_entitlement() {
    let mut world = m8_world();
    let pilot = world.pilot;
    fund(&mut world, pilot, 2_000_000);
    let (payer, _) = credit(&world, pilot);
    // The pilot's own heirloom die, in a satchel they carry (staff seed heirlooms: 17h §4).
    let satchel = world
        .rig
        .tw
        .world_mut()
        .create(
            mush_core::ObjKind::Thing,
            "Pilot's satchel",
            pilot,
            Some(pilot),
        )
        .expect("a satchel");
    god(
        &mut world.rig,
        &format!(
            "@vault/create PilotStash=container:#{} owner=#{}",
            satchel.0, pilot.0
        ),
    );
    let stash = economy(&world.rig.tw)
        .snapshot()
        .vaults()
        .find(|v| v.name.as_str() == "PilotStash")
        .expect("the pilot's stash")
        .id;
    let (_, die) = world.perform(
        "industry",
        "issue_heirloom_die",
        GOD,
        json!({"vault":stash.0}),
    );
    let lot: mush_inventory::LotId =
        serde_json::from_value(die["lot"].clone()).expect("the heirloom's lot");
    let agent = world.deployment.hardware.agents[1].actor;
    let (offer, _) = world.perform(
        "industry",
        "offer_core_integration",
        agent,
        json!({"requester":pilot.0,"class":DieClass::III,"site":"core_ungovern"}),
    );
    let accept = json!({"offer":offer.to_hex(),"payer":payer.0.to_string(),
        "input_lot":lot.to_string()});

    // No exemption: the back room's acceptance is refused an unregistered requester.
    let before = credit(&world, pilot).1;
    let refused = world
        .try_perform("industry", "accept_core_integration", pilot, accept.clone())
        .expect_err("a heirloom is refused a requester with no entitlement");
    assert!(refused.contains(MANUFACTURE_REFUSAL), "{refused}");
    assert_eq!(credit(&world, pilot).1, before);

    // Registered, the same offer is accepted and the heirloom becomes an ungoverned Class III.
    let done = run(&mut world.rig.tw, pilot, "+enduser/apply go");
    assert!(
        done.contains(END_USER_DONE.split("{fee}").next().expect("a head")),
        "{done}"
    );
    let (accepted, order) = world.perform("industry", "accept_core_integration", pilot, accept);
    let id: mush_industry::workorder::WorkOrderId =
        serde_json::from_value(order["order"].clone()).expect("the work order");
    let params = json!({"order":id.to_string()});
    world.perform("industry", "reserve_core_output", pilot, params.clone());
    world.progress("advance_core_integration", 96);
    let (_, completed) = world.perform(
        "industry",
        "complete_core_integration",
        pilot,
        params.clone(),
    );
    let core: ObjId = serde_json::from_value(completed["object"].clone()).expect("the core");
    world.perform("industry", "deliver_core_output", pilot, params);
    assert_eq!(
        world.rig.tw.world().get(core).and_then(|o| o.location),
        Some(pilot),
        "delivered"
    );
    let birth = economy(&world.rig.tw)
        .core_title_birth(core)
        .expect("the title's history")
        .expect("the core's birth");
    assert_eq!(
        (birth.class, birth.source),
        (CoreClass::III, DieSource::Heirloom)
    );
    assert!(require_core_delivery(world.rig.tw.world(), accepted).is_ok());
    crate::excursions::no_refused_writes(&world.rig.tw);
}

/// **No launch, no route.** On a world that composes no launch the Registry takes no
/// registrations: the verb is registered and says so, and nothing is written. (An `Initialize`
/// launch answers the same, by the same test of the deployment's mode; a world cannot be
/// rebooted back to `Initialize` once its commerce is initialised, so the port world stands in.)
#[test]
fn mw23_the_route_exists_only_on_an_active_launch() {
    use tacsim::launch::end_user::END_USER_NOT_OPEN;
    let mut rig = crate::excursions::stocked();
    let who = rig.tw.spawn_player("Drifter");
    for line in ["+enduser", "+enduser/apply", "+enduser/apply go"] {
        let said = run(&mut rig.tw, who, line);
        assert!(said.contains(END_USER_NOT_OPEN), "{line}: {said}");
    }
    assert!(
        economy(&rig.tw)
            .snapshot()
            .accounts_of(&PrincipalId::Object(who))
            .all(|a| a.instrument.as_str() != CORE3_ENTITLEMENT),
        "nothing was issued"
    );
}
