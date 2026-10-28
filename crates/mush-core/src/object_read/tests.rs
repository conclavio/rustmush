#![cfg(test)]

use super::*;
use crate::attr::{Attr, AttrFlags};
use crate::lock::{LockExpr, LockType, MAX_LOCK_DEPTH, eval_lock, eval_lock_with, parse_lock};
use crate::obj::{GOD, LIMBO, ObjKind};
use crate::world::{attr_with, can_move_to_with};
use serde_json::json;

fn expression(text: &str) -> LockExpr {
    parse_lock(text, |_| None).expect("valid test lock")
}

fn tracked_answer(world: &World, text: &str, actor: ObjId, depth: u8) -> bool {
    let expr = expression(text);
    let mut read = TrackedRead::new(world);
    let result = eval_lock_with(&mut read, &expr, actor, depth).expect("bounded fixture");
    assert_eq!(result, eval_lock(world, &expr, actor, depth));
    assert!(read.finish().expect("healthy trace").matches(world));
    result
}

#[test]
fn full_rows_absence_and_unrelated_allocation_revalidate_exactly() {
    let mut world = World::new_minimal();
    let mut read = TrackedRead::new(&world);
    read.object(GOD).unwrap();
    assert_eq!(read.object_slots().unwrap(), world.object_slot_count());
    let trace = read.finish().unwrap();
    world
        .create(ObjKind::Thing, "unrelated", GOD, None)
        .unwrap();
    assert!(trace.matches(&world), "allocation is not a semantic edge");
    world.get_mut(GOD).unwrap().modified += 1;
    assert!(
        !trace.matches(&world),
        "full fields, not only selected leaves"
    );

    let absent = ObjId(world.next_id());
    let mut read = TrackedRead::new(&world);
    assert!(read.object(absent).unwrap().is_none());
    let trace = read.finish().unwrap();
    assert!(trace.matches(&world));
    assert_eq!(
        world.create(ObjKind::Thing, "new", GOD, None).unwrap(),
        absent
    );
    assert!(!trace.matches(&world), "absence must be a dependency too");
}

#[test]
fn going_rows_retain_the_plain_read_semantics() {
    let mut world = World::new_minimal();
    world
        .get_mut(GOD)
        .unwrap()
        .flags
        .insert(crate::Flags::GOING);
    let mut plain = PlainRead::new(&world);
    let mut tracked = TrackedRead::new(&world);
    assert_eq!(plain.object(GOD).unwrap(), tracked.object(GOD).unwrap());
    assert!(tracked.finish().unwrap().matches(&world));
}

#[test]
fn every_limit_is_positive_and_cached_reads_still_spend_visits() {
    let world = World::new_minimal();
    for limits in [
        ReadLimits {
            max_objects: 0,
            ..ReadLimits::default()
        },
        ReadLimits {
            max_bytes: 0,
            ..ReadLimits::default()
        },
        ReadLimits {
            max_visits: 0,
            ..ReadLimits::default()
        },
        ReadLimits {
            max_work: 0,
            ..ReadLimits::default()
        },
    ] {
        assert!(matches!(
            TrackedRead::with_limits(&world, limits),
            Err(ReadLimit::InvalidLimits)
        ));
    }
    let mut read = TrackedRead::with_limits(
        &world,
        ReadLimits {
            max_visits: 2,
            ..ReadLimits::default()
        },
    )
    .unwrap();
    read.object(GOD).unwrap();
    read.object(GOD).unwrap();
    assert_eq!(read.object(GOD), Err(ReadLimit::Visits));
    assert_eq!(read.charge_work(0), Err(ReadLimit::Visits));
    assert!(matches!(read.finish(), Err(ReadLimit::Visits)));
}

#[test]
fn absent_rows_count_toward_object_limit_and_errors_are_sticky() {
    let world = World::new_minimal();
    let mut read = TrackedRead::with_limits(
        &world,
        ReadLimits {
            max_objects: 1,
            ..ReadLimits::default()
        },
    )
    .unwrap();
    read.object(ObjId(90)).unwrap();
    assert_eq!(read.object(ObjId(91)), Err(ReadLimit::Objects));
    assert_eq!(read.object(ObjId(90)), Err(ReadLimit::Objects));
    assert!(matches!(read.finish(), Err(ReadLimit::Objects)));
}

#[test]
fn streaming_bytes_bound_the_complete_entry_and_aggregate() {
    let world = World::new_minimal();
    let bytes = serde_json::to_vec(&(GOD, world.get(GOD))).unwrap().len();
    let mut exact = TrackedRead::with_limits(
        &world,
        ReadLimits {
            max_bytes: bytes,
            ..ReadLimits::default()
        },
    )
    .unwrap();
    exact.object(GOD).unwrap();
    assert!(exact.finish().unwrap().matches(&world));
    let mut short = TrackedRead::with_limits(
        &world,
        ReadLimits {
            max_bytes: bytes - 1,
            ..ReadLimits::default()
        },
    )
    .unwrap();
    assert_eq!(short.object(GOD), Err(ReadLimit::Bytes));
    assert!(matches!(short.finish(), Err(ReadLimit::Bytes)));
    let mut aggregate = TrackedRead::with_limits(
        &world,
        ReadLimits {
            max_bytes: bytes,
            ..ReadLimits::default()
        },
    )
    .unwrap();
    aggregate.object(GOD).unwrap();
    assert_eq!(aggregate.object(ObjId(90)), Err(ReadLimit::Bytes));
}

