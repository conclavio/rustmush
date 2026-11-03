//! **Doc 41 cut RS.3 — the claim's close (story b, extended)** (Q-41-20 to Q-41-23, Q-41-28,
//! Q-38-19, Q-S13, Q-S14; round-2 items 41-1, 41-8, 41-10), over the composed world
//! (`driving`'s keyed rig) on Urvara Reach (512) and on Ahuna Flank (256), with five parties:
//!
//! - **the organization's officer** (T): Brask Freight's grantee, whose carrier stands on the
//!   pad; Brask's hull `a` and T's own hull `c` lie beside it;
//! - **the opportunist** (O), whose rig tows `a` and `c` into O's carrier and files a claim on
//!   each, and **a second salvager** (P), whose rig does the same with `b`;
//! - **the landholder** (L), who holds the Registry's claims on the blocks `a` and `c` lie in and
//!   delegates the recovery of `c`'s block to O;
//! - **Occator's house agent**, who answers for Occator's spare rig `b` lying out of sight.
//!
//! The three contest kinds each vacate or fail on their named source: on `a` (an org title, off
//! L's ground without authority) T's `procedure` finds no defect in the registry's records and
//! T's `right` finds no right, and T's `recovery` on T's own sensor record vacates; on `c` (T's
//! title, recovered under T's `+salvage/authorize` and L's block delegation, both read at the
//! attach) the recovery is lawful, L's `right` — an authorization of L's own — is no ground,
//! and T's `right` vacates (the authorization bound the claimant); on `b` Occator's agent is
//! told, contests on bare title, and the claim is awarded: an organization's hull with no
//! lease is awardable (Q-41-22). The at-risk line in the bay quote and the notice is the bay
//! story's (`shard_1/bay_36a.rs`, `b36a_rs3_…`): the bay works only at a workshop port.
//!
//! **What is a fixture.** As GC.6's story: the rigs' power is destroyed and they are put on the
//! site's index by staff hand, their titles moved by `@asset/transfer`, the organization made by
//! `@org/create` and `@org/grant`, and L's block claims seeded as typed records.

use mush_core::{GOD, ObjId, ObjKind};
use mush_economy::{ContractId, ContractState, EconomyStore};
use mush_org::PrincipalId;
use tacsim::surface::claim::UnlawfulKind;
use tacsim::surface::salvage::{ClaimSeam, SalvageCustody};
use tacsim::surface::site_record::SiteRecord;

use crate::driving_rig::{
    Rig, cid, command, launched_at, on_pad_as_at, pilot, port_map, ramp_at, run, seed_typed,
    staff_hand, stocked_tuned, to_cell_on,
};
use crate::field_host_rig::{second, site_record_at, tune};
use crate::site_record_rig::{staff, staff_member};

/// The compressed contest window, real seconds.
const WINDOW: i64 = 600;

/// Short tow stages and a ten-minute contest window, on the field host's lab tempo.
fn quick(cfg: &mut tacsim::TacsimConfig) {
    tune(cfg);
    cfg.recovery.inspect_s = 30;
    cfg.recovery.attach_s = 30;
    cfg.recovery.load_s = 60;
    cfg.recovery.claim_window_s = WINDOW;
}

fn record(rig: &mut Rig, entry: &str) -> SiteRecord {
    site_record_at(rig, entry).expect("the site's record")
}

fn custody(rig: &mut Rig, entry: &str, hull: ObjId) -> Option<SalvageCustody> {
    record(rig, entry)
        .custody
        .into_iter()
        .find(|row| row.hull == hull)
}

fn title(rig: &Rig, hull: ObjId) -> Option<PrincipalId> {
    rig.tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().asset(hull).map(|a| a.title_owner.clone()))
}

fn contract_state(rig: &Rig, id: u64) -> Option<ContractState> {
    rig.tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().contract(ContractId(id)).map(|c| c.state))
}

/// `line` as `who`, every submission it queued landed; the reply.
fn go(rig: &mut Rig, who: ObjId, line: &str) -> String {
    let (out, queued) = command(rig, who, line);
    assert!(!queued.is_empty(), "{line}: a submission: {out}");
    for submit in &queued {
        crate::driving_rig::land(rig, submit).unwrap_or_else(|e| panic!("{line}: {e}"));
    }
    out
}

