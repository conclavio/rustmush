//! **The wait budget** — raw waits and wall-clock reads in story files (doc 51
//! cut FL.1, §3.9, Q-51-6).
//!
//! A story is a test that drives a real server. When it sleeps to let the
//! server get somewhere, or bounds a read with a span someone chose at the
//! call, it passes on a quiet box and fails on a loaded one; when it reads the
//! wall clock to place itself, its result follows the minute the run started.
//! Doc 51 §2 counted these: 92 percent of the wall-clock waits in the tree are
//! written in a story file, not in a rig, and a wait in a story is a wait
//! someone copied. The shared helpers live in `mush-storykit`. This gate is
//! what stops the old shape from coming back while the suites move onto them:
//! the counts may only fall.
//!
//! ## What is counted
//!
//! Every `.rs` file under `crates/<pkg>/tests/`, after comments and literals
//! are blanked, by three counters:
//!
//! - `wait`, **a raw wait**: a call of `sleep(`, `sleep_until(`, `timeout(`,
//!   `timeout_at(`, `recv_timeout(` or `wait_timeout(`, by any path. A method
//!   call `.timeout(` is a client's configuration, not a wait, and is not
//!   counted.
//! - `clock`, **a wall-clock read**: `unix_now_s(`, `unix_now_ms(` or
//!   `SystemTime::now(`.
//! - `span`, **a literal span passed to a wait**: a stub that counts nothing
//!   until doc 51's unit FL.9 baselines it, when the kit has a helper to move
//!   each one to. The budget files already read the word.
//!
//! What is not counted:
//!
//! - **a rig file**, where the primitives belong: a path with `/support/` in
//!   it, a file named `harness.rs`, and the kit itself;
//! - **a site inside a `start_paused` test**, which drives its own clock. The
//!   test is the function the site is written in: the attribute is read from
//!   that function's own header, so a helper that only paused tests call is
//!   still counted.
//!
//! ## The `src` scope
//!
//! For the crates in [`SRC_SCOPE`] (doc 51 unit FL.6: mush-server) the gate
//! also counts the **test code under `src/`**, where a crate's in-process
//! tests of its own loop live. Test code is a file whose path has a `tests`
//! directory in it, a file named `tests.rs` or `*_tests.rs`, a file fenced by
//! a leading `#![cfg(test)]`, and in any other file every `#[cfg(test)]` item,
//! brace-counted from the attribute to its close. Production code is never
//! counted: a retry loop's sleep in the store writer is not a test's wait.
//! The rows read `src/<path>`, beside the `tests/<path>` rows.
//!
//! A crate enters the scope with a baseline: its first bless writes its
//! `src/` counts as their own low-water marks and the line `# scope src`.
//! From then on a `src/` row rises only as a `tests/` row does, with a reason.
//!
//! `scripts/wait_audit.py --budget` prints the same counts from the same
//! rules, one row a file. The two must agree; doc 51's cut note for FL.1
//! records both at the baseline.
//!
//! ## The budgets
//!
//! One `wait_budget/<crate>.txt` per crate with any site, one line per file
//! and counter: `<counter> <budget> <low-water> <path>`, the path relative to
//! the crate. The gate is equality, as the fiat budget's is: a file's count
//! must be its budget, so a removal is re-blessed down (and stays down), and
//! the low-water mark is the least the file has held. A budget above its
//! low-water mark, a new file included (low-water 0), must carry
//! `# raised <counter> <path> to <n>: <reason>` with a written reason.
//! Regenerate with:
//!
//! ```text
//! WAIT_BUDGET_BLESS=1 cargo test -p mush-contract-tests --test ci_policy wait_budget::
//! ```
//!
//! and raise a file by adding
//! `WAIT_BUDGET_RAISE="<counter> <path>: <why the kit cannot serve>"`.
//! The prefer-instead is `mush_storykit::wait_until` under `STEP_TIMEOUT` or a
//! `StoryBudget`, and a rig that boots at a fixed instant.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::time::Duration;

use mush_contract_tests::scan;

/// The raw wait primitives (doc 51 §2.1).
const WAIT_NEEDLES: [&str; 6] = [
    "sleep",
    "sleep_until",
    "timeout",
    "timeout_at",
    "recv_timeout",
    "wait_timeout",
];

/// The wall-clock reads that are one identifier and a call.
const CLOCK_NEEDLES: [&str; 2] = ["unix_now_s", "unix_now_ms"];

/// The crate that is a rig whole: the kit, where the primitives belong.
const KIT_CRATE: &str = "mush-storykit";

/// The crates whose test code under `src/` is counted too (the `src` scope).
const SRC_SCOPE: [&str; 1] = ["mush-server"];

