#![cfg(test)]

//! Doc 46 CT.0: **the bay refit pinned on its real owner path.** The UD.4b rig drives Offer,
//! Accept, Advance, Install and Cancel through the real Org, Economy, Inventory, Industry, Sim
//! and Tactics owners and the kernel's `bay_refit` protocol; this walk writes, at every
//! transition, the staged bundle (its definitions, outcome and every owner's replay write: in
//! full for Economy, the kernel's capacity reservation and the bay's own records, by key for
//! every other owner, `ct0_snapshot::Scope`), and every bay or Economy record read back (the
//! Industry order, offer, census and hull head, Economy's subject, payment history, hold and
//! accounts, the kernel's capacity reservation), then compares the whole with
//! `tests/ct0/refit_bay.snap`. A cold boot must rebuild exactly the live records, the unit
//! image, lots and hull location included (compared, not written).
use super::*;
use crate::ct0_snapshot::{self as ct0, Result, json};
use crate::workorder::WorkOrderId;
use mush_testkit::snapshot::Snapshot;

const SNAP: &str = "tests/ct0/refit_bay.snap";

/// The bay's own documents: its six state families and its census key.
const SCOPE: ct0::Scope<'static> = ct0::Scope {
    families: &[
        "industry_bay_orders",
        "industry_bay_offers",
        "industry_bay_deployment",
        "industry_bay_hulls",
        "industry_bay_schedule",
        "industry_bay_schedule_order",
    ],
    index_key: records::INDEX_KEY,
};

/// One transition through the rig's own prepare and stage, written in full.
fn step(
    r: &mut Rig,
    s: &mut Snapshot,
    title: &str,
    phase: usize,
    inv: &ActionInvocation,
    n: u8,
) -> Result {
    let action = r.runtime()?.actions[phase].clone();
    let p = r.prepare_inv(phase, inv, n)?;
    let b = ct0::bundle(action, inv, n, &p)?;
    check!(
        b.writes() == p.0.authoritative_replay_writes().as_slice(),
        "the bundle carries the preparation's writes"
    );
    let before = ct0::documents(&r.boot)?;
    r.stage(phase, inv, n, p)?;
    let after = ct0::documents(&r.boot)?;
    ct0::transition(
        s,
        &SCOPE,
        title,
        r.w.world().now(),
        inv,
        &b,
        [&before, &after],
    )
}

/// A refused transition: the rig's preparation fails, and its refusal is written.
fn refused(
    r: &mut Rig,
    s: &mut Snapshot,
    label: &str,
    phase: usize,
    inv: &ActionInvocation,
    n: u8,
) -> Result {
    match r.prepare_inv(phase, inv, n) {
        Ok(_) => Err(format!("{label}: the transition was admitted").into()),
        Err(error) => {
            s.line(label, &error.to_string());
            Ok(())
        }
    }
}

/// The due Advance's invocation through the real scheduler ingress, as `Rig::advance` plans it.
fn advance_inv(r: &mut Rig, order: WorkOrderId) -> Result<ActionInvocation> {
    let due = r.order(order)?.progress.cursor().next_due().at;
    r.w.world_mut().set_now(due);
    let action = r.runtime()?.actions[3].clone();
    let page =
        r.w.engine()
            .discover_scheduled_actions(mush_core::ScheduleDriveCause::Live, None, 256)?;
    let candidate = page
        .candidates()
        .iter()
        .find(|c| c.action() == &action)
        .ok_or("a due bay schedule")?;
    let ingress = mush_core::scheduled_action_ingress(&action, candidate.advance(), 1)?;
    let (_, inv, _) =
        r.w.engine()
            .plan_scheduled_action(candidate, &ingress)?
            .into_parts();
    Ok(inv)
}

fn offer_parameters(r: &Rig) -> Value {
    json!({"requester":r.requester,"hull":r.hull,"facility":r.bay,
        "target":"bay_unit_b","part":r.lot,"multiplier_permille":1000})
}

fn order_parameters(id: WorkOrderId) -> Value {
    json!({"order":id.to_string()})
}

