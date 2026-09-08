//! Freeing a deep lock tree — the walk nobody writes and everybody takes.
//!
//! `LockExpr` used to get its `Drop` from the compiler: freeing
//! `Not(Not(Not(…)))` called into itself once per level. That is the last
//! recursion in the module that no *entry* bound can help with, because it does
//! not happen when a tree arrives, it happens when one leaves — overwrite an
//! object's lock with `@lock`, destroy the object, or simply let a decoded
//! World go out of scope at shutdown, and the free walks the guard page on the
//! world task, which may not die. WS01-S2 replaced it with a hand-written
//! `Drop` that dismantles the tree onto a heap worklist.
//!
//! **Its own test binary**, for the reason all the `lock_*depth*` targets are:
//! a stack overflow kills the process and the harness reports an opaque abort,
//! so a regression only names itself if it is alone. Reproduce with
//! `cargo test -p mush-core --test lock_depth_drop -- --test-threads=1`.
//!
//! And on a 128 KiB stack, because unlike evaluation and printing the free is
//! now *fully* iterative: it should not need more stack for a 200_000-level
//! tree than for a leaf. A recursive `Drop` dies here at once; this one has
//! two frames of headroom to spare.

use std::thread;

use mush_core::lock::{LockExpr, LockType};
use mush_core::obj::{GOD, LIMBO, ObjId, ObjKind};
use mush_core::world::World;

/// Levels in each tree under test — far past what any stack holds one frame
/// per level of, and past what `parse_lock` (48) or serde (~127) will admit.
const TALL: usize = 200_000;

/// Run `f` on a 128 KiB stack. The join returning **is** the assertion.
fn on_a_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    thread::Builder::new()
        .stack_size(128 * 1024)
        .name("lock-depth-drop".into())
        .spawn(f)
        .expect("spawn the small-stack thread")
        .join()
        .expect("freeing the tree must return, not die on the guard page")
}

/// `!!!…#1`, `n` levels, built bottom-up by a loop.
fn not_chain(n: usize) -> LockExpr {
    let mut e = LockExpr::Is(ObjId(1));
    for _ in 1..n {
        e = LockExpr::Not(Box::new(e));
    }
    e
}

/// `#1 & #TRUE & …`, `n` levels, left-nested as the parser's loop nests it.
fn and_chain(n: usize) -> LockExpr {
    let mut e = LockExpr::Is(ObjId(1));
    for _ in 1..n {
        e = LockExpr::And(Box::new(e), Box::new(LockExpr::Const(true)));
    }
    e
}

#[test]
fn dropping_a_200k_deep_not_chain_does_not_recurse() {
    on_a_small_stack(|| {
        let e = not_chain(TALL);
        assert_eq!(e.depth(), TALL as u32);
        drop(e);
    });
}

#[test]
fn dropping_a_200k_term_and_chain_does_not_recurse() {
    on_a_small_stack(|| {
        let e = and_chain(TALL);
        assert_eq!(e.depth(), TALL as u32);
        drop(e);
    });
}

#[test]
fn overwriting_a_deep_lock_frees_the_old_one_safely() {
    // The shape the game actually reaches this on: `@lock` replacing a key.
    // The `insert` returns nothing to the caller, so the old value is freed
    // right there, inside the world task's command handler.
    on_a_small_stack(|| {
        let mut w = World::new_minimal();
        let door = w
            .create(ObjKind::Thing, "door", GOD, Some(LIMBO))
            .expect("create the door");
        let locks = &mut w.get_mut(door).expect("the door exists").locks;
        locks.insert(LockType::Basic, and_chain(TALL));
        locks.insert(LockType::Basic, LockExpr::Const(true));
        assert_eq!(locks.get(&LockType::Basic), Some(&LockExpr::Const(true)));
    });
}

#[test]
fn dropping_the_world_that_holds_a_deep_lock_is_safe() {
    // And the shape shutdown reaches it on: the whole World goes away, and
    // every lock in it is freed as part of the same collapse.
    on_a_small_stack(|| {
        let mut w = World::new_minimal();
        let door = w
            .create(ObjKind::Thing, "door", GOD, Some(LIMBO))
            .expect("create the door");
        w.get_mut(door)
            .expect("the door exists")
            .locks
            .insert(LockType::Enter, not_chain(TALL));
        drop(w);
    });
}

#[test]
fn clamping_frees_the_truncated_subtree_safely() {
    // `clamp_depth` is the repair path, and the repair *is* a free: the
    // subtree it replaces with `#FALSE` is dropped on the spot, and it is by
    // construction the deepest part of the tree.
    on_a_small_stack(|| {
        let mut e = not_chain(TALL);
        assert_eq!(e.clamp_depth(48), 1);
        assert_eq!(e.depth(), 48);
    });
}