/// The line a budget file carries once its crate's `src` scope is baselined.
const SCOPE_LINE: &str = "scope src";

const BUDGET_DIR: &str = "wait_budget";
const BLESS_VAR: &str = "WAIT_BUDGET_BLESS";
const RAISE_VAR: &str = "WAIT_BUDGET_RAISE";

/// What a row counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Counter {
    /// A raw wait.
    Wait,
    /// A wall-clock read.
    Clock,
    /// A literal span passed to a wait: stubbed until doc 51's unit FL.9.
    Span,
}

impl Counter {
    const ALL: [Counter; 3] = [Counter::Wait, Counter::Clock, Counter::Span];

    /// The word a budget file writes.
    fn word(self) -> &'static str {
        match self {
            Counter::Wait => "wait",
            Counter::Clock => "clock",
            Counter::Span => "span",
        }
    }

    fn parse(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.word() == word)
    }

    /// `n` of what the counter counts, for a message.
    fn count(self, n: usize) -> String {
        let (one, many) = match self {
            Counter::Wait => ("raw wait", "raw waits"),
            Counter::Clock => ("wall-clock read", "wall-clock reads"),
            Counter::Span => (
                "literal span passed to a wait",
                "literal spans passed to a wait",
            ),
        };
        format!("{n} {}", if n == 1 { one } else { many })
    }

    /// What to do instead of raising it: one sentence, with no full stop.
    fn instead(self) -> &'static str {
        match self {
            Counter::Wait => {
                "Wait through the story kit (`mush_storykit::wait_until` under `STEP_TIMEOUT`, \
                 or a `StoryBudget`), not through a raw sleep or timeout in a story"
            }
            Counter::Clock => {
                "A story does not place itself by the wall clock (doc 51 rule T): boot the rig \
                 at a fixed instant"
            }
            Counter::Span => "Derive the span from the figure it restates (doc 51 rule F)",
        }
    }
}

// ---------------------------------------------------------------------------
// The scan
// ---------------------------------------------------------------------------

/// One file's counts.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Sites {
    wait: usize,
    clock: usize,
}

impl Sites {
    fn of(self, counter: Counter) -> usize {
        match counter {
            Counter::Wait => self.wait,
            Counter::Clock => self.clock,
            // The stub: nothing is counted until FL.9 baselines it.
            Counter::Span => 0,
        }
    }
}

fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// The index of the first byte at or after `at` that is not whitespace.
fn skip_space(bytes: &[u8], mut at: usize) -> usize {
    while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
        at += 1;
    }
    at
}

/// Whether a block's header (the text since the last `{`, `}` or `;`) opens a
/// function, and whether that function is a `start_paused` test. The
/// attribute sits before `fn` with no `;` between, so it is in the header.
fn function_header(header: &str) -> Option<bool> {
    let bytes = header.as_bytes();
    let opens_fn = header.match_indices("fn").any(|(at, _)| {
        let before = at.checked_sub(1).and_then(|i| bytes.get(i)).copied();
        let name = skip_space(bytes, at + 2);
        !before.is_some_and(is_ident)
            && name > at + 2
            && bytes.get(name).copied().is_some_and(is_ident)
    });
    opens_fn.then(|| header.contains("start_paused"))
}

/// The raw waits and wall-clock reads in one test source file, outside any
/// `start_paused` test.
fn story_sites(source: &str) -> Sites {
    let code = scan::strip_comments_and_literals(source);
    let bytes = code.as_bytes();
    let mut sites = Sites::default();
    // One entry a `{`: `Some(paused)` for a function's body, `None` for any
    // other block. A site belongs to the innermost function around it.
    let mut blocks: Vec<Option<bool>> = Vec::new();
    let mut header_start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => {
                blocks.push(function_header(&code[header_start..i]));
                header_start = i + 1;
            }
            b'}' => {
                blocks.pop();
                header_start = i + 1;
            }
            b';' => header_start = i + 1,
            _ => {}
        }
        let starts_word = is_ident(bytes[i]) && !(i > 0 && is_ident(bytes[i - 1]));
        if !starts_word {
            i += 1;
            continue;
        }
        let mut end = i;
        while bytes.get(end).copied().is_some_and(is_ident) {
            end += 1;
        }
        let word = &code[i..end];
        let after = skip_space(bytes, end);
        let called = bytes.get(after) == Some(&b'(');
        let paused = blocks.iter().rev().find_map(|block| *block) == Some(true);
        if called && WAIT_NEEDLES.contains(&word) {
            // `.timeout(` configures a client; it does not wait.
            let configures = word == "timeout" && i > 0 && bytes[i - 1] == b'.';
            if !configures && !paused {
                sites.wait += 1;
            }
        } else if called && CLOCK_NEEDLES.contains(&word) {
            if !paused {
                sites.clock += 1;
            }
        } else if word == "SystemTime" && code[after..].starts_with("::") {
            // `SystemTime::now(`, however it is spaced.
            let method = skip_space(bytes, after + 2);
            let open = skip_space(bytes, method + 3);
            let now = code[method..].starts_with("now")
                && !bytes.get(method + 3).copied().is_some_and(is_ident);
            if now && bytes.get(open) == Some(&b'(') && !paused {
                sites.clock += 1;
            }
        }
        i = end;
    }
    sites
}

