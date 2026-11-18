//! **Doc 48 unit PL.6 — the Ceres yard** (Q-48-25), over the composed world (`driving`'s keyed
//! rig), through the production command paths.
//!
//! Lane v40 opens a yard in Ceres Freeport's Visiting Berth Gallery, run by Freeport Works and
//! holding `tacsim.yard` alone. Stickney's yard holds `tacsim.yard.heavy` beside it, which every
//! hull recipe but the six light ones requires (`tacsim.industry.hulls` v3), so the industry's
//! own `requires` test decides what each yard builds.
//!
//! 1. **The menus.** At Ceres, `+yard` lists exactly the six light hulls, though the world also
//!    holds the hull pack's v2, which let any yard build any hull; at Stickney it lists all
//!    seventeen.
//! 2. **The refusals.** `+yard/quote` and `+yard/order` of a heavy hull at Ceres name Stickney
//!    Yards as the place to order it ([`BUILT_ELSEWHERE`]), and no order is booked. The yard's
//!    facility record itself does not offer a heavy hull's capabilities.
//! 3. **The builds.** A captain at Ceres orders all six light hulls. Each bill is laid in its
//!    escrow (a fixture: "bought and delivered earlier in the story"), the two slots run them in
//!    turn, and `+yard/collect` commissions each: the Dray and the Dray-P stand docked at Ceres,
//!    and the four rigs on Ceres's workshop floor, every one titled to the captain under a fresh
//!    `SYB-` mark.
//!
//! **Seeds** (doc 43 rule 2.3 #4): the hull pack's published v2 beside v3 (a kept world); the six
//! bills in their escrows; the money. Every claim is reached by the players' commands and the
//! port tick.

use std::collections::BTreeSet;

use mush_core::{CanonicalId, GOD, ObjId};
use mush_economy::VaultId;
use mush_inventory::LotId;
use mush_org::PrincipalId;
use mush_tactics::field_ops::{ComponentCondition, ProvenanceLegalityStatus};
use tacsim::port::commission::{Step, YardCommission};
use tacsim::port::works::{
    BUILT_ELSEWHERE, Close, WorksContract, WorksState, YARD_CAPABILITY, YARD_HEAVY_CAPABILITY,
};

use crate::driving_rig::{
    Rig, cid, ic_now, pilot, port_map, run, seed_part, seed_pool, station_room, stocked_tuned,
    store, teleport,
};

const CERES: &str = "ceres_port";
const STICKNEY: &str = "stickney_port";
const DAY: i64 = 86_400;
/// The light hulls (Q-48-25): what both yards build.
const LIGHT: [&str; 6] = [
    "dray",
    "dray_p",
    "tv_escort",
    "tv_prospector",
    "tv_tender",
    "tv_wrecker",
];
/// The heavy hulls: Stickney's alone.
const HEAVY: [&str; 11] = [
    "biped_mk1",
    "biped_mk2",
    "heavy_mk1",
    "line_tank",
    "line_tank_eccm",
    "line_tank_ecm",
    "line_tank_laser",
    "line_tank_thermal",
    "scout_mk1",
    "tycho_crawler",
    "tycho_lifter",
];

fn mint(rig: &mut Rig, pilot: ObjId) {
    let treasury = port_map(&rig.tw, CERES).treasury.expect("a treasury");
    let account = tacsim::port::credit::own_account(rig.tw.world(), pilot)
        .map(|a| a.id)
        .expect("the pilot's account");
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {treasury}={} 10000000", account.0),
    );
}

fn contracts(rig: &Rig) -> Vec<WorksContract> {
    store(&rig.tw).works_contracts()
}

fn contract(rig: &Rig, id: u64) -> WorksContract {
    match store(&rig.tw).works_contract(id) {
        Some(WorksState::Present(c)) => (**c).clone(),
        other => panic!("contract {id}: {other:?}"),
    }
}

fn commission(rig: &Rig, order: u64) -> YardCommission {
    tacsim::port::commission::commission(rig.tw.world(), order)
        .cloned()
        .unwrap_or_else(|| panic!("commission {order}"))
}

