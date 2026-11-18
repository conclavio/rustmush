#![cfg(test)]

//! Doc 46 CT.0: the core refit walk, pinned in `tests/ct0/refit_core.snap`. The core refit has
//! no test rig of its own; this one mirrors the bay refit's: inert typed rows driven through the
//! closed validator (`validate_changes`) for Accept, Advance, Install and Cancel, each accepted
//! write set applied and booted cold by `ct0_kit::Rig`.
//!
//! One shortcut, named: the protocol's family audit requires a registered core integration
//! declaration whose descriptors are private to that family, so the rig freezes the families
//! and then sets `core_refit` on the frozen registry. The audit's refusal without the
//! prerequisite is pinned in the identity section.
use super::*;
use crate::action_state::terminal_capacity::tests::ct0_kit::{self as kit, Write};
use crate::{DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder};
use mush_testkit::snapshot::Snapshot;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const OWNERS: [&str; 5] = ["industry", "economy", "tactics", "sim", "org"];
const EXTRA: [&str; 2] = ["industry_refit_deployment", "industry_capacity_index"];
const HOLD: i64 = crate::player_wait::REFIT_RESERVATION_SECONDS;

fn op(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
fn stamp(at: i64) -> ClockStamp {
    ClockStamp {
        clock: crate::ClockRef::WorldIc { generation: 1 },
        at,
    }
}
pub(super) fn reference(o: &str, k: &str, id: &str, version: u32) -> TestResult<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(
        OwnerId::new(o)?,
        DefinitionDescriptor::new(
            DefinitionKind::new(k)?,
            DefinitionId::new(id)?,
            version,
            json!({"fixture": id}),
        )?,
    )?)
}

pub(super) struct Defs {
    pub(super) participants: [DefinitionRef; 3],
    pub(super) actions: [DefinitionRef; 4],
    policy: Value,
    component: Value,
}

pub(super) fn defs() -> TestResult<Defs> {
    let participants = [
        reference("industry", "owner_participant", "core_production", 3)?,
        reference("economy", "owner_participant", "initial_filing", 1)?,
        reference("tactics", "owner_participant", "component_replacement", 1)?,
    ];
    let policy = json!({"recipe":reference("industry","recipe","core_refit",1)?,
        "fitted_units":[reference("tacsim","unit","line_tank",1)?,reference("tacsim","unit","line_tank_mk2",1)?,
            reference("tacsim","unit","line_tank_mk3",1)?],
        "factory":10});
    let component = json!("core");
    let actions = [
        action_reference(&policy, &component, 0)?,
        action_reference(&policy, &component, 1)?,
        action_reference(&policy, &component, 2)?,
        action_reference(&policy, &component, 3)?,
    ];
    Ok(Defs {
        participants,
        actions,
        policy,
        component,
    })
}

fn extra_descriptor(f: &str) -> TestResult<StateFamilyDescriptor> {
    let c = |s: &str| CanonicalId::new(format!("industry.{f}.{s}.v1"));
    Ok(StateFamilyDescriptor::new(
        StateFamilyId::new(f)?,
        c("key")?,
        c("value")?,
        1,
        c("decode")?,
        c("validate")?,
        c("canonical")?,
    )?)
}

