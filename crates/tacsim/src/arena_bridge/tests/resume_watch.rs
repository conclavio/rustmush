#![cfg(test)]
//! Actual native acceptance + snapshot decode feed the production recovery
//! helper; actual Engine output/Frames clock exercise the notice and shield.
use super::super::{OrderDeadline, on_frames, say_due_orders, say_expired_orders};
use super::*;
use crate::arena::{
    Arena, Channel, Controller, Intent, IntentKind, IntentOutcome, IntentSource, MachineState,
    Snapshot, apply_intent, test_support::arena_at_epoch,
};
use crate::deliberation::{
    SquadRoster,
    order::{Constraints, Objective, Order},
};
use mush_core::{Ctx, Outbox};
fn squad() -> SquadId {
    SquadId {
        team: PartyId::RED,
        index: 1,
    }
}
fn native(by: CharacterRef, issuer: Option<ObjId>, ttl: u32) -> Arena {
    let mut arena = arena_at_epoch(
        vec![
            tank(RA, PartyId::RED, pose(1500, 1500)),
            tank(RB, PartyId::RED, pose(2500, 1500)),
            tank(BA, PartyId::BLUE, pose(6500, 6500)),
        ],
        1,
    );
    arena.set_controller(RB, Controller::Mind, MachineState::Patrol);
    arena.squads = vec![SquadRoster {
        id: squad(),
        commander: RB,
        units: vec![RB],
    }];
    let order = Order {
        squad: squad(),
        objective: Objective::Hold,
        target: None,
        horizon_ms: ttl,
        constraints: Constraints {
            hold_fire: true,
            ..Constraints::default()
        },
        ttl_ms: ttl,
        issued_ms: 123,
        issuer,
        by,
        spot: None,
    };
    let intent = Intent {
        unit: RB,
        channel: Channel::Helm,
        epoch: arena.epochs.current(RB, Channel::Helm),
        seq: 1,
        kind: IntentKind::Order(Box::new(order)),
    };
    assert_eq!(
        apply_intent(&mut arena, &intent, IntentSource::Seat),
        IntentOutcome::AppliedContinuous
    );
    let snapshot = Snapshot::encode(&arena).expect("actual held order snapshot");
    Snapshot::decode(&snapshot.document, arena.definition.as_ref().clone())
        .expect("decoded held order")
}
fn call(tw: &mut mush_testkit::TestWorld, f: impl FnOnce(&mut Ctx<'_>)) -> Vec<(ObjId, String)> {
    let mut out = Outbox::new();
    let engine = tw.engine_mut();
    f(&mut Ctx {
        w: &mut engine.world,
        actor: GOD,
        switches: Default::default(),
        args: Default::default(),
        out: &mut out,
        reg: &engine.registry,
        module: Some(crate::NS.into()),
        typed_state: mush_core::ModuleActionState::unavailable(),
    });
    told(&out)
        .into_iter()
        .filter(|(_, line)| line.contains("order has run out"))
        .collect()
}
fn restore(tw: &mut mush_testkit::TestWorld, arena: &Arena) {
    let reg = tw
        .world_mut()
        .module_state_mut::<ArenaRegistry>()
        .expect("registry");
    reg.track(DROP_MATCH);
    reg.matches.get_mut(&DROP_MATCH).unwrap().now_ms = arena.now_ms;
    reg.restore_order_watches(DROP_MATCH, arena.recovered_order_watches());
    let state = reg.deliberation.get(&DROP_MATCH).unwrap();
    assert_eq!(
        (state.issued, state.human_issued, state.in_flight),
        (0, 0, 0)
    );
}
#[test]
fn r4_recovered_character_shield_and_notice_wait_for_actual_floored_arena_clock() {
    let (mut tw, ada, bo) = dropped_world(crate::TacsimConfig::default());
    let mut arena = native(CharacterRef::Character { id: ada }, Some(RA), 5000);
    restore(&mut tw, &arena);
    arena
        .clock
        .pin_tempo_for_tests(crate::arena::Tempo::Engaged);
    arena.clock.set_floor(Some(5000));
    arena.clock.reanchor(0);
    tw.set_now(tw.world().now() + 1000);
    arena.step_settled(100_000, &[], &[]);
    assert!(
        arena.now_ms < 5000,
        "actual lag clamp must leave order live"
    );
    let say = call(&mut tw, |ctx| {
        on_frames(ctx, DROP_MATCH, Vec::new(), arena.now_ms, None);
        say_expired_orders(ctx);
    });
    assert!(say.is_empty());
    let reg = tw.world().module_state::<ArenaRegistry>().unwrap();
    assert_eq!(
        reg.order_author(DROP_MATCH, squad()),
        Some(CharacterRef::Character { id: ada })
    );
    assert_eq!(reg.live_orders_of(DROP_MATCH, RA, i64::MAX).len(), 1);
    assert!(reg.deliberation.get(&DROP_MATCH).unwrap().human_holds(
        squad(),
        i64::MAX,
        Some(arena.now_ms)
    ));
    for wall in [200_000, 300_000, 400_000] {
        arena.step_settled(wall, &[], &[]);
    }
    assert!(arena.now_ms >= 5000);
    let say = call(&mut tw, |ctx| {
        on_frames(ctx, DROP_MATCH, Vec::new(), arena.now_ms, None);
        say_expired_orders(ctx);
    });
    assert_eq!(say.iter().filter(|(who, _)| *who == ada).count(), 1);
    assert_eq!(say.iter().filter(|(who, _)| *who == bo).count(), 0);
    assert_eq!(say.iter().filter(|(who, _)| *who == GOD).count(), 1);
    assert!(
        call(&mut tw, |ctx| {
            on_frames(ctx, DROP_MATCH, Vec::new(), arena.now_ms, None);
            say_expired_orders(ctx);
        })
        .is_empty()
    );
    let reg = tw.world().module_state::<ArenaRegistry>().unwrap();
    assert!(reg.live_orders_of(DROP_MATCH, RA, i64::MIN).is_empty());
}
#[test]
fn r4_recovered_persona_scope_and_terminal_only_say_actual_due_watches() {
    let (mut tw, ada, bo) = dropped_world(crate::TacsimConfig::default());
    let arena = native(CharacterRef::Persona { squad: squad() }, None, 5000);
    restore(&mut tw, &arena);
    tw.set_now(tw.world().now() + 1000);
    assert!(
        call(&mut tw, |ctx| {
            on_frames(ctx, DROP_MATCH, Vec::new(), 4999, None);
            say_due_orders(ctx, Some(DROP_MATCH));
        })
        .is_empty()
    );
    let said = call(&mut tw, |ctx| {
        on_frames(ctx, DROP_MATCH, Vec::new(), 5000, None);
        say_due_orders(ctx, Some(DROP_MATCH));
    });
    assert_eq!(said.iter().filter(|(who, _)| *who == ada).count(), 1);
    assert_eq!(said.iter().filter(|(who, _)| *who == bo).count(), 0);
    assert_eq!(said.iter().filter(|(who, _)| *who == GOD).count(), 1);
    assert!(
        said.iter()
            .all(|(_, line)| line.contains("red lance leader 1's hold"))
    );
    assert!(call(&mut tw, |ctx| say_due_orders(ctx, Some(DROP_MATCH))).is_empty());
}
#[test]
fn r4_missing_arena_clock_defers_recovery_and_new_world_order_replaces_it() {
    let (mut tw, ada, _) = dropped_world(crate::TacsimConfig::default());
    let arena = native(CharacterRef::Character { id: ada }, Some(RA), 5000);
    restore(&mut tw, &arena);
    let reg = tw.world_mut().module_state_mut::<ArenaRegistry>().unwrap();
    reg.matches.remove(&DROP_MATCH);
    assert!(!OrderDeadline::Arena(5000).due(i64::MAX, None));
    assert_eq!(reg.live_orders_of(DROP_MATCH, RA, i64::MAX).len(), 1);
    reg.note_human_order(
        DROP_MATCH,
        squad(),
        "withdraw".into(),
        RA,
        CharacterRef::Character { id: ada },
        1000,
    );
    tw.set_now(2);
    let said = call(&mut tw, say_expired_orders);
    assert_eq!(said.iter().filter(|(who, _)| *who == ada).count(), 1);
    assert!(said.iter().all(|(_, line)| line.contains("withdraw")));
}

#[test]
fn r4_actual_ended_publication_says_due_recovery_before_untracking_only() {
    for due in [false, true] {
        let (mut tw, ada, bo) = dropped_world(crate::TacsimConfig::default());
        let mut arena = native(CharacterRef::Persona { squad: squad() }, None, 5000);
        restore(&mut tw, &arena);
        // An actual stopped arena produces the ending publication; the bridge
        // receives that exact end and ledger digest, without a guessed wall TTL.
        arena
            .clock
            .pin_tempo_for_tests(crate::arena::Tempo::Engaged);
        arena.clock.set_floor(Some(1000));
        arena.clock.reanchor(0);
        let target = if due { 5000 } else { 4000 };
        let mut wall = 0;
        while arena.now_ms < target && wall < 60_000 {
            wall += 500;
            let output = arena.step_settled(wall, &[], &[]);
            assert!(
                output.ended.is_none(),
                "clock-only held-fire fixture stays live"
            );
        }
        let terminal_ms = arena.now_ms;
        assert!(terminal_ms >= target);
        assert_eq!(terminal_ms >= 5000, due);
        arena.stop(crate::arena::StopReason::Abort);
        let result = arena.step_settled(wall + 100, &[], &[]);
        let end = result.ended.expect("native stopped end");
        assert_eq!(end.t_ms, terminal_ms);
        let said = call(&mut tw, |ctx| {
            super::super::on_ended(ctx, DROP_MATCH, end, arena.ledger.digest(), &[]);
        });
        assert_eq!(
            said.iter().filter(|(who, _)| *who == ada).count(),
            usize::from(due)
        );
        assert_eq!(said.iter().filter(|(who, _)| *who == bo).count(), 0);
        assert_eq!(
            said.iter().filter(|(who, _)| *who == GOD).count(),
            usize::from(due)
        );
        let reg = tw.world().module_state::<ArenaRegistry>().unwrap();
        assert!(!reg.matches.contains_key(&DROP_MATCH));
        assert!(!reg.deliberation.contains_key(&DROP_MATCH));
        assert!(call(&mut tw, say_expired_orders).is_empty());
    }
}
