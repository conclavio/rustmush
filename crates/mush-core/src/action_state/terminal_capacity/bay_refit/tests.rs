//! Doc 31 cut UD.4b: the bay refit protocol over inert typed rows. They grant
//! no payment, part or Tactics authority; the owner integration tests in the
//! Industry, Economy and Inventory crates exercise those images.
use super::*;
use crate::{DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder};
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
fn op(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
fn reference(o: &str, k: &str, id: &str) -> TestResult<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(
        owner_id(o)?,
        DefinitionDescriptor::new(
            DefinitionKind::new(k)?,
            DefinitionId::new(id)?,
            1,
            json!({"fixture": id}),
        )?,
    )?)
}
fn stamp(at: i64) -> ClockStamp {
    ClockStamp {
        clock: crate::ClockRef::WorldIc { generation: 1 },
        at,
    }
}
const OWNERS: [&str; 6] = ["industry", "economy", "inventory", "tactics", "sim", "org"];
/// The families the fixture registers: the protocol's, the ones Accept also
/// writes, and the shared capacity index.
const EXTRA: [&str; 4] = [
    DEPLOYMENT,
    PART_HEAD,
    PART_CENSUS,
    "industry_capacity_index",
];
struct Defs {
    participants: [DefinitionRef; 3],
    actions: [DefinitionRef; 4],
    policy: Value,
}
fn defs() -> TestResult<Defs> {
    let participants = [
        reference("industry", "owner_participant", "bay_refit")?,
        reference("economy", "owner_participant", "initial_filing")?,
        reference("tactics", "owner_participant", "component_refit")?,
    ];
    let policy =
        json!({"format":1,"recipe":reference("industry","recipe","bay_refit")?,"facilities":[40]});
    let actions = [
        action_reference(&policy, 0)?,
        action_reference(&policy, 1)?,
        action_reference(&policy, 2)?,
        action_reference(&policy, 3)?,
    ];
    Ok(Defs {
        participants,
        actions,
        policy,
    })
}
fn registry(d: &Defs) -> TestResult<StateFamilyRegistry> {
    let mut builder = StateFamilyRegistryBuilder::new();
    for (o, f, _) in FAMILIES {
        builder.register(
            owner_id(o)?,
            descriptor(f)?,
            |k: &String| Ok(k.clone()),
            |v: &Value| Ok(v.clone()),
            |_: &Value| Ok(()),
            |v: &Value| Ok(v.clone()),
        )?;
    }
    for f in EXTRA {
        let d = if f == "industry_capacity_index" {
            let c = |s: &str| CanonicalId::new(format!("industry.{f}.{s}.v1"));
            StateFamilyDescriptor::new(
                StateFamilyId::new(f)?,
                c("key")?,
                c("value")?,
                1,
                c("decode")?,
                c("validate")?,
                c("canonical")?,
            )?
        } else {
            descriptor(f)?
        };
        builder.register(
            owner_id(family_owner(f))?,
            d,
            |k: &String| Ok(k.clone()),
            |v: &Value| Ok(v.clone()),
            |_: &Value| Ok(()),
            |v: &Value| Ok(v.clone()),
        )?;
    }
    builder.delegate(owner_id("inventory")?, owner_id("industry")?)?;
    builder.bay_refit_capacity(
        &owner_id("industry")?,
        d.participants.clone(),
        d.actions.clone(),
    )?;
    let owners = OWNERS
        .iter()
        .map(|o| owner_id(o))
        .collect::<Result<_, _>>()?;
    let readiness = crate::mechanics::MechanicalOwnerReadiness::for_owners(&owners);
    Ok(builder.freeze_for_owners_with_readiness(&owners, readiness)?)
}
fn envelope(registry: &StateFamilyRegistry, k: &Key, v: &Value) -> TestResult<(StateKey, Value)> {
    let k = k.resolve(registry)?;
    let d = registry
        .entries
        .get(&(k.owner.clone(), k.family.clone()))
        .ok_or("descriptor")?
        .descriptor();
    let e = json!({"family":{"owner":k.owner,"id":k.family},"key":k.encoded,"schema":{"id":d.value_schema(),"version":d.schema_version()},"value":v});
    Ok((k, e))
}
fn put(registry: &StateFamilyRegistry, k: &Key, v: Value) -> TestResult<(StateKey, TxnMutation)> {
    let (k, document) = envelope(registry, k, &v)?;
    Ok((
        k,
        TxnMutation::Put {
            value: Box::new(v),
            canonical: CanonicalStateBytes(
                canonical_json_bytes_with_limit(&document, 262144)?.into(),
            ),
            document,
        },
    ))
}
fn del(registry: &StateFamilyRegistry, k: &Key) -> TestResult<(StateKey, TxnMutation)> {
    Ok((k.resolve(registry)?, TxnMutation::Delete))
}
/// A bay order: the line tank's cannon out of the turret, a laser in from
/// lot 5 in stores 8, the cannon back to stores as lot 6.
fn admission(d: &Defs, parts: (bool, bool)) -> TestResult<Value> {
    let (part, returned) = parts;
    let offer = json!({"operation":op(20),"requester":7,"hull":99,"unit_revision":3,
        "before":reference("tacsim","unit","line_tank")?,"target":reference("tacsim","unit","line_tank_laser")?,
        "remove":if returned {json!("cannon")} else {Value::Null},
        "install":if part {json!("laser")} else {Value::Null},
        "facility":40,"stores":8,
        "part":if part {json!(5)} else {Value::Null},
        "returned":if returned {json!(6)} else {Value::Null},
        "recipe":{"work_units":2},"policy":d.policy});
    let intent = json!({"order":1,"offer":offer,"payer":1,"accepted_at":stamp(0),"expires_at":stamp(604800)});
    let id = ScheduleId::derive(
        &owner_id("industry")?,
        &CanonicalId::new("bay.0000000000000001")?,
    )?;
    let cursor = crate::DurableSchedule::new(crate::DurableScheduleDraft {
        id,
        owner: owner_id("industry")?,
        clock: stamp(0).clock,
        next_due: stamp(3600),
        cadence: Some(3600),
        generation: 1,
        next_ordinal: 0,
        restart: crate::ScheduleRestartPolicy::Resume,
        catch_up_order: crate::ScheduleCatchUpOrder::OldestFirst,
        max_catch_up: 256,
    })?;
    let progress = json!({"order":1,"production":{"schedule":id,"started":stamp(0),"clock_policy":"bay.refit","total_units":2,"progress_units":0},"cursor":cursor,"advances":[]});
    let mut raw = json!({"format":1,"sequence":1,"accepted":op(1),"action":d.actions[0],"intent":intent,
        "funding":{"commit":op(1),"action":d.actions[0],"journal_seq":10},
        "capacity":crate::ReservationId(crate::CanonicalHash::from_bytes([7;32])),
        "progress":progress,"terminal":null,"installation":null});
    let r = Row::parse(raw.clone(), 1)?;
    raw["capacity"] = json!(crate::ReservationId::derive(&r.holder()?, &r.resource()?)?);
    Ok(raw)
}
/// The row advanced through both of its work units.
fn advanced(raw: &Value) -> TestResult<Value> {
    let mut raw = raw.clone();
    let cursor: crate::DurableSchedule = parsed(&raw["progress"]["cursor"])?;
    let crate::ScheduleEvaluation::Advance(planned) = cursor.plan_due(
        cursor.revision(),
        &stamp(7200),
        crate::ScheduleDriveCause::Live,
    )?
    else {
        return Err("not due".into());
    };
    raw["progress"]["cursor"] = json!(planned.post_image());
    raw["progress"]["production"]["progress_units"] = json!(2);
    raw["progress"]["advances"] = json!([{"commit":op(2),"observed":stamp(7200),"cause":"live","occurrences":planned.occurrences().len()}]);
    Ok(raw)
}
/// The book holding the order's slot, and the scalar it is held under.
fn slot(r: &Row) -> TestResult<(BoundedScalar, ReservationBook)> {
    let resource = r.resource()?;
    let scalar = BoundedScalar::new(0, 2, 2, 1)?;
    let book = ReservationBook::new(resource.clone(), scalar)?;
    let held = crate::ReservationRecord::held(
        parsed(field(&r.raw, "capacity")?)?,
        resource,
        r.holder()?,
        1,
        stamp(604800),
    )?;
    let book = book.checked_hold(book.revision(), scalar, held, &stamp(0))?;
    Ok((scalar, book))
}
/// Every record a retained order has, with the stores' lot census at 3.
fn retained(r: &Row) -> TestResult<Vec<(Key, Value)>> {
    let (scalar, book) = slot(r)?;
    let mut out = vec![
        (order_key(1)?, r.raw.clone()),
        (r.key(HULL)?, history::hull(r, false)?),
        (r.key(SLOTS)?, json!(scalar)),
        (r.key(BOOK)?, json!(book)),
        (r.key(SCHEDULE)?, r.raw["progress"]["cursor"].clone()),
        (r.key(JOB)?, json!(1)),
        (r.key(CENSUS)?, json!(3)),
    ];
    if let Some(lot) = r.part()? {
        out.push((r.key(PART)?, history::part(r, lot)?));
        out.push((
            r.part_lot()?.ok_or("lot")?,
            json!({"id":lot,"endpoint":8,"quantity":1}),
        ));
    }
    Ok(out)
}
fn store(registry: &StateFamilyRegistry, rows: &[(Key, Value)]) -> TestResult<ActionStateStore> {
    let mut store = ActionStateStore::new(registry.clone());
    for (k, v) in rows {
        let (k, e) = envelope(registry, k, v)?;
        store.records.insert(
            k,
            StoredStateRecord {
                value: Box::new(v.clone()),
                canonical: CanonicalStateBytes(canonical_json_bytes_with_limit(&e, 262144)?.into()),
            },
        );
    }
    Ok(store)
}
fn reservation(r: &Row) -> TestResult<Reservation> {
    Ok(Reservation {
        format: 1,
        funding: r.accepted,
        effect: r.action.clone(),
        sponsor: order_key(r.order)?,
        targets: targets(r)?,
        protocol_fingerprint: Some(r.fingerprint),
    })
}
fn view(store: &ActionStateStore) -> View<'_> {
    View {
        registry: &store.registry,
        records: &store.records,
        mutations: None,
    }
}
fn identity(d: &Defs, index: usize, commit: CommitToken) -> RegisteredTxnIdentity {
    RegisteredTxnIdentity {
        action: d.actions[index].clone(),
        ordinal: 0,
        commit,
        actor: ObjId(7),
        principal: PrincipalId::Object { id: ObjId(7) },
    }
}

