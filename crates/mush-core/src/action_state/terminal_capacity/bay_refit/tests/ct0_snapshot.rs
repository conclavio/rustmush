//! Doc 46 CT.0: the bay refit walk, pinned in `tests/ct0/refit_bay.snap`. It drives the
//! closed validator (`validate_changes`) with this rig's inert typed rows through Accept,
//! Advance, Install and Cancel (`ct0_kit::Rig` applies each accepted write set and boots the
//! result cold through the coordinator's own `hydrate`).
use super::*;
use crate::action_state::terminal_capacity::tests::ct0_kit as kit;
use mush_testkit::snapshot::Snapshot;

use kit::Write;

/// The walk's committed state (`kit::Rig`) and this rig's definitions.
struct Walk {
    d: Defs,
    rig: kit::Rig,
}

impl Walk {
    fn new(before: Vec<(Key, Value)>) -> TestResult<Self> {
        let d = defs()?;
        let reg = registry(&d)?;
        Ok(Self {
            rig: kit::Rig::new(reg, validate_changes, "bay refit", before)?,
            d,
        })
    }
}

fn industry_id(accepted: CommitToken) -> TestResult<ReservationId> {
    Ok(ReservationId(owner_id("industry")?, accepted.to_hex()))
}

/// The row's own identity: its fingerprint, keys, slot, holder and schedule.
fn row_lines(snap: &mut Snapshot, r: &Row) -> TestResult {
    snap.hex("capacity fingerprint", r.fingerprint.as_bytes());
    snap.wire("resource", &r.resource()?)?;
    snap.wire("holder", &r.holder()?)?;
    snap.wire("schedule", &r.schedule()?)?;
    snap.wire(
        "slot reservation id",
        &crate::ReservationId::derive(&r.holder()?, &r.resource()?)?,
    )?;
    for (family, k, mode) in r.targets()? {
        snap.line(
            &format!("row target {family}"),
            &format!("{} {}", k.encoded, json!(mode)),
        );
    }
    for target in targets(r)? {
        snap.wire("target", &target)?;
    }
    Ok(())
}

fn accepted(snap: &mut Snapshot, title: &str, parts: (bool, bool)) -> TestResult<(Walk, Row)> {
    let (part, returned) = parts;
    let mut before = Vec::new();
    let d = defs()?;
    let r = Row::parse(admission(&d, parts)?, 1)?;
    if part || returned {
        before.push((r.key(CENSUS)?, json!(3)));
    }
    if let Some(lot) = r.part_lot()? {
        before.push((lot, json!({"id":5,"endpoint":8,"quantity":1})));
    }
    let mut w = Walk::new(before)?;
    snap.section(&format!("bay refit: {title}: admission row"));
    row_lines(snap, &r)?;
    let mut writes: Vec<Write> = retained(&r)?
        .into_iter()
        .filter(|(k, _)| k.family.as_str() != LOT && k.family.as_str() != CENSUS)
        .map(|(k, v)| (k, Some(v)))
        .collect();
    writes.push((
        key(DEPLOYMENT, "registry".into())?,
        Some(json!({"accepted":1})),
    ));
    if part {
        writes.push((r.key(PART_HEAD)?, Some(json!({"order":1}))));
        writes.push((key(PART_CENSUS, "registry".into())?, Some(json!(1))));
    }
    writes.push((
        key(
            "industry_capacity_index",
            parsed::<crate::ReservationId>(&r.raw["capacity"])?
                .0
                .to_hex(),
        )?,
        Some(json!(1)),
    ));
    let funding = || -> TestResult<Changes> {
        Ok(Changes {
            funding: Some(reservation(&r)?),
            ..Changes::default()
        })
    };
    w.rig.refuse(
        snap,
        "accept as the advance action",
        &writes,
        &funding()?,
        &identity(&w.d, 1, op(1)),
    )?;
    w.rig.refuse(
        snap,
        "accept without the reservation",
        &writes,
        &Changes::default(),
        &identity(&w.d, 0, op(1)),
    )?;
    let mut extra = writes.clone();
    extra.push((key(JOB, "ff".repeat(32))?, Some(json!(1))));
    w.rig.refuse(
        snap,
        "accept with an extra write",
        &extra,
        &funding()?,
        &identity(&w.d, 0, op(1)),
    )?;
    let changes = funding()?;
    let accept = identity(&w.d, 0, op(1));
    w.rig.accept(
        snap,
        &format!("{title}: accepted"),
        writes,
        changes,
        &accept,
    )?;
    Ok((w, r))
}

