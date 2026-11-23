//! **The known flaky tests** (doc 51 unit FL.2, ruling Q-51-5): the reader of
//! `flaky-known.toml`, the nextest retry filter it generates, and the checks
//! `tests/ci_policy/flaky_known.rs` holds both to.
//!
//! The policy is doc 51 §4.3: no blanket retry. A test is retried once only
//! while it has a row in the list, and a row lives fourteen days. The retry
//! override in `.config/nextest.toml` is not written by hand: it is the block
//! [`block`] prints, between two marker lines, and the gate refuses a file
//! whose block differs from the list or that asks for a retry anywhere else.
//!
//! This module is in `src/` and not in the test file for one reason: the
//! expiry check reads today's date, and a wall-clock read in a file under
//! `tests/` is what doc 51's ratchet counts (§3.9). The read is [`today`],
//! here, once.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::scan;

/// The list, relative to the workspace root.
pub const LIST_PATH: &str = "crates/mush-contract-tests/flaky-known.toml";
/// nextest's configuration, relative to the workspace root.
pub const NEXTEST_PATH: &str = ".config/nextest.toml";
/// The hand-written record of causes a classed row points into.
pub const DOC_PATH: &str = "docs/flaky-tests.md";
/// How long a row lives, and the furthest ahead of today an expiry may be set.
pub const LIFE_DAYS: i64 = 14;

/// Doc 51's classes (§2.1 and §2.4). `unknown` is a row whose cause is not
/// found yet; it still names its row of the record.
pub const CLASSES: [&str; 8] = [
    "figure",
    "dice",
    "clock",
    "clock-phase",
    "stream",
    "process",
    "event",
    "unknown",
];
/// The class of a row carried over from the hand-kept filter with no failure
/// on record (doc 51 §4.3 point 5).
pub const UNCLASSED: &str = "unclassed";

/// The first line of the generated block in `.config/nextest.toml`.
pub const BEGIN: &str = "# BEGIN GENERATED flaky-known";
/// The last line of the generated block.
pub const END: &str = "# END GENERATED flaky-known";

/// One row of the list.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    /// nextest's binary id: `<package>::<target>`, or the bare package for
    /// its lib tests.
    pub binary: String,
    /// The one test, by its full name in the binary.
    pub test: Option<String>,
    /// Every test of one module of the binary. A carried row only.
    pub module: Option<String>,
    /// Every test of the binary. A carried row only.
    #[serde(default)]
    pub whole_binary: bool,
    /// One of [`CLASSES`], or [`UNCLASSED`].
    pub class: String,
    /// The row of `docs/flaky-tests.md` that explains a classed row.
    pub row: Option<u32>,
    /// Where an unclassed row came from.
    pub carried: Option<String>,
    /// The day the row was written, `YYYY-MM-DD`.
    pub listed: String,
    /// The last day the row holds, `YYYY-MM-DD`.
    pub expires: String,
    /// Anything a reader of the row should know.
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListFile {
    #[serde(default)]
    flaky: Vec<Row>,
}

impl Row {
    /// What the row covers, as the list spells it.
    pub fn scope(&self) -> String {
        match (&self.test, &self.module) {
            (Some(test), _) => format!("{} {test}", self.binary),
            (None, Some(module)) => format!("{} {module}::*", self.binary),
            (None, None) => format!("{} (the whole binary)", self.binary),
        }
    }

    /// The row's term of the retry filter.
    fn term(&self) -> String {
        match (&self.test, &self.module) {
            (Some(test), _) => format!("(binary_id({}) & test(={test}))", self.binary),
            (None, Some(module)) => format!("(binary_id({}) & test(/^{module}::/))", self.binary),
            (None, None) => format!("binary_id({})", self.binary),
        }
    }
}

/// The rows of `text`, in the file's order.
pub fn parse(text: &str) -> Result<Vec<Row>, String> {
    toml::from_str::<ListFile>(text)
        .map(|file| file.flaky)
        .map_err(|error| format!("{LIST_PATH}: {error}"))
}