#[test]
fn ud4b_the_protocol_pins_its_participants_and_actions() -> TestResult {
    let d = defs()?;
    let industry = owner_id("industry")?;
    Protocol::new(&industry, d.participants.clone(), d.actions.clone())?;
    let [a, b, c] = d.participants.clone();
    assert!(
        Protocol::new(
            &industry,
            [b.clone(), a.clone(), c.clone()],
            d.actions.clone()
        )
        .is_err()
    );
    let core = reference("tactics", "owner_participant", "component_replacement")?;
    assert!(
        Protocol::new(&industry, [a, b, core], d.actions.clone()).is_err(),
        "the core refit's swap is not the bay's"
    );
    let [w, x, y, z] = d.actions.clone();
    assert!(Protocol::new(&industry, d.participants.clone(), [x, w, y, z]).is_err());
    assert!(Protocol::new(&owner_id("tacsim")?, d.participants, d.actions).is_err());
    Ok(())
}

#[test]
fn ud4b_a_retained_order_verifies_and_reserves_the_returned_lot() -> TestResult {
    let d = defs()?;
    let reg = registry(&d)?;
    let r = Row::parse(admission(&d, (true, true))?, 1)?;
    let s = store(&reg, &retained(&r)?)?;
    let res = reservation(&r)?;
    verify(Phase::Funding, &res, &view(&s))?;
    verify(Phase::Retained, &res, &view(&s))?;
    assert!(verify(Phase::Terminal, &res, &view(&s)).is_err());
    assert_eq!(
        res.targets.len(),
        10,
        "six fixed, the part, its lot, the return, the census"
    );
    let created: Vec<_> = res
        .targets
        .iter()
        .filter(|t| t.mode == SlotMode::Create)
        .collect();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].key, r.returned_lot()?.ok_or("returned")?);
    // A pod coming off moves no part in: no part, no held lot.
    let off = Row::parse(admission(&d, (false, true))?, 1)?;
    assert_eq!(targets(&off)?.len(), 8);
    Ok(())
}

