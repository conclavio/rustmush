//! The read paths over a tree the parser never saw: evaluation and printing.
//!
//! WS01-S1 bounded what `parse_lock` will *build*. It could not bound what is
//! already in RAM: a lock decoded from Redis, or one a module assembled,
//! reaches `eval_lock` and `Display` without having passed any guard, and both
//! of those recursed once per `!`/`&`/`|` level. On the world task — the one
//! tokio task that owns the World and may never die — that is a guard-page
//! fault, not an error return. So both now carry a structural budget and fail
//! closed at it, `clamp_depth` exists to repair such a tree in place, and this
//! file drives all of it with trees 200_000 levels tall.
//!
//! **This is its own test binary on purpose**, like its `lock_parse_depth_*`
//! siblings: the failure mode is a stack overflow, which kills the whole test
//! process and reports an opaque abort rather than a named failing assert.
//! Alone in a target, a regression names itself. Reproduce in isolation with
//! `cargo test -p mush-core --test lock_depth_eval -- --test-threads=1`.
//!
//! Every case runs on a fixed 2 MiB thread stack, and **the successful join is
//! the assertion**. The stack is larger than the parser tests' 128 KiB because
//! the fence is a bound on recursion, not an abolition of it: the budget is
//! 1024 levels and the walk really does spend them, so a test that did not
//! leave room would be measuring itself rather than the tree. Measured here
//! (debug, x86_64): 512 KiB overflows and 1 MiB passes, i.e. the bounded walk
//! costs under a megabyte of debug frames, and 2 MiB is a 2x margin over that.
//! The unbounded walk these trees would provoke needs ~140 MB, so the two
//! outcomes are two orders of magnitude apart — no margin hides a regression.

use std::thread;

use mush_core::lock::{
    LockExpr, LockType, MAX_LOCK_DEPTH, MAX_LOCK_EXPR_DEPTH, eval_lock, parse_lock,
};
use mush_core::obj::{GOD, LIMBO, ObjId, ObjKind};
use mush_core::world::World;

/// Levels in each tree under test: three orders of magnitude past anything
/// `parse_lock` (48) or serde_json (~127) will admit, and far past what any
/// stack could hold one frame per level of.
const TALL: usize = 200_000;

/// Run `f` on a small stack and hand back what it returned. A panic inside
/// (or a guard-page death, which takes the process with it) is the failure.
fn on_a_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .name("lock-depth-eval".into())
        .spawn(f)
        .expect("spawn the small-stack thread")
        .join()
        .expect("the bounded walk must return, not die on the guard page")
}

/// `!!!…#1`, `n` levels, built bottom-up by a loop — no recursion is involved
/// in making one, so the test can build a tree no recursive code could.
fn not_chain(n: usize) -> LockExpr {
    let mut e = LockExpr::Is(ObjId(1));
    for _ in 1..n {
        e = LockExpr::Not(Box::new(e));
    }
    e
}

/// `#1 & #TRUE & …`, `n` levels, left-nested exactly as `parse_and`'s loop
/// nests it — the shape a flat operator chain produces, and the one a frame
/// counter in the parser is blind to.
fn and_chain(n: usize) -> LockExpr {
    let mut e = LockExpr::Is(ObjId(1));
    for _ in 1..n {
        e = LockExpr::And(Box::new(e), Box::new(LockExpr::Const(true)));
    }
    e
}

/// A world with one player, so the leaves have something to test against.
fn world() -> (World, ObjId) {
    let mut w = World::new_minimal();
    let nick = w
        .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
        .expect("create the player");
    (w, nick)
}

#[test]
fn evaluating_a_200k_deep_not_chain_returns_instead_of_overflowing() {
    let (deep, deeper) = on_a_small_stack(|| {
        let (w, nick) = world();
        (
            eval_lock(&w, &not_chain(TALL), nick, MAX_LOCK_DEPTH),
            eval_lock(&w, &not_chain(TALL + 1), nick, MAX_LOCK_DEPTH),
        )
    });
    // The walk stops at a fixed level from the root and fails closed there;
    // the negations stacked above invert that answer once per level, so the
    // value itself is a parity artifact and means nothing. What it does show
    // is that the answer no longer depends on how much tree is underneath —
    // the extra level is never read.
    assert_eq!(
        deep, deeper,
        "past the budget the answer must stop depending on the tree below it"
    );
}

