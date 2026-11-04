//! **The fiat budget** — test-side state written by fiat (doc 43 cut TS.6,
//! D-43-9).
//!
//! A test that writes a module's state slot directly (`module_state_mut`)
//! proves the module from a state no verb produced. Sometimes that is the
//! point: a seed no verb can reach, named by an as-built section that says so.
//! More often it is a shortcut past the seam the test should cross, which is
//! how a module proves itself while nothing proves the wiring (doc 43 §5.2).
//! `state-slot-owners.toml` registers the cross-crate writes; same-crate
//! test writes were outside every register. This is their ratchet.
//!
//! ## What is counted
//!
//! The unchecked slot accessors — `module_state_mut`, `module_state_pair_mut`,
//! `get_pair_mut`, `get_captured_pair_mut`, as whole words, so a
//! path-qualified spelling (`World::module_state_mut(&mut w)`) counts the same
//! as a method call — in every `.rs` file under `crates/<pkg>/tests/`, after
//! comments and literals are blanked. The checked `state_mut`/`try_state_mut`
//! are not counted: they go through the owner check. Inline `#[cfg(test)]`
//! modules in `src/` are not counted either; they are rare and in-crate unit
//! tests of the slot's own owner.
//!
//! ## The budgets
//!
//! One `fiat_budget/<crate>-tests.txt` per crate with any site, one line per
//! file: `<budget> <low-water> <path>`, the path relative to the crate. The
//! gate is equality: a file's count must be its budget, so a removal is
//! re-blessed down (and stays down), and the low-water mark is the least the
//! file has held. A budget above its low-water mark, a new file included
//! (low-water 0), must carry `# raised <path> to <n>: <reason>`, and the
//! reason must name the as-built section (`§`) that allows the seed.
//! Regenerate with:
//!
//! ```text
//! FIAT_BUDGET_BLESS=1 cargo test -p mush-contract-tests --test ci_policy fiat_budget::
//! ```
//!
//! and raise a file by adding `FIAT_BUDGET_RAISE="<path>: <reason naming §…>"`.
//! The prefer-instead for the common case is `TestWorld::exec_ok`: drive the
//! state through the verb that makes it.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use mush_contract_tests::scan;

/// The unchecked slot accessors.
const NEEDLES: [&str; 4] = [
    "module_state_mut",
    "module_state_pair_mut",
    "get_pair_mut",
    "get_captured_pair_mut",
];

const BUDGET_DIR: &str = "fiat_budget";
const BLESS_VAR: &str = "FIAT_BUDGET_BLESS";
const RAISE_VAR: &str = "FIAT_BUDGET_RAISE";

/// Whole-word occurrences of `needle` in `code`.
fn word_count(code: &str, needle: &str) -> usize {
    let bytes = code.as_bytes();
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    code.match_indices(needle)
        .filter(|(at, _)| {
            let before = at.checked_sub(1).and_then(|i| bytes.get(i)).copied();
            let after = bytes.get(at + needle.len()).copied();
            !before.is_some_and(ident) && !after.is_some_and(ident)
        })
        .count()
}

/// The fiat sites in one test source file.
fn fiat_sites(source: &str) -> usize {
    let code = scan::strip_comments_and_literals(source);
    NEEDLES.iter().map(|n| word_count(&code, n)).sum()
}

/// Every crate's test files with at least one site: crate → (path → count).
fn measure(root: &Path) -> BTreeMap<String, BTreeMap<String, usize>> {
    let mut measured = BTreeMap::new();
    for dir in scan::crate_dirs(root) {
        let Some(name) = dir.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        let tests = dir.join("tests");
        if !tests.is_dir() {
            continue;
        }
        let mut files = BTreeMap::new();
        for path in scan::rust_files(&tests) {
            let Ok(source) = fs::read_to_string(&path) else {
                continue;
            };
            let n = fiat_sites(&source);
            if n > 0 {
                files.insert(scan::relative(&dir, &path), n);
            }
        }
        if !files.is_empty() {
            measured.insert(name, files);
        }
    }
    measured
}

/// One file's row.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    budget: usize,
    low_water: usize,
}

/// One parsed `fiat_budget/<crate>-tests.txt`.
#[derive(Debug, Default)]
struct Budget {
    rows: BTreeMap<String, Row>,
    raises: BTreeMap<String, (usize, String)>,
}