/// Every submission staged, the industry's included, landed in order.
fn pump(rig: &mut Rig) -> Vec<String> {
    let mut queued = Vec::new();
    while let Some(submit) = rig.tw.engine_mut().take_authoritative_submit() {
        queued.push(*submit);
    }
    queued
        .iter()
        .filter_map(|submit| {
            crate::driving_rig::land(rig, submit)
                .err()
                .map(|e| format!("{}: {e}", submit.action.key().id))
        })
        .collect()
}

/// Every due production occurrence, landed as the server's schedule driver lands it.
fn produce(rig: &mut Rig) {
    for _ in 0..4_096 {
        let page = rig
            .tw
            .engine()
            .discover_scheduled_actions(mush_core::ScheduleDriveCause::Live, None, 64)
            .expect("the due schedules");
        let Some(candidate) = page
            .candidates()
            .iter()
            .find(|c| c.action().key().owner.as_str() == "industry")
            .cloned()
        else {
            return;
        };
        let ingress =
            mush_core::scheduled_action_ingress(candidate.action(), candidate.advance(), 1)
                .expect("the schedule's ingress");
        let plan = rig
            .tw
            .engine()
            .plan_scheduled_action(&candidate, &ingress)
            .expect("the schedule's plan");
        let (_, invocation, _) = plan.into_parts();
        let token = crate::driving_rig::next_token();
        let (prepared, _) = rig
            .tw
            .engine_mut()
            .prepare_registered_scheduled_authoritative_action(
                &candidate,
                &invocation,
                mush_core::CommitToken::from_bytes(token),
            )
            .expect("the occurrence prepares");
        let (ops, install) = prepared.into_parts();
        rig.tw
            .engine_mut()
            .install_prepared_action(install)
            .expect("the occurrence installs");
        crate::driving_rig::keep(rig, ops);
    }
}

/// One port fire (6 s on), everything it staged landed.
fn fire_all(rig: &mut Rig) -> Vec<String> {
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    produce(rig);
    pump(rig)
}

fn settle(rig: &mut Rig, fires: usize) {
    for _ in 0..fires {
        let refused = fire_all(rig);
        assert!(refused.is_empty(), "refused on a fire: {refused:?}");
    }
}

/// A command, its submissions landed, then `fires` port fires.
fn typed_in(rig: &mut Rig, who: ObjId, line: &str, fires: usize) -> String {
    let out = run(&mut rig.tw, who, line);
    let refused = pump(rig);
    assert!(refused.is_empty(), "{line}: {refused:?}\n{out}");
    settle(rig, fires);
    out
}

fn ic_to(rig: &mut Rig, target: i64) {
    let rate = i64::from(crate::driving_rig::world_rate(&rig.tw).max(1));
    let ahead = target.saturating_sub(ic_now(&rig.tw)).max(0);
    let real = ((ahead + rate - 1) / rate).max(1);
    let t = rig.tw.world().now().saturating_add(real);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    let refused = pump(rig);
    assert!(refused.is_empty(), "refused on a jump: {refused:?}");
    produce(rig);
}

/// The IC clock `days` on, in jumps the production catch-up can take, firing between.
fn days_on(rig: &mut Rig, days: i64) {
    let target = ic_now(&rig.tw) + days * DAY + 3_600;
    while ic_now(&rig.tw) < target {
        let next = (ic_now(&rig.tw) + 4 * DAY).min(target);
        ic_to(rig, next);
        settle(rig, 2);
    }
}

fn booked(rig: &mut Rig, who: ObjId, line: &str) -> u64 {
    let before: Vec<u64> = contracts(rig).into_iter().map(|c| c.id).collect();
    let out = typed_in(rig, who, line, 1);
    assert!(out.contains("booked at the yard"), "{line}: {out}");
    let id = contracts(rig)
        .into_iter()
        .map(|c| c.id)
        .find(|id| !before.contains(id))
        .expect("a new order");
    assert!(
        contract(rig, id).order.is_some(),
        "the draft landed and bound"
    );
    id
}

fn titled(rig: &Rig, obj: ObjId) -> PrincipalId {
    rig.tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|s| s.snapshot().asset(obj).map(|a| a.title_owner.clone()))
        .expect("an asset")
}

/// The recipes `+yard` lists here: each `  <id>: <bill> -> <kit>, <n> IC days.` line.
fn menu(rig: &mut Rig, who: ObjId) -> BTreeSet<String> {
    let out = run(&mut rig.tw, who, "+yard");
    assert!(
        out.starts_with("The yard here: 2 slots"),
        "the yard's head line: {out}"
    );
    out.lines()
        .filter(|line| line.contains(" -> "))
        .filter_map(|line| line.strip_prefix("  ")?.split_once(':'))
        .map(|(id, _)| id.to_owned())
        .collect()
}

