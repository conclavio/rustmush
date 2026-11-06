//! **Doc 39 cut IN.8 — the mass budget and scrap**, over the composed world (`driving`'s keyed
//! rig), through the production command paths.
//!
//! **The workshop half** (the port half, once — doc 39 §4.9): a captain's Dray-P sails from Ceres
//! to Stickney Yards with a spare escort of Occator's pool in its bay, titled to the captain, and
//! a titled heavy. At the Service Counter (P11, Stickney Yards' workshop):
//!
//! 1. The escort's partition is the document's (§4.8): 2,565 kg of structure and 855 of fittings
//!    over 2,990 structural points.
//! 2. `+bay/scrap` breaks the whole intact escort up: 1 t of `structural_alloys` goes into the
//!    Yards' pool (1,539 kg of yield; its 539 kg stay in the Yards' shop bale), its four parts go
//!    into the Yards' stores as lots with their scrapped provenance, the Dray's hold is untouched,
//!    and the captain is paid about 45,000 of the escort's 75,500 (§4.6).
//! 3. A ten-point armour patch on the heavy draws its own design's alloys and fittings from
//!    the bale it opens (§4.8), and `+bay/return` sells the bale's unused kilograms back to the
//!    Yards at the references, into the shop bale, which mints a tonne into the pool when it
//!    passes 1,000 kg (§4.10).
//!
//! **The site half** at both sizes (Q-38-29): remains scrapped where they lie mint the capped
//! yield into the carrier's hold — an escort's on Ahuna Flank (256) yields 1 t, a Dray's on
//! Urvara Reach (512) 40 t, the fifth of its 90,000 buyout at 450 a tonne — and a full hold
//! refuses the next.
//!
//! **Seeds** (doc 43 rule 2.3 #4; doc 36a §8): the hulls reach the captain's title by staff
//! (`@asset/transfer`, `@unit/create` + `@asset/title`) and the bay by staff placement, as IN.1's
//! story does; the heavy's dents are staff damage through the unit's one plan path; the remains
//! are dropped by staff (GC.6's seam); the hold's alloys and fittings are seeded. Every claim —
//! the scrap, the lots, the pool, the credit, the patch's draw, the return, the site's yield and
//! its refusal — is reached by the player's command.

use mush_core::{GOD, ObjId};
use mush_inventory::record::EndpointRef;
use mush_tactics::{CellAddr, UnitMutationOp, UnitMutationPlan};
use tacsim::port::HullRow;
use tacsim::port::bay_work::{BayHullState, KG_PER_T};
use tacsim::surface::site_record::GroundKind;

use crate::driving_rig::{
    Rig, at_counter, board, cid, command, drain, file_and_launch, fire, hold_pool, hold_vault_of,
    land, land_all, land_quietly, load, on_pad_as_at, port_map, prospector, run, seed_hold,
    staff_hand, station_room, stocked, store, to_cell_on, to_ic, typed,
};
use crate::field_host_rig::{second, site_record_at};
use crate::grounds_rig::{AHUNA_FLANK, Ground, URVARA_REACH};
use crate::site_record_rig::{staff, staff_member};

const STICKNEY: &str = "stickney_port";

fn unit(rig: &Rig, hull: ObjId) -> mush_tactics::TacticalUnit {
    mush_tactics::module::unit_of(rig.tw.world(), hull)
        .cloned()
        .expect("a unit")
}

/// Staff damage through the unit's one plan path.
fn damage(rig: &mut Rig, hull: ObjId, ops: Vec<UnitMutationOp>) {
    let u = unit(rig, hull);
    let plan = UnitMutationPlan::new(ops, Some(u.revision()), None).expect("a plan");
    staff_hand(rig, |ctx| {
        mush_tactics::module::apply_plan(ctx, hull, &plan).expect("the damage lands");
    });
}

/// `points` off the armoured facets, the first that have them.
fn dent(rig: &mut Rig, hull: ObjId, mut points: i64) {
    let u = unit(rig, hull);
    let mut ops = Vec::new();
    for (addr, cell) in u.cells() {
        if points == 0 {
            break;
        }
        if matches!(addr, CellAddr::Facet { .. }) && cell.current > cell.min {
            let off = (cell.current - cell.min).min(points);
            points -= off;
            ops.push(UnitMutationOp::SetCell {
                addr: addr.clone(),
                value: cell.current - off,
            });
        }
    }
    assert_eq!(points, 0, "the armour holds the dent");
    damage(rig, hull, ops);
}