fn advance(snap: &mut Snapshot, w: &mut Walk, title: &str, r: &Row) -> TestResult<Row> {
    let raw = advanced(&r.raw)?;
    let n = Row::parse(raw.clone(), 1)?;
    let cursor: crate::DurableSchedule = parsed(&r.raw["progress"]["cursor"])?;
    let crate::ScheduleEvaluation::Advance(planned) = cursor.plan_due(
        cursor.revision(),
        &stamp(7200),
        crate::ScheduleDriveCause::Live,
    )?
    else {
        return Err("not due".into());
    };
    let ingress = crate::scheduled_action_ingress(&w.d.actions[1], &planned, 0)
        .map_err(|error| error.to_string())?;
    let writes: Vec<Write> = vec![
        (order_key(1)?, Some(raw.clone())),
        (n.key(SCHEDULE)?, Some(raw["progress"]["cursor"].clone())),
    ];
    let changes = || -> TestResult<Changes> {
        Ok(Changes {
            progress: Some(industry_id(r.accepted)?),
            ..Changes::default()
        })
    };
    w.rig.refuse(
        snap,
        "advance by the requester, not the scheduler",
        &writes,
        &changes()?,
        &identity(&w.d, 1, op(2)),
    )?;
    let scheduler = RegisteredTxnIdentity {
        principal: ingress.principal().clone(),
        ..identity(&w.d, 1, op(2))
    };
    w.rig.refuse(
        snap,
        "advance without the progress marker",
        &writes,
        &Changes::default(),
        &scheduler,
    )?;
    w.rig.accept(
        snap,
        &format!("{title}: advanced"),
        writes,
        changes()?,
        &scheduler,
    )?;
    Ok(n)
}

fn release(w: &Walk, r: &Row) -> TestResult<(Value, Value)> {
    let s = w.rig.store()?;
    let scalar: BoundedScalar = parsed(&required(&view(&s), &r.key(SLOTS)?)?)?;
    let book: ReservationBook = parsed(&required(&view(&s), &r.key(BOOK)?)?)?;
    let released = book.checked_release(book.revision(), scalar, parsed(&r.raw["capacity"])?, 1)?;
    Ok((json!(scalar), json!(released)))
}

fn install(snap: &mut Snapshot, w: &mut Walk, title: &str, r: &Row) -> TestResult {
    let (part, returned) = (r.part()?.is_some(), r.returned()?.is_some());
    let refit = json!({"object":99,"from_revision":3,"to_revision":4,
        "before":r.offer["before"],"after":r.offer["target"],
        "remove":r.offer["remove"],"install":r.offer["install"],
        "removed":if returned {json!({"component":"cannon"})} else {Value::Null},
        "inserted":if part {json!({"component":"laser"})} else {Value::Null}});
    let refit_digest = digest("industry.bay_refit.refit.v1", &refit)?;
    let mut next = r.raw.clone();
    next["terminal"] = json!({"commit":op(3),"action":w.d.actions[2],"journal_seq":20,"posted_at":500,"at":stamp(7300),
        "result":{"outcome":"installed","refit":refit_digest}});
    next["installation"] = json!({"refit":refit,"participants":[w.d.participants[1],w.d.participants[2]],"at_utc":500});
    let n = Row::parse(next.clone(), 1)?;
    let (scalar, released) = release(w, r)?;
    let writes = |census: u64, back: bool| -> TestResult<Vec<Write>> {
        let mut out: Vec<Write> = vec![
            (order_key(1)?, Some(next.clone())),
            (n.key(HULL)?, Some(history::hull(&n, true)?)),
            (n.key(SLOTS)?, Some(scalar.clone())),
            (n.key(BOOK)?, Some(released.clone())),
            (n.key(SCHEDULE)?, None),
            (n.key(JOB)?, None),
        ];
        if let Some(lot) = n.part()? {
            out.push((n.key(PART)?, Some(history::part(&n, lot)?)));
            out.push((n.part_lot()?.ok_or("lot")?, None));
        }
        if back {
            out.push((
                n.returned_lot()?.ok_or("returned")?,
                Some(json!({"id":6,"endpoint":8,"quantity":1})),
            ));
        }
        if part || returned {
            out.push((n.key(CENSUS)?, Some(json!(census))));
        }
        Ok(out)
    };
    let census = 3 - u64::from(part) + u64::from(returned);
    let changes = || -> TestResult<Changes> {
        Ok(Changes {
            terminal: Some(industry_id(r.accepted)?),
            ..Changes::default()
        })
    };
    let actor = identity(&w.d, 2, op(3));
    w.rig.refuse(
        snap,
        "install with the stores census off by one",
        &writes(census + 1, returned)?,
        &changes()?,
        &actor,
    )?;
    if returned {
        w.rig.refuse(
            snap,
            "install that keeps the part taken out",
            &writes(census - 1, false)?,
            &changes()?,
            &actor,
        )?;
    }
    w.rig.refuse(
        snap,
        "install as the cancel action",
        &writes(census, returned)?,
        &changes()?,
        &identity(&w.d, 3, op(3)),
    )?;
    snap.section(&format!("bay refit: {title}: installed row"));
    snap.hex("refit digest", refit_digest.as_bytes());
    w.rig.accept(
        snap,
        &format!("{title}: installed"),
        writes(census, returned)?,
        changes()?,
        &actor,
    )
}

