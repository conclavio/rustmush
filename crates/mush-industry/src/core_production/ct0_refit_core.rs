#![cfg(test)]

//! Doc 46 CT.0: **the certified core refit pinned on its real owner path.** The fitting rig
//! drives a certified core through manufacture and delivery, then Offer, Accept, Advance,
//! Install and Cancel through the real Org, Economy, Inventory, Industry, Sim and Tactics owners
//! and the kernel's `core_refit` protocol. At every transition this walk writes the staged
//! bundle (definitions, outcome, every owner's replay write: in full for Economy, the kernel's
//! capacity reservation and core exchange, and the refit's own records, by key for every other
//! owner, `ct0_snapshot::Scope`), and every refit or Economy record read back (the Industry
//! order, offer, census, asset and hull heads, Economy's subject, payment history, hold and
//! accounts, the kernel's capacity reservation), then compares the whole with
//! `tests/ct0/refit_core.snap`. A cold boot must rebuild exactly the live records, the unit
//! image and object locations included (compared, not written).
use super::*;
use crate::ct0_snapshot::{self as ct0, Result, json};
use mush_testkit::snapshot::Snapshot;
const SNAP: &str = "tests/ct0/refit_core.snap";

/// The refit's own documents: its seven state families, the kernel's core exchange records
/// its validator joins, and its census key.
const SCOPE: ct0::Scope<'static> = ct0::Scope {
    families: &[
        "industry_refit_offers",
        "industry_refit_orders",
        "industry_refit_deployment",
        "industry_refit_assets",
        "industry_refit_hulls",
        "industry_refit_schedule",
        "industry_refit_schedule_order",
        "core_component_exchanges",
    ],
    index_key: crate::core_production::refit::records::INDEX_KEY,
};

/// One transition through the rig's own prepare, stage and install, written in full.
fn step(
    s: &mut Fitting,
    snap: &mut Snapshot,
    title: &str,
    phase: usize,
    inv: &ActionInvocation,
    n: u8,
) -> Result {
    let action = s.action(phase)?;
    let p = s.prepare_inv(phase, inv, n)?;
    let b = ct0::bundle(action, inv, n, &p)?;
    let before = ct0::documents(&s.f.r.boot)?;
    s.stage(phase, inv, n, p)?;
    let writes = s.f.take_install()?;
    check!(
        writes.as_slice() == b.writes(),
        "the staged bundle carries the preparation's writes"
    );
    s.f.r.flush();
    let after = ct0::documents(&s.f.r.boot)?;
    ct0::transition(
        snap,
        &SCOPE,
        title,
        s.f.r.w.world().now(),
        inv,
        &b,
        [&before, &after],
    )
}

/// A refused transition: the rig's preparation fails, and its refusal is written.
fn refused(
    s: &mut Fitting,
    snap: &mut Snapshot,
    label: &str,
    phase: usize,
    inv: &ActionInvocation,
    n: u8,
) -> Result {
    match s.prepare_inv(phase, inv, n) {
        Ok(_) => Err(format!("{label}: the transition was admitted").into()),
        Err(error) => {
            snap.line(label, &error.to_string());
            Ok(())
        }
    }
}

/// What a walk reads back: the offers and orders it opened, and the objects it moved.
#[derive(Default)]
struct Seen {
    orders: Vec<(u8, Option<WorkOrderId>)>,
    objects: Vec<ObjId>,
}

/// Offer (request `n`) and Accept (request `n + 1`), each written; the order Accept opens.
fn offer_and_accept(
    s: &mut Fitting,
    snap: &mut Snapshot,
    seen: &mut Seen,
    walk: &str,
    n: u8,
) -> Result<WorkOrderId> {
    let operator = s.f.r.policy.agents[1].actor;
    let inv = invocation(operator, n, s.offer_parameters());
    step(s, snap, &format!("{walk}: offered"), 1, &inv, n)?;
    seen.orders.push((n, None));
    seen.objects.push(s.incoming);
    ct0::lines(snap, &records(s, seen)?);
    let accept = json!({"offer":token(n),"payer":s.f.payer.0.to_string()});
    let inv = invocation(s.f.buyer, n + 1, accept);
    step(s, snap, &format!("{walk}: accepted"), 2, &inv, n + 1)?;
    let id = crate::workorder::derive_work_order_id(RequestUid::from_bytes([n + 1; 16]), 0)?;
    snap.line("work order id", &id.to_string());
    if let Some(last) = seen.orders.last_mut() {
        last.1 = Some(id);
    }
    ct0::lines(snap, &records(s, seen)?);
    Ok(id)
}