/// The test code of one file under a crate's `src/`, `rel` being its path
/// relative to the crate: the whole file (comments and literals blanked) when
/// the file is test code whole, else its `#[cfg(test)]` items with everything
/// else blanked, or `None` when it holds no test code.
fn src_test_code(rel: &str, source: &str) -> Option<String> {
    let stripped = scan::strip_comments_and_literals(source);
    let parts: Vec<&str> = rel.split('/').collect();
    let name = parts.last().copied().unwrap_or(rel);
    let dirs = parts.get(1..parts.len().saturating_sub(1)).unwrap_or(&[]);
    if dirs.contains(&"tests")
        || name == "tests.rs"
        || name.ends_with("_tests.rs")
        || scan::file_is_test_only(&stripped)
    {
        return Some(stripped);
    }
    let blanked = scan::blank_cfg_test_items(&stripped);
    if blanked == stripped {
        return None;
    }
    let kept: Vec<u8> = stripped
        .bytes()
        .zip(blanked.bytes())
        .map(|(was, now)| {
            if was != now || was == b'\n' {
                was
            } else {
                b' '
            }
        })
        .collect();
    Some(String::from_utf8_lossy(&kept).into_owned())
}

/// Whether `path` (relative to its crate, forward slashes) is a rig file.
fn is_rig(path: &str) -> bool {
    path.contains("/support/") || path.ends_with("/harness.rs")
}

/// One crate's rows: (path, counter) → count, for every count above zero.
type Counts = BTreeMap<(String, Counter), usize>;

/// Every crate's story files with at least one site.
fn measure(root: &Path) -> BTreeMap<String, Counts> {
    let mut measured = BTreeMap::new();
    for dir in scan::crate_dirs(root) {
        let Some(name) = dir.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        if name == KIT_CRATE {
            continue;
        }
        let mut rows = Counts::new();
        let mut count = |rel: String, code: &str| {
            let sites = story_sites(code);
            for counter in Counter::ALL {
                let n = sites.of(counter);
                if n > 0 {
                    rows.insert((rel.clone(), counter), n);
                }
            }
        };
        let tests = dir.join("tests");
        if tests.is_dir() {
            for path in scan::rust_files(&tests) {
                let rel = scan::relative(&dir, &path);
                if is_rig(&rel) {
                    continue;
                }
                let Ok(source) = fs::read_to_string(&path) else {
                    continue;
                };
                count(rel, &source);
            }
        }
        let src = dir.join("src");
        if SRC_SCOPE.contains(&name.as_str()) && src.is_dir() {
            for path in scan::rust_files(&src) {
                let rel = scan::relative(&dir, &path);
                let Ok(source) = fs::read_to_string(&path) else {
                    continue;
                };
                if let Some(code) = src_test_code(&rel, &source) {
                    count(rel, &code);
                }
            }
        }
        if !rows.is_empty() {
            measured.insert(name, rows);
        }
    }
    measured
}

/// The sum of one counter over a crate's rows.
fn total(rows: &Counts, counter: Counter) -> usize {
    rows.iter()
        .filter(|((_, c), _)| *c == counter)
        .map(|(_, n)| n)
        .sum()
}

// ---------------------------------------------------------------------------
// The budget files
// ---------------------------------------------------------------------------

/// One file's row for one counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Row {
    budget: usize,
    low_water: usize,
}

/// One parsed `wait_budget/<crate>.txt`.
#[derive(Debug, Default)]
struct Budget {
    rows: BTreeMap<(String, Counter), Row>,
    raises: BTreeMap<(String, Counter), (usize, String)>,
    /// The `src` scope is baselined: the file carries `# scope src`.
    src_scope: bool,
}

const RAISE_SHAPE: &str = "a raise reads `# raised <counter> <path> to <n>: <reason>`";