/// Offer (request 20) and Accept (request 21), each written; the order Accept opens.
fn offer_and_accept(r: &mut Rig, s: &mut Snapshot, walk: &str) -> Result<WorkOrderId> {
    let inv = r.invocation(r.operator, 20, offer_parameters(r))?;
    step(r, s, &format!("{walk}: offered"), 1, &inv, 20)?;
    ct0::lines(s, &records(r, None)?);
    let accept = json!({"offer":token(20),"payer":r.payer.0.to_string()});
    let inv = r.invocation(r.requester, 21, accept)?;
    step(r, s, &format!("{walk}: accepted"), 2, &inv, 21)?;
    let id = crate::workorder::derive_work_order_id(RequestUid::from_bytes([21; 16]), 0)?;
    s.line("work order id", &id.to_string());
    ct0::lines(s, &records(r, Some(id))?);
    Ok(id)
}

/// Every typed record the walk can read back, labelled, in each struct's field order.
fn records(r: &Rig, order: Option<WorkOrderId>) -> Result<ct0::Records> {
    let rt = r.runtime()?;
    let state = r.w.engine().action_state();
    let mut out = ct0::Records::new();
    let mut put = |written: bool, label: &str, value: String| {
        out.push((written, label.to_owned(), value));
    };
    let absent = || "absent".to_owned();
    match state.get(&rt.families.deployment, &())? {
        Some(index) => put(true, "record BayIndex", json(index)?),
        None => put(true, "record BayIndex", absent()),
    }
    match state.get(&rt.families.offers, &token(20))? {
        Some(offer) => {
            put(true, "record BayOffer", json(offer)?);
            put(true, "offer digest", offer.digest()?.to_hex());
        }
        None => put(true, "record BayOffer", absent()),
    }
    match state.get(rt.families.hulls(), &r.hull)? {
        Some(head) => put(true, "record BayHullHead", json(head)?),
        None => put(true, "record BayHullHead", absent()),
    }
    let economy =
        r.w.world()
            .module_state::<EconomyStore>()
            .ok_or("economy")?;
    put(
        true,
        "payment policy digest",
        rt.config.payment.digest()?.to_hex(),
    );
    let mut lots = vec![r.lot];
    if let Some(id) = order {
        let row = state.get(&rt.families.orders, &id)?.ok_or("order")?;
        put(true, "record BayOrder", json(row)?);
        let subject = row.intent.subject(&rt.config.payment)?;
        put(true, "economy subject", json(&subject)?);
        put(true, "economy subject digest", subject.digest()?.to_hex());
        if let Some(installation) = &row.installation {
            put(
                true,
                "refit evidence digest",
                installation.refit.digest()?.to_hex(),
            );
        }
        put(
            true,
            "economy payment history",
            json(economy.bay_refit_history(row.accepted)?)?,
        );
        match economy.snapshot().hold(row.funding.hold()) {
            Some(hold) => put(true, "economy hold", json(hold)?),
            None => put(true, "economy hold", absent()),
        }
        let key = format!("action-state/capacity/v1/{}", row.accepted.to_hex());
        let reservation = r
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
        match state.get(rt.families.schedules(), &schedule)? {
            Some(cursor) => put(true, "schedule", json(cursor)?),
            None => put(true, "schedule", absent()),
        }
        match state.get(rt.families.jobs(), &schedule)? {
            Some(job) => put(true, "schedule order", json(job)?),
            None => put(true, "schedule order", absent()),
        }
        lots.extend(row.intent.offer.returned);
    }
    for (label, account) in [("payer", r.payer), ("provider", r.provider)] {
        let a = economy
            .snapshot()
            .account(account)
            .ok_or("account")?
            .clone();
        put(true, &format!("account {label}"), json(&a)?);
    }
    let f = r.inventory()?;
    for lot in lots {
        match state.get(&f.lots, &lot)? {
            Some(row) => put(false, &format!("lot {}", lot.0), json(row)?),
            None => put(false, &format!("lot {}", lot.0), absent()),
        }
    }
    put(false, "unit", json(&r.unit()?)?);
    put(
        false,
        "hull location",
        json(&r.w.world().get(r.hull).and_then(|o| o.location))?,
    );
    Ok(out)
}

