//! **No space think runs on the world thread, and none makes it wait** (doc 44 unit MW.11;
//! Q-44-28).
//!
//! Until MW.11 every space mind thought as `MindPool::shared(1).submit(..).wait()` on the
//! world thread: the customs corvette, the raider and the interdictor each time their bubble
//! stepped, the backup mind of a hull with no crew, and each collector drone on the mining
//! tick and in the command that launched it. Since MW.11 a think is collected on the world
//! thread and thought on the server's pool, and the one place that knows the pool is the
//! think host (`crates/tacsim/src/space/thinks.rs`).
//!
//! Three rules over `crates/tacsim/src/space/`, tests aside:
//!
//! 1. Nothing but the host names the pool, a batch's handle, or the function that runs a
//!    batch where it stands. So no space code can submit a think and wait for it.
//! 2. Nothing there runs a think itself: no `.think(` call on a job or a runner, and no
//!    runner built outside the host.
//! 3. The host waits for a batch in exactly one place, and that place is behind its check
//!    that the host does not hold late steps: the server's world loop sets it to hold
//!    (`run_on_the_wall_clock`, called from `mush-server`'s world task), so there the wait is
//!    reached only for a batch that is already done.

use mush_contract_tests::scan;

/// The think host: the one file that may name the pool.
const HOST: &str = "crates/tacsim/src/space/thinks.rs";

/// What only the host may name.
const POOL_NAMES: [&str; 4] = ["MindPool", "BatchHandle", "run_batch_inline", "mind_pool("];

/// What runs a think where it stands, or builds a runner.
const THINKS_HERE: [&str; 4] = [
    ".think(",
    "RhaiRunner",
    "install_sealed_runner",
    "scripted_runner",
];

/// Every production line of `crates/tacsim/src/space/`, as `(file, line number, text)`, with
/// comments, literals and `#[cfg(test)]` items blanked, and the files that are tests whole
/// (a leading `#![cfg(test)]`) left out.
fn space_lines() -> Vec<(String, usize, String)> {
    let root = scan::repo_root();
    let space = root.join("crates/tacsim/src/space");
    let files = scan::rust_files(&space);
    assert!(
        files.len() >= 11,
        "crates/tacsim/src/space moved: {} files",
        files.len()
    );
    let mut lines = Vec::new();
    for file in files {
        let name = scan::relative(&root, &file);
        let source = std::fs::read_to_string(&file).expect("a readable source file");
        let stripped = scan::strip_comments_and_literals(&source);
        if scan::file_is_test_only(&stripped) {
            continue;
        }
        let stripped = scan::blank_cfg_test_items(&stripped);
        for (number, line) in stripped.lines().enumerate() {
            lines.push((name.clone(), number + 1, line.to_owned()));
        }
    }
    lines
}

#[test]
fn only_the_think_host_names_the_pool() {
    let mut found = Vec::new();
    let mut in_host = 0;
    for (file, number, line) in space_lines() {
        for name in POOL_NAMES {
            if !line.contains(name) {
                continue;
            }
            if file == HOST {
                in_host += 1;
            } else {
                found.push(format!("{file}:{number}: {name}"));
            }
        }
    }
    assert!(
        found.is_empty(),
        "a space think goes through the think host (space/thinks.rs), which holds a late step \
         and never waits on the world thread:\n{}",
        found.join("\n")
    );
    assert!(
        in_host >= 3,
        "the scan found {in_host} uses of the pool in the host; it has stopped seeing them"
    );
}

#[test]
fn no_space_code_runs_a_think_where_it_stands() {
    let mut found = Vec::new();
    let mut in_host = 0;
    for (file, number, line) in space_lines() {
        for name in THINKS_HERE {
            if !line.contains(name) {
                continue;
            }
            if file == HOST {
                in_host += 1;
            } else {
                found.push(format!("{file}:{number}: {name}"));
            }
        }
    }
    assert!(
        found.is_empty(),
        "a space think is collected here and thought on the pool; nothing in space/ runs one \
         or builds a runner outside the think host:\n{}",
        found.join("\n")
    );
    // The host builds each digest's runner, once.
    assert_eq!(
        in_host, 1,
        "the host builds a runner in one place (`scripted_runner`) and runs no think itself"
    );
}

#[test]
fn the_host_waits_in_one_place_behind_its_hold_check() {
    let lines: Vec<(usize, String)> = space_lines()
        .into_iter()
        .filter(|(file, _, _)| file == HOST)
        .map(|(_, number, line)| (number, line))
        .collect();
    let waits: Vec<usize> = lines
        .iter()
        .filter(|(_, line)| line.contains(".wait("))
        .map(|(number, _)| *number)
        .collect();
    assert_eq!(
        waits.len(),
        1,
        "the host waits for a batch in one place: {waits:?}"
    );
    let wait_at = waits.first().copied().unwrap_or(0);
    // The check that turns a batch that is not done into a held step, and returns.
    let hold_check = lines
        .iter()
        .position(|(_, line)| line.contains("if !done && self.holds_late_steps()"))
        .expect("the host checks whether it holds late steps before it waits");
    let (check_at, _) = lines.get(hold_check).expect("the check's line");
    let returns_held = lines
        .iter()
        .skip(hold_check)
        .take(20)
        .any(|(_, line)| line.contains("return Owed::Held;"));
    assert!(
        returns_held,
        "a host that holds late steps answers `Held` for a batch that is not done"
    );
    assert!(
        *check_at < wait_at,
        "the wait (line {wait_at}) comes after the hold check (line {check_at})"
    );
    // Both are in the one function that settles a due step.
    let settle_at = lines
        .iter()
        .find(|(_, line)| line.contains("pub fn settle("))
        .map(|(number, _)| *number)
        .expect("the host settles a due step");
    let next_fn = lines
        .iter()
        .find(|(number, line)| *number > settle_at && line.contains(" fn "))
        .map_or(usize::MAX, |(number, _)| *number);
    assert!(
        settle_at < *check_at && wait_at < next_fn,
        "the hold check and the wait are both in `settle`"
    );
}

#[test]
fn the_world_loop_tells_the_host_to_hold() {
    let root = scan::repo_root();
    let source = std::fs::read_to_string(root.join("crates/mush-server/src/world_task.rs"))
        .expect("a readable source file");
    let stripped = scan::blank_cfg_test_items(&scan::strip_comments_and_literals(&source));
    assert_eq!(
        stripped
            .matches("tacsim::space::thinks::run_on_the_wall_clock(")
            .count(),
        1,
        "the server's world loop tells the space minds it runs on the wall clock, once"
    );
}