fn cancel(snap: &mut Snapshot, w: &mut Walk, title: &str, r: &Row) -> TestResult {
    let mut next = r.raw.clone();
    next["terminal"] = json!({"commit":op(4),"action":w.d.actions[3],"journal_seq":20,"posted_at":500,"at":stamp(7300),
        "result":{"outcome":"cancelled"}});
    let n = Row::parse(next.clone(), 1)?;
    let (scalar, released) = release(w, r)?;
    let mut writes: Vec<Write> = vec![
        (order_key(1)?, Some(next)),
        (n.key(HULL)?, Some(history::hull(&n, true)?)),
        (n.key(SLOTS)?, Some(scalar)),
        (n.key(BOOK)?, Some(released)),
        (n.key(SCHEDULE)?, None),
        (n.key(JOB)?, None),
    ];
    if let Some(lot) = n.part()? {
        writes.push((n.key(PART)?, Some(history::part(&n, lot)?)));
    }
    let changes = || -> TestResult<Changes> {
        Ok(Changes {
            terminal: Some(industry_id(r.accepted)?),
            ..Changes::default()
        })
    };
    w.rig.refuse(
        snap,
        "cancel as the install action",
        &writes,
        &changes()?,
        &identity(&w.d, 2, op(4)),
    )?;
    let mut held = writes.clone();
    held.retain(|(k, _)| k.family.as_str() != SCHEDULE);
    w.rig.refuse(
        snap,
        "cancel that keeps the schedule",
        &held,
        &changes()?,
        &identity(&w.d, 3, op(4)),
    )?;
    w.rig.accept(
        snap,
        &format!("{title}: cancelled"),
        writes,
        changes()?,
        &identity(&w.d, 3, op(4)),
    )
}

fn identity_section(snap: &mut Snapshot) -> TestResult {
    snap.section("bay refit: identity");
    for (index, id) in ACTIONS.iter().enumerate() {
        snap.line(&format!("action id {index}"), id);
    }
    for (owner, name) in FAMILY_NAMES {
        snap.line("family name", &format!("{owner} {name}"));
        snap.wire(
            &format!("descriptor {name}"),
            &descriptor(name)?.canonical_value(),
        )?;
        snap.wire(
            &format!("public descriptor {name}"),
            &family_descriptor(name)?.canonical_value(),
        )?;
    }
    for (owner, name, ceiling) in FAMILIES {
        snap.line("reserved family", &format!("{owner} {name} {ceiling}"));
    }
    snap.line("core hull family", CORE_HULL);
    for (name, value) in [
        ("order bytes", ORDER_BYTES),
        ("order ceiling", ORDER_CEILING),
        ("maximum work units", usize::try_from(MAX_WORK_UNITS)?),
        ("cadence", usize::try_from(CADENCE)?),
        ("reservation seconds", usize::try_from(RESERVATION_SECONDS)?),
    ] {
        snap.line(name, &value.to_string());
    }
    let d = defs()?;
    snap.wire("policy", &d.policy)?;
    snap.wire("participants", &d.participants)?;
    for index in 0..4 {
        let descriptor = action_descriptor(&d.policy, index)?;
        snap.wire(&format!("action payload {index}"), descriptor.payload())?;
        snap.wire(
            &format!("action references {index}"),
            descriptor.references(),
        )?;
        snap.wire(
            &format!("action reference {index}"),
            &action_reference(&d.policy, index)?,
        )?;
    }
    for index in [2, 3] {
        snap.wire(
            &format!("closed action reference {index}"),
            &action_reference(&Value::Null, index)?,
        )?;
    }
    for action in ["initialize_bay_refit", "offer_bay_refit"] {
        let descriptor = owner_action_descriptor(&d.policy, action)?;
        snap.wire(
            &format!("owner action payload {action}"),
            descriptor.payload(),
        )?;
        snap.wire(
            &format!("owner action reference {action}"),
            &DefinitionRegistryBuilder::new().register(owner_id("industry")?, descriptor)?,
        )?;
    }
    let reg = registry(&d)?;
    let protocol = reg.bay_refit.as_ref().ok_or("protocol")?;
    snap.wire("protocol", protocol)?;
    protocol.validate_families(&reg)?;
    snap.line("families", "valid");
    let industry = owner_id("industry")?;
    let [a, b, c] = d.participants.clone();
    let [w, x, y, z] = d.actions.clone();
    let refusals: [(&str, Result<Protocol, ActionStateError>); 4] = [
        (
            "declaration by another owner",
            Protocol::new(
                &owner_id("tacsim")?,
                d.participants.clone(),
                d.actions.clone(),
            ),
        ),
        (
            "declaration with swapped participants",
            Protocol::new(&industry, [b, a, c], d.actions.clone()),
        ),
        (
            "declaration with reordered actions",
            Protocol::new(&industry, d.participants.clone(), [x, w, y, z]),
        ),
        (
            "declaration with a policy-bound install",
            Protocol::new(
                &industry,
                d.participants.clone(),
                [
                    d.actions[0].clone(),
                    d.actions[1].clone(),
                    action_reference(&d.policy, 1)?,
                    d.actions[3].clone(),
                ],
            ),
        ),
    ];
    for (label, result) in refusals {
        kit::refusal(snap, label, result.map_err(Into::into))?;
    }
    kit::refusal(
        snap,
        "an unknown owner action",
        owner_action_descriptor(&d.policy, "rush_bay_refit").map_err(Into::into),
    )?;
    let mut policy = d.policy.clone();
    policy["facilities"] = json!([40, 40]);
    kit::refusal(
        snap,
        "a policy with a repeated bay",
        protocol::validate_policy(&policy).map_err(Into::into),
    )?;
    policy = d.policy.clone();
    policy["extra"] = json!(1);
    kit::refusal(
        snap,
        "a policy with an extra field",
        protocol::validate_policy(&policy).map_err(Into::into),
    )
}

