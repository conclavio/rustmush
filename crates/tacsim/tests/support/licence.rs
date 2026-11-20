//! **A prospecting licence bought through the real verb** (doc 20 cut SM.5): `+tv/lease` now
//! refuses without a licence in force, so every suite whose pilots lease a rig licenses them
//! here first, once, through `+licence/buy go` (never a hand-written record).

use mush_core::ObjId;
use mush_testkit::TestWorld;

/// `+licence/buy go` as `pilot`, from wherever they stand (the verb is accepted from
/// anywhere), asserting the Registry licensed them; what the pilot heard.
pub fn buy_licence(tw: &mut TestWorld, pilot: ObjId) -> String {
    let events = tw.run(pilot, "+license/buy go");
    let out = tw.seen_by(&events, pilot).join("\n");
    assert!(
        out.contains("licenses you to prospect at its sites until"),
        "the pilot is licensed: {out}"
    );
    out
}

/// **The picket's papers for `who`** (doc 50 unit LW.1, Q-50-65): a license through the real
/// verb (bought, or extended when held), and a live registered claim on the lane's first
/// strategic rock, written by hand as a typed record (staking one means flying to the rock).
/// Port fires follow until the board's claims mirror reads them. A story that trades strategic
/// ore on the Ceres board gives its trader these.
pub fn hold_the_papers(tw: &mut TestWorld, who: ObjId) {
    let events = tw.run(who, "+license/buy go");
    let bought = tw.seen_by(&events, who).join("\n");
    let (cluster, letter) = tacsim::space::mining::all_orbital_clusters(tw.world())
        .into_iter()
        .find_map(|cluster| {
            cluster
                .rocks
                .iter()
                .find(|rock| {
                    rock.spectral
                        .families()
                        .contains(&tacsim::surface::OreFamily::StrategicElements)
                })
                .map(|rock| (cluster.id.clone(), rock.letter))
        })
        .expect("a strategic rock");
    let now = tw.world().world_ic_now().at;
    let mut tenure = tacsim::surface::claims::BlockTenure::new(cluster.clone(), letter, who);
    tenure.claim = Some(tacsim::surface::claims::BlockClaim {
        holder: mush_org::PrincipalId::Object(who),
        account: tw
            .world()
            .module_state::<mush_sim::AccountOracle>()
            .and_then(|oracle| oracle.account_of(who)),
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
    let handles = tw
        .world()
        .module_state::<tacsim::port::PortHandles>()
        .cloned()
        .expect("the port's handles");
    let key = tacsim::surface::claims::BlockKey {
        site: cluster,
        letter,
    };
    let engine = tw.engine_mut();
    let prepared = {
        let mut txn = engine
            .action_transaction(&handles.families.claims)
            .expect("a transaction");
        if txn
            .get(&handles.families.claims, &key)
            .expect("the claims read")
            .is_some()
        {
            txn.replace(&handles.families.claims, &key, tenure)
                .expect("the claim writes");
        } else {
            txn.insert(&handles.families.claims, &key, tenure)
                .expect("the claim writes");
        }
        txn.prepare().expect("prepares")
    };
    let (_, install) = prepared.into_parts();
    engine.install_prepared_action(install).expect("installs");
    for _ in 0..60 {
        if tacsim::port::law::papers_clear(tw.world(), who, tw.world().world_ic_now().at) {
            return;
        }
        let t = tw.world().now().saturating_add(1);
        tw.set_now(t);
        let _ = tw.fire_due(t.saturating_mul(1_000));
    }
    let licensed =
        tacsim::port::licence::licence_gate(tw.world(), who, tw.world().world_ic_now().at);
    panic!("the papers never cleared (license: {licensed:?}): {bought}");
}