/// Staff put `who` in `room`: into a hull's interior space, or into a plain port room.
fn put(rig: &mut Rig, who: ObjId, room: ObjId, interior: bool) {
    staff_hand(rig, |ctx| {
        if interior {
            mush_sim::place_interior(ctx, who, room, None).expect("into the space");
        } else {
            let _ = mush_sim::unplace(ctx, who, "staff");
            ctx.w.move_to(who, room).expect("into the room");
        }
    });
}

fn mint(rig: &mut Rig, pilot: ObjId) {
    let treasury = port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("a treasury");
    let account = tacsim::port::credit::own_account(rig.tw.world(), pilot)
        .map(|a| a.id.0)
        .expect("the pilot's account");
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {treasury}={account} 10000000"),
    );
}

/// Mint the Stickney Yards fifty thousand from Ceres' treasury (a seed, doc 43 rule 2.3 #4).
fn fund_the_yards(rig: &mut Rig) {
    let treasury = port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("a treasury");
    let account = port_map(&rig.tw, STICKNEY).orgs[&cid("stickney_yards")]
        .account
        .expect("the Yards' account");
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {treasury}={account} 5000000"),
    );
}

/// The pilot's available credit, minor units.
fn credit(rig: &Rig, pilot: ObjId) -> i64 {
    tacsim::port::credit::own_account(rig.tw.world(), pilot)
        .and_then(mush_economy::Account::available)
        .map(|amount| amount.minor())
        .expect("the pilot's account")
}

fn record(rig: &Rig, hull: ObjId) -> tacsim::port::bay_work::BayHull {
    match store(&rig.tw).bay_hull(hull) {
        Some(BayHullState::Present(row)) => row.clone(),
        other => panic!("the bay record: {other:?}"),
    }
}

/// A command, its submissions landed, then fires until `hull`'s record owes nothing.
fn settled(rig: &mut Rig, who: ObjId, line: &str, hull: ObjId) -> String {
    let out = run(&mut rig.tw, who, line);
    let queued = drain(&mut rig.tw);
    land_all(rig, &queued);
    for _ in 0..6 {
        if matches!(store(&rig.tw).bay_hull(hull), Some(BayHullState::Present(r)) if r.owed.is_none())
        {
            return out;
        }
        let (_, queued) = fire(rig);
        land_quietly(rig, &queued);
    }
    panic!("{line}: the bay's act did not settle\n{out}");
}

fn lot_record(rig: &mut Rig, lot: mush_inventory::LotId) -> Option<mush_inventory::Lot> {
    let handles = rig
        .tw
        .world()
        .module_state::<tacsim::port::bay_work::act::BayActHandles>()
        .cloned()
        .expect("the bay's handles");
    let inventory = handles.inventory.expect("the inventory");
    typed(&mut rig.tw, &inventory.lots, &lot)
}

fn provenance(
    rig: &mut Rig,
    lot: mush_inventory::LotId,
) -> tacsim::port::bay_work::act::PartProvenance {
    let handles = rig
        .tw
        .world()
        .module_state::<tacsim::port::bay_work::act::BayActHandles>()
        .cloned()
        .expect("the bay's handles");
    typed(&mut rig.tw, &handles.parts, &lot).expect("the part's provenance")
}

/// The first unit of `schema` that is no rig's lease and not `except`.
fn spare_of(rig: &Rig, schema: &str, except: &[ObjId]) -> ObjId {
    rig.tw
        .world()
        .module_state::<mush_tactics::UnitStore>()
        .expect("the units")
        .units
        .iter()
        .filter(|(id, u)| u.schema().key().id.as_str() == schema && !except.contains(id))
        .map(|(id, _)| *id)
        .find(|id| store(&rig.tw).tv_lease_on(*id).is_none())
        .unwrap_or_else(|| panic!("a spare {schema}"))
}