/// The order's two Advances through the real scheduler ingress, each written.
fn work(
    s: &mut Fitting,
    snap: &mut Snapshot,
    seen: &Seen,
    walk: &str,
    id: WorkOrderId,
    n: u8,
) -> Result {
    for (c, title) in [(n, "advanced once"), (n + 1, "advanced twice")] {
        let inv = s.advance_inv(id)?;
        step(s, snap, &format!("{walk}: {title}"), 3, &inv, c)?;
        ct0::lines(snap, &records(s, seen)?);
    }
    Ok(())
}

/// Every typed record the walk can read back, labelled, in each struct's field order.
fn records(s: &Fitting, seen: &Seen) -> Result<ct0::Records> {
    let rt = s.runtime()?;
    let state = s.f.r.w.engine().action_state();
    let economy =
        s.f.r
            .w
            .world()
            .module_state::<EconomyStore>()
            .ok_or("economy")?;
    let mut out = ct0::Records::new();
    let mut put = |written: bool, label: &str, value: String| {
        out.push((written, label.to_owned(), value));
    };
    let absent = || "absent".to_owned();
    match state.get(&rt.families.deployment, &())? {
        Some(index) => put(true, "record RefitIndex", json(index)?),
        None => put(true, "record RefitIndex", absent()),
    }
    put(
        true,
        "payment policy digest",
        rt.config.payment().digest()?.to_hex(),
    );
    for (n, order) in &seen.orders {
        match state.get(&rt.families.offers, &token(*n))? {
            Some(offer) => {
                put(true, &format!("record RefitOffer {n}"), json(offer)?);
                put(true, &format!("offer digest {n}"), offer.digest()?.to_hex());
            }
            None => put(true, &format!("record RefitOffer {n}"), absent()),
        }
        let Some(id) = order else { continue };
        let row = state.get(&rt.families.orders, id)?.ok_or("refit order")?;
        put(true, &format!("record RefitOrder {id}"), json(row)?);
        let subject = row.intent.subject()?;
        put(true, "economy subject", json(&subject)?);
        put(true, "economy subject digest", subject.digest()?.to_hex());
        if let Some(installation) = &row.installation {
            put(
                true,
                "replacement evidence digest",
                installation.replacement.digest()?.to_hex(),
            );
        }
        put(
            true,
            "economy payment history",
            json(economy.core_refit_history(row.accepted)?)?,
        );
        match economy.snapshot().hold(row.funding.hold()) {
            Some(hold) => put(true, "economy hold", json(hold)?),
            None => put(true, "economy hold", absent()),
        }
        let key = format!("action-state/capacity/v1/{}", row.accepted.to_hex());
        let reservation =
            s.f.r
                .boot
                .get("industry")
                .ok_or("industry")?
                .kv
                .iter()
                .find(|(k, _)| k.as_str() == key)
                .map(|(_, v)| json(v))
                .transpose()?;
        put(
            true,
            &format!("capacity reservation {key}"),
            reservation.unwrap_or_else(absent),
        );
        let schedule = row.progress.cursor().id();
        match state.get(&rt.families.schedules, &schedule)? {
            Some(cursor) => put(true, "schedule", json(cursor)?),
            None => put(true, "schedule", absent()),
        }
        match state.get(&rt.families.jobs, &schedule)? {
            Some(job) => put(true, "schedule order", json(job)?),
            None => put(true, "schedule order", absent()),
        }
    }
    for object in &seen.objects {
        match state.get(&rt.families.assets, object)? {
            Some(head) => put(true, &format!("record AssetHead {}", object.0), json(head)?),
            None => put(true, &format!("record AssetHead {}", object.0), absent()),
        }
        put(
            false,
            &format!("location {}", object.0),
            json(&s.f.r.w.world().get(*object).and_then(|o| o.location))?,
        );
    }
    match state.get(&rt.families.hulls, &s.hull)? {
        Some(head) => put(true, "record HullHead", json(head)?),
        None => put(true, "record HullHead", absent()),
    }
    let provider = rt.config.payment().provider.account;
    for (label, account) in [("payer", s.f.payer), ("provider", provider)] {
        let a = economy
            .snapshot()
            .account(account)
            .ok_or("account")?
            .clone();
        put(true, &format!("account {label}"), json(&a)?);
    }
    put(false, "unit", json(&s.unit()?)?);
    put(
        false,
        "hull location",
        json(&s.f.r.w.world().get(s.hull).and_then(|o| o.location))?,
    );
    Ok(out)
}