/// A cold boot from the persisted documents (Economy's `bridge::recover`, Industry's audit):
/// what the store takes at boot is written, and every record must equal the live one.
fn cold_boot(r: &mut Rig, s: &mut Snapshot, walk: &str, order: WorkOrderId) -> Result {
    let live = records(r, Some(order))?;
    let before = ct0::documents(&r.boot)?;
    r.restart()?;
    let after = ct0::documents(&r.boot)?;
    let recovered = records(r, Some(order))?;
    check_eq!(recovered, live, "the cold boot rebuilds every live record");
    ct0::cold_boot(
        s,
        &SCOPE,
        &format!("{walk}: cold boot"),
        &before,
        &after,
        &live,
        &recovered,
    )
}

/// The requester's attempt to carry the hull away, as the requester sees it.
fn carry(r: &mut Rig, s: &mut Snapshot, label: &str) {
    let requester = r.requester;
    let events = r.w.run(requester, "get Bay hull");
    for (number, line) in r.w.seen_by(&events, requester).iter().enumerate() {
        s.text(&format!("{label} {number}"), line);
    }
}

/// The identity strings and registered definitions, read from the running composition.
fn identities(r: &Rig, s: &mut Snapshot) -> Result {
    s.section("identities");
    s.wire("industry actions", &ACTIONS)?;
    s.line("industry offers family", OFFERS);
    s.line("industry capacity index family", CAPACITY_INDEX);
    s.line("industry clock policy", CLOCK_POLICY);
    s.line("industry cadence", &CADENCE.to_string());
    s.line("industry max work units", &MAX_WORK_UNITS.to_string());
    s.line("industry offer seconds", &OFFER_SECONDS.to_string());
    s.line("industry refit digest domain", REFIT_DIGEST_DOMAIN);
    s.line("industry bay capability", BAY_CAPABILITY);
    s.line("industry index key", records::INDEX_KEY);
    s.line(
        "industry record bounds",
        &format!(
            "orders={} offer={} order={} installation={} index={}",
            records::MAX_ORDERS,
            records::OFFER_BYTES,
            records::ORDER_BYTES,
            records::INSTALLATION_BYTES,
            records::INDEX_BYTES
        ),
    );
    s.wire(
        "economy actions",
        &[
            mush_economy::bay_refit::OFFER_ACTION,
            mush_economy::bay_refit::ACCEPT_ACTION,
            mush_economy::bay_refit::INSTALL_ACTION,
            mush_economy::bay_refit::CANCEL_ACTION,
        ],
    )?;
    s.line(
        "economy operator capability",
        mush_economy::bay_refit::OPERATOR_CAPABILITY,
    );
    s.line(
        "economy reservation seconds",
        &mush_economy::bay_refit::RESERVATION_SECONDS.to_string(),
    );
    s.wire("kernel actions", &mush_core::bay_refit_protocol::ACTIONS)?;
    for (owner, name) in mush_core::bay_refit_protocol::FAMILY_NAMES {
        let d = mush_core::bay_refit_protocol::family_descriptor(name)?;
        ct0::family(s, &format!("kernel family {owner}"), &d);
    }
    let rt = r.runtime()?;
    for (name, action) in ACTIONS.iter().zip(&rt.actions) {
        s.wire(&format!("action {name}"), action)?;
        ct0::definition(
            s,
            &format!("action {name}"),
            &rt.config.policy.action_descriptor(name)?,
        )?;
    }
    for participant in &rt.participants {
        s.wire("install participant", participant)?;
    }
    s.wire("outcome", &rt.outcome)?;
    ct0::definition(s, "outcome", &runtime::outcome_descriptor()?)?;
    s.wire("owner participant", &rt.config.participant()?)?;
    ct0::definition(s, "owner participant", &rt.config.participant_descriptor()?)?;
    s.wire("tactics participant", &rt.config.tactics_participant()?)?;
    s.wire("economy owner", &rt.config.economy_owner)?;
    s.wire("refit policy", &rt.config.policy)?;
    s.wire("payment policy", &rt.config.payment)?;
    ct0::family(s, "family orders", rt.families.orders.descriptor());
    ct0::family(s, "family offers", rt.families.offers.descriptor());
    ct0::family(s, "family deployment", rt.families.deployment.descriptor());
    ct0::family(s, "family hulls", rt.families.hulls().descriptor());
    ct0::family(s, "family schedules", rt.families.schedules().descriptor());
    ct0::family(s, "family jobs", rt.families.jobs().descriptor());
    ct0::registered(s, r.w.engine(), &["bay", "refit", "initial_filing"])?;
    Ok(())
}