#[test]
fn ud4b_what_comes_out_and_goes_in_matches_the_parts() -> TestResult {
    let d = defs()?;
    let mut raw = admission(&d, (true, true))?;
    raw["intent"]["offer"]["part"] = Value::Null;
    assert!(Row::parse(raw, 1).is_err(), "a part fitted from no lot");
    let mut raw = admission(&d, (true, true))?;
    raw["intent"]["offer"]["returned"] = Value::Null;
    assert!(Row::parse(raw, 1).is_err(), "a part taken out to nowhere");
    let mut raw = admission(&d, (true, true))?;
    raw["intent"]["offer"]["facility"] = json!(41);
    assert!(
        Row::parse(raw, 1).is_err(),
        "a bay the policy does not admit"
    );
    let mut raw = admission(&d, (true, true))?;
    raw["intent"]["offer"]["returned"] = json!(5);
    assert!(Row::parse(raw, 1).is_err(), "the return is not the part");
    assert!(admission(&d, (false, false)).is_err(), "nothing moves");
    Ok(())
}

#[test]
fn ud4b_a_cold_boot_audits_the_order_and_its_counterparts() -> TestResult {
    let d = defs()?;
    let reg = registry(&d)?;
    let r = Row::parse(admission(&d, (true, true))?, 1)?;
    let rows = retained(&r)?;
    let saved = Arc::new(encode(reservation(&r)?)?);
    let reservations = BTreeMap::from([(saved.id(), saved)]);
    let s = store(&reg, &rows)?;
    audit_boot(&view(&s), &reservations)?;
    assert!(audit_boot(&view(&s), &BTreeMap::new()).is_err());
    for family in [PART, HULL, SCHEDULE, JOB] {
        let kept: Vec<_> = rows
            .iter()
            .filter(|(k, _)| k.family.as_str() != family)
            .cloned()
            .collect();
        assert!(
            audit_boot(&view(&store(&reg, &kept)?), &reservations).is_err(),
            "{family} missing"
        );
    }
    let mut orphan = rows.clone();
    orphan.push((key(PART, format!("{:016x}", 9))?, json!({"format":1})));
    assert!(audit_boot(&view(&store(&reg, &orphan)?), &reservations).is_err());
    Ok(())
}