/// A cold boot from the persisted documents: what the store takes at boot is written, and
/// every record must equal the live one.
fn cold_boot(s: &mut Fitting, snap: &mut Snapshot, seen: &Seen, walk: &str) -> Result {
    let live = records(s, seen)?;
    let before = ct0::documents(&s.f.r.boot)?;
    s.f.restart(CoreOriginMode::Active)?;
    let after = ct0::documents(&s.f.r.boot)?;
    let recovered = records(s, seen)?;
    check_eq!(recovered, live, "the cold boot rebuilds every live record");
    ct0::cold_boot(
        snap,
        &SCOPE,
        &format!("{walk}: cold boot"),
        &before,
        &after,
        &live,
        &recovered,
    )
}

/// The identity strings and registered definitions, read from the running composition.
fn identities(s: &Fitting, snap: &mut Snapshot) -> Result {
    use crate::core_production::refit::records as r;
    snap.section("identities");
    snap.wire("industry actions", &refit::ACTIONS)?;
    snap.line("industry cadence", &refit::CADENCE.to_string());
    snap.line(
        "industry max work units",
        &refit::MAX_WORK_UNITS.to_string(),
    );
    snap.line(
        "industry max progress bytes",
        &refit::MAX_PROGRESS_BYTES.to_string(),
    );
    snap.line("industry index key", r::INDEX_KEY);
    snap.line("industry offer seconds", &r::OFFER_SECONDS.to_string());
    snap.line(
        "industry record bounds",
        &format!(
            "orders={} offer={} order={} installation={} index={} asset={}",
            r::MAX_ORDERS,
            r::OFFER_BYTES,
            r::ORDER_BYTES,
            r::INSTALLATION_BYTES,
            r::INDEX_BYTES,
            r::ASSET_BYTES
        ),
    );
    snap.wire(
        "economy actions",
        &[
            mush_economy::core_refit::OFFER_ACTION,
            mush_economy::core_refit::ACCEPT_ACTION,
            mush_economy::core_refit::INSTALL_ACTION,
            mush_economy::core_refit::CANCEL_ACTION,
        ],
    )?;
    snap.line(
        "economy fitting minor",
        &mush_economy::core_refit::FITTING_MINOR.to_string(),
    );
    snap.line(
        "economy reservation seconds",
        &mush_economy::core_refit::RESERVATION_SECONDS.to_string(),
    );
    let rt = s.runtime()?;
    for (name, action) in refit::ACTIONS.iter().zip(&rt.actions) {
        snap.wire(&format!("action {name}"), action)?;
        ct0::definition(
            snap,
            &format!("action {name}"),
            &rt.config.action_descriptor(name)?,
        )?;
    }
    for participant in &rt.participants {
        snap.wire("install participant", participant)?;
    }
    snap.wire("outcome", &rt.outcome)?;
    ct0::definition(snap, "outcome", &refit::runtime::outcome_descriptor()?)?;
    snap.wire("tactics owner", &rt.config.tactics_owner)?;
    snap.wire("component", rt.config.component())?;
    snap.wire("payment policy", rt.config.payment())?;
    let f = &rt.families;
    ct0::family(snap, "family offers", f.offers.descriptor());
    ct0::family(snap, "family orders", f.orders.descriptor());
    ct0::family(snap, "family deployment", f.deployment.descriptor());
    ct0::family(snap, "family assets", f.assets.descriptor());
    ct0::family(snap, "family hulls", f.hulls.descriptor());
    ct0::family(snap, "family schedules", f.schedules.descriptor());
    ct0::family(snap, "family jobs", f.jobs.descriptor());
    ct0::registered(
        snap,
        s.f.r.w.engine(),
        &[
            "refit",
            "core_component",
            "core_production",
            "initial_filing",
            "replacement",
        ],
    )?;
    Ok(())
}