/// The families frozen, then the protocol set on the frozen registry (see the module note).
fn registry(d: &Defs) -> TestResult<StateFamilyRegistry> {
    let mut builder = StateFamilyRegistryBuilder::new();
    let industry = owner_id()?;
    for (name, _) in FAMILIES {
        builder.register(
            industry.clone(),
            descriptor(name)?,
            |k: &String| Ok(k.clone()),
            |v: &Value| Ok(v.clone()),
            |_: &Value| Ok(()),
            |v: &Value| Ok(v.clone()),
        )?;
    }
    for f in EXTRA {
        builder.register(
            industry.clone(),
            extra_descriptor(f)?,
            |k: &String| Ok(k.clone()),
            |v: &Value| Ok(v.clone()),
            |_: &Value| Ok(()),
            |v: &Value| Ok(v.clone()),
        )?;
    }
    let owners = OWNERS
        .iter()
        .map(|o| OwnerId::new(*o))
        .collect::<Result<_, _>>()?;
    let readiness = crate::mechanics::MechanicalOwnerReadiness::for_owners(&owners);
    let mut reg = builder.freeze_for_owners_with_readiness(&owners, readiness)?;
    reg.core_refit = Some(Box::new(Protocol::new(
        &industry,
        d.participants.clone(),
        d.actions.clone(),
    )?));
    Ok(reg)
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

/// One order of the walk: its number, admission time, journal sequence, commits and parts.
pub(super) struct Order {
    pub(super) n: u64,
    pub(super) at: i64,
    pub(super) journal: u64,
    pub(super) byte: u8,
    pub(super) revision: u64,
    pub(super) incoming: u32,
    pub(super) displaced: Option<u32>,
}

impl Order {
    fn accepted(&self) -> CommitToken {
        op(self.byte)
    }
    fn advanced(&self) -> CommitToken {
        op(self.byte + 1)
    }
    fn terminal(&self) -> CommitToken {
        op(self.byte + 2)
    }
    fn id(&self) -> TestResult<ReservationId> {
        Ok(ReservationId(owner_id()?, self.accepted().to_hex()))
    }
}

/// The admission row and its slot hold, taken from the walk's current book when it has one.
fn admission(rig: &kit::Rig, d: &Defs, o: &Order) -> TestResult<(Row, Value, Value)> {
    let mut offer = json!({"operation":op(o.byte + 100),"requester":7,"hull":99,"unit_revision":o.revision,
        "before":reference("tacsim","unit","line_tank",1)?,"component":d.component,
        "incoming":o.incoming,"recipe":{"work_units":2},"policy":d.policy});
    if let Some(old) = o.displaced {
        offer["displaced"] = json!({"component":"core","installed":{"asset":old}});
    }
    let intent = json!({"order":o.n,"offer":offer,"payer":1,"accepted_at":stamp(o.at),"expires_at":stamp(o.at + HOLD)});
    let id = ScheduleId::derive(
        &owner_id()?,
        &CanonicalId::new(format!("refit.{:016x}", o.n))?,
    )?;
    let cursor = crate::DurableSchedule::new(crate::DurableScheduleDraft {
        id,
        owner: owner_id()?,
        clock: stamp(o.at).clock,
        next_due: stamp(o.at + 3600),
        cadence: Some(3600),
        generation: 1,
        next_ordinal: 0,
        restart: crate::ScheduleRestartPolicy::Resume,
        catch_up_order: crate::ScheduleCatchUpOrder::OldestFirst,
        max_catch_up: 256,
    })?;
    let progress = json!({"order":o.n,"production":{"schedule":id,"started":stamp(o.at),"clock_policy":"core.refit","total_units":2,"progress_units":0},"cursor":cursor,"advances":[]});
    let mut raw = json!({"format":1,"sequence":o.n,"accepted":o.accepted(),"action":d.actions[0],"intent":intent,
        "funding":{"commit":o.accepted(),"action":d.actions[0],"journal_seq":o.journal},
        "capacity":crate::ReservationId(crate::CanonicalHash::from_bytes([7;32])),
        "progress":progress,"terminal":null,"installation":null});
    let r = Row::parse(raw.clone(), o.n)?;
    let held_id = crate::ReservationId::derive(&r.holder()?, &r.resource()?)?;
    raw["capacity"] = json!(held_id);
    let r = Row::parse(raw, o.n)?;
    let scalar = BoundedScalar::new(0, 2, 2, 1)?;
    let book = match rig.get(&r.key(BOOK)?) {
        Ok(current) => parsed::<ReservationBook>(&current)?,
        Err(_) => ReservationBook::new(r.resource()?, scalar)?,
    };
    let held =
        crate::ReservationRecord::held(held_id, r.resource()?, r.holder()?, 1, stamp(o.at + HOLD))?;
    let book = book.checked_hold(book.revision(), scalar, held, &stamp(o.at))?;
    Ok((r, json!(scalar), json!(book)))
}

fn row_lines(snap: &mut Snapshot, r: &Row) -> TestResult {
    snap.hex("capacity fingerprint", r.fingerprint.as_bytes());
    snap.wire("resource", &r.resource()?)?;
    snap.wire("holder", &r.holder()?)?;
    for (family, k) in r.keys()? {
        snap.line(&format!("row key {family}"), &k.encoded);
    }
    for target in targets(r)? {
        snap.wire("target", &target)?;
    }
    Ok(())
}

pub(super) fn accept(
    snap: &mut Snapshot,
    rig: &mut kit::Rig,
    d: &Defs,
    title: &str,
    o: &Order,
) -> TestResult<Row> {
    let (r, scalar, book) = admission(rig, d, o)?;
    snap.section(&format!("core refit: {title}: admission row"));
    row_lines(snap, &r)?;
    let writes: Vec<Write> = vec![
        (order_key(o.n)?, Some(r.raw.clone())),
        (r.key(ASSET)?, Some(history::asset(&r, false)?)),
        (r.key(HULL)?, Some(history::hull(&r, false)?)),
        (r.key(SLOTS)?, Some(scalar)),
        (r.key(BOOK)?, Some(book)),
        (r.key(SCHEDULE)?, Some(r.raw["progress"]["cursor"].clone())),
        (r.key(JOB)?, Some(json!(o.n))),
        (
            key(EXTRA[0], "registry".into())?,
            Some(json!({"accepted":o.n})),
        ),
        (
            key(
                EXTRA[1],
                parsed::<crate::ReservationId>(&r.raw["capacity"])?
                    .0
                    .to_hex(),
            )?,
            Some(json!(1)),
        ),
    ];
    let funding = || -> TestResult<Changes> {
        Ok(Changes {
            funding: Some(Reservation {
                format: 1,
                funding: r.accepted,
                effect: r.action.clone(),
                sponsor: order_key(o.n)?,
                targets: targets(&r)?,
                protocol_fingerprint: Some(r.fingerprint),
            }),
            ..Changes::default()
        })
    };
    rig.refuse(
        snap,
        "accept without the reservation",
        &writes,
        &Changes::default(),
        &identity(d, 0, o.accepted()),
    )?;
    rig.refuse(
        snap,
        "accept by another actor",
        &writes,
        &funding()?,
        &RegisteredTxnIdentity {
            actor: ObjId(8),
            principal: PrincipalId::Object { id: ObjId(8) },
            ..identity(d, 0, o.accepted())
        },
    )?;
    let mut short = writes.clone();
    short.retain(|(k, _)| k.family.as_str() != EXTRA[0]);
    rig.refuse(
        snap,
        "accept without the deployment census",
        &short,
        &funding()?,
        &identity(d, 0, o.accepted()),
    )?;
    rig.accept(
        snap,
        &format!("{title}: accepted"),
        writes,
        funding()?,
        &identity(d, 0, o.accepted()),
    )?;
    Ok(r)
}

fn advance(
    snap: &mut Snapshot,
    rig: &mut kit::Rig,
    d: &Defs,
    title: &str,
    o: &Order,
    r: &Row,
) -> TestResult<Row> {
    let cursor: crate::DurableSchedule = parsed(&r.raw["progress"]["cursor"])?;
    let crate::ScheduleEvaluation::Advance(planned) = cursor.plan_due(
        cursor.revision(),
        &stamp(o.at + 7200),
        crate::ScheduleDriveCause::Live,
    )?
    else {
        return Err("not due".into());
    };
    let mut raw = r.raw.clone();
    raw["progress"]["cursor"] = json!(planned.post_image());
    raw["progress"]["production"]["progress_units"] = json!(2);
    raw["progress"]["advances"] = json!([{"commit":o.advanced(),"observed":stamp(o.at + 7200),"cause":"live","occurrences":planned.occurrences().len()}]);
    let n = Row::parse(raw.clone(), o.n)?;
    let ingress = crate::scheduled_action_ingress(&d.actions[1], &planned, 0)
        .map_err(|error| error.to_string())?;
    let writes: Vec<Write> = vec![
        (order_key(o.n)?, Some(raw.clone())),
        (n.key(SCHEDULE)?, Some(raw["progress"]["cursor"].clone())),
    ];
    let changes = || -> TestResult<Changes> {
        Ok(Changes {
            progress: Some(o.id()?),
            ..Changes::default()
        })
    };
    rig.refuse(
        snap,
        "advance by the requester, not the scheduler",
        &writes,
        &changes()?,
        &identity(d, 1, o.advanced()),
    )?;
    let scheduler = RegisteredTxnIdentity {
        principal: ingress.principal().clone(),
        ..identity(d, 1, o.advanced())
    };
    rig.accept(
        snap,
        &format!("{title}: advanced"),
        writes,
        changes()?,
        &scheduler,
    )?;
    Ok(n)
}

fn release(rig: &kit::Rig, r: &Row) -> TestResult<(Value, Value)> {
    let scalar: BoundedScalar = parsed(&rig.get(&r.key(SLOTS)?)?)?;
    let book: ReservationBook = parsed(&rig.get(&r.key(BOOK)?)?)?;
    let released = book.checked_release(book.revision(), scalar, parsed(&r.raw["capacity"])?, 1)?;
    Ok((json!(scalar), json!(released)))
}

fn terminal_changes(o: &Order) -> TestResult<Changes> {
    Ok(Changes {
        terminal: Some(o.id()?),
        ..Changes::default()
    })
}

fn install(
    snap: &mut Snapshot,
    rig: &mut kit::Rig,
    d: &Defs,
    title: &str,
    o: &Order,
    r: &Row,
) -> TestResult {
    let mut replacement = json!({"object":99,"from_revision":o.revision,"to_revision":o.revision + 1,
        "before":r.offer["before"],"component":r.offer["component"],
        "inserted":{"installed":{"asset":o.incoming}}});
    if let Some(displaced) = r.offer.get("displaced") {
        replacement["displaced"] = displaced.clone();
    }
    let posted = 500 + i64::try_from(o.n)?;
    let exchange = json!({"format":1,"order":o.n,"operation":o.terminal(),"action":d.actions[2],
        "requester":7,"hull":99,"unit_revision":o.revision,"incoming":o.incoming,"source":7,
        "outgoing":o.displaced,"return_to":o.displaced.map(|_| 7),"at_utc":posted,
        "participants":[d.participants[1],d.participants[2]]});
    let replacement_digest = digest("industry.core_refit.replacement.v1", &replacement)?;
    let receipt_digest = digest("industry.core_component_exchange.receipt.v1", &exchange)?;
    let mut next = r.raw.clone();
    next["terminal"] = json!({"commit":o.terminal(),"action":d.actions[2],"journal_seq":o.journal + 10,
        "posted_at":posted,"at":stamp(o.at + 7300),
        "result":{"outcome":"installed","replacement":replacement_digest}});
    next["installation"] =
        json!({"replacement":replacement,"core_receipt":receipt_digest,"at_utc":posted});
    let n = Row::parse(next.clone(), o.n)?;
    let (scalar, released) = release(rig, r)?;
    let mut writes: Vec<Write> = vec![
        (order_key(o.n)?, Some(next)),
        (n.key(ASSET)?, Some(history::asset(&n, true)?)),
        (n.key(HULL)?, Some(history::hull(&n, true)?)),
        (n.key(SLOTS)?, Some(scalar)),
        (n.key(BOOK)?, Some(released)),
        (n.key(SCHEDULE)?, None),
        (n.key(JOB)?, None),
        (n.key(EXCHANGE)?, Some(exchange.clone())),
    ];
    if let Some(old) = o.displaced {
        writes.push((key(ASSET, old.to_string())?, Some(history::detached(&n)?)));
        snap.section(&format!("core refit: {title}: displaced condition"));
        snap.hex(
            "displaced condition digest",
            digest("industry.core_refit.displaced.v1", &r.offer["displaced"])?.as_bytes(),
        );
    }
    let mut forged = writes.clone();
    if let Some((_, Some(value))) = forged
        .iter_mut()
        .find(|(k, _)| k.family.as_str() == EXCHANGE)
    {
        value["outgoing"] = json!(1234);
    }
    rig.refuse(
        snap,
        "install with a forged exchange receipt",
        &forged,
        &terminal_changes(o)?,
        &identity(d, 2, o.terminal()),
    )?;
    let mut kept = writes.clone();
    kept.retain(|(k, _)| k.family.as_str() != EXCHANGE);
    rig.refuse(
        snap,
        "install without the exchange receipt",
        &kept,
        &terminal_changes(o)?,
        &identity(d, 2, o.terminal()),
    )?;
    rig.refuse(
        snap,
        "install as the cancel action",
        &writes,
        &terminal_changes(o)?,
        &identity(d, 3, o.terminal()),
    )?;
    snap.section(&format!("core refit: {title}: installed row"));
    snap.hex("replacement digest", replacement_digest.as_bytes());
    snap.hex("core receipt digest", receipt_digest.as_bytes());
    rig.accept(
        snap,
        &format!("{title}: installed"),
        writes,
        terminal_changes(o)?,
        &identity(d, 2, o.terminal()),
    )
}

fn cancel(
    snap: &mut Snapshot,
    rig: &mut kit::Rig,
    d: &Defs,
    title: &str,
    o: &Order,
    r: &Row,
) -> TestResult {
    let mut next = r.raw.clone();
    next["terminal"] = json!({"commit":o.terminal(),"action":d.actions[3],"journal_seq":o.journal + 10,
        "posted_at":500,"at":stamp(o.at + 7300),"result":{"outcome":"cancelled"}});
    let n = Row::parse(next.clone(), o.n)?;
    let (scalar, released) = release(rig, r)?;
    let writes: Vec<Write> = vec![
        (order_key(o.n)?, Some(next)),
        (n.key(ASSET)?, Some(history::asset(&n, true)?)),
        (n.key(HULL)?, Some(history::hull(&n, true)?)),
        (n.key(SLOTS)?, Some(scalar)),
        (n.key(BOOK)?, Some(released)),
        (n.key(SCHEDULE)?, None),
        (n.key(JOB)?, None),
    ];
    rig.refuse(
        snap,
        "cancel as the install action",
        &writes,
        &terminal_changes(o)?,
        &identity(d, 2, o.terminal()),
    )?;
    let mut held = writes.clone();
    held.retain(|(k, _)| k.family.as_str() != JOB);
    rig.refuse(
        snap,
        "cancel that keeps the job",
        &held,
        &terminal_changes(o)?,
        &identity(d, 3, o.terminal()),
    )?;
    rig.accept(
        snap,
        &format!("{title}: cancelled"),
        writes,
        terminal_changes(o)?,
        &identity(d, 3, o.terminal()),
    )
}

fn identity_section(snap: &mut Snapshot, d: &Defs) -> TestResult {
    snap.section("core refit: identity");
    for (name, ceiling) in FAMILIES {
        snap.line("reserved family", &format!("industry {name} {ceiling}"));
        snap.wire(
            &format!("descriptor {name}"),
            &descriptor(name)?.canonical_value(),
        )?;
    }
    for (name, value) in [
        ("order bytes", ORDER_BYTES),
        ("order ceiling", ORDER_CEILING),
    ] {
        snap.line(name, &value.to_string());
    }
    snap.wire("policy", &d.policy)?;
    snap.wire(
        "policy references",
        &protocol::policy_references(&d.policy)?,
    )?;
    snap.wire("participants", &d.participants)?;
    for (index, action) in d.actions.iter().enumerate() {
        snap.wire(&format!("action reference {index}"), action)?;
    }
    for index in [2, 3] {
        snap.wire(
            &format!("closed action reference {index}"),
            &action_reference(&Value::Null, &Value::Null, index)?,
        )?;
    }
    let exchange = crate::core_exchange::action_descriptor().map_err(|error| error.to_string())?;
    snap.wire("exchange action payload", exchange.payload())?;
    let reg = registry(d)?;
    let protocol = reg.core_refit.as_ref().ok_or("protocol")?;
    snap.wire("protocol", protocol)?;
    snap.wire("industry participant", protocol.industry_participant())?;
    snap.wire("terminal participants", &protocol.terminal_participants())?;
    kit::refusal(
        snap,
        "families without the core integration declaration",
        protocol.validate_families(&reg).map_err(Into::into),
    )?;
    let industry = owner_id()?;
    let [a, b, c] = d.participants.clone();
    let [w, x, y, z] = d.actions.clone();
    let refusals: [(&str, Result<Protocol, ActionStateError>); 5] = [
        (
            "declaration by another owner",
            Protocol::new(
                &OwnerId::new("tacsim")?,
                d.participants.clone(),
                d.actions.clone(),
            ),
        ),
        (
            "declaration with swapped participants",
            Protocol::new(
                &industry,
                [b.clone(), a.clone(), c.clone()],
                d.actions.clone(),
            ),
        ),
        (
            "declaration with the version 2 producer",
            Protocol::new(
                &industry,
                [
                    reference("industry", "owner_participant", "core_production", 2)?,
                    b,
                    c,
                ],
                d.actions.clone(),
            ),
        ),
        (
            "declaration with reordered actions",
            Protocol::new(&industry, d.participants.clone(), [x, w, y, z]),
        ),
        (
            "declaration with a policy-bound cancel",
            Protocol::new(
                &industry,
                d.participants.clone(),
                [
                    d.actions[0].clone(),
                    d.actions[1].clone(),
                    d.actions[2].clone(),
                    reference("industry", "action", "cancel_core_refit", 1)?,
                ],
            ),
        ),
    ];
    for (label, result) in refusals {
        kit::refusal(snap, label, result.map_err(Into::into))?;
    }
    let mut policy = d.policy.clone();
    policy["recipe"] = json!(reference("industry", "recipe", "core_ungovern", 1)?);
    kit::refusal(
        snap,
        "a policy naming another recipe",
        protocol::policy_references(&policy).map_err(Into::into),
    )
}

pub(super) fn rig(d: &Defs) -> TestResult<kit::Rig> {
    kit::Rig::new(registry(d)?, validate_changes, "core refit", Vec::new())
}

#[test]
fn ct0_refit_core_walk() -> TestResult {
    let d = defs()?;
    let mut snap = Snapshot::new();
    identity_section(&mut snap, &d)?;
    let first = Order {
        n: 1,
        at: 0,
        journal: 10,
        byte: 1,
        revision: 3,
        incoming: 50,
        displaced: None,
    };
    let second = Order {
        n: 2,
        at: 20_000,
        journal: 30,
        byte: 11,
        revision: 4,
        incoming: 51,
        displaced: Some(50),
    };

    let mut w = rig(&d)?;
    let title = "fitted into a coreless base, then replaced";
    let r = accept(&mut snap, &mut w, &d, &format!("{title}: order 1"), &first)?;
    let n = advance(
        &mut snap,
        &mut w,
        &d,
        &format!("{title}: order 1"),
        &first,
        &r,
    )?;
    install(
        &mut snap,
        &mut w,
        &d,
        &format!("{title}: order 1"),
        &first,
        &n,
    )?;
    let r = accept(&mut snap, &mut w, &d, &format!("{title}: order 2"), &second)?;
    let n = advance(
        &mut snap,
        &mut w,
        &d,
        &format!("{title}: order 2"),
        &second,
        &r,
    )?;
    install(
        &mut snap,
        &mut w,
        &d,
        &format!("{title}: order 2"),
        &second,
        &n,
    )?;

    let mut w = rig(&d)?;
    let title = "cancelled at once";
    let r = accept(&mut snap, &mut w, &d, title, &first)?;
    cancel(&mut snap, &mut w, &d, title, &first, &r)?;

    let mut w = rig(&d)?;
    let title = "fitted, then a replacement cancelled after progress";
    let r = accept(&mut snap, &mut w, &d, &format!("{title}: order 1"), &first)?;
    let n = advance(
        &mut snap,
        &mut w,
        &d,
        &format!("{title}: order 1"),
        &first,
        &r,
    )?;
    install(
        &mut snap,
        &mut w,
        &d,
        &format!("{title}: order 1"),
        &first,
        &n,
    )?;
    let r = accept(&mut snap, &mut w, &d, &format!("{title}: order 2"), &second)?;
    let n = advance(
        &mut snap,
        &mut w,
        &d,
        &format!("{title}: order 2"),
        &second,
        &r,
    )?;
    cancel(
        &mut snap,
        &mut w,
        &d,
        &format!("{title}: order 2"),
        &second,
        &n,
    )?;

    snap.check(env!("CARGO_MANIFEST_DIR"), "tests/ct0/refit_core.snap")?;
    Ok(())
}