fn parse(text: &str) -> Result<Budget, String> {
    let mut b = Budget::default();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(comment) = line.strip_prefix('#') {
            if comment.trim() == SCOPE_LINE {
                b.src_scope = true;
            } else if let Some(rest) = comment.trim().strip_prefix("raised ") {
                let (head, reason) = rest.split_once(':').ok_or(RAISE_SHAPE)?;
                let (what, to) = head.rsplit_once(" to ").ok_or(RAISE_SHAPE)?;
                let (counter, path) = what.trim().split_once(' ').ok_or(RAISE_SHAPE)?;
                let counter = Counter::parse(counter)
                    .ok_or_else(|| format!("raise `{head}`: `{counter}` is no counter"))?;
                let to = to
                    .trim()
                    .parse()
                    .map_err(|e| format!("raise `{head}`: {e}"))?;
                b.raises.insert(
                    (path.trim().to_owned(), counter),
                    (to, reason.trim().to_owned()),
                );
            }
            continue;
        }
        let mut words = line.split_whitespace();
        let (Some(counter), Some(budget), Some(low), Some(path), None) = (
            words.next(),
            words.next(),
            words.next(),
            words.next(),
            words.next(),
        ) else {
            return Err(format!(
                "`{line}` is not `<counter> <budget> <low-water> <path>`"
            ));
        };
        let counter = Counter::parse(counter)
            .ok_or_else(|| format!("`{line}`: `{counter}` is no counter"))?;
        let row = Row {
            budget: budget.parse().map_err(|e| format!("`{line}`: {e}"))?,
            low_water: low.parse().map_err(|e| format!("`{line}`: {e}"))?,
        };
        if row.low_water > row.budget {
            return Err(format!("`{line}`: the low-water mark is above the budget"));
        }
        b.rows.insert((path.to_owned(), counter), row);
    }
    Ok(b)
}

fn render(crate_name: &str, b: &Budget) -> String {
    let sum = |counter: Counter| -> usize {
        b.rows
            .iter()
            .filter(|((_, c), _)| *c == counter)
            .map(|(_, row)| row.budget)
            .sum()
    };
    let mut text = format!(
        "# {crate_name} — the wait budget of its story files (doc 51 cut FL.1).\n\
         #\n\
         # `<counter> <budget> <low-water> <path>`: `wait` is a raw sleep or timeout,\n\
         # `clock` a wall-clock read. A count may only fall. Regenerate with:\n\
         #   {BLESS_VAR}=1 cargo test -p mush-contract-tests --test ci_policy wait_budget::\n\
         # A raise carries a written reason; the prefer-instead is `mush-storykit`.\n\
         #\n\
         # total: {}, {}\n",
        Counter::Wait.count(sum(Counter::Wait)),
        Counter::Clock.count(sum(Counter::Clock)),
    );
    if b.src_scope {
        text.push_str(&format!(
            "#\n# The test code under `src/` is counted too (doc 51 cut FL.6).\n# {SCOPE_LINE}\n"
        ));
    }
    for ((path, counter), (to, reason)) in &b.raises {
        text.push_str(&format!(
            "# raised {} {path} to {to}: {reason}\n",
            counter.word()
        ));
    }
    for ((path, counter), row) in &b.rows {
        text.push_str(&format!(
            "{} {} {} {path}\n",
            counter.word(),
            row.budget,
            row.low_water
        ));
    }
    text
}

fn budget_path(root: &Path, crate_name: &str) -> std::path::PathBuf {
    root.join(BUDGET_DIR).join(format!("{crate_name}.txt"))
}