/// `hull` titled to `pilot` by staff.
fn title_to(rig: &mut Rig, hull: ObjId, pilot: ObjId) {
    let moved = run(
        &mut rig.tw,
        GOD,
        &format!("@asset/transfer #{}=#{}", hull.0, pilot.0),
    );
    assert_eq!(
        rig.tw
            .world()
            .module_state::<mush_economy::EconomyStore>()
            .and_then(|e| e.snapshot().asset(hull).map(|a| a.title_owner.clone())),
        Some(mush_org::PrincipalId::Object(pilot)),
        "{moved}"
    );
}

/// **The captain's Dray-P at Ceres with a titled spare escort in its bay, and a titled heavy**
/// (IN.1's seed: the heavy has no `logistics`, so staff put it in the bay at Stickney).
fn two_titled_hulls(rig: &mut Rig) -> (ObjId, HullRow, ObjId, ObjId) {
    let (pilot, row) = prospector(rig, "Pilot", 1, "Salt Moth");
    mint(rig, pilot);
    at_counter(rig, pilot);
    let bay = ObjId(row.rooms[&cid("bay")]);
    let spare = spare_of(rig, "tacsim.unit.tv_escort", &[]);
    title_to(rig, spare, pilot);
    put(rig, spare, bay, true);
    let made = run(&mut rig.tw, GOD, "@create Iron Lantern");
    let heavy = made
        .split('#')
        .nth(1)
        .and_then(|rest| {
            rest.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .ok()
        })
        .map(ObjId)
        .unwrap_or_else(|| panic!("a thing: {made}"));
    let unit = run(
        &mut rig.tw,
        GOD,
        &format!("@unit/create #{}=tacsim.unit.heavy_mk1", heavy.0),
    );
    assert!(
        mush_tactics::module::unit_of(rig.tw.world(), heavy).is_some(),
        "{unit}"
    );
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@asset/title #{}=#{}", heavy.0, pilot.0),
    );
    (pilot, row, spare, heavy)
}