/// Offer and Accept, an Install refused before the work, two Advances and Install of the
/// delivered core over the hull's stock core; then the installed core is damaged, a second
/// certified core is made, and a second fitting displaces the first core (its condition kept)
/// back to the requester; a cold boot.
/// The rig, the first core and both orders go on to the codec pins.
fn install_walk(snap: &mut Snapshot) -> Result<(Fitting, ObjId, WorkOrderId, WorkOrderId)> {
    let walk = "install walk";
    let mut s = Fitting::new(false)?;
    identities(&s, snap)?;
    let mut seen = Seen::default();
    snap.section(&format!("{walk}: delivered"));
    ct0::lines(snap, &records(&s, &seen)?);
    let first = offer_and_accept(&mut s, snap, &mut seen, walk, 40)?;
    let early = invocation(s.f.buyer, 44, FactoryRig::order_parameters(first));
    refused(
        &mut s,
        snap,
        "install before the work is done refused",
        4,
        &early,
        44,
    )?;
    work(&mut s, snap, &seen, walk, first, 42)?;
    let inv = invocation(s.f.buyer, 44, FactoryRig::order_parameters(first));
    step(&mut s, snap, &format!("{walk}: installed"), 4, &inv, 44)?;
    ct0::lines(snap, &records(&s, &seen)?);
    check!(s.order(first)?.installation.is_some());
    let core = mush_tactics::ComponentId::new("core")?;
    damage_installed_core(&mut s, &core)?;
    second_core(&mut s)?;
    let walk = "displacing install walk";
    snap.section(&format!(
        "{walk}: installed core damaged, second core delivered"
    ));
    ct0::lines(snap, &records(&s, &seen)?);
    let second = offer_and_accept(&mut s, snap, &mut seen, walk, 70)?;
    work(&mut s, snap, &seen, walk, second, 72)?;
    let inv = invocation(s.f.buyer, 74, FactoryRig::order_parameters(second));
    step(&mut s, snap, &format!("{walk}: installed"), 4, &inv, 74)?;
    ct0::lines(snap, &records(&s, &seen)?);
    check!(
        s.order(second)?
            .installation
            .as_ref()
            .is_some_and(|i| i.replacement.displaced.is_some())
    );
    // What the snapshot no longer writes: the second core is mounted in the hull's unit and
    // sits in the hull; the displaced first core is back with the requester.
    check_eq!(
        SavedComponent::from_unit(&s.unit()?, &core)?
            .installed
            .asset,
        Some(s.incoming)
    );
    check_eq!(
        s.f.r.w.world().get(s.incoming).and_then(|o| o.location),
        Some(s.hull)
    );
    let displaced = seen.objects.first().copied().ok_or("the first core")?;
    check_eq!(
        s.f.r.w.world().get(displaced).and_then(|o| o.location),
        Some(s.f.buyer)
    );
    cold_boot(&mut s, snap, &seen, walk)?;
    let old = seen.objects.first().copied().ok_or("the first core")?;
    Ok((s, old, first, second))
}

/// Offer, Accept, one Advance, Cancel: the fee, the slot and the core released; a cold boot.
fn cancel_walk(snap: &mut Snapshot) -> Result {
    let walk = "cancel walk";
    let mut s = Fitting::new(false)?;
    let mut seen = Seen::default();
    let id = offer_and_accept(&mut s, snap, &mut seen, walk, 40)?;
    let inv = s.advance_inv(id)?;
    step(&mut s, snap, &format!("{walk}: advanced once"), 3, &inv, 42)?;
    ct0::lines(snap, &records(&s, &seen)?);
    let inv = invocation(s.f.buyer, 45, FactoryRig::order_parameters(id));
    step(&mut s, snap, &format!("{walk}: cancelled"), 5, &inv, 45)?;
    ct0::lines(snap, &records(&s, &seen)?);
    check!(s.order(id)?.terminal.is_some());
    cold_boot(&mut s, snap, &seen, walk)
}

/// Offer, Accept and the whole work, then the reservation's deadline: Install is refused,
/// Cancel releases at the deadline; a cold boot.
fn expired_walk(snap: &mut Snapshot) -> Result {
    let walk = "expired reservation walk";
    let mut s = Fitting::new(false)?;
    let mut seen = Seen::default();
    let id = offer_and_accept(&mut s, snap, &mut seen, walk, 40)?;
    s.ready(id, 42)?;
    s.f.r.flush();
    let deadline = s.order(id)?.intent.expires_at.at;
    s.f.r.w.world_mut().set_now(deadline);
    snap.section(&format!("{walk}: worked, at the deadline"));
    snap.line("now utc", &deadline.to_string());
    let late = invocation(s.f.buyer, 44, FactoryRig::order_parameters(id));
    refused(
        &mut s,
        snap,
        "install at the deadline refused",
        4,
        &late,
        44,
    )?;
    let inv = invocation(s.f.buyer, 45, FactoryRig::order_parameters(id));
    step(
        &mut s,
        snap,
        &format!("{walk}: cancelled at the deadline"),
        5,
        &inv,
        45,
    )?;
    ct0::lines(snap, &records(&s, &seen)?);
    cold_boot(&mut s, snap, &seen, walk)
}