/// Offer, Accept, an Install refused before the work, two Advances, Install, a cold boot.
/// The rig and its installed order go on to the codec pins.
fn install_walk(s: &mut Snapshot) -> Result<(Box<Rig>, WorkOrderId)> {
    let mut r = Rig::new()?;
    identities(&r, s)?;
    s.section("install walk: initialized");
    ct0::lines(s, &records(&r, None)?);
    let id = offer_and_accept(&mut r, s, "install walk")?;
    carry(&mut r, s, "carry while held");
    let early = r.invocation(r.requester, 22, order_parameters(id))?;
    refused(
        &mut r,
        s,
        "install before the work is done refused",
        4,
        &early,
        22,
    )?;
    for (n, title) in [(30, "advanced once"), (31, "advanced twice")] {
        let inv = advance_inv(&mut r, id)?;
        step(&mut r, s, &format!("install walk: {title}"), 3, &inv, n)?;
        ct0::lines(s, &records(&r, Some(id))?);
    }
    let inv = r.invocation(r.requester, 32, order_parameters(id))?;
    step(&mut r, s, "install walk: installed", 4, &inv, 32)?;
    ct0::lines(s, &records(&r, Some(id))?);
    check!(r.order(id)?.installation.is_some());
    // What the snapshot no longer writes: the hull is refitted to the laser variant, the laser
    // lot is consumed and the cannon is returned to the bay's stores.
    check_eq!(r.unit()?.schema().key().id.as_str(), "bay_unit_b");
    let lots = r.inventory()?.lots;
    check!(r.w.engine().action_state().get(&lots, &r.lot)?.is_none());
    let returned = r.order(id)?.intent.offer.returned.ok_or("a returned lot")?;
    let cannon =
        r.w.engine()
            .action_state()
            .get(&lots, &returned)?
            .ok_or("the cannon in stores")?
            .clone();
    check_eq!(cannon.commodity().as_str(), "part.bay_cannon");
    check_eq!(cannon.endpoint(), r.stores);
    cold_boot(&mut r, s, "install walk", id)?;
    Ok((r, id))
}

/// Offer, Accept, Cancel at once, a cold boot.
fn cancel_walk(s: &mut Snapshot) -> Result {
    let mut r = Rig::new()?;
    let walk = "cancel walk";
    let id = offer_and_accept(&mut r, s, walk)?;
    let inv = r.invocation(r.requester, 22, order_parameters(id))?;
    step(&mut r, s, &format!("{walk}: cancelled"), 5, &inv, 22)?;
    ct0::lines(s, &records(&r, Some(id))?);
    cold_boot(&mut r, s, walk, id)
}

/// Offer, Accept, one Advance, Cancel after part of the work, a cold boot, and the released
/// hull carried away.
fn cancel_after_work_walk(s: &mut Snapshot) -> Result {
    let mut r = Rig::new()?;
    let walk = "cancel after work walk";
    let id = offer_and_accept(&mut r, s, walk)?;
    let inv = advance_inv(&mut r, id)?;
    step(&mut r, s, &format!("{walk}: advanced once"), 3, &inv, 30)?;
    ct0::lines(s, &records(&r, Some(id))?);
    let inv = r.invocation(r.requester, 33, order_parameters(id))?;
    step(&mut r, s, &format!("{walk}: cancelled"), 5, &inv, 33)?;
    ct0::lines(s, &records(&r, Some(id))?);
    check!(r.order(id)?.terminal.is_some());
    cold_boot(&mut r, s, walk, id)?;
    // The released hull can be carried away again (the engine's own sentence is not pinned).
    let requester = r.requester;
    r.w.run(requester, "get Bay hull");
    check_eq!(
        r.w.world().get(r.hull).and_then(|o| o.location),
        Some(requester),
        "a released hull can be taken again"
    );
    Ok(())
}