#[test]
fn oversized_strings_and_broad_json_refuse_before_clone() {
    let mut world = World::new_minimal();
    world.get_mut(GOD).unwrap().name = "x".repeat(10_000);
    let mut read = TrackedRead::with_limits(
        &world,
        ReadLimits {
            max_bytes: 1024,
            ..ReadLimits::default()
        },
    )
    .unwrap();
    assert_eq!(read.object(GOD), Err(ReadLimit::Bytes));
    drop(read);
    world.get_mut(GOD).unwrap().name = "God".into();
    world
        .set_attr(GOD, "broad", Attr::json(json!(vec![0; 10_000]), GOD))
        .unwrap();
    let mut read = TrackedRead::with_limits(
        &world,
        ReadLimits {
            max_work: 4096,
            ..ReadLimits::default()
        },
    )
    .unwrap();
    assert_eq!(read.object(GOD), Err(ReadLimit::Work));
    assert!(matches!(read.finish(), Err(ReadLimit::Work)));
}

#[test]
fn deep_lock_and_json_are_checked_before_snapshot_or_live_equality() {
    let mut world = World::new_minimal();
    let mut read = TrackedRead::new(&world);
    read.object(GOD).unwrap();
    let trace = read.finish().unwrap();
    world
        .get_mut(GOD)
        .unwrap()
        .locks
        .insert(LockType::Basic, LockExpr::and_chain(200_000));
    assert!(!trace.matches(&world));
    let mut read = TrackedRead::new(&world);
    assert_eq!(read.object(GOD), Err(ReadLimit::Nesting));
    drop(read);
    world.get_mut(GOD).unwrap().locks.clear();
    let mut value = json!(0);
    for _ in 0..150 {
        value = json!([value]);
    }
    world.set_attr(GOD, "deep", Attr::json(value, GOD)).unwrap();
    assert!(!trace.matches(&world));
    let mut read = TrackedRead::new(&world);
    assert_eq!(read.object(GOD), Err(ReadLimit::Nesting));
}

#[test]
fn lock_leaves_and_inheritance_keep_legacy_answers() {
    let mut world = World::new_minimal();
    let actor = world
        .create(ObjKind::Player, "reader", GOD, Some(LIMBO))
        .unwrap();
    let carried = world
        .create(ObjKind::Thing, "token", GOD, Some(actor))
        .unwrap();
    let parent = world.create(ObjKind::Thing, "parent", GOD, None).unwrap();
    let grandparent = world
        .create(ObjKind::Thing, "grandparent", GOD, None)
        .unwrap();
    world.set_parent(actor, Some(parent)).unwrap();
    world.set_parent(parent, Some(grandparent)).unwrap();
    world
        .set_attr(grandparent, "kind", Attr::text("ÉCLAIR", GOD))
        .unwrap();
    let mut local = Attr::text("wrong", GOD);
    local.flags = AttrFlags::NO_INHERIT;
    world.set_attr(parent, "kind", local).unwrap();
    world
        .set_attr(actor, "payload", Attr::json(json!({"a": [1, true]}), GOD))
        .unwrap();
    world
        .get_mut(actor)
        .unwrap()
        .markers
        .insert("game:pilot".into());
    for (text, expected) in [
        ("#TRUE".to_string(), true),
        (format!("{actor}"), true),
        (format!("+{carried}"), true),
        (format!("${actor}"), true),
        ("flag^GAME:PILOT".into(), true),
        ("flag^WIZARD".into(), false),
        ("kind:écl*".into(), true),
        ("payload:*true*".into(), true),
        (format!("@{parent}/use"), true),
        ("@#9999/enter".into(), false),
        ("!@#9999/enter".into(), true),
    ] {
        assert_eq!(
            tracked_answer(&world, &text, actor, MAX_LOCK_DEPTH),
            expected,
            "{text}"
        );
    }
    assert!(tracked_answer(&world, "kind:wrong", parent, MAX_LOCK_DEPTH));
    assert!(
        tracked_answer(&world, &format!("!@{parent}/enter"), actor, 0),
        "legacy indirect cutoff is ordinary false, not new resource poison"
    );
}