/// The filterset the list generates: one term a row, sorted, joined by `|`.
/// Empty when the list is.
pub fn filter(rows: &[Row]) -> String {
    let terms: BTreeSet<String> = rows.iter().map(Row::term).collect();
    terms.into_iter().collect::<Vec<_>>().join("\n  | ")
}

/// The whole generated block, marker lines included, with no trailing newline.
pub fn block(rows: &[Row]) -> String {
    let mut out = vec![
        format!("{BEGIN} (doc 51 FL.2). This block is generated from"),
        format!("# {LIST_PATH}: one term a row, one retry, nothing else."),
        "# Do not edit it. Change the list, then regenerate:".to_string(),
        "#   FLAKY_KNOWN_BLESS=1 cargo test -p mush-contract-tests --test ci_policy flaky_known::"
            .to_string(),
    ];
    if rows.is_empty() {
        out.push("# The list is empty: no test is retried.".to_string());
    } else {
        out.push("[[profile.default.overrides]]".to_string());
        out.push("filter = '''".to_string());
        out.push(format!("    {}", filter(rows)));
        out.push("'''".to_string());
        out.push("retries = 1".to_string());
    }
    out.push(END.to_string());
    out.join("\n")
}

/// The line span `[begin, end]` of the generated block in `lines`.
fn block_span(lines: &[&str]) -> Result<(usize, usize), String> {
    let begins: Vec<usize> = positions(lines, BEGIN);
    let ends: Vec<usize> = positions(lines, END);
    match (begins.as_slice(), ends.as_slice()) {
        ([begin], [end]) if begin < end => Ok((*begin, *end)),
        _ => Err(format!(
            "{NEXTEST_PATH} must hold exactly one `{BEGIN}` line and one `{END}` line after it; \
             found {} and {}",
            begins.len(),
            ends.len()
        )),
    }
}

fn positions(lines: &[&str], marker: &str) -> Vec<usize> {
    lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.starts_with(marker))
        .map(|(at, _)| at)
        .collect()
}

/// `nextest` with its generated block replaced by the one `rows` generate.
pub fn splice(nextest: &str, rows: &[Row]) -> Result<String, String> {
    let text = nextest.replace("\r\n", "\n");
    let lines: Vec<&str> = text.lines().collect();
    let (begin, end) = block_span(&lines)?;
    let mut out: Vec<String> = lines.iter().take(begin).map(|l| (*l).to_string()).collect();
    out.push(block(rows));
    out.extend(lines.iter().skip(end + 1).map(|l| (*l).to_string()));
    Ok(out.join("\n") + "\n")
}