fn words(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

fn capabilities(ids: &[&str]) -> BTreeSet<CanonicalId> {
    ids.iter().map(|id| cid(id)).collect()
}

/// **The published hull pack v2** (doc 39 cut IN.9b), rebuilt from v3: every recipe requiring
/// `tacsim.yard` alone, re-signed at 2 to its published digest. A kept world holds it beside v3.
fn hulls_v2() -> String {
    let mut doc: serde_json::Value =
        serde_json::from_str(tacsim::definitions::content::INDUSTRY_HULLS_JSON)
            .expect("the hull pack is JSON");
    doc["manifest"]["version"] = 2.into();
    doc["manifest"]["digest"] = tacsim::ground::UNSIGNED.into();
    for entry in doc["entries"]
        .as_object_mut()
        .expect("entries")
        .values_mut()
    {
        entry["def"]["requires"] = serde_json::json!([YARD_CAPABILITY]);
    }
    let signed = tacsim::ground::sign(&serde_json::to_string_pretty(&doc).expect("serializes"));
    let pack = mush_content::load_pack(&signed).expect("v2 verifies");
    assert_eq!(
        pack.manifest().digest().to_hex(),
        "aaeeae95687c477addbcff89314bc919573f781e85994cdc842703c5aa0098df",
        "the published v2"
    );
    signed
}

/// The yard facility on `entry`'s authored yard room.
fn yard_of(rig: &Rig, room: ObjId) -> mush_industry::FacilityRecord {
    rig.tw
        .world()
        .module_state::<mush_industry::IndustryStore>()
        .and_then(|store| store.facility(room).cloned())
        .unwrap_or_else(|| panic!("a facility on #{}", room.0))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn pl6_ceres_builds_the_light_hulls_and_stickney_the_heavy() {
    // A kept world: the hull pack's published v2 (every hull at any yard) beside v3.
    let mut rig = stocked_tuned(|cfg| {
        cfg.seeds.push(tacsim::config::PackSource {
            name: "ops/hulls-v2.json".to_owned(),
            bytes: hulls_v2(),
        });
    });

    // ---- The captain: enrolled and released at Ceres, an account and no hull ----
    let captain = pilot(&mut rig, "Shipwright", 1);
    let out = run(&mut rig.tw, captain, "+coop/enrol Long Ox");
    assert!(out.contains("now the Long Ox"), "{out}");
    let out = run(&mut rig.tw, captain, "+coop/release");
    assert!(!out.contains("Usage"), "{out}");
    mint(&mut rig, captain);
    let gallery = station_room(&rig.tw, CERES, "room.c01_berth_gallery");
    teleport(&mut rig.tw, captain, gallery);

    // ---- (1) The menus, and the facilities under them ----
    // The latest version decides: the heavy recipes' v2 never stands in at Ceres.
    assert_eq!(
        menu(&mut rig, captain),
        words(&LIGHT),
        "Ceres builds the light hulls"
    );
    let ceres_yard = yard_of(&rig, gallery);
    assert_eq!(
        ceres_yard.capabilities(),
        &capabilities(&[YARD_CAPABILITY]),
        "Ceres's yard is a light yard"
    );
    let works = port_map(&rig.tw, CERES).orgs[&cid("freeport_works")].org;
    assert_eq!(
        ceres_yard.owner(),
        &PrincipalId::Organization(mush_org::OrgId(works)),
        "Freeport Works runs it"
    );
    assert_eq!(
        titled(&rig, gallery),
        PrincipalId::Organization(mush_org::OrgId(works)),
        "titled to its operator (doc 48 unit PL.9)"
    );
    let heavy = capabilities(&[YARD_CAPABILITY, YARD_HEAVY_CAPABILITY]);
    assert!(
        !ceres_yard.offers(&heavy) && ceres_yard.offers(&capabilities(&[YARD_CAPABILITY])),
        "the industry's own test keeps a heavy hull off Ceres's slots"
    );
    let stickney_yard = yard_of(
        &rig,
        station_room(&rig.tw, STICKNEY, "room.p12_observation"),
    );
    assert_eq!(
        stickney_yard.capabilities(),
        &heavy,
        "Stickney's yard is heavy"
    );

    // ---- (2) A heavy hull at Ceres: refused, naming the yard that builds it ----
    let elsewhere = |recipe: &str| {
        BUILT_ELSEWHERE
            .replace("{recipe}", recipe)
            .replace("{port}", "Stickney Yards")
    };
    let quote = run(&mut rig.tw, captain, "+yard/quote line_tank");
    assert!(quote.contains(&elsewhere("line_tank")), "{quote}");
    let orders = contracts(&rig).len();
    let order = run(&mut rig.tw, captain, "+yard/order tycho_lifter");
    assert!(order.contains(&elsewhere("tycho_lifter")), "{order}");
    assert!(pump(&mut rig).is_empty());
    assert_eq!(contracts(&rig).len(), orders, "no order is booked");
    let unknown = run(&mut rig.tw, captain, "+yard/quote cathedral");
    assert!(
        unknown.contains("The yard here does not make that."),
        "a recipe no yard builds: {unknown}"
    );

    // ---- (3) All six light hulls ordered at Ceres, their bills in their escrows ----
    let mut lot = 9_800;
    let mut built = Vec::new();
    for design in LIGHT {
        let n = booked(&mut rig, captain, &format!("+yard/order {design}"));
        let escrow = VaultId(n);
        for line in contract(&rig, n).bill {
            if line.good.as_str().starts_with("tacsim.part.") {
                for _ in 0..line.units {
                    lot += 1;
                    seed_part(
                        &mut rig,
                        escrow,
                        line.good.as_str(),
                        LotId(lot),
                        ComponentCondition::Intact,
                        ProvenanceLegalityStatus::CleanTitle,
                    );
                }
            } else {
                lot += 1;
                seed_pool(&mut rig, escrow, line.good.as_str(), line.units, LotId(lot));
            }
        }
        built.push((design, n));
    }
    settle(&mut rig, 2);
    // Two slots run the six in turn: the Drays' twelve days, then the rigs' two days a pair, each
    // next order taking its slot on a fire after one frees (`days_on` jumps four days between its
    // fires). The two Drays finish together, so the works arm makes one of the captain's
    // submissions a fire (doc 48 unit PL.6: a second in one drain voided both, and neither Dray
    // delivered).
    days_on(&mut rig, 30);
    let workshop = station_room(&rig.tw, CERES, "room.c14_dockmaster");
    let mut marks = BTreeSet::new();
    let board = run(&mut rig.tw, captain, "+yard");
    for (design, n) in built {
        let out = typed_in(&mut rig, captain, &format!("+yard/collect {n}"), 3);
        assert!(
            out.contains("commissions her as SYB-"),
            "{design}: {out}\n{board}"
        );
        let done = commission(&rig, n);
        assert_eq!(done.step, Step::Done, "{design}: {done:?}");
        let hull = done.object().expect("the hull");
        assert_eq!(titled(&rig, hull), PrincipalId::Object(captain), "{design}");
        if design.starts_with("dray") {
            assert!(
                matches!(tacsim::port::ship::berth_of(rig.tw.world(), hull),
                    tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == CERES),
                "{design} is docked at Ceres"
            );
        } else {
            assert_eq!(
                rig.tw.world().get(hull).and_then(|o| o.location),
                Some(workshop),
                "{design} stands on Ceres's workshop floor"
            );
        }
        let owned = tacsim::port::owned::owned(rig.tw.world(), hull).expect("a register row");
        assert!(
            marks.insert(owned.mark.clone()),
            "a fresh mark: {}",
            owned.mark
        );
        assert_eq!(contract(&rig, n).closed, Some(Close::Collected), "{design}");
    }
    assert_eq!(marks.len(), LIGHT.len());

    // ---- (4) Stickney still builds every hull ----
    let stickney = station_room(&rig.tw, STICKNEY, "room.p11_service");
    teleport(&mut rig.tw, captain, stickney);
    let all: BTreeSet<String> = words(&LIGHT).into_iter().chain(words(&HEAVY)).collect();
    assert_eq!(menu(&mut rig, captain), all, "Stickney builds them all");
    let quote = run(&mut rig.tw, captain, "+yard/quote line_tank");
    assert!(quote.contains("line_tank x1: bill "), "{quote}");
}
