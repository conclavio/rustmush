//! Consumer fixture: existing public notice-watch bookkeeping, then the real
//! private lapse producer via public egress drain. This asserts presentation,
//! not native order acceptance, custody, or restored match authority.
use super::*;
use mush_core::{Audience, Ctx, Effects, Engine, ModuleActionState, Outbox, Registry};
use tacsim::{
    arena_bridge::ArenaRegistry,
    arena_task::MatchId,
    deliberation::{CharacterRef, SquadId},
};

fn drain(engine: &mut Engine) -> Outbox {
    let mut out = Outbox::new();
    tacsim::arena_bridge::drain_egress(&mut Ctx {
        w: &mut engine.world,
        actor: GOD,
        switches: Default::default(),
        args: Default::default(),
        out: &mut out,
        reg: &engine.registry,
        module: None,
        typed_state: ModuleActionState::unavailable(),
    });
    out
}

#[test]
fn r14_actual_lapse_events_are_neutral_through_the_bot_reader() {
    let mut world = World::new_minimal();
    world.set_now(0);
    let pilot = world
        .create(ObjKind::Player, "Pilot", GOD, None)
        .expect("pilot");
    world.set_connected(GOD, true);
    let mut engine = Engine::with_modules(
        world,
        Registry::new(),
        vec![Box::new(tacsim::TacsimRuleset::default())],
    )
    .expect("actual Tacsim registration");
    engine
        .modules_loaded(Default::default(), &mut Outbox::new(), &mut Effects::new())
        .expect("actual empty boot");
    let red = SquadId {
        team: Team::Red,
        index: 1,
    };
    let blue = SquadId {
        team: Team::Blue,
        index: 1,
    };
    let watch = engine
        .world
        .module_state_mut::<ArenaRegistry>()
        .expect("actual registry");
    watch.note_human_order(
        MatchId(81),
        red,
        "hold".into(),
        ObjId(901),
        CharacterRef::Character { id: pilot },
        1_000,
    );
    // Public watch fixture only: a Persona by exercises the existing staff
    // recipient without inventing a match roster or a native accepted order.
    watch.note_human_order(
        MatchId(81),
        blue,
        "screen".into(),
        ObjId(902),
        CharacterRef::Persona { squad: blue },
        1_000,
    );
    assert!(drain(&mut engine).events().is_empty(), "not expired");
    engine.world.set_now(1);
    let out = drain(&mut engine);
    assert_eq!(out.events().len(), 3, "pilot plus two real staff notices");
    let mut reader = Reader::new();
    let mut own = 0;
    let mut persona = 0;
    for (audience, event) in out.events() {
        match (audience, event) {
            (Audience::Actor(actor), OutputEvent::Sys(text)) if *actor == pilot => {
                let expected = "Squad red-1: your hold order has run out; back on their minds.";
                assert_eq!(
                    text.0
                        .iter()
                        .map(|span| span.text.as_str())
                        .collect::<String>(),
                    expected
                );
                own += 1;
            }
            (Audience::Actor(actor), OutputEvent::Notice { text, .. }) if *actor == GOD => {
                if text
                    .0
                    .iter()
                    .map(|span| span.text.as_str())
                    .collect::<String>()
                    .contains("blue lance leader 1's screen order")
                {
                    persona += 1;
                }
            }
            _ => panic!("unexpected lapse recipient or event shape"),
        }
        let lines = reader.event(event);
        assert!(!lines.is_empty());
        assert!(
            lines.iter().all(|line| matches!(line, Line::Other(_))),
            "actual notice must remain neutral"
        );
    }
    assert_eq!((own, persona), (1, 1));
    assert!(drain(&mut engine).events().is_empty(), "latch speaks once");
    let open = &reader.picture.open;
    assert!(open.contacts.is_none());
    assert!(open.weapons.is_none());
    assert!(open.status.is_none());
    assert!(!open.lock_line && !open.brief);
}