/// What is wrong with the retries `nextest` asks for, given the list.
///
/// The generated block must be exactly what the list prints, and outside it
/// no profile and no override may ask for a retry. `retries = 0` is not a
/// retry: the `soak` profile says it aloud.
pub fn retry_problems(nextest: &str, rows: &[Row]) -> Vec<String> {
    let text = nextest.replace("\r\n", "\n");
    let lines: Vec<&str> = text.lines().collect();
    let (begin, end) = match block_span(&lines) {
        Ok(span) => span,
        Err(why) => return vec![why],
    };
    let mut problems = Vec::new();
    let found: Vec<&str> = lines
        .iter()
        .skip(begin)
        .take(end + 1 - begin)
        .copied()
        .collect();
    if found.join("\n") != block(rows) {
        problems.push(format!(
            "the generated block of {NEXTEST_PATH} is not what {LIST_PATH} generates: the list \
             changed and the block did not, or the block was edited by hand. Regenerate it:\n    \
             FLAKY_KNOWN_BLESS=1 cargo test -p mush-contract-tests --test ci_policy flaky_known::"
        ));
    }
    let rest: Vec<&str> = lines
        .iter()
        .take(begin)
        .chain(lines.iter().skip(end + 1))
        .copied()
        .collect();
    let parsed: toml::Value = match toml::from_str(&rest.join("\n")) {
        Ok(value) => value,
        Err(error) => {
            problems.push(format!("{NEXTEST_PATH} does not parse: {error}"));
            return problems;
        }
    };
    let profiles = parsed.get("profile").and_then(toml::Value::as_table);
    for (name, profile) in profiles.into_iter().flatten() {
        if asks_for_a_retry(profile.get("retries")) {
            problems.push(format!(
                "[profile.{name}] sets `retries`: that is a blanket retry, which doc 51 §4.3 \
                 rules out. A test is retried only through a row of {LIST_PATH}"
            ));
        }
        let overrides = profile.get("overrides").and_then(toml::Value::as_array);
        for (at, entry) in overrides.into_iter().flatten().enumerate() {
            if asks_for_a_retry(entry.get("retries")) {
                let filter: String = entry
                    .get("filter")
                    .and_then(toml::Value::as_str)
                    .unwrap_or("<no filter>")
                    .chars()
                    .take(96)
                    .collect();
                problems.push(format!(
                    "[[profile.{name}.overrides]] #{at} asks for a retry outside the generated \
                     block (its filter starts `{filter}`). The list does not generate it: add a \
                     row to {LIST_PATH} and regenerate, or drop the `retries` line"
                ));
            }
        }
    }
    problems
}

/// Whether a `retries` value asks for one or more retries. nextest takes an
/// integer or a table with a `count`.
fn asks_for_a_retry(value: Option<&toml::Value>) -> bool {
    match value {
        None => false,
        Some(toml::Value::Integer(count)) => *count != 0,
        Some(toml::Value::Table(table)) => table
            .get("count")
            .and_then(toml::Value::as_integer)
            .is_none_or(|count| count != 0),
        Some(_) => true,
    }
}

/// Days since 1970-01-01 of a `YYYY-MM-DD` date in the proleptic Gregorian
/// calendar.
pub fn day_number(date: &str) -> Result<i64, String> {
    let bad = || format!("`{date}` is not a YYYY-MM-DD date");
    let parts: Vec<&str> = date.split('-').collect();
    let [year, month, day] = parts.as_slice() else {
        return Err(bad());
    };
    if year.len() != 4 || month.len() != 2 || day.len() != 2 {
        return Err(bad());
    }
    let year: i64 = year.parse().map_err(|_| bad())?;
    let month: i64 = month.parse().map_err(|_| bad())?;
    let day: i64 = day.parse().map_err(|_| bad())?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return Err(bad());
    }
    // Howard Hinnant's days_from_civil.
    let shifted = if month <= 2 { year - 1 } else { year };
    let era = shifted.div_euclid(400);
    let year_of_era = shifted.rem_euclid(400);
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let number = era * 146_097 + day_of_era - 719_468;
    if date_of(number) == date {
        Ok(number)
    } else {
        Err(bad())
    }
}