fn parse(text: &str) -> Result<Budget, String> {
    let mut b = Budget::default();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(comment) = line.strip_prefix('#') {
            if let Some(rest) = comment.trim().strip_prefix("raised ") {
                let (head, reason) = rest
                    .split_once(':')
                    .ok_or("a raise reads `# raised <path> to <n>: <reason>`")?;
                let (path, to) = head
                    .rsplit_once(" to ")
                    .ok_or("a raise reads `# raised <path> to <n>: <reason>`")?;
                let to = to
                    .trim()
                    .parse()
                    .map_err(|e| format!("raise `{head}`: {e}"))?;
                b.raises
                    .insert(path.trim().to_owned(), (to, reason.trim().to_owned()));
            }
            continue;
        }
        let mut words = line.split_whitespace();
        let (Some(budget), Some(low), Some(path), None) =
            (words.next(), words.next(), words.next(), words.next())
        else {
            return Err(format!("`{line}` is not `<budget> <low-water> <path>`"));
        };
        let row = Row {
            budget: budget.parse().map_err(|e| format!("`{line}`: {e}"))?,
            low_water: low.parse().map_err(|e| format!("`{line}`: {e}"))?,
        };
        if row.low_water > row.budget {
            return Err(format!("`{line}`: the low-water mark is above the budget"));
        }
        b.rows.insert(path.to_owned(), row);
    }
    Ok(b)
}

fn render(crate_name: &str, b: &Budget) -> String {
    let mut text = format!(
        "# {crate_name} — test-side fiat budget (doc 43 cut TS.6).\n\
         #\n\
         # `<budget> <low-water> <path>`: each test file's unchecked state-slot writes.\n\
         # Regenerate with:\n\
         #   {BLESS_VAR}=1 cargo test -p mush-contract-tests --test ci_policy fiat_budget::\n\
         # A raise names the as-built section that allows the seed.\n"
    );
    for (path, (to, reason)) in &b.raises {
        text.push_str(&format!("# raised {path} to {to}: {reason}\n"));
    }
    for (path, row) in &b.rows {
        text.push_str(&format!("{} {} {path}\n", row.budget, row.low_water));
    }
    text
}

fn budget_path(root: &Path, crate_name: &str) -> std::path::PathBuf {
    root.join(BUDGET_DIR)
        .join(format!("{crate_name}-tests.txt"))
}

fn read_all(root: &Path) -> BTreeMap<String, Budget> {
    let mut all = BTreeMap::new();
    let Ok(entries) = fs::read_dir(root.join(BUDGET_DIR)) else {
        return all;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(crate_name) = name.strip_suffix("-tests.txt") else {
            continue;
        };
        let text = fs::read_to_string(entry.path()).unwrap_or_default();
        let budget = parse(&text).unwrap_or_else(|e| panic!("{BUDGET_DIR}/{name}: {e}"));
        all.insert(crate_name.to_owned(), budget);
    }
    all
}

/// Every way the measured tree and the budgets disagree.
fn violations(
    measured: &BTreeMap<String, BTreeMap<String, usize>>,
    budgets: &BTreeMap<String, Budget>,
) -> Vec<String> {
    let mut out = Vec::new();
    let empty = Budget::default();
    let names: std::collections::BTreeSet<&String> =
        measured.keys().chain(budgets.keys()).collect();
    for name in names {
        let files = measured.get(name).cloned().unwrap_or_default();
        let budget = budgets.get(name).unwrap_or(&empty);
        for (path, &actual) in &files {
            let row = budget.rows.get(path).cloned().unwrap_or(Row {
                budget: 0,
                low_water: 0,
            });
            if actual != row.budget {
                out.push(format!(
                    "{name}/{path}: {actual} fiat writes against a budget of {} ({})",
                    row.budget,
                    if actual > row.budget {
                        "drive the state through a verb (`TestWorld::exec_ok`), or raise it with a reason naming the as-built section"
                    } else {
                        "re-bless to lower it"
                    }
                ));
            }
        }
        for (path, row) in &budget.rows {
            if !files.contains_key(path) && row.budget > 0 {
                out.push(format!(
                    "{name}/{path}: a budget of {} for a file with none (re-bless)",
                    row.budget
                ));
            }
            if row.budget > row.low_water {
                match budget.raises.get(path) {
                    Some((to, reason)) if *to == row.budget && reason.contains('§') => {}
                    Some((to, _)) if *to != row.budget => out.push(format!(
                        "{name}/{path}: raised to {to}, but the budget is {}",
                        row.budget
                    )),
                    Some(_) => out.push(format!(
                        "{name}/{path}: the raise's reason names no as-built section (`§`)"
                    )),
                    None => out.push(format!(
                        "{name}/{path}: a budget of {} above its low-water {} carries no `# raised` line",
                        row.budget, row.low_water
                    )),
                }
            }
        }
    }
    out
}