/// **IN.8's workshop half**: the whole intact escort scrapped at Stickney, and a heavy's bale
/// drawn by its own figures and returned.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn in8_a_whole_escort_scrapped_at_stickney_pays_for_its_parts_and_a_tonne() {
    let mut rig = stocked();
    let (pilot, row, escort, heavy) = two_titled_hulls(&mut rig);
    let hull = ObjId(row.hull);
    let vault = hold_vault_of(&rig.tw, &row);
    seed_hold(&mut rig, &row, "structural_alloys", 2, 9_700);
    seed_hold(&mut rig, &row, "industrial_parts", 2, 9_701);

    // (1) The partition (§4.8), computed at the catalogue build.
    let budget = store(&rig.tw)
        .definitions()
        .budget_of("tacsim.unit.tv_escort")
        .cloned()
        .expect("the escort's budget");
    assert_eq!(
        (
            budget.dry_mass_kg,
            budget.parts_kg,
            budget.core_kg,
            budget.structure_kg,
            budget.fittings_kg,
            budget.points
        ),
        (8_000, 4_330, 250, 2_565, 855, 2_990),
        "{budget:?}"
    );
    assert_eq!(
        budget.per_point_centi(),
        (86, 29),
        "0.86 and 0.29 kg a point"
    );
    // A whole Dray (after SF.R15, which gave her the heat pair) counts its structural points
    // and no heat points: the scrap reads exactly what the partition spread its mass over.
    let dray = spare_of(&rig, "tacsim.unit.dray", &[ObjId(row.hull)]);
    let dray_budget = store(&rig.tw)
        .definitions()
        .budget_of("tacsim.unit.dray")
        .cloned()
        .expect("the Dray's budget");
    assert_eq!(
        tacsim::port::bay_work::scrap::surviving_points(&unit(&rig, dray)),
        dray_budget.points,
        "{dray_budget:?}"
    );
    assert_eq!(
        dray_budget.remains_yield_kg() / KG_PER_T,
        94,
        "94 t of alloys in a Dray"
    );

    // ---- To Stickney Yards ----
    load(&mut rig, &row, 60, 40, 9_500);
    board(&mut rig, pilot);
    let t = file_and_launch(&mut rig, pilot, hull, "+ship/undock stickney");
    let _ = to_ic(&mut rig, t.plan.depart_ic + t.plan.total_s);
    let _ = to_ic(&mut rig, t.plan.depart_ic + t.plan.total_s + 3_600);
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), hull),
            tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == STICKNEY
        ),
        "docked at Stickney Yards"
    );
    put(&mut rig, heavy, ObjId(row.rooms[&cid("bay")]), true);
    let workshop = station_room(&rig.tw, STICKNEY, "room.p11_service");
    put(&mut rig, pilot, workshop, false);
    for berthing in [escort, heavy] {
        let berthed = run(&mut rig.tw, pilot, &format!("+bay/berth #{}", berthing.0));
        assert!(berthed.contains("berthed in the workshop"), "{berthed}");
    }
    let yards = port_map(&rig.tw, STICKNEY).vendors[&cid("stickney_yards")].vault;
    let yards = mush_economy::VaultId(yards);
    let hold_before = hold_pool(&mut rig.tw, vault, "structural_alloys");
    let paid_before = credit(&rig, pilot);

    // (2) The whole intact escort, scrapped. Doc 41's RS.9a follow-up: the matter census holds
    // while the scrap's mint is owed (its source counted only when the lots land) and after.
    let before_scrap = crate::in11_matter::census(&mut rig);
    let line = format!("+bay/scrap #{}", escort.0);
    let ordered = run(&mut rig.tw, pilot, &line);
    let owed = crate::in11_matter::census(&mut rig);
    assert_eq!(
        (owed["balance"], owed["scrapped"]),
        (before_scrap["balance"], before_scrap["scrapped"]),
        "an owed scrap mints nothing yet: {before_scrap:?} → {owed:?}\n{ordered}"
    );
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    for _ in 0..6 {
        if matches!(store(&rig.tw).bay_hull(escort), Some(BayHullState::Present(r)) if r.owed.is_none())
        {
            break;
        }
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
    }
    let out = ordered;
    let landed = crate::in11_matter::census(&mut rig);
    assert_eq!(
        landed["balance"], before_scrap["balance"],
        "the landed scrap balances: {before_scrap:?} → {landed:?}"
    );
    assert!(
        landed["scrapped"] - before_scrap["scrapped"] >= 1_539,
        "the yield is a source: {landed:?}"
    );
    assert!(
        out.contains("1539 kg of structural alloys (1 t to the workshop's stock) and 4 part(s)"),
        "{out}"
    );
    let tonne = lot_record(
        &mut rig,
        tacsim::port::bay_work::act::lot_id(escort, 1, 0).expect("the lot"),
    )
    .expect("the alloys' lot");
    assert_eq!(
        (
            tonne.commodity().as_str(),
            tonne.quantity(),
            tonne.endpoint()
        ),
        ("structural_alloys", 1, EndpointRef(yards)),
        "1 t in the Yards' stock"
    );
    assert_eq!(
        hold_pool(&mut rig.tw, vault, "structural_alloys"),
        hold_before,
        "nothing in the Dray's hold"
    );
    let shop = store(&rig.tw)
        .shop_bale(&cid(STICKNEY))
        .expect("the Yards' shop bale");
    assert_eq!(
        (shop.kg.alloys_kg, shop.pending),
        (539, None),
        "the fraction stays at the Yards"
    );
    let mut parts = Vec::new();
    for n in 2..6 {
        let lot = tacsim::port::bay_work::act::lot_id(escort, 1, n).expect("the lot");
        let found = lot_record(&mut rig, lot).expect("a part's lot");
        assert!(found.commodity().as_str().starts_with("tacsim.part."));
        assert_eq!(found.endpoint(), EndpointRef(yards), "in the Yards' stores");
        let part = provenance(&mut rig, lot);
        assert!(part.stripped().is_some_and(|s| s.scrapped));
        parts.push(found.commodity().as_str().to_owned());
    }
    parts.sort();
    assert_eq!(
        parts,
        [
            "tacsim.part.plant.vehicle_mk1",
            "tacsim.part.sensor.visual_mk1",
            "tacsim.part.weapon.cannon_105",
            "tacsim.part.weapon.mg_coax",
        ]
    );
    // §4.6: 61,500 of parts used at 0.75 less four removals, and the structure's 1,539 kg at
    // the alloys' reference under the fifth of 14,000: about 45,000 of 75,500.
    let paid = credit(&rig, pilot) - paid_before;
    assert!((4_450_000..4_600_000).contains(&paid), "paid {paid}");
    let shell = tacsim::field_ops::assess_tacsim_unit(&unit(&rig, escort), None).expect("assessed");
    assert_eq!(
        shell.state,
        mush_tactics::field_ops::OperationalState::Irrecoverable
    );

    // (3) The heavy's ten-point patch draws its own figures; its bale goes back to the Yards.
    // Doc 39 cut IN.5: the Yards' 50,000 of capital also pays the works' house purchases of
    // Kasei's alloys, so after the escort's 45,000 it cannot buy the bale; the seed refills it.
    fund_the_yards(&mut rig);
    let heavy_budget = store(&rig.tw)
        .definitions()
        .budget_of("tacsim.unit.heavy_mk1")
        .cloned()
        .expect("the heavy's budget");
    dent(&mut rig, heavy, 10);
    // The owner's IN.11 census ruling (2026-11-05): a window holding a repair still balances.
    let window = crate::in11_matter::census(&mut rig);
    let taken = run(
        &mut rig.tw,
        pilot,
        &format!("+bay/order #{}=armor", heavy.0),
    );
    assert!(taken.contains("taken in the workshop"), "{taken}");
    let order = record(&rig, heavy).order.expect("the order");
    let _ = to_ic(&mut rig, order.due_ic + 60);
    let _ = to_ic(&mut rig, order.due_ic + 600);
    let bale = record(&rig, heavy).bale.expect("the opened bale");
    let repaired = crate::in11_matter::census(&mut rig);
    assert_eq!(
        repaired["repaired"] - window["repaired"],
        i128::from(heavy_budget.structure_kg_for(10) + heavy_budget.fittings_kg_for(10)),
        "the patch's kilograms are summed: {repaired:?}"
    );
    assert_eq!(
        repaired["balance"], window["balance"],
        "the repair window balances: {window:?} → {repaired:?}"
    );
    assert_eq!(
        (bale.alloys_kg, bale.fittings_kg),
        (
            KG_PER_T - heavy_budget.structure_kg_for(10),
            KG_PER_T - heavy_budget.fittings_kg_for(10)
        ),
        "the patch drew the heavy's own alloys and fittings"
    );
    let paid_before = credit(&rig, pilot);
    let out = settled(&mut rig, pilot, &format!("+bay/return #{}", heavy.0), heavy);
    assert!(out.contains("bale goes back to the workshop"), "{out}");
    let returned = crate::in11_matter::census(&mut rig);
    assert_eq!(
        returned["balance"], window["balance"],
        "the return balances too: {returned:?}"
    );
    assert!(
        record(&rig, heavy).bale.is_none(),
        "the bale is the Yards' now"
    );
    assert!(credit(&rig, pilot) > paid_before, "the bale is paid for");
    // The shop bale took the returned kilograms on top of the scrap's 539, minting a tonne each
    // time it passed 1,000 kg: nothing under a tonne is lost at a facility.
    let shop = store(&rig.tw)
        .shop_bale(&cid(STICKNEY))
        .expect("the Yards' shop bale");
    let act = record(&rig, heavy).acts;
    let minted = lot_record(
        &mut rig,
        tacsim::port::bay_work::act::lot_id(heavy, act, 0).expect("the lot"),
    )
    .filter(|lot| lot.commodity().as_str() == "structural_alloys")
    .map_or(0, |lot| {
        assert_eq!(lot.endpoint(), EndpointRef(yards), "in the Yards' stock");
        lot.quantity()
    });
    assert_eq!(
        minted * KG_PER_T + shop.kg.alloys_kg,
        539 + bale.alloys_kg,
        "the alloys are conserved"
    );
    assert_eq!(minted, 1, "past a tonne, it mints");
    let again = run(&mut rig.tw, pilot, &format!("+bay/return #{}", heavy.0));
    assert!(again.contains("holds nothing to return"), "{again}");
}