#[test]
fn ud4b_accept_writes_exactly_its_set() -> TestResult {
    let d = defs()?;
    let reg = registry(&d)?;
    let r = Row::parse(admission(&d, (true, true))?, 1)?;
    let lot = (
        r.part_lot()?.ok_or("lot")?,
        json!({"id":5,"endpoint":8,"quantity":1}),
    );
    let s = store(&reg, &[lot.clone(), (r.key(CENSUS)?, json!(3))])?;
    let build = || -> TestResult<BTreeMap<StateKey, TxnMutation>> {
        let mut writes = BTreeMap::new();
        for (k, v) in retained(&r)? {
            if k.family.as_str() != LOT && k.family.as_str() != CENSUS {
                let (k, m) = put(&reg, &k, v)?;
                writes.insert(k, m);
            }
        }
        for (k, v) in [
            (key(DEPLOYMENT, "registry".into())?, json!({"accepted":1})),
            (r.key(PART_HEAD)?, json!({"order":1})),
            (key(PART_CENSUS, "registry".into())?, json!(1)),
            (
                key(
                    "industry_capacity_index",
                    parsed::<crate::ReservationId>(&r.raw["capacity"])?
                        .0
                        .to_hex(),
                )?,
                json!(1),
            ),
        ] {
            let (k, m) = put(&reg, &k, v)?;
            writes.insert(k, m);
        }
        Ok(writes)
    };
    let writes = build()?;
    let changes = Changes {
        funding: Some(reservation(&r)?),
        ..Changes::default()
    };
    validate_changes(&s, &writes, &changes, Some(&identity(&d, 0, op(1))))?;
    assert!(
        validate_changes(&s, &writes, &changes, Some(&identity(&d, 1, op(1)))).is_err(),
        "not the admission action"
    );
    let mut extra = build()?;
    let (k, m) = put(&reg, &lot.0, lot.1.clone())?;
    extra.insert(k, m);
    assert!(
        validate_changes(&s, &extra, &changes, Some(&identity(&d, 0, op(1)))).is_err(),
        "the lot itself is not written at Accept"
    );
    let held = store(
        &reg,
        &[
            lot,
            (r.key(CENSUS)?, json!(3)),
            (
                r.key(HULL)?,
                json!({"format":1,"hull":99,"operation":op(30),"pending":9}),
            ),
        ],
    )?;
    assert!(
        validate_changes(&held, &writes, &changes, Some(&identity(&d, 0, op(1)))).is_err(),
        "a hull already in a bay"
    );
    Ok(())
}