#[test]
fn evaluating_a_200k_term_and_chain_fails_closed() {
    let passed = on_a_small_stack(|| {
        let (w, nick) = world();
        eval_lock(&w, &and_chain(TALL), nick, MAX_LOCK_DEPTH)
    });
    // No negation is in the way here, so the fail-closed answer at the budget
    // propagates straight up through the `&&`s: a tree too tall to read is a
    // tree that does not pass.
    assert!(!passed, "an unreadably deep lock must not open anything");
}

#[test]
fn printing_a_200k_deep_tree_returns_a_bounded_string() {
    let (nots, ands) =
        on_a_small_stack(|| (not_chain(TALL).to_string(), and_chain(TALL).to_string()));
    // Truncated at the budget, in the syntax: `!!!…#FALSE` and
    // `#FALSE & #TRUE & …`. Unbounded, these would be 200_000 and 1.6M
    // characters (if they returned at all); bounded they are ~1 KiB and
    // ~8 KiB, so any generous ceiling separates the two outcomes.
    assert!(nots.ends_with("#FALSE"), "unexpected print: {nots:.40}…");
    assert!(
        nots.len() < 4 * 1024,
        "the printer walked past its budget: {} chars",
        nots.len()
    );
    assert!(ands.starts_with("#FALSE"), "unexpected print: {ands:.40}…");
    assert!(
        ands.len() < 32 * 1024,
        "the printer walked past its budget: {} chars",
        ands.len()
    );
}

#[test]
fn measuring_a_200k_deep_tree_is_exact_and_iterative() {
    let (nots, ands) = on_a_small_stack(|| (not_chain(TALL).depth(), and_chain(TALL).depth()));
    assert_eq!(nots, TALL as u32);
    assert_eq!(ands, TALL as u32);
}

#[test]
fn clamping_a_200k_deep_tree_makes_it_readable_again() {
    let (cut_nots, depth_nots, cut_ands, depth_ands, ands_pass) = on_a_small_stack(|| {
        let (w, nick) = world();
        let mut nots = not_chain(TALL);
        let cut_nots = nots.clamp_depth(48);
        let mut ands = and_chain(TALL);
        let cut_ands = ands.clamp_depth(48);
        let ands_pass = eval_lock(&w, &ands, nick, MAX_LOCK_DEPTH);
        (cut_nots, nots.depth(), cut_ands, ands.depth(), ands_pass)
    });
    // One cut each: both shapes are a single spine, so the truncation happens
    // once, at the level the caller named.
    assert_eq!(cut_nots, 1, "the spine must have been cut");
    assert_eq!(depth_nots, 48, "clamped to exactly the requested height");
    assert_eq!(cut_ands, 1, "the spine must have been cut");
    assert_eq!(depth_ands, 48);
    // With the deep half replaced by `#FALSE`, the `&` chain is false:
    // clamping is fail-closed at the cut, not fail-open.
    assert!(!ands_pass, "a clamped & chain must not pass");
}

#[test]
fn a_clamped_tree_is_shallow_enough_for_the_parser_that_refused_it() {
    // The repair is not merely "shorter": clamping to MAX_LOCK_EXPR_DEPTH
    // yields a tree the parser would have built, so every later consumer is
    // back inside its own bound and the key round-trips again.
    let printed = on_a_small_stack(|| {
        let mut e = and_chain(TALL);
        let cut = e.clamp_depth(MAX_LOCK_EXPR_DEPTH);
        assert!(cut > 0, "a 200_000-level tree must have been clamped");
        assert!(e.depth() <= MAX_LOCK_EXPR_DEPTH);
        e.to_string()
    });
    let reparsed = parse_lock(&printed, |_| Some(ObjId(1)))
        .expect("a clamped tree prints as a key the parser accepts");
    assert!(reparsed.depth() <= MAX_LOCK_EXPR_DEPTH);
}

#[test]
fn a_deep_tree_reached_through_an_indirect_lock_is_bounded_too() {
    // The structural budget is reseeded at every indirection, so it has to
    // hold on the tree behind the hop as well — and that is exactly the tree
    // a decode delivers, sitting in some other object's lock table.
    let passed = on_a_small_stack(|| {
        let (mut w, nick) = world();
        let door = w
            .create(ObjKind::Thing, "door", GOD, Some(LIMBO))
            .expect("create the door");
        w.get_mut(door)
            .expect("the door exists")
            .locks
            .insert(LockType::Basic, and_chain(TALL));
        let e = LockExpr::Indirect(door, LockType::Basic);
        eval_lock(&w, &e, nick, MAX_LOCK_DEPTH)
    });
    assert!(!passed, "an unreadably deep indirect lock must not pass");
}