#[test]
fn ct0_refit_bay_walk() -> TestResult {
    let mut snap = Snapshot::new();
    identity_section(&mut snap)?;

    let title = "part in and out, installed";
    let (mut w, r) = accepted(&mut snap, title, (true, true))?;
    let held = Walk::new(vec![
        (r.key(CENSUS)?, json!(3)),
        (
            r.part_lot()?.ok_or("lot")?,
            json!({"id":5,"endpoint":8,"quantity":1}),
        ),
        (
            r.key(HULL)?,
            json!({"format":1,"hull":99,"operation":op(30),"pending":9}),
        ),
    ])?;
    let mut writes: Vec<Write> = Vec::new();
    for (k, v) in retained(&r)? {
        if k.family.as_str() != LOT && k.family.as_str() != CENSUS {
            writes.push((k, Some(v)));
        }
    }
    writes.push((
        key(DEPLOYMENT, "registry".into())?,
        Some(json!({"accepted":1})),
    ));
    writes.push((r.key(PART_HEAD)?, Some(json!({"order":1}))));
    writes.push((key(PART_CENSUS, "registry".into())?, Some(json!(1))));
    writes.push((
        key(
            "industry_capacity_index",
            parsed::<crate::ReservationId>(&r.raw["capacity"])?
                .0
                .to_hex(),
        )?,
        Some(json!(1)),
    ));
    held.rig.refuse(
        &mut snap,
        "accept of a hull already in a bay",
        &writes,
        &Changes {
            funding: Some(reservation(&r)?),
            ..Changes::default()
        },
        &identity(&held.d, 0, op(1)),
    )?;
    let n = advance(&mut snap, &mut w, title, &r)?;
    install(&mut snap, &mut w, title, &n)?;

    let title = "part in and out, cancelled at once";
    let (mut w, r) = accepted(&mut snap, title, (true, true))?;
    cancel(&mut snap, &mut w, title, &r)?;

    let title = "part in and out, cancelled after progress";
    let (mut w, r) = accepted(&mut snap, title, (true, true))?;
    let n = advance(&mut snap, &mut w, title, &r)?;
    cancel(&mut snap, &mut w, title, &n)?;

    let title = "part out only, installed";
    let (mut w, r) = accepted(&mut snap, title, (false, true))?;
    let n = advance(&mut snap, &mut w, title, &r)?;
    install(&mut snap, &mut w, title, &n)?;

    let title = "part in only, installed";
    let (mut w, r) = accepted(&mut snap, title, (true, false))?;
    let n = advance(&mut snap, &mut w, title, &r)?;
    install(&mut snap, &mut w, title, &n)?;

    snap.check(env!("CARGO_MANIFEST_DIR"), "tests/ct0/refit_bay.snap")?;
    Ok(())
}