fn read_all(root: &Path) -> BTreeMap<String, Budget> {
    let mut all = BTreeMap::new();
    let Ok(entries) = fs::read_dir(root.join(BUDGET_DIR)) else {
        return all;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(crate_name) = name.strip_suffix(".txt") else {
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
    measured: &BTreeMap<String, Counts>,
    budgets: &BTreeMap<String, Budget>,
) -> Vec<String> {
    let mut out = Vec::new();
    let empty = Budget::default();
    let names: BTreeSet<&String> = measured.keys().chain(budgets.keys()).collect();
    for name in names {
        let counts = measured.get(name).cloned().unwrap_or_default();
        let budget = budgets.get(name).unwrap_or(&empty);
        for (key, &actual) in &counts {
            let (path, counter) = key;
            let allowed = budget.rows.get(key).map_or(0, |row| row.budget);
            if actual > allowed {
                out.push(format!(
                    "{name}/{path}: {} against a budget of {allowed}. {}. Or raise the budget \
                     with a written reason.",
                    counter.count(actual),
                    counter.instead()
                ));
            } else if actual < allowed {
                out.push(format!(
                    "{name}/{path}: {} against a budget of {allowed}. The count fell: lower \
                     the budget to match (re-bless), and it stays down.",
                    counter.count(actual)
                ));
            }
        }
        for (key, row) in &budget.rows {
            let (path, counter) = key;
            if !counts.contains_key(key) && row.budget > 0 {
                out.push(format!(
                    "{name}/{path}: a budget of {} for a file that holds none. Lower the \
                     budget to match (re-bless).",
                    counter.count(row.budget)
                ));
            }
            if row.budget > row.low_water {
                match budget.raises.get(key) {
                    Some((to, reason)) if *to == row.budget && !reason.is_empty() => {}
                    Some((to, _)) if *to != row.budget => out.push(format!(
                        "{name}/{path}: `{}` raised to {to}, but the budget is {}",
                        counter.word(),
                        row.budget
                    )),
                    Some(_) => out.push(format!(
                        "{name}/{path}: the `{}` raise gives no reason",
                        counter.word()
                    )),
                    None => out.push(format!(
                        "{name}/{path}: a `{}` budget of {} above its low-water {} carries no \
                         `# raised` line",
                        counter.word(),
                        row.budget,
                        row.low_water
                    )),
                }
            }
        }
    }
    out
}

/// What the bless variable's raise names: (counter, path, reason).
fn parse_raise(raise: &str) -> Option<(Counter, &str, &str)> {
    let (what, reason) = raise.split_once(':')?;
    let (counter, path) = what.trim().split_once(' ')?;
    let reason = reason.trim();
    (!reason.is_empty()).then_some((Counter::parse(counter)?, path.trim(), reason))
}

/// The next budgets: every budget lowered to its count, and raised only where
/// `raise` names the row and gives a reason. `baseline` is the first bless of
/// all, with no budget file yet: every count is its own low-water mark.
fn blessed(
    measured: &BTreeMap<String, Counts>,
    budgets: &BTreeMap<String, Budget>,
    raise: Option<(Counter, &str, &str)>,
    baseline: bool,
) -> Result<BTreeMap<String, Budget>, String> {
    let mut all = BTreeMap::new();
    let names: BTreeSet<&String> = measured.keys().chain(budgets.keys()).collect();
    for name in names {
        let counts = measured.get(name).cloned().unwrap_or_default();
        let old = budgets.get(name);
        let mut next = Budget::default();
        // A crate entering the `src` scope: its `src/` rows are baselined.
        let in_scope = SRC_SCOPE.contains(&name.as_str());
        let src_baseline = in_scope && !old.is_some_and(|b| b.src_scope);
        next.src_scope = in_scope;
        for (key, actual) in counts {
            let (path, counter) = &key;
            let was = old.and_then(|b| b.rows.get(&key)).copied();
            let low = match was {
                Some(row) => row.low_water.min(actual),
                None if baseline => actual,
                None if src_baseline && path.starts_with("src/") => actual,
                None => 0,
            };
            if actual > was.map_or(low, |row| row.budget) {
                let carried = old
                    .and_then(|b| b.raises.get(&key))
                    .filter(|(to, _)| *to == actual);
                let reason = match (raise, carried) {
                    (Some((c, p, reason)), _) if c == *counter && p == path => reason.to_owned(),
                    (_, Some((_, reason))) => reason.clone(),
                    _ => {
                        return Err(format!(
                            "{name}/{path} rises to {}. {}. Or set {RAISE_VAR}=\"{} {path}: \
                             <why the kit cannot serve>\".",
                            counter.count(actual),
                            counter.instead(),
                            counter.word()
                        ));
                    }
                };
                next.raises.insert(key.clone(), (actual, reason));
            } else if actual > low
                && let Some(kept) = old.and_then(|b| b.raises.get(&key))
            {
                next.raises.insert(key.clone(), (actual, kept.1.clone()));
            }
            next.rows.insert(
                key,
                Row {
                    budget: actual,
                    low_water: low,
                },
            );
        }
        all.insert(name.clone(), next);
    }
    Ok(all)
}

/// Re-bless the budget files in place.
fn bless(root: &Path, measured: &BTreeMap<String, Counts>, budgets: &BTreeMap<String, Budget>) {
    let raise = std::env::var(RAISE_VAR).ok();
    let raise = raise.as_deref().map(|text| {
        parse_raise(text).unwrap_or_else(|| {
            panic!("{RAISE_VAR} reads `<counter> <path>: <reason>`, not `{text}`")
        })
    });
    let next =
        blessed(measured, budgets, raise, budgets.is_empty()).unwrap_or_else(|e| panic!("{e}"));
    for (name, budget) in &next {
        let file = budget_path(root, name);
        if budget.rows.is_empty() {
            let _ = fs::remove_file(file);
        } else {
            fs::create_dir_all(root.join(BUDGET_DIR)).expect("the budget directory");
            fs::write(file, render(name, budget)).expect("a budget file");
        }
    }
}

// ---------------------------------------------------------------------------
// The gates
// ---------------------------------------------------------------------------

#[test]
fn no_story_file_holds_more_raw_waits_or_clock_reads_than_its_budget() {
    let root = scan::repo_root();
    let measured = measure(&root);
    let budgets = read_all(&root);
    if std::env::var_os(BLESS_VAR).is_some() {
        bless(&root, &measured, &budgets);
        for (name, rows) in &measured {
            println!(
                "{name}: {}, {}",
                Counter::Wait.count(total(rows, Counter::Wait)),
                Counter::Clock.count(total(rows, Counter::Clock))
            );
        }
        return;
    }
    let problems = violations(&measured, &budgets);
    assert!(
        problems.is_empty(),
        "the wait budget (doc 51 §3.9): a story file's raw waits and wall-clock reads may only \
         fall.\n  {}\n\nRegenerate: {BLESS_VAR}=1 cargo test -p mush-contract-tests --test \
         ci_policy wait_budget::",
        problems.join("\n  ")
    );
}

/// `StoryBudget::new` refuses a budget the runner would end first, and it
/// knows the runner's limit as a constant. This holds the constant to
/// `.config/nextest.toml`: the `ci` profile terminates at exactly that span,
/// and no profile that terminates at all does so sooner.
#[test]
fn the_kit_knows_when_nextest_terminates_a_test() {
    let path = scan::repo_root().join(".config/nextest.toml");
    let text = fs::read_to_string(&path).expect(".config/nextest.toml is readable");
    let parsed: toml::Value = toml::from_str(&text).expect(".config/nextest.toml parses");
    let profiles = parsed
        .get("profile")
        .and_then(toml::Value::as_table)
        .expect("nextest.toml has profiles");
    let terminate = |profile: &toml::Value| -> Option<Duration> {
        let slow = profile.get("slow-timeout")?;
        let after = u32::try_from(slow.get("terminate-after")?.as_integer()?).ok()?;
        let period = slow.get("period")?.as_str()?.strip_suffix('s')?;
        Some(Duration::from_secs(period.parse().ok()?) * after)
    };
    let ci = profiles.get("ci").and_then(terminate);
    assert_eq!(
        ci,
        Some(mush_storykit::NEXTEST_TERMINATE),
        "`mush_storykit::NEXTEST_TERMINATE` is not the `ci` profile's terminate \
         (`slow-timeout`, period times `terminate-after`): move them together"
    );
    for (name, profile) in profiles {
        if let Some(at) = terminate(profile) {
            assert!(
                at >= mush_storykit::NEXTEST_TERMINATE,
                "profile `{name}` terminates a test at {at:?}, sooner than the \
                 {:?} every `StoryBudget` is sized under",
                mush_storykit::NEXTEST_TERMINATE
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The scanner's and the ratchet's own proofs
// ---------------------------------------------------------------------------

#[test]
fn the_count_sees_every_needle_and_nothing_in_prose_or_a_paused_test() {
    let source = r#"
        // tokio::time::sleep( in a comment is not a wait
        const NOTE: &str = "timeout(1) and SystemTime::now()";

        async fn a_story() {
            tokio::time::sleep(STEP).await;
            std::thread::sleep(STEP);
            sleep_until(deadline).await;
            let _ = tokio::time::timeout(STEP, read()).await;
            let _ = timeout_at (deadline, read()).await;
            let _ = rx.recv_timeout(STEP);
            let _ = cvar.wait_timeout(guard, STEP);
            let client = Client::builder()
                .timeout(STEP)
                .build();
            let client = builder.timeout(STEP);
            let asleep = sleeper(STEP);
            let limit = timeout;
            let now = mush_config::unix_now_s();
            let ms = unix_now_ms ();
            let wall = std::time::SystemTime::now();
            let spaced = SystemTime :: now ();
            let other = SystemTime::now_ish();
            let mono = Instant::now();
        }

        #[tokio::test(flavor = "current_thread", start_paused = true)]
        async fn a_paused_test_drives_its_own_clock() {
            tokio::time::sleep(STEP).await;
            let _ = SystemTime::now();
            let poll = async || {
                tokio::time::sleep(STEP).await;
            };
            async fn a_helper_written_inside_it() {
                tokio::time::sleep(STEP).await;
            }
        }

        #[tokio::test]
        async fn after_it_the_count_resumes() {
            loop {
                tokio::time::sleep(STEP).await;
            }
        }
    "#;
    assert_eq!(story_sites(source), Sites { wait: 9, clock: 4 });
}

#[test]
fn a_rig_file_is_a_path_with_support_in_it_or_a_harness() {
    for rig in [
        "tests/support/mod.rs",
        "tests/support/field/driving_rig.rs",
        "tests/split_halves/harness.rs",
        "tests/harness.rs",
    ] {
        assert!(is_rig(rig), "{rig}");
    }
    for story in [
        "tests/sim_lenses/observer_rule.rs",
        "tests/support.rs",
        "tests/split_halves/harness_story.rs",
    ] {
        assert!(!is_rig(story), "{story}");
    }
}

#[test]
fn the_src_scope_counts_test_code_and_never_production_code() {
    let mixed = r#"
        async fn retry_forever() {
            loop {
                tokio::time::sleep(BACKOFF).await;
            }
        }

        #[cfg(test)]
        mod tests {
            #[tokio::test]
            async fn a_test_of_the_loop() {
                tokio::time::sleep(STEP).await;
                let _ = mush_config::unix_now_s();
            }

            #[tokio::test(start_paused = true)]
            async fn a_paused_one() {
                tokio::time::sleep(STEP).await;
            }
        }

        fn after_the_tests() {
            std::thread::sleep(PACE);
        }
    "#;
    let code = src_test_code("src/persist.rs", mixed).expect("the file holds test code");
    assert_eq!(story_sites(&code), Sites { wait: 1, clock: 1 });

    let production = "fn pace() { std::thread::sleep(PACE); }";
    assert_eq!(src_test_code("src/lib.rs", production), None);

    // A file that is test code whole, by its path or by a leading fence.
    let whole = "async fn helper() { tokio::time::sleep(STEP).await; }";
    for rel in [
        "src/world_task/tests.rs",
        "src/world_task/submission_disposition/tests/retry.rs",
        "src/lib_tests.rs",
    ] {
        let code = src_test_code(rel, whole).expect("test code whole");
        assert_eq!(story_sites(&code), Sites { wait: 1, clock: 0 }, "{rel}");
    }
    let fenced = format!("#![cfg(test)]\n{whole}");
    assert!(src_test_code("src/probe.rs", &fenced).is_some());
    // `tests` must be a directory under `src/`, not the file's stem.
    assert_eq!(src_test_code("src/tests_of_nothing.rs", production), None);
}

#[test]
fn a_crate_entering_the_src_scope_is_baselined_once() {
    let crate_name = SRC_SCOPE[0].to_owned();
    let measured = BTreeMap::from([(
        crate_name.clone(),
        Counts::from([
            (("src/a.rs".to_owned(), Counter::Wait), 2),
            (("tests/b.rs".to_owned(), Counter::Wait), 1),
        ]),
    )]);
    let old = BTreeMap::from([(
        crate_name.clone(),
        parse("wait 1 1 tests/b.rs\n").expect("the budget parses"),
    )]);
    // The first bless in scope takes the `src/` count as its own low-water mark.
    let first = blessed(&measured, &old, None, false).expect("a baseline");
    let text = render(&crate_name, &first[&crate_name]);
    assert!(text.contains("# scope src"), "{text}");
    assert!(text.contains("wait 2 2 src/a.rs"), "{text}");
    let reread = BTreeMap::from([(crate_name.clone(), parse(&text).expect("it parses"))]);
    assert_eq!(violations(&measured, &reread), Vec::<String>::new());
    // After it, a `src/` row rises only with a reason.
    let risen = BTreeMap::from([(
        crate_name.clone(),
        Counts::from([
            (("src/a.rs".to_owned(), Counter::Wait), 2),
            (("src/new.rs".to_owned(), Counter::Wait), 1),
            (("tests/b.rs".to_owned(), Counter::Wait), 1),
        ]),
    )]);
    assert!(blessed(&risen, &reread, None, false).is_err());
    // Out of scope there is no baseline: a `src/` row is a rise from zero.
    let elsewhere = one("src/a.rs", Counter::Wait, 1);
    assert!(blessed(&elsewhere, &budgets("wait 1 1 tests/b.rs\n"), None, false).is_err());
}

fn one(path: &str, counter: Counter, n: usize) -> BTreeMap<String, Counts> {
    BTreeMap::from([(
        "c".to_owned(),
        Counts::from([((path.to_owned(), counter), n)]),
    )])
}

fn budgets(text: &str) -> BTreeMap<String, Budget> {
    BTreeMap::from([("c".to_owned(), parse(text).expect("the budget parses"))])
}

#[test]
fn a_count_may_only_fall_and_a_raise_needs_a_written_reason() {
    let measured = one("tests/a.rs", Counter::Wait, 3);
    for ok in [
        "wait 3 3 tests/a.rs\n",
        "# raised wait tests/a.rs to 3: the kit has no reader for a raw pipe yet\nwait 3 1 tests/a.rs\n",
    ] {
        assert_eq!(violations(&measured, &budgets(ok)), Vec::<String>::new());
    }
    for (text, want) in [
        // The count rose: the message names the kit.
        (
            "wait 2 2 tests/a.rs\n",
            "3 raw waits against a budget of 2. Wait through the story kit",
        ),
        // The count fell and the budget did not follow it down.
        ("wait 4 4 tests/a.rs\n", "lower the budget to match"),
        // A budget above its low-water mark with no raise, an empty one, a stale one.
        ("wait 3 1 tests/a.rs\n", "carries no `# raised` line"),
        (
            "# raised wait tests/a.rs to 3:\nwait 3 1 tests/a.rs\n",
            "gives no reason",
        ),
        (
            "# raised wait tests/a.rs to 2: once\nwait 3 1 tests/a.rs\n",
            "raised to 2",
        ),
        // A raise for the other counter does not cover this one.
        (
            "# raised clock tests/a.rs to 3: once\nwait 3 1 tests/a.rs\n",
            "carries no `# raised` line",
        ),
        // The file is gone and its row is not.
        (
            "wait 3 3 tests/a.rs\nwait 2 2 tests/gone.rs\n",
            "a budget of 2 raw waits for a file that holds none",
        ),
    ] {
        let v = violations(&measured, &budgets(text));
        assert!(v.iter().any(|p| p.contains(want)), "{text:?}: {v:?}");
    }
    // A file the budget has never seen is a rise from zero.
    let v = violations(&measured, &BTreeMap::new());
    assert!(
        v.iter().any(|p| p.contains("against a budget of 0")),
        "{v:?}"
    );
    // The clock counter says what to do in its own words.
    let v = violations(&one("tests/a.rs", Counter::Clock, 1), &BTreeMap::new());
    assert!(
        v.iter()
            .any(|p| p.contains("1 wall-clock read against") && p.contains("fixed instant")),
        "{v:?}"
    );
}

#[test]
fn a_bless_lowers_freely_and_raises_only_with_a_reason() {
    let old = budgets("wait 3 3 tests/a.rs\n");
    let row =
        |all: &BTreeMap<String, Budget>| all["c"].rows[&("tests/a.rs".to_owned(), Counter::Wait)];

    // Down: the budget and the low-water mark follow the count.
    let down = blessed(&one("tests/a.rs", Counter::Wait, 2), &old, None, false).unwrap();
    assert_eq!(
        row(&down),
        Row {
            budget: 2,
            low_water: 2
        }
    );

    // Up without a reason is refused, and the refusal names the kit.
    let up = one("tests/a.rs", Counter::Wait, 4);
    let refused = blessed(&up, &old, None, false).unwrap_err();
    assert!(
        refused.contains("the story kit") && refused.contains(RAISE_VAR),
        "{refused}"
    );

    // Up with a reason for another file or another counter is still refused.
    for other in [
        (Counter::Wait, "tests/b.rs", "why"),
        (Counter::Clock, "tests/a.rs", "why"),
    ] {
        assert!(blessed(&up, &old, Some(other), false).is_err());
    }

    // Up with its reason: the budget rises, the low-water mark does not, and
    // the rendered file carries the raise and passes the gate.
    let raise = parse_raise("wait tests/a.rs: a raw pipe the kit cannot read").unwrap();
    let raised = blessed(&up, &old, Some(raise), false).unwrap();
    assert_eq!(
        row(&raised),
        Row {
            budget: 4,
            low_water: 3
        }
    );
    let text = render("c", &raised["c"]);
    assert!(text.contains("# raised wait tests/a.rs to 4: a raw pipe the kit cannot read"));
    assert!(text.contains("# total: 4 raw waits, 0 wall-clock reads"));
    assert_eq!(violations(&up, &budgets(&text)), Vec::<String>::new());

    // A new file is a rise from zero, except at the baseline, the first bless
    // of all, where every count is its own low-water mark.
    assert!(blessed(&up, &BTreeMap::new(), None, false).is_err());
    let baseline = blessed(&up, &BTreeMap::new(), None, true).unwrap();
    assert_eq!(
        row(&baseline),
        Row {
            budget: 4,
            low_water: 4
        }
    );

    // A file whose sites are all gone loses its row.
    let gone = blessed(&BTreeMap::new(), &old, None, false).unwrap();
    assert!(gone["c"].rows.is_empty());

    assert!(parse_raise("wait tests/a.rs:").is_none());
    assert!(parse_raise("naps tests/a.rs: why").is_none());
}
