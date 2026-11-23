//! **The known flaky tests and the one retry** (doc 51 unit FL.2, ruling
//! Q-51-5).
//!
//! Until this gate `.config/nextest.toml` gave one retry to a hand-kept
//! filter of 23 entries, most of them whole modules. Eighteen matched no
//! failure on record, twelve of the seventeen known flakes were not on it,
//! and a pass on retry was printed in 120 green runs before anyone read one
//! (doc 51 §4.3). The policy since: no blanket retry; a test is retried once
//! only while `flaky-known.toml` has a row for it; a row names its class and
//! its row of `docs/flaky-tests.md`, and lives fourteen days.
//!
//! ## What fails here
//!
//! - **An expired row.** The test is fixed and the row deleted, or the owner
//!   moves the date, at most fourteen days ahead, in a diff he sees.
//! - **A row that names nothing**: a test, module or binary the tree no
//!   longer holds. A renamed test would otherwise lose its retry silently and
//!   keep its row for ever.
//! - **A retry the list does not generate.** The retry override in
//!   `.config/nextest.toml` is the generated block between the two marker
//!   lines, exactly as `mush_contract_tests::flaky_known::block` prints it.
//!   A `retries` key anywhere else in the file, in any profile, is refused
//!   (`retries = 0`, which the `soak` profile says aloud, is not a retry).
//! - **A row out of shape**: no class, no row of the record, a row of the
//!   record that does not name the test, a classed row wider than one test.
//!
//! ## Changing the list
//!
//! Edit `crates/mush-contract-tests/flaky-known.toml`, then
//!
//! ```text
//! FLAKY_KNOWN_BLESS=1 cargo test -p mush-contract-tests --test ci_policy flaky_known::
//! ```
//!
//! rewrites the block. `python3 scripts/flake_report.py` reads the same list
//! and says which listed tests were seen failing, and which failing tests are
//! not listed.

use std::fs;

use mush_contract_tests::flaky_known::{
    self, LIFE_DAYS, LIST_PATH, NEXTEST_PATH, Row, block, date_of, day_number, list_problems,
    retry_problems, splice,
};
use mush_contract_tests::scan;

const BLESS_VAR: &str = "FLAKY_KNOWN_BLESS";

fn read(path: &str) -> String {
    let full = scan::repo_root().join(path);
    fs::read_to_string(&full).unwrap_or_else(|error| panic!("{}: {error}", full.display()))
}

fn the_list() -> Vec<Row> {
    flaky_known::parse(&read(LIST_PATH)).unwrap_or_else(|why| panic!("{why}"))
}