#[test]
fn privacy_poison_and_resource_poison_preserve_short_circuiting() {
    let mut world = World::new_minimal();
    let mut secret = Attr::text("yes", GOD);
    secret.flags = AttrFlags::SELF_ONLY;
    world.set_attr(GOD, "secret", secret).unwrap();
    for text in ["secret:yes", "!secret:yes", "secret:yes | #TRUE"] {
        assert!(!tracked_answer(&world, text, GOD, MAX_LOCK_DEPTH));
    }
    for text in ["#FALSE & secret:yes", "#TRUE | secret:yes"] {
        let mut read = TrackedRead::with_limits(
            &world,
            ReadLimits {
                max_work: 2,
                ..ReadLimits::default()
            },
        )
        .unwrap();
        assert_eq!(
            eval_lock_with(&mut read, &expression(text), GOD, MAX_LOCK_DEPTH).unwrap(),
            text.starts_with("#TRUE")
        );
        assert!(
            read.finish().unwrap().rows.is_empty(),
            "untaken branch is unread"
        );
    }
    for text in ["!@#9999/enter", "@#9999/enter | #TRUE"] {
        let mut read = TrackedRead::with_limits(
            &world,
            ReadLimits {
                max_work: 2,
                ..ReadLimits::default()
            },
        )
        .unwrap();
        assert_eq!(
            eval_lock_with(&mut read, &expression(text), GOD, MAX_LOCK_DEPTH),
            Err(ReadLimit::Work)
        );
        assert!(matches!(read.finish(), Err(ReadLimit::Work)), "{text}");
    }
}

#[test]
fn glob_backtracking_is_charged_after_a_cached_object_read() {
    let mut world = World::new_minimal();
    world
        .set_attr(GOD, "text", Attr::text("a".repeat(2048), GOD))
        .unwrap();
    let mut read = TrackedRead::new(&world);
    read.object(GOD).unwrap();
    let keep = 10_000;
    read.charge_work(read.budget.limits.max_work - read.budget.work - keep)
        .unwrap();
    let expr = expression(&format!("text:*{}b", "a".repeat(512)));
    assert_eq!(
        eval_lock_with(&mut read, &expr, GOD, MAX_LOCK_DEPTH),
        Err(ReadLimit::Work)
    );
    assert!(matches!(read.finish(), Err(ReadLimit::Work)));
}

#[test]
fn missing_parent_and_indirect_targets_invalidate_negative_results() {
    let mut world = World::new_minimal();
    let missing = ObjId(world.next_id());
    world.get_mut(GOD).unwrap().parent = Some(missing);
    let mut read = TrackedRead::new(&world);
    assert!(attr_with(&mut read, GOD, "new").unwrap().is_none());
    let parent_trace = read.finish().unwrap();
    let mut read = TrackedRead::new(&world);
    assert!(
        eval_lock_with(
            &mut read,
            &expression(&format!("!@{missing}/enter")),
            GOD,
            MAX_LOCK_DEPTH
        )
        .unwrap()
    );
    let lock_trace = read.finish().unwrap();
    world.create(ObjKind::Thing, "arrived", GOD, None).unwrap();
    assert!(!parent_trace.matches(&world));
    assert!(!lock_trace.matches(&world));
}

#[test]
fn inherited_hop_limit_and_cycles_are_unchanged() {
    let mut world = World::new_minimal();
    let ids: Vec<_> = (0..12)
        .map(|index| {
            world
                .create(ObjKind::Thing, &format!("p{index}"), GOD, None)
                .unwrap()
        })
        .collect();
    for pair in ids.windows(2) {
        world.get_mut(pair[0]).unwrap().parent = Some(pair[1]);
    }
    world
        .set_attr(ids[11], "kind", Attr::text("far", GOD))
        .unwrap();
    assert!(!tracked_answer(&world, "kind:far", ids[0], MAX_LOCK_DEPTH));
    assert!(tracked_answer(&world, "kind:far", ids[1], MAX_LOCK_DEPTH));
    world.get_mut(ids[11]).unwrap().parent = Some(ids[0]);
    assert!(!tracked_answer(&world, "missing:*", ids[0], MAX_LOCK_DEPTH));
}

#[test]
fn containment_validation_shares_legacy_results_and_tracks_absence() {
    let mut world = World::new_minimal();
    let bag = world.create(ObjKind::Thing, "bag", GOD, None).unwrap();
    let inside = world
        .create(ObjKind::Thing, "inside", GOD, Some(bag))
        .unwrap();
    for (object, dest) in [
        (bag, inside),
        (bag, bag),
        (bag, GOD),
        (bag, ObjId(99)),
        (ObjId(99), bag),
    ] {
        let mut read = TrackedRead::new(&world);
        assert_eq!(
            can_move_to_with(&mut read, object, dest).unwrap(),
            world.can_move_to(object, dest)
        );
        assert!(read.finish().unwrap().matches(&world));
    }
    world.get_mut(inside).unwrap().location = Some(inside);
    let mut read = TrackedRead::new(&world);
    assert!(can_move_to_with(&mut read, bag, inside).unwrap().is_ok());
    let trace = read.finish().unwrap();
    world
        .create(ObjKind::Thing, "unrelated", GOD, None)
        .unwrap();
    assert!(trace.matches(&world));
    let mut limited = TrackedRead::with_limits(
        &world,
        ReadLimits {
            max_visits: 3,
            ..ReadLimits::default()
        },
    )
    .unwrap();
    assert_eq!(
        can_move_to_with(&mut limited, bag, inside),
        Err(ReadLimit::Visits)
    );
    assert!(matches!(limited.finish(), Err(ReadLimit::Visits)));
}