/// `line` as `who`, every submission it queued landed; the reply.
fn go(rig: &mut Rig, who: ObjId, line: &str) -> String {
    let (out, queued) = command(rig, who, line);
    for submit in &queued {
        land(rig, submit).unwrap_or_else(|e| panic!("{line}: {e}"));
    }
    out
}

/// **IN.8's site half on `ground`**: two remains of a titled `schema` hull dropped beside the
/// captain's rig; the first scraps into the hold at `yield_t`, and a full hold refuses the second.
fn remains_on(ground: &Ground, schema: &str, yield_t: u64) {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad_as_at(
        &mut rig,
        ("Pilot", 1, "Salt Moth"),
        (ground.venue, ground.entry),
        1,
    );
    let tug = tvs[0];
    let carrier = ObjId(row.hull);
    let hull = spare_of(&rig, schema, &[carrier, tug]);
    title_to(&mut rig, hull, pilot);
    let (x, y) = ground.read;
    to_cell_on(&mut rig, ground.entry, tug, (x, y));
    let s = staff_member(&mut rig);
    for _ in 0..2 {
        staff(
            &mut rig,
            s,
            &format!(
                "@tacsim/site {} drop remains {},{} 8000 #{}",
                ground.entry,
                x + 1,
                y,
                hull.0
            ),
        );
    }
    let _ = second(&mut rig);
    let remains: Vec<u32> = site_record_at(&mut rig, ground.entry)
        .expect("the site's record")
        .objects
        .iter()
        .filter(|obj| matches!(obj.kind, GroundKind::Remains { of: Some(of), .. } if of == hull))
        .map(|obj| obj.id)
        .collect();
    let [first, next] = remains.as_slice() else {
        panic!("two remains: {remains:?}");
    };
    let vault = hold_vault_of(&rig.tw, &row);
    let preview = run(&mut rig.tw, pilot, &format!("+tv/1 scrap {first}"));
    assert!(
        preview.contains(&format!("{yield_t} t of structural_alloys")),
        "{preview}"
    );
    // The owner's IN.11 census ruling (2026-11-05): a window holding a remains scrap balances.
    let window = crate::in11_matter::census(&mut rig);
    let out = go(&mut rig, pilot, &format!("+tv/1 scrap {first} go"));
    let scrapped = crate::in11_matter::census(&mut rig);
    assert_eq!(
        scrapped["scrapped"] - window["scrapped"],
        i128::from(yield_t) * 1_000,
        "the minted alloys are summed: {scrapped:?}"
    );
    assert_eq!(
        scrapped["balance"], window["balance"],
        "the scrap window balances: {window:?} → {scrapped:?}"
    );
    assert!(out.contains("breaks up"), "{out}");
    let lot = tacsim::surface::claim::remains_lot(&cid(ground.entry), *first).expect("the lot");
    let found = lot_record(&mut rig, lot).expect("the alloys");
    assert_eq!(found.commodity().as_str(), "structural_alloys");
    assert_eq!(found.quantity(), yield_t);
    assert_eq!(found.endpoint(), EndpointRef(vault));
    // The full hold refuses the next, at the command, and the remains stay on the ground.
    seed_hold(&mut rig, &row, "structural_alloys", 86, 9_800);
    let refused = run(&mut rig.tw, pilot, &format!("+tv/1 scrap {next} go"));
    assert!(refused.contains("hold will not take"), "{refused}");
    assert!(
        site_record_at(&mut rig, ground.entry)
            .expect("the site's record")
            .objects
            .iter()
            .any(|obj| obj.id == *next),
        "the remains still lie there"
    );
}

/// **An escort's remains on Ahuna Flank (256)**: its 1,539 kg of yield is a tonne.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn in8_an_escorts_remains_on_ahuna_yield_a_tonne_and_a_full_hold_refuses() {
    remains_on(&AHUNA_FLANK, "tacsim.unit.tv_escort", 1);
}

/// **A Dray's remains on Urvara Reach (512)**: its 94 t of yield capped at the fifth of its
/// 90,000 buyout, 40 t at 450 a tonne.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn in8_a_drays_remains_on_urvara_yield_forty_tonnes_and_a_full_hold_refuses() {
    remains_on(&URVARA_REACH, "tacsim.unit.dray", 40);
}