/// Offer, Accept and the whole work, then the reservation's deadline: Install is refused,
/// Cancel releases at the deadline, a cold boot.
fn expired_walk(s: &mut Snapshot) -> Result {
    let mut r = Rig::new()?;
    let walk = "expired reservation walk";
    let id = offer_and_accept(&mut r, s, walk)?;
    for n in [30, 31] {
        let inv = advance_inv(&mut r, id)?;
        let p = r.prepare_inv(3, &inv, n)?;
        r.stage(3, &inv, n, p)?;
    }
    let deadline = r.order(id)?.intent.expires_at.at;
    r.w.world_mut().set_now(deadline);
    s.section(&format!("{walk}: worked, at the deadline"));
    s.line("now utc", &deadline.to_string());
    let late = r.invocation(r.requester, 32, order_parameters(id))?;
    refused(&mut r, s, "install at the deadline refused", 4, &late, 32)?;
    let inv = r.invocation(r.requester, 34, order_parameters(id))?;
    step(
        &mut r,
        s,
        &format!("{walk}: cancelled at the deadline"),
        5,
        &inv,
        34,
    )?;
    ct0::lines(s, &records(&r, Some(id))?);
    cold_boot(&mut r, s, walk, id)
}

/// Offer, then Accept once the offer has stood its time: refused, nothing held.
fn expired_offer_walk(s: &mut Snapshot) -> Result {
    let mut r = Rig::new()?;
    let walk = "expired offer walk";
    let inv = r.invocation(r.operator, 20, offer_parameters(&r))?;
    step(&mut r, s, &format!("{walk}: offered"), 1, &inv, 20)?;
    let now = r.w.world().now();
    r.w.world_mut().set_now(now + OFFER_SECONDS);
    s.section(&format!("{walk}: after the offer stood"));
    s.line("now utc", &r.w.world().now().to_string());
    let accept = json!({"offer":token(20),"payer":r.payer.0.to_string()});
    let inv = r.invocation(r.requester, 21, accept)?;
    refused(
        &mut r,
        s,
        "accept after the offer expired refused",
        2,
        &inv,
        21,
    )?;
    ct0::lines(s, &records(&r, None)?);
    Ok(())
}

/// Codec pins of Industry's strict bay records, from the installed order the walk made: the
/// bytes in field order, and the refusal of an unknown field and of a broken value (a
/// `required_option` field dropped where the type has one, else a field of the wrong type).
fn codecs(r: &Rig, s: &mut Snapshot, id: WorkOrderId) -> Result {
    let rt = r.runtime()?;
    let state = r.w.engine().action_state();
    let order = state.get(&rt.families.orders, &id)?.ok_or("order")?;
    let installation = order.installation.as_ref().ok_or("installation")?;
    let offer = &order.intent.offer;
    s.codec("BayRefitPolicy", &rt.config.policy, ct0::mistyped("format"))?;
    s.codec("BayRecipe", &offer.recipe, ct0::mistyped("work_units"))?;
    s.codec("BayOffer", offer, ct0::without("returned"))?;
    let mut bare = offer.clone();
    bare.remove = None;
    bare.returned = None;
    s.codec(
        "BayOffer (nothing taken out)",
        &bare,
        ct0::without("remove"),
    )?;
    s.codec("BayIntent", &order.intent, ct0::without("expires_at"))?;
    s.codec(
        "RefitEvidence",
        &installation.refit,
        ct0::without("removed"),
    )?;
    s.codec("BayInstallation", installation, ct0::mistyped("at_utc"))?;
    s.codec("BayOrder", order, ct0::without("installation"))?;
    let index = state.get(&rt.families.deployment, &())?.ok_or("index")?;
    s.codec("BayIndex", index, ct0::mistyped("format"))?;
    let head = state
        .get(rt.families.hulls(), &r.hull)?
        .ok_or("hull head")?;
    s.codec("BayHullHead", head, ct0::without("pending"))?;
    Ok(())
}

#[test]
fn ct0_bay_refit_walks_match_the_committed_snapshot() -> Result {
    let mut s = Snapshot::new();
    let (installed, id) = install_walk(&mut s)?;
    cancel_walk(&mut s)?;
    cancel_after_work_walk(&mut s)?;
    expired_walk(&mut s)?;
    expired_offer_walk(&mut s)?;
    codecs(&installed, &mut s, id)?;
    s.check(env!("CARGO_MANIFEST_DIR"), SNAP)?;
    Ok(())
}
