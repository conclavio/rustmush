//! Interface cut I10 hardening: the per-subject cap on outstanding
//! server-initiated requests holds at the engine — a producer that keeps
//! asking is dropped at `MAX_OPEN_REQUESTS_PER_SUBJECT`, an answer or a
//! deadline frees a slot, and one subject's cap never touches another's.

use std::sync::Arc;

use mush_core::engine::{Effects, Engine, Inbound};
use mush_core::request::{MAX_OPEN_REQUESTS_PER_SUBJECT, RequestCompletion, RequestOutcome};
use mush_core::{
    Audience, CmdError, CommandCompatibility, CommandSpec, Ctx, GOD, GameModule, LIMBO, ModError,
    ObjId, ObjKind, Outbox, Registrar, Registry, World, register_core_commands,
};
use mush_proto::{OutputEvent, RequestKind, RespondChoice, RichText};

struct Done;

impl RequestCompletion for Done {
    fn on_respond(&self, _ctx: &mut Ctx<'_>, _payload: &str, _outcome: RequestOutcome) {}
}

/// `@ask` opens one Confirm request to the actor with a one-minute deadline.
struct Asker;

impl GameModule for Asker {
    fn name(&self) -> &'static str {
        "asker"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(CommandCompatibility::ReadOnly);
        reg.request_completion(Arc::new(Done))?;
        reg.command(
            CommandSpec {
                name: "@ask".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let subject = ctx.actor;
                ctx.open_request(
                    subject,
                    RequestKind::Confirm,
                    RichText::plain("Really?"),
                    Vec::new(),
                    60_000,
                    "ask",
                );
                Ok(())
            }),
        )?;
        Ok(())
    }
}

fn setup() -> (Engine, ObjId) {
    let mut world = World::new_minimal();
    world.set_now(1_000);
    let nick = world
        .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
        .unwrap();
    let mut registry = Registry::new();
    register_core_commands(&mut registry).unwrap();
    let engine = Engine::with_modules(world, registry, vec![Box::new(Asker)]).unwrap();
    (engine, nick)
}

fn run(engine: &mut Engine, actor: ObjId, line: &str) -> Outbox {
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.step(
        Inbound::Cmd {
            conn: 1,
            actor,
            line: line.into(),
        },
        &mut out,
        &mut fx,
    );
    out
}

/// The ids of every `Request` delivered to `who`.
fn request_ids(out: &Outbox, who: ObjId) -> Vec<u64> {
    out.events()
        .iter()
        .filter_map(|(aud, e)| match (aud, e) {
            (Audience::Actor(a), OutputEvent::Request { id, .. }) if *a == who => Some(*id),
            _ => None,
        })
        .collect()
}

fn ask(engine: &mut Engine, who: ObjId) -> Vec<u64> {
    request_ids(&run(engine, who, "@ask"), who)
}

#[test]
fn open_requests_are_capped_per_subject_and_an_answer_frees_a_slot() {
    let (mut engine, nick) = setup();
    let cap = MAX_OPEN_REQUESTS_PER_SUBJECT as usize;

    let mut ids = Vec::new();
    for _ in 0..(cap + 8) {
        ids.extend(ask(&mut engine, nick));
    }
    assert_eq!(
        ids.len(),
        cap,
        "opens past the cap are dropped, never queued"
    );
    assert!(
        ask(&mut engine, nick).is_empty(),
        "at the cap, another open is dropped"
    );

    // An answer frees exactly one slot.
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.step(
        Inbound::Respond {
            conn: 1,
            actor: nick,
            id: ids[0],
            choice: RespondChoice::Confirm(true),
        },
        &mut out,
        &mut fx,
    );
    assert_eq!(ask(&mut engine, nick).len(), 1, "the answered slot is free");
    assert!(ask(&mut engine, nick).is_empty(), "and only that one");

    // Another subject's cap is their own.
    assert_eq!(ask(&mut engine, GOD).len(), 1);
}

#[test]
fn a_deadline_frees_the_slots_it_expires() {
    let (mut engine, nick) = setup();
    let cap = MAX_OPEN_REQUESTS_PER_SUBJECT as usize;
    for _ in 0..cap {
        assert_eq!(ask(&mut engine, nick).len(), 1);
    }
    assert!(ask(&mut engine, nick).is_empty());

    // Past the one-minute deadline every request expires with the server
    // default, and the subject may be asked again.
    engine.set_now(1_000 + 61);
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.fire_due((1_000 + 61) * 1000, &mut out, &mut fx);
    let mut reopened = Vec::new();
    for _ in 0..cap {
        reopened.extend(ask(&mut engine, nick));
    }
    assert_eq!(reopened.len(), cap, "expired requests no longer count");
    assert!(
        reopened.iter().all(|id| *id > cap as u64),
        "ids are never reused: {reopened:?}"
    );
}