/// Offer, then Accept once the offer has stood its time: refused, nothing held.
fn expired_offer_walk(snap: &mut Snapshot) -> Result {
    let walk = "expired offer walk";
    let mut s = Fitting::new(false)?;
    let mut seen = Seen::default();
    let operator = s.f.r.policy.agents[1].actor;
    let inv = invocation(operator, 40, s.offer_parameters());
    step(&mut s, snap, &format!("{walk}: offered"), 1, &inv, 40)?;
    seen.orders.push((40, None));
    seen.objects.push(s.incoming);
    let now = s.f.r.w.world().now();
    s.f.r
        .w
        .world_mut()
        .set_now(now + crate::core_production::refit::records::OFFER_SECONDS);
    snap.section(&format!("{walk}: after the offer stood"));
    snap.line("now utc", &s.f.r.w.world().now().to_string());
    let accept = json!({"offer":token(40),"payer":s.f.payer.0.to_string()});
    let inv = invocation(s.f.buyer, 41, accept);
    refused(
        &mut s,
        snap,
        "accept after the offer expired refused",
        2,
        &inv,
        41,
    )?;
    ct0::lines(snap, &records(&s, &seen)?);
    Ok(())
}

/// Codec pins of Industry's strict core refit records, from the two installed orders the walk
/// made: the bytes in field order, and the refusal of an unknown field and of a broken value (a
/// `required_option` field dropped where the type has one, a present `core_refit` kind, else a
/// field of the wrong type). The skipped optional fields are pinned present and absent.
fn codecs(
    s: &Fitting,
    snap: &mut Snapshot,
    old: ObjId,
    first: WorkOrderId,
    second: WorkOrderId,
) -> Result {
    let rt = s.runtime()?;
    let state = s.f.r.w.engine().action_state();
    let order = state.get(&rt.families.orders, &second)?.ok_or("order")?;
    let installation = order.installation.as_ref().ok_or("installation")?;
    let evidence = &installation.replacement;
    let offer = &order.intent.offer;
    snap.codec("SavedComponent", &evidence.inserted, ct0::mistyped("cells"))?;
    snap.codec(
        "ReplacementEvidence (displaced)",
        evidence,
        ct0::mistyped("from_revision"),
    )?;
    let mut coreless = evidence.clone();
    coreless.displaced = None;
    snap.codec(
        "ReplacementEvidence (coreless)",
        &coreless,
        ct0::mistyped("from_revision"),
    )?;
    let present = |json: &mut Value| {
        if let Some(object) = json.as_object_mut() {
            object.insert("kind".into(), "core_refit".into());
        }
    };
    snap.codec("RefitOffer (displaced)", offer, present)?;
    let mut bare = offer.clone();
    bare.displaced = None;
    snap.codec("RefitOffer (coreless)", &bare, present)?;
    // The ungoverning kind is written by presence; this rig has no ungoverning site, so the
    // value is the walk's offer with the kind set (a codec fixture, not an admitted offer).
    let mut ungovern = offer.clone();
    ungovern.kind = mush_economy::core_refit::RefitKind::CoreUngovern;
    snap.codec("RefitOffer (ungovern)", &ungovern, present)?;
    snap.codec("AcceptedIntent", &order.intent, ct0::without("payer"))?;
    snap.codec("Installation", installation, ct0::mistyped("at_utc"))?;
    snap.codec("RefitOrder", order, ct0::without("terminal"))?;
    let index = state.get(&rt.families.deployment, &())?.ok_or("index")?;
    snap.codec("RefitIndex", index, ct0::mistyped("format"))?;
    let head = state.get(&rt.families.assets, &old)?.ok_or("asset head")?;
    snap.codec("AssetHead", head, ct0::mistyped("format"))?;
    let lost = |json: &mut Value| {
        if let Some(object) = json.as_object_mut() {
            object.insert("state".into(), "ct0_lost".into());
        }
    };
    let detached = head.position.clone();
    for (name, position) in [
        ("pending", AssetPosition::Pending { order: first }),
        (
            "mounted",
            AssetPosition::Mounted {
                order: first,
                hull: s.hull,
            },
        ),
        ("released", AssetPosition::Released { order: first }),
        ("detached", detached),
    ] {
        snap.codec(&format!("AssetPosition ({name})"), &position, lost)?;
    }
    let hull = state.get(&rt.families.hulls, &s.hull)?.ok_or("hull head")?;
    snap.codec("HullHead", hull, ct0::without("mounted"))?;
    Ok(())
}

#[test]
fn ct0_core_refit_walks_match_the_committed_snapshot() -> Result {
    let mut snap = Snapshot::new();
    let (installed, old, first, second) = install_walk(&mut snap)?;
    cancel_walk(&mut snap)?;
    expired_walk(&mut snap)?;
    expired_offer_walk(&mut snap)?;
    codecs(&installed, &mut snap, old, first, second)?;
    snap.check(env!("CARGO_MANIFEST_DIR"), SNAP)?;
    Ok(())
}