/// `+salvage/authorize` on a hull as `who`: Org's unit-grant submission, which the plain
/// drain leaves out, taken as it queues and landed on the composed prepare (GC.0's hook).
fn issue(rig: &mut Rig, who: ObjId, line: &str) -> String {
    let out = run(&mut rig.tw, who, line);
    let mut grants = Vec::new();
    while let Some(submit) = rig.tw.engine_mut().take_authoritative_submit() {
        grants.push(*submit);
    }
    assert!(
        grants
            .iter()
            .any(|submit| submit.action.key().id.as_str() == "unitgrant_issue"),
        "{line}: the grant is submitted: {out}"
    );
    for grant in &grants {
        crate::driving_rig::land_composed(rig, grant).unwrap_or_else(|e| panic!("{line}: {e}"));
    }
    out
}

/// Seconds until `done` holds, at most `limit`, every tow landing accepted.
fn seconds_until(rig: &mut Rig, limit: u32, mut done: impl FnMut(&mut Rig) -> bool) -> bool {
    for _ in 0..limit {
        if done(rig) {
            return true;
        }
        let errors: Vec<String> = second(rig)
            .into_iter()
            .filter(|error| error.contains(tacsim::surface::tow::TOW_ACTION))
            .collect();
        assert!(errors.is_empty(), "a tow landing refused: {errors:?}");
    }
    done(rig)
}

/// Move real time `secs` on, then a second of ticks.
fn jump(rig: &mut Rig, secs: i64) {
    let t = rig.tw.world().now().saturating_add(secs);
    rig.tw.set_now(t);
    let _ = second(rig);
}

/// **Destroy `unit`'s power component** (GC.4's fixture): a casualty.
fn powerless(rig: &mut Rig, unit: ObjId) {
    staff_hand(rig, |ctx| {
        let plan = mush_tactics::UnitMutationPlan::new(
            vec![mush_tactics::UnitMutationOp::SetComponentCondition {
                component: mush_tactics::unit::ComponentId::new("power").expect("an id"),
                condition: mush_tactics::ConditionId::new("destroyed").expect("a condition"),
            }],
            None,
            Some(smol_str::SmolStr::new_static("test:rs3-powerless")),
        )
        .expect("a plan");
        mush_tactics::module::apply_plan(ctx, unit, &plan).expect("applied");
    });
}

/// The block letter of `cell` on a site `side` cells a side.
fn letter_of(side: u16, cell: (i64, i64)) -> char {
    tacsim::field_ops::block_letter_at_site_pose(
        mush_tactics::field_ops::FieldPose {
            site: ObjId(1),
            x_cm: cell.0 * 1_000 + 500,
            y_cm: cell.1 * 1_000 + 500,
        },
        tacsim::surface::address::Scheme::of_site(side),
    )
    .expect("a block")
}

/// L's claim on `letter` of `entry`, seeded (GC.6's fixture).
fn seed_block(rig: &mut Rig, entry: &str, letter: char, l: ObjId) {
    let place = port_map(&rig.tw, entry)
        .site
        .and_then(|site| site.blocks.get(letter.to_string().as_str()).copied())
        .map(ObjId)
        .expect("the block's place");
    let now = rig.tw.world().world_ic_now().at;
    let mut tenure = tacsim::surface::claims::BlockTenure::new(cid(entry), letter, place);
    tenure.claim = Some(tacsim::surface::claims::BlockClaim {
        holder: PrincipalId::Object(l),
        account: rig
            .tw
            .world()
            .module_state::<mush_sim::AccountOracle>()
            .and_then(|oracle| oracle.account_of(l)),
        epoch: 1,
        opened_ic: None,
        registered_ic: now,
        term_start_ic: now,
        term_end_ic: now + 365 * 86_400,
        worked_at_term_start: 0,
        discovered_ic: None,
        declared: None,
        fee: None,
        refund: None,
        lapsed_ic: None,
    });
    let handles = rig
        .tw
        .world()
        .module_state::<tacsim::port::PortHandles>()
        .cloned()
        .expect("the port's handles");
    seed_typed(
        rig,
        &handles.families.claims,
        &tacsim::surface::claims::BlockKey {
            site: cid(entry),
            letter,
        },
        tenure,
    );
}

/// O's rig tows `casualty` from beside it and loads it into O's carrier at the ramp.
fn recover(rig: &mut Rig, entry: &str, (o, tug): (ObjId, ObjId), casualty: ObjId, at: (i64, i64)) {
    to_cell_on(rig, entry, tug, (at.0 + 1, at.1));
    let _ = second(rig);
    let out = go(rig, o, &format!("+tv/1 tow #{} go", casualty.0));
    assert!(
        seconds_until(rig, 120, |rig| record(rig, entry)
            .tows
            .towing(tug)
            .map(|op| op.casualty())
            == Some(casualty)),
        "the tow attaches: {out}"
    );
    let ramp = {
        let (x, y, _) = ramp_at(&rig.tw, entry);
        (x.div_euclid(1_000), y.div_euclid(1_000))
    };
    to_cell_on(rig, entry, tug, ramp);
    let _ = second(rig);
    let out = go(rig, o, "+tv/1 load go");
    assert!(
        seconds_until(rig, 120, |rig| custody(rig, entry, casualty).is_some()
            && record(rig, entry).tows.act_of(tug).is_none()),
        "the load lands in custody: {out}"
    );
}