/// The `YYYY-MM-DD` date of a day number. The inverse of [`day_number`].
pub fn date_of(number: i64) -> String {
    // Howard Hinnant's civil_from_days.
    let shifted = number + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Today's day number, UTC, from the OS clock. The one wall-clock read of
/// the gate: an expiry is a real date by design.
pub fn today() -> i64 {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    i64::try_from(seconds / 86_400).unwrap_or(i64::MAX)
}

/// What is wrong with the list on day `today`, given the tree under `root`.
pub fn list_problems(root: &Path, rows: &[Row], today: i64) -> Vec<String> {
    let record = fs::read_to_string(root.join(DOC_PATH)).unwrap_or_default();
    let mut problems = Vec::new();
    let mut seen = BTreeSet::new();
    for row in rows {
        let scope = row.scope();
        let mut say = |what: String| problems.push(format!("{scope}: {what}"));
        if !seen.insert(row.term()) {
            say("listed twice".to_string());
        }
        let scopes = usize::from(row.test.is_some())
            + usize::from(row.module.is_some())
            + usize::from(row.whole_binary);
        if scopes != 1 {
            say("a row names exactly one of `test`, `module` or `whole_binary = true`".to_string());
            continue;
        }
        if row.class == UNCLASSED {
            if row
                .carried
                .as_deref()
                .is_none_or(|why| why.trim().is_empty())
            {
                say(format!(
                    "an `{UNCLASSED}` row must say in `carried` where it came from"
                ));
            }
            if row.row.is_some() {
                say(format!(
                    "an `{UNCLASSED}` row has no row of {DOC_PATH}; give it a class when it gets one"
                ));
            }
        } else if CLASSES.contains(&row.class.as_str()) {
            match (&row.test, row.row) {
                (None, _) => say(format!(
                    "a classed row names one test (doc 51 §4.3: a whole-module entry is narrowed \
                     to the tests that flaked); only an `{UNCLASSED}` carried row may cover a \
                     module or a binary"
                )),
                (Some(_), None) => say(format!(
                    "a classed row names its row of {DOC_PATH} in `row`"
                )),
                (Some(test), Some(number)) => {
                    if let Err(why) = record_names(&record, number, test) {
                        say(why);
                    }
                }
            }
            if row.carried.is_some() {
                say("a classed row is not `carried`; drop the key".to_string());
            }
        } else {
            say(format!(
                "class `{}` is not one of {} or `{UNCLASSED}`",
                row.class,
                CLASSES.join(", ")
            ));
        }
        match (day_number(&row.listed), day_number(&row.expires)) {
            (Ok(listed), Ok(expires)) => {
                if expires < listed {
                    say(format!(
                        "expires {} before it was listed {}",
                        row.expires, row.listed
                    ));
                }
                if expires > today + LIFE_DAYS {
                    say(format!(
                        "expires {}, more than {LIFE_DAYS} days after today ({}); a row is given \
                         or renewed {LIFE_DAYS} days at a time, so the latest date is {}",
                        row.expires,
                        date_of(today),
                        date_of(today + LIFE_DAYS)
                    ));
                }
                if today > expires {
                    say(format!(
                        "EXPIRED on {} (today is {}). Fix the test and delete the row, or, on \
                         evidence (a soak failure, a cause written down), set a new `expires` no \
                         later than {} in a diff the owner sees",
                        row.expires,
                        date_of(today),
                        date_of(today + LIFE_DAYS)
                    ));
                }
            }
            (listed, expires) => {
                for why in [listed.err(), expires.err()].into_iter().flatten() {
                    say(why);
                }
            }
        }
        if let Err(why) = find(root, row) {
            say(format!("names nothing in the tree: {why}"));
        }
    }
    problems
}

/// Whether row `number` of the record's table names `test`.
fn record_names(record: &str, number: u32, test: &str) -> Result<(), String> {
    let start = format!("| {number} |");
    let Some(line) = record.lines().find(|line| line.starts_with(&start)) else {
        return Err(format!("{DOC_PATH} has no row {number}"));
    };
    if line.contains(test) {
        return Ok(());
    }
    // The record writes a root-level test bare and a module's as `module::test`.
    let short = test.rsplit("::").next().unwrap_or(test);
    if line.contains(short) {
        Ok(())
    } else {
        Err(format!(
            "row {number} of {DOC_PATH} does not name `{short}`"
        ))
    }
}

/// The crate directory of a package, by the `name` in its manifest.
fn package_dir(root: &Path, package: &str) -> Option<PathBuf> {
    let wanted = format!("name = \"{package}\"");
    scan::crate_dirs(root).into_iter().find(|dir| {
        fs::read_to_string(dir.join("Cargo.toml"))
            .is_ok_and(|manifest| manifest.lines().any(|line| line.trim() == wanted))
    })
}

/// Finds what `row` names: its binary's root file, then its module path by
/// the `mod` declarations a file at a time, then (for a test row) a function
/// of that name under a test attribute.
fn find(root: &Path, row: &Row) -> Result<(), String> {
    let (package, target) = match row.binary.split_once("::") {
        Some((package, target)) => (package, Some(target)),
        None => (row.binary.as_str(), None),
    };
    let dir = package_dir(root, package).ok_or_else(|| format!("no package `{package}`"))?;
    let root_file = match target {
        Some(target) => dir.join("tests").join(format!("{target}.rs")),
        None => dir.join("src").join("lib.rs"),
    };
    if !root_file.is_file() {
        return Err(format!("no {}", scan::relative(root, &root_file)));
    }
    let path = row.test.as_deref().or(row.module.as_deref());
    let Some(path) = path else {
        return Ok(());
    };
    let mut segments: Vec<&str> = path.split("::").collect();
    let function = if row.test.is_some() {
        segments.pop()
    } else {
        None
    };
    let mut file = root_file;
    let mut is_root = true;
    for segment in segments {
        let (next, next_is_root) = descend(&file, is_root, segment)
            .ok_or_else(|| format!("no module `{segment}` in {}", scan::relative(root, &file)))?;
        file = next;
        is_root = next_is_root;
    }
    let source = fs::read_to_string(&file).map_err(|error| error.to_string())?;
    let code = scan::strip_comments_and_literals(&source);
    match function {
        Some(function) if !has_test(&code, function) => Err(format!(
            "no test `{function}` in {}",
            scan::relative(root, &file)
        )),
        None if !code.contains("test]") && !code.contains("test(") => {
            Err(format!("no test in {}", scan::relative(root, &file)))
        }
        _ => Ok(()),
    }
}

/// The file module `name` of `file` lives in, and whether that file owns its
/// directory (a crate root, a `mod.rs`, a `#[path]` target): a `#[path]`
/// attribute, a `mod name;` beside or under the file, or an inline
/// `mod name {` (the same file).
fn descend(file: &Path, is_root: bool, name: &str) -> Option<(PathBuf, bool)> {
    let source = fs::read_to_string(file).ok()?;
    let dir = file.parent()?;
    let declaration = format!("mod {name};");
    let inline = format!("mod {name} {{");
    let mut path_attribute: Option<&str> = None;
    for line in source.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("#[path = \"") {
            path_attribute = rest.strip_suffix("\"]");
            continue;
        }
        let line = line
            .strip_prefix("pub(crate) ")
            .or_else(|| line.strip_prefix("pub "))
            .unwrap_or(line);
        if line == declaration {
            if let Some(relative) = path_attribute {
                // rustc reads a file named by `#[path]` as it reads a
                // `mod.rs`: its own `mod x;` lines are files beside it.
                let target = dir.join(relative);
                return target.is_file().then_some((target, true));
            }
            let base = if is_root {
                dir.to_path_buf()
            } else {
                dir.join(file.file_stem()?)
            };
            let flat = base.join(format!("{name}.rs"));
            if flat.is_file() {
                return Some((flat, false));
            }
            let nested = base.join(name).join("mod.rs");
            return nested.is_file().then_some((nested, true));
        }
        if line.starts_with(&inline) {
            return Some((file.to_path_buf(), is_root));
        }
        if !line.starts_with("#[") {
            path_attribute = None;
        }
    }
    None
}

/// Whether `code` (comments and literals blanked) defines a function `name`
/// with a test attribute among the attributes above it.
fn has_test(code: &str, name: &str) -> bool {
    let needle = format!("fn {name}(");
    code.match_indices(&needle).any(|(at, _)| {
        let before = code.get(..at).unwrap_or("");
        let item_start = before.rfind(['}', ';']).map_or(0, |end| end + 1);
        before
            .get(item_start..)
            .is_some_and(|head| head.contains("test"))
    })
}