#[test]
fn every_row_is_live_classed_and_names_a_test_that_exists() {
    let root = scan::repo_root();
    let problems = list_problems(&root, &the_list(), flaky_known::today());
    assert!(
        problems.is_empty(),
        "{LIST_PATH} (doc 51 §4.3; the file's header says what a row is):\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn the_retry_filter_is_exactly_what_the_list_generates() {
    let rows = the_list();
    let nextest = read(NEXTEST_PATH);
    if std::env::var_os(BLESS_VAR).is_some() {
        let blessed = splice(&nextest, &rows).unwrap_or_else(|why| panic!("{why}"));
        if blessed != nextest.replace("\r\n", "\n") {
            let path = scan::repo_root().join(NEXTEST_PATH);
            fs::write(&path, &blessed)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            println!("{NEXTEST_PATH}: the generated block rewritten from {LIST_PATH}");
        }
        return;
    }
    let problems = retry_problems(&nextest, &rows);
    assert!(
        problems.is_empty(),
        "{NEXTEST_PATH} retries something {LIST_PATH} does not generate (doc 51 §4.3: no \
         blanket retry, one retry for a listed test):\n  {}",
        problems.join("\n  ")
    );
}

// The gate's own proofs: each failure the module doc promises, on a doctored
// copy of the real list or the real configuration.

const TODAY: &str = "2026-11-23";

fn doctored(edit: impl FnOnce(&mut Vec<Row>)) -> Vec<String> {
    let mut rows = the_list();
    // Every row live on the fixed day, whatever the real dates are by now.
    for row in &mut rows {
        row.listed = TODAY.to_string();
        row.expires = date_of(day(TODAY) + LIFE_DAYS);
    }
    edit(&mut rows);
    list_problems(&scan::repo_root(), &rows, day(TODAY))
}

fn day(date: &str) -> i64 {
    day_number(date).unwrap_or_else(|why| panic!("{why}"))
}

fn first(rows: &mut [Row]) -> &mut Row {
    rows.first_mut().expect("the list has a row to doctor")
}

#[track_caller]
fn assert_one(problems: &[String], needle: &str) {
    assert!(
        problems.len() == 1 && problems.iter().any(|p| p.contains(needle)),
        "expected one problem holding {needle:?}, got {problems:#?}"
    );
}

#[test]
fn the_list_as_seeded_is_clean_on_a_fixed_day() {
    assert_eq!(doctored(|_| {}), Vec::<String>::new());
}

#[test]
fn an_expired_row_is_refused() {
    let problems = doctored(|rows| {
        first(rows).listed = "2026-11-08".to_string();
        first(rows).expires = "2026-11-22".to_string();
    });
    assert_one(&problems, "EXPIRED on 2026-11-22");
    // The last day of a row is a day it holds.
    assert_eq!(
        doctored(|rows| first(rows).expires = TODAY.to_string()),
        Vec::<String>::new()
    );
}

#[test]
fn an_expiry_further_out_than_the_life_of_a_row_is_refused() {
    let problems = doctored(|rows| first(rows).expires = "2026-12-08".to_string());
    assert_one(&problems, "more than 14 days after today");
}

#[test]
fn a_row_naming_a_test_that_no_longer_exists_is_refused() {
    let problems = doctored(|rows| {
        let row = first(rows);
        row.test = row.test.as_ref().map(|name| format!("{name}_renamed"));
    });
    assert!(
        problems
            .iter()
            .any(|p| p.contains("names nothing in the tree: no test")),
        "{problems:#?}"
    );
    let problems = doctored(|rows| {
        let carried = rows
            .iter_mut()
            .find(|row| row.module.is_some())
            .expect("the seed carries a module row");
        carried.module = Some("a_module_that_was_deleted".to_string());
    });
    assert_one(&problems, "no module `a_module_that_was_deleted`");
    let problems = doctored(|rows| {
        let carried = rows
            .iter_mut()
            .find(|row| row.whole_binary)
            .expect("the seed carries a whole-binary row");
        carried.binary = "mush-worldgen::a_target_that_was_deleted".to_string();
    });
    assert_one(&problems, "names nothing in the tree: no crates");
}

#[test]
fn a_row_with_no_class_or_no_row_of_the_record_is_refused() {
    let problems = doctored(|rows| first(rows).class = "load".to_string());
    assert_one(&problems, "class `load` is not one of");
    let problems = doctored(|rows| first(rows).row = None);
    assert_one(&problems, "names its row of docs/flaky-tests.md");
    let problems = doctored(|rows| first(rows).row = Some(2));
    assert_one(&problems, "row 2 of docs/flaky-tests.md does not name");
    let problems = doctored(|rows| first(rows).row = Some(999));
    assert_one(&problems, "has no row 999");
    // A classed row is one test: a module is narrowed to the tests that flaked.
    let problems = doctored(|rows| {
        let carried = rows
            .iter_mut()
            .find(|row| row.module.is_some())
            .expect("the seed carries a module row");
        carried.class = "clock".to_string();
        carried.carried = None;
    });
    assert_one(&problems, "a classed row names one test");
}

#[test]
fn a_retry_the_list_does_not_generate_is_refused() {
    if std::env::var_os(BLESS_VAR).is_some() {
        // A bless run rewrites the file this test reads, beside it.
        return;
    }
    let rows = the_list();
    let nextest = read(NEXTEST_PATH);
    assert_eq!(retry_problems(&nextest, &rows), Vec::<String>::new());

    // A hand-written retry override, in any profile.
    let by_hand = format!(
        "{nextest}\n[[profile.ci.overrides]]\nfilter = 'binary_id(mushd::sim_lenses)'\nretries = 1\n"
    );
    assert_one(
        &retry_problems(&by_hand, &rows),
        "[[profile.ci.overrides]] #0 asks for a retry outside the generated block",
    );

    // A blanket retry.
    let blanket = nextest.replace("[profile.ci]\n", "[profile.ci]\nretries = 1\n");
    assert_ne!(blanket, nextest, "the ci profile's header moved");
    assert_one(
        &retry_problems(&blanket, &rows),
        "[profile.ci] sets `retries`",
    );

    // A row added to the list and the block left as it was.
    let mut more = rows.clone();
    let mut extra = first(&mut more).clone();
    extra.test = Some("a_test_added_to_the_list_only".to_string());
    more.push(extra);
    assert_one(
        &retry_problems(&nextest, &more),
        "is not what crates/mush-contract-tests/flaky-known.toml generates",
    );

    // The block edited by hand: two retries where the list gives one.
    let widened = nextest.replacen("\nretries = 1\n", "\nretries = 2\n", 1);
    assert_ne!(
        widened, nextest,
        "the generated block has no `retries = 1` line"
    );
    assert_one(
        &retry_problems(&widened, &rows),
        "is not what crates/mush-contract-tests/flaky-known.toml generates",
    );

    // Zero is not a retry: the soak profile says it aloud.
    let zero = format!("{nextest}\n[[profile.ci.overrides]]\nfilter = 'all()'\nretries = 0\n");
    assert_eq!(retry_problems(&zero, &rows), Vec::<String>::new());
}

#[test]
fn the_generated_block_is_one_term_a_row_and_one_retry() {
    let rows = the_list();
    let text = block(&rows);
    assert_eq!(text.matches("binary_id(").count(), rows.len());
    assert_eq!(text.matches("retries = 1").count(), 1);
    assert!(block(&[]).lines().all(|line| line.starts_with('#')));
}

#[test]
fn dates_round_trip_and_bad_ones_are_refused() {
    assert_eq!(day("1970-01-01"), 0);
    assert_eq!(day("2026-12-07") - day("2026-11-23"), LIFE_DAYS);
    assert_eq!(day("2028-03-01") - day("2028-02-28"), 2);
    assert_eq!(date_of(day("2026-11-23") + LIFE_DAYS), "2026-12-07");
    for bad in ["2026-11-31", "2026-13-01", "26-11-23", "2026/11/23", "soon"] {
        assert!(day_number(bad).is_err(), "{bad} parsed as a date");
    }
}