/// O files the claim on `hull`; the filed contract.
fn file(rig: &mut Rig, entry: &str, o: ObjId, hull: ObjId) -> u64 {
    let out = go(rig, o, &format!("+salvage/claim #{} go", hull.0));
    assert!(out.contains("You file a salvage claim"), "{out}");
    match custody(rig, entry, hull).map(|row| row.claim) {
        Some(ClaimSeam::Filed { contract }) => contract,
        other => panic!("filed: {other:?}"),
    }
}

/// **The story at one site**: `(venue, entry, side)`.
#[allow(clippy::too_many_lines)]
fn story(venue: &str, entry: &str, side: u16) {
    let mut rig = stocked_tuned(quick);
    let (t, _, _) = on_pad_as_at(&mut rig, ("Pilot", 1, "Salt Moth"), (venue, entry), 1);
    let (o, _, o_tvs) = on_pad_as_at(&mut rig, ("Second", 2, "Red Moth"), (venue, entry), 1);
    let tug = o_tvs[0];
    // P: a second salvager, whose carrier's bay takes `b` (O's holds two).
    let (p, _, p_tvs) = on_pad_as_at(&mut rig, ("Third", 4, "Ash Moth"), (venue, entry), 1);
    let p_tug = p_tvs[0];
    let l = pilot(&mut rig, "Lander", 3);
    // Brask Freight: a player organization whose grantee T is.
    for line in [
        "@org/create Brask".to_owned(),
        format!("@org/grant #{}=Brask economy.account.post", t.0),
    ] {
        let out = run(&mut rig.tw, GOD, &line);
        assert!(!out.contains("Permission denied"), "{line}: {out}");
    }
    // Three spare rigs of Occator's pool (no lease on any): `a` titled to Brask, `c` to T, `b`
    // left Occator's.
    let schema = mush_tactics::module::unit_of(rig.tw.world(), tug)
        .expect("the tug's unit")
        .schema()
        .clone();
    let spares: Vec<ObjId> = rig
        .tw
        .world()
        .module_state::<mush_tactics::UnitStore>()
        .expect("the units")
        .units
        .iter()
        .filter(|(id, unit)| **id != tug && **id != p_tug && unit.schema() == &schema)
        .map(|(id, _)| *id)
        .filter(|id| {
            crate::driving_rig::store(&rig.tw)
                .tv_lease_on(*id)
                .is_none()
        })
        .take(3)
        .collect();
    let [a, b, c] = spares.as_slice() else {
        panic!("three spare rigs in Occator's pool: {spares:?}");
    };
    let (a, b, c) = (*a, *b, *c);
    let occator = title(&rig, b);
    assert!(
        matches!(occator, Some(PrincipalId::Organization(_))),
        "{occator:?}"
    );
    let root = rig
        .tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .and_then(|store| {
            store
                .state()
                .orgs()
                .find(|org| org.name.as_str() == "Brask")
                .map(|org| org.root_scope.0)
        })
        .expect("Brask's root scope");
    let moved = run(
        &mut rig.tw,
        GOD,
        &format!("@asset/transfer #{}=org:Brask scope={root}", a.0),
    );
    assert!(
        matches!(title(&rig, a), Some(PrincipalId::Organization(org)) if Some(PrincipalId::Organization(org)) != occator),
        "{moved}"
    );
    let moved = run(
        &mut rig.tw,
        GOD,
        &format!("@asset/transfer #{}=#{}", c.0, t.0),
    );
    assert_eq!(title(&rig, c), Some(PrincipalId::Object(t)), "{moved}");

    // The ground: `a` and `c` beside the pad (in sight of T's carrier) on two of L's blocks,
    // `b` out of anyone's sight on open ground.
    let (rx, ry) = {
        let (x, y, _) = ramp_at(&rig.tw, entry);
        (x.div_euclid(1_000), y.div_euclid(1_000))
    };
    let (a_cell, c_cell, b_cell) = ((rx + 3, ry - 2), (rx - 3, ry - 2), (rx + 3, ry + 44));
    let (a_letter, c_letter, b_letter) = (
        letter_of(side, a_cell),
        letter_of(side, c_cell),
        letter_of(side, b_cell),
    );
    assert_ne!(a_letter, c_letter, "`a` and `c` lie on two blocks");
    assert!(
        b_letter != a_letter && b_letter != c_letter,
        "`b` lies on open ground"
    );
    let s = staff_member(&mut rig);
    for (hull, (x, y)) in [(a, a_cell), (b, b_cell), (c, c_cell)] {
        to_cell_on(&mut rig, entry, hull, (x, y));
        powerless(&mut rig, hull);
        staff(
            &mut rig,
            s,
            &format!("@tacsim/site {entry} leave #{} casualty {x},{y}", hull.0),
        );
    }
    seed_block(&mut rig, entry, a_letter, l);
    seed_block(&mut rig, entry, c_letter, l);

    // 1. The authorizations (Q-41-21), before any attach: T authorizes O, and L, to recover
    //    `c`; L delegates the recovery of `c`'s block to O. A stranger may authorize nothing.
    let refused = run(
        &mut rig.tw,
        l,
        &format!("+salvage/authorize #{}=Second", c.0),
    );
    assert!(refused.contains("not yours to authorize"), "{refused}");
    let preview = run(
        &mut rig.tw,
        t,
        &format!("+salvage/authorize #{}=Second", c.0),
    );
    assert!(preview.contains("You would authorize Second"), "{preview}");
    for who in ["Second", "Lander"] {
        let out = issue(
            &mut rig,
            t,
            &format!("+salvage/authorize #{}={who} go", c.0),
        );
        assert!(out.contains("unit authority records the grant"), "{out}");
    }
    let block = format!("{entry}/{c_letter}");
    let refused = run(
        &mut rig.tw,
        o,
        &format!("+salvage/authorize {block}=Lander"),
    );
    assert!(refused.contains("no live claim"), "{refused}");
    let out = go(
        &mut rig,
        l,
        &format!("+salvage/authorize {block}=Second go"),
    );
    assert!(out.contains("recover hulls off your block"), "{out}");
    assert_eq!(record(&mut rig, entry).land_grants.len(), 1);

    // 2. O recovers all three into O's carrier; each pin reads who answered and the
    //    authorizations at the attach.
    launched_at(&mut rig, entry, o, tug);
    for (hull, at) in [(a, a_cell), (c, c_cell)] {
        recover(&mut rig, entry, (o, tug), hull, at);
    }
    launched_at(&mut rig, entry, p, p_tug);
    recover(&mut rig, entry, (p, p_tug), b, b_cell);
    let pin_of = |rig: &mut Rig, hull: ObjId| {
        custody(rig, entry, hull)
            .and_then(|row| row.pin)
            .expect("the pin rode into custody")
    };
    let a_pin = pin_of(&mut rig, a);
    assert!(
        a_pin.answerers.contains(&t),
        "Brask's grantee answers: {a_pin:?}"
    );
    assert!(a_pin.owner_grant.is_none() && !a_pin.landholder_authorized);
    assert_eq!(a_pin.landholder(), Some(l));
    let c_pin = pin_of(&mut rig, c);
    assert_eq!(c_pin.answering, Some(t));
    assert_eq!(c_pin.owner_grant.map(|g| g.grantor), Some(t), "{c_pin:?}");
    assert!(c_pin.landholder_authorized, "{c_pin:?}");
    let b_pin = pin_of(&mut rig, b);
    let agent = b_pin
        .answerers
        .iter()
        .copied()
        .find(|who| {
            rig.tw
                .world()
                .get(*who)
                .is_some_and(|o| o.kind == ObjKind::Npc)
        })
        .unwrap_or_else(|| panic!("Occator's house agent answers for its rig: {b_pin:?}"));
    let unlawful: Vec<u32> = record(&mut rig, entry)
        .unlawful
        .iter()
        .filter(|act| act.kind == UnlawfulKind::Recovery && act.actor == o)
        .map(|act| act.subject)
        .collect();
    assert_eq!(unlawful, vec![a.0], "only `a`'s recovery was unauthorized");

    // 3. Filing tells everyone who answers for each title (Q-41-22, Q-41-28).
    let (c_a, c_b, c_c) = (
        file(&mut rig, entry, o, a),
        file(&mut rig, entry, p, b),
        file(&mut rig, entry, o, c),
    );
    let told = |rig: &mut Rig, hull: ObjId| custody(rig, entry, hull).expect("in custody").told;
    let a_told = told(&mut rig, a);
    assert!(a_told.contains(&t) && a_told.contains(&l), "{a_told:?}");
    assert!(
        told(&mut rig, b).contains(&agent),
        "the house agent is told"
    );
    assert_eq!(told(&mut rig, c), vec![t, l]);
    let lawful = |rig: &mut Rig, hull: ObjId| {
        custody(rig, entry, hull)
            .and_then(|row| row.file)
            .is_some_and(|file| file.entitlement_at_attach.is_lawful())
    };
    assert!(!lawful(&mut rig, a) && lawful(&mut rig, c) && lawful(&mut rig, b));

    // 4. `a`: procedure fails on the registry's own records (everyone told, the pin O's tug's,
    //    no box line to contradict the custody), right finds nothing that bound O, and T's
    //    sensor record vacates the claim.
    let refused = run(
        &mut rig.tw,
        t,
        &format!("+salvage/contest #{}=procedure", a.0),
    );
    assert!(refused.contains("show no defect"), "{refused}");
    let refused = run(
        &mut rig.tw,
        o,
        &format!("+salvage/contest #{}=procedure", a.0),
    );
    assert!(refused.contains("Only someone who answers"), "{refused}");
    let refused = run(&mut rig.tw, t, &format!("+salvage/contest #{}=right", a.0));
    assert!(refused.contains("no right that bound Second"), "{refused}");
    let refused = run(
        &mut rig.tw,
        t,
        &format!("+salvage/contest #{}=salvage", a.0),
    );
    assert!(refused.contains("none of them"), "{refused}");
    let preview = run(
        &mut rig.tw,
        t,
        &format!("+salvage/contest #{}=recovery", a.0),
    );
    assert!(preview.contains("sensor record"), "{preview}");
    let _ = go(
        &mut rig,
        t,
        &format!("+salvage/contest #{}=recovery go", a.0),
    );

    // 5. `c`: L's authorization of L's own is no right against O; T's, which bound O at the
    //    attach, vacates the claim (the obligor is the claimant).
    let refused = run(&mut rig.tw, l, &format!("+salvage/contest #{}=right", c.0));
    assert!(refused.contains("no right that bound Second"), "{refused}");
    let preview = run(&mut rig.tw, t, &format!("+salvage/contest #{}=right", c.0));
    assert!(preview.contains("bound Second to deliver her"), "{preview}");
    let _ = go(&mut rig, t, &format!("+salvage/contest #{}=right go", c.0));
    assert!(
        seconds_until(&mut rig, 10, |rig| custody(rig, entry, a).is_none()
            && custody(rig, entry, c).is_none()),
        "both vacating contests settle at once"
    );
    for (hull, id, owner) in [
        (a, c_a, title(&rig, a)),
        (c, c_c, Some(PrincipalId::Object(t))),
    ] {
        assert_eq!(contract_state(&rig, id), Some(ContractState::Cancelled));
        assert_eq!(title(&rig, hull), owner, "the title never moved");
    }
    let vacated: Vec<u32> = record(&mut rig, entry)
        .unlawful
        .iter()
        .filter(|act| act.kind == UnlawfulKind::Vacated)
        .map(|act| act.subject)
        .collect();
    assert_eq!(vacated, vec![a.0, c.0]);

    // 6. `b`: Occator's house agent contests on bare title, which never defeats a lawful
    //    claim; the window runs out and the registry awards the organization's unleased rig to P.
    let preview = run(&mut rig.tw, agent, &format!("+salvage/contest #{}", b.0));
    assert!(preview.contains("Bare title never defeats"), "{preview}");
    let _ = go(&mut rig, agent, &format!("+salvage/contest #{} go", b.0));
    jump(&mut rig, WINDOW + 30);
    assert!(
        seconds_until(&mut rig, 30, |rig| custody(rig, entry, b).is_none()),
        "the claim on `b` settles"
    );
    assert_eq!(title(&rig, b), Some(PrincipalId::Object(p)), "awarded to P");
    assert!(
        seconds_until(&mut rig, 120, |rig| contract_state(rig, c_b)
            == Some(ContractState::Expired)),
        "the sweep lapses the claim's contract"
    );
}

/// **RS.3's gate on Urvara Reach (512)**: story (b), extended.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rs3_story_b_the_claims_close_on_urvara_reach() {
    story("urvara", "urvara_port", 512);
}

/// **RS.3's gate on Ahuna Flank (256)**: story (b), extended.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rs3_story_b_the_claims_close_on_ahuna_flank() {
    story("ahuna", "ahuna_port", 256);
}