#[test]
fn ud4b_install_consumes_the_part_and_returns_the_displaced_one() -> TestResult {
    let d = defs()?;
    let reg = registry(&d)?;
    let r = Row::parse(advanced(&admission(&d, (true, true))?)?, 1)?;
    let mut s = store(&reg, &retained(&r)?)?;
    let saved = Arc::new(encode(reservation(&r)?)?);
    s.capacity = Arc::new(CapacityState {
        records: BTreeMap::from([(saved.id(), saved)]),
        ..CapacityState::default()
    });
    let refit = json!({"object":99,"from_revision":3,"to_revision":4,
        "before":r.offer["before"],"after":r.offer["target"],"remove":"cannon","install":"laser",
        "removed":{"component":"cannon"},"inserted":{"component":"laser"}});
    let mut next = r.raw.clone();
    next["terminal"] = json!({"commit":op(3),"action":d.actions[2],"journal_seq":20,"posted_at":500,"at":stamp(7300),
        "result":{"outcome":"installed","refit":digest("industry.bay_refit.refit.v1",&refit)?}});
    next["installation"] =
        json!({"refit":refit,"participants":[d.participants[1],d.participants[2]],"at_utc":500});
    let n = Row::parse(next.clone(), 1)?;
    let (scalar, book) = slot(&r)?;
    let released = book.checked_release(book.revision(), scalar, parsed(&r.raw["capacity"])?, 1)?;
    let writes = |census: u64, returned: bool| -> TestResult<BTreeMap<StateKey, TxnMutation>> {
        let mut w = vec![
            put(&reg, &order_key(1)?, next.clone())?,
            put(&reg, &n.key(HULL)?, history::hull(&n, true)?)?,
            put(&reg, &n.key(SLOTS)?, json!(scalar))?,
            put(&reg, &n.key(BOOK)?, json!(released))?,
            del(&reg, &n.key(SCHEDULE)?)?,
            del(&reg, &n.key(JOB)?)?,
            put(&reg, &n.key(PART)?, history::part(&n, 5)?)?,
            del(&reg, &n.part_lot()?.ok_or("lot")?)?,
            put(&reg, &n.key(CENSUS)?, json!(census))?,
        ];
        if returned {
            w.push(put(
                &reg,
                &n.returned_lot()?.ok_or("returned")?,
                json!({"id":6,"endpoint":8,"quantity":1}),
            )?);
        }
        Ok(w.into_iter().collect())
    };
    let changes = Changes {
        terminal: Some(ReservationId(owner_id("industry")?, op(1).to_hex())),
        ..Changes::default()
    };
    let install = identity(&d, 2, op(3));
    validate_changes(&s, &writes(3, true)?, &changes, Some(&install))?;
    assert!(
        validate_changes(&s, &writes(2, true)?, &changes, Some(&install)).is_err(),
        "one out and one back leaves the census where it was"
    );
    assert!(
        validate_changes(&s, &writes(3, false)?, &changes, Some(&install)).is_err(),
        "the part taken out goes back to stores"
    );
    Ok(())
}