/// Re-bless: lower every budget to its count, raise only where `FIAT_BUDGET_RAISE`
/// names the path and a `§`.
fn bless(
    root: &Path,
    measured: &BTreeMap<String, BTreeMap<String, usize>>,
    budgets: &BTreeMap<String, Budget>,
) {
    let raise = std::env::var(RAISE_VAR).ok();
    let raise = raise
        .as_deref()
        .and_then(|r| r.split_once(':'))
        .map(|(p, r)| (p.trim(), r.trim()));
    let names: std::collections::BTreeSet<&String> =
        measured.keys().chain(budgets.keys()).collect();
    for name in names {
        let files = measured.get(name).cloned().unwrap_or_default();
        let old = budgets.get(name);
        let mut next = Budget::default();
        for (path, actual) in files {
            let was = old.and_then(|b| b.rows.get(&path)).cloned();
            let low = was.as_ref().map_or(0, |r| r.low_water).min(actual);
            if actual > was.as_ref().map_or(0, |r| r.budget) {
                let carried = old
                    .and_then(|b| b.raises.get(&path))
                    .filter(|(to, _)| *to == actual);
                let reason = match (raise, carried) {
                    (Some((p, reason)), _) if p == path && reason.contains('§') => {
                        reason.to_owned()
                    }
                    (_, Some((_, reason))) => reason.clone(),
                    _ => panic!(
                        "{name}/{path} rises to {actual}: set {RAISE_VAR}=\"{path}: <reason naming the as-built §>\""
                    ),
                };
                next.raises.insert(path.clone(), (actual, reason));
            } else if actual > low
                && let Some(kept) = old.and_then(|b| b.raises.get(&path))
            {
                next.raises.insert(path.clone(), (actual, kept.1.clone()));
            }
            next.rows.insert(
                path,
                Row {
                    budget: actual,
                    low_water: low,
                },
            );
        }
        let file = budget_path(root, name);
        if next.rows.is_empty() {
            let _ = fs::remove_file(file);
        } else {
            fs::create_dir_all(root.join(BUDGET_DIR)).expect("the budget directory");
            fs::write(file, render(name, &next)).expect("a budget file");
        }
    }
}

#[test]
fn no_test_file_writes_more_state_by_fiat_than_its_budget() {
    let root = scan::repo_root();
    let measured = measure(&root);
    let budgets = read_all(&root);
    if std::env::var_os(BLESS_VAR).is_some() {
        bless(&root, &measured, &budgets);
        return;
    }
    let problems = violations(&measured, &budgets);
    assert!(
        problems.is_empty(),
        "the fiat budget (doc 43 cut TS.6):\n  {}\n\nRegenerate: {BLESS_VAR}=1 cargo test -p mush-contract-tests --test ci_policy fiat_budget::",
        problems.join("\n  ")
    );
}

#[test]
fn the_count_sees_every_spelling_and_nothing_in_prose() {
    let source = r#"
        // module_state_mut in a comment is not a write
        let s = "module_state_mut";
        w.module_state_mut::<A>();
        World::module_state_mut::<B>(&mut w);
        w.module_state_pair_mut::<A, B>();
        m.get_pair_mut::<A, B>();
        m.get_captured_pair_mut::<A, B>();
        w.module_state_mut_checked();
        ctx.state_mut::<A>();
    "#;
    assert_eq!(fiat_sites(source), 5);
}

#[test]
fn a_raise_must_name_its_section_and_match_its_budget() {
    let measured = BTreeMap::from([(
        "c".to_owned(),
        BTreeMap::from([("tests/a.rs".to_owned(), 3)]),
    )]);
    let ok = parse("# raised tests/a.rs to 3: §4.2 seeds the ring\n3 1 tests/a.rs\n").unwrap();
    assert!(violations(&measured, &BTreeMap::from([("c".to_owned(), ok)])).is_empty());
    for (text, want) in [
        ("3 1 tests/a.rs\n", "no `# raised` line"),
        (
            "# raised tests/a.rs to 3: because\n3 1 tests/a.rs\n",
            "names no as-built section",
        ),
        (
            "# raised tests/a.rs to 2: §1\n3 1 tests/a.rs\n",
            "raised to 2",
        ),
        ("2 2 tests/a.rs\n", "against a budget of 2"),
        ("4 4 tests/a.rs\n", "re-bless to lower"),
    ] {
        let b = parse(text).unwrap();
        let v = violations(&measured, &BTreeMap::from([("c".to_owned(), b)]));
        assert!(v.iter().any(|p| p.contains(want)), "{text:?}: {v:?}");
    }
    // A file the budget has never seen is a raise from zero.
    let v = violations(&measured, &BTreeMap::new());
    assert!(
        v.iter().any(|p| p.contains("against a budget of 0")),
        "{v:?}"
    );
}
