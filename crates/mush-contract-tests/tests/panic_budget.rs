//! **The panic-surface budget** — a workspace policy gate (WS01-S11).
//!
//! One tokio task owns the world. A panic inside it is not a failed command:
//! it unwinds the task that holds every object in the game while the store
//! writer keeps its queue and the players keep their sockets, so the cost of a
//! wrong `unwrap()` there is the game, not the turn. WS01-S4 gave that task a
//! boundary — the unwind is caught, classified [`WorldOutcome::Faulted`] and
//! turned into an orderly non-zero exit that drains the writer and fsyncs the
//! quarantine. This file is the other half: the boundary catches panics, and
//! the budget below keeps their number from growing.
//!
//! It is deliberately **not** an enumeration of the existing sites. The surface
//! is ~650 lexical hits across 41 production crates and removing them is
//! unbounded work; a ratchet that refuses the 651st is what makes the work
//! finite, and it is hard from day one — a raise is a diff a reviewer signs.
//!
//! ## What is counted
//!
//! Six lexical needles — `.unwrap(`, `.expect(`, `panic!`, `unreachable!`,
//! `todo!`, `unimplemented!` — in every `.rs` file under `crates/<pkg>/`
//! **except**:
//!
//! - `crates/<pkg>/tests/` and `crates/<pkg>/benches/` (not production), and
//!   `crates/<pkg>/build.rs` (runs in cargo, not in the world task);
//! - any brace-delimited item carrying `#[cfg(test)]`, matched by counting
//!   braces rather than by line, so a `#[cfg(test)] mod tests { .. }` block at
//!   the foot of a production file is out of scope the way the compiler sees
//!   it. `#[cfg(any(test, feature = "testkit"))]` is deliberately **in** scope:
//!   that item compiles into a production build whenever the feature is on;
//! - a whole source file fenced by a leading `#![cfg(test)]` inner attribute.
//!   Out-of-line test modules carry that fence themselves; directory names or
//!   another module's declaration alone do not exempt a file from this walk;
//! - comments and the interiors of string, raw-string and character literals,
//!   which a line-oriented grep cannot tell from code. The scan below blanks
//!   them first, so a doc comment naming `unwrap()` costs nothing.
//!
//! Slice indexing is **not** counted. `foo[i]` is a panic site every bit as
//! real, but it is not a lexical pattern — telling it from `HashMap` indexing
//! or an array type needs a compiler. That surface belongs to clippy's
//! `indexing_slicing`, enabled per crate by WS01-S12, and stays outside this
//! gate rather than being half-measured inside it.
//!
//! Three paths are excluded by name, each asserted to exist so a rename cannot
//! silently widen the gate: `crates/mush-contract-tests/**` (this crate — a
//! contract gate that cannot assert is not a gate), `crates/mush-testkit/**`
//! (test scaffolding that is only ever linked by test targets), and the single
//! file `crates/mush-store/src/suite.rs`, whose own doc says it panics on any
//! contract violation because that is what a shared store conformance suite
//! does.
//!
//! ## Two tiers
//!
//! - **ratchet** — the gateway half (`mush-gateway`, `mush-net-discord`,
//!   `mush-net-ssh`, `mush-session`, `mush-config`, `mush-privileges`,
//!   `mush-proto`, `mush-ipc`, `mush-ansi`). These crates face the network
//!   before authentication and are the half a hostile line reaches first, so
//!   their budgets go down and stay down: each file records the lowest number
//!   it has ever held, and a budget above that low-water mark must carry a
//!   `# raised to <n>: <reason>` line. `--bless` will not raise one silently.
//! - **ceiling** — everything else, including the world-task crates. A plain
//!   `actual <= budget`; raising it is allowed and shows up as a reviewable
//!   one-line diff in the pull request that needs it.
//!
//! ## The budgets
//!
//! One file per crate under `panic_budget/`, so two branches touching two
//! crates do not conflict on a shared table, and a crate with no file has a
//! budget of **zero** — a new crate is born clean and its first `unwrap()` is
//! a deliberate act. Regenerate with:
//!
//! ```text
//! PANIC_BUDGET_BLESS=1 cargo test -p mush-contract-tests --test panic_budget
//! ```
//!
//! and raise a ratchet crate by adding `PANIC_BUDGET_RAISE="<why>"`.
//!
//! ## `panic = "abort"`
//!
//! The second test forbids it in every profile of every manifest. Abort is not
//! a stricter form of the same posture — it deletes WS01-S4's boundary
//! outright (`catch_unwind` never runs), replaces the orderly exit with
//! `SIGABRT`, and takes the writer drain and the quarantine fsync with it. A
//! process that aborts loses the batch it was holding; a process that unwinds
//! into the boundary writes it out and exits non-zero. The gate here is that
//! nobody can flip the first into the second in a one-line profile edit.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// The six lexical forms of "this line may take the process down".
const NEEDLES: [&str; 6] = [
    ".unwrap(",
    ".expect(",
    "panic!",
    "unreachable!",
    "todo!",
    "unimplemented!",
];

/// The pre-auth network half, whose budgets may only fall.
const RATCHET_CRATES: [&str; 9] = [
    "mush-ansi",
    "mush-config",
    "mush-gateway",
    "mush-ipc",
    "mush-net-discord",
    "mush-net-ssh",
    "mush-privileges",
    "mush-proto",
    "mush-session",
];

/// Crates excluded wholesale, by name: the test harnesses, and `mush-bot`, the
/// bot laboratory — out of the engine (no engine, store or gateway among its
/// production dependencies; it plays the game over SSH as a player would), so an
/// unwrap in it ends a bot, never a world. The owner's call, 2026-09-10.
const EXCLUDED_CRATES: [&str; 3] = ["mush-contract-tests", "mush-testkit", "mush-bot"];

/// Single files excluded by path, relative to the workspace root.
const EXCLUDED_FILES: [&str; 1] = ["crates/mush-store/src/suite.rs"];

/// The directory holding one `<crate>.txt` budget per crate.
const BUDGET_DIR: &str = "panic_budget";

/// The environment variable that turns this test into its own generator.
const BLESS_VAR: &str = "PANIC_BUDGET_BLESS";

/// The environment variable carrying the reason a ratchet budget goes up.
const RAISE_VAR: &str = "PANIC_BUDGET_RAISE";

/// The workspace root, from this crate's manifest directory.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<pkg> has a workspace root two levels up")
        .to_path_buf()
}

// ---------------------------------------------------------------------------
// The scan
// ---------------------------------------------------------------------------

/// Blank every comment and every literal interior, preserving byte offsets and
/// newlines so the brace scan that follows still lines up with the source.
///
/// Not a lexer for Rust — a lexer for the four things that make a grep lie:
/// `//` and `/* */` comments (doc comments included, since they are comments),
/// `"…"` and `r#"…"#` strings, and `'c'` character literals, which must be
/// told from lifetimes or `'"'` would open a string that never closes.
fn strip_comments_and_literals(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = bytes.to_vec();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        // A line comment runs to the newline.
        if bytes[i] == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            while i < len && bytes[i] != b'\n' {
                out[i] = b' ';
                i += 1;
            }
            continue;
        }
        // Block comments nest in Rust.
        if bytes[i] == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            let mut depth = 0usize;
            while i < len {
                if bytes[i] == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
                    depth += 1;
                    out[i] = b' ';
                    out[i + 1] = b' ';
                    i += 2;
                    continue;
                }
                if bytes[i] == b'*' && i + 1 < len && bytes[i + 1] == b'/' {
                    depth -= 1;
                    out[i] = b' ';
                    out[i + 1] = b' ';
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                    continue;
                }
                if bytes[i] != b'\n' {
                    out[i] = b' ';
                }
                i += 1;
            }
            continue;
        }
        // `b"…"`, `b'c'`, `br"…"`: blank the prefix and let the arms below
        // handle the literal itself.
        if bytes[i] == b'b'
            && i + 1 < len
            && (bytes[i + 1] == b'"'
                || bytes[i + 1] == b'\''
                || (bytes[i + 1] == b'r'
                    && i + 2 < len
                    && (bytes[i + 2] == b'"' || bytes[i + 2] == b'#')))
        {
            out[i] = b' ';
            i += 1;
            continue;
        }
        // A raw string has no escapes and is closed by `"` plus its hashes.
        if bytes[i] == b'r' && i + 1 < len && (bytes[i + 1] == b'"' || bytes[i + 1] == b'#') {
            let mut j = i + 1;
            let mut hashes = 0usize;
            while j < len && bytes[j] == b'#' {
                hashes += 1;
                j += 1;
            }
            if j < len && bytes[j] == b'"' {
                let mut terminator = String::from('"');
                for _ in 0..hashes {
                    terminator.push('#');
                }
                let body = j + 1;
                let end = src[body..]
                    .find(&terminator)
                    .map_or(len, |rel| body + rel + terminator.len());
                for slot in &mut out[i..end] {
                    if *slot != b'\n' {
                        *slot = b' ';
                    }
                }
                i = end;
                continue;
            }
        }
        // An ordinary string, whose escapes may hide the closing quote.
        if bytes[i] == b'"' {
            out[i] = b' ';
            i += 1;
            while i < len {
                if bytes[i] == b'\\' {
                    out[i] = b' ';
                    if i + 1 < len && bytes[i + 1] != b'\n' {
                        out[i + 1] = b' ';
                    }
                    i += 2;
                    continue;
                }
                let closing = bytes[i] == b'"';
                if bytes[i] != b'\n' {
                    out[i] = b' ';
                }
                i += 1;
                if closing {
                    break;
                }
            }
            continue;
        }
        // A quote is a character literal only when one character later comes
        // its twin; otherwise it opens a lifetime and is ordinary code.
        if bytes[i] == b'\'' {
            if i + 1 < len && bytes[i + 1] == b'\\' {
                let mut j = i + 2;
                while j < len && bytes[j] != b'\'' {
                    j += 1;
                }
                let end = (j + 1).min(len);
                for slot in &mut out[i..end] {
                    *slot = b' ';
                }
                i = end;
                continue;
            }
            if let Some(c) = src[i + 1..].chars().next() {
                let after = i + 1 + c.len_utf8();
                if after < len && bytes[after] == b'\'' {
                    for slot in &mut out[i..=after] {
                        *slot = b' ';
                    }
                    i = after + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    String::from_utf8(out).expect("blanking whole byte ranges leaves valid UTF-8")
}

/// The attribute that puts an item out of a production build entirely.
const CFG_TEST: &str = "#[cfg(test)]";

/// Blank each `#[cfg(test)]` item, brace-counted from the attribute to the
/// matching close. Input must already be [`strip_comments_and_literals`]ed, so
/// a brace inside a string cannot throw the depth off.
///
/// An attribute whose item is not brace-delimited (`#[cfg(test)] use super::*;`)
/// reaches its `;` first and is left alone; it holds no needles either way.
fn blank_cfg_test_items(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = bytes.to_vec();
    let mut search = 0;
    while let Some(rel) = src[search..].find(CFG_TEST) {
        let at = search + rel;
        let mut cursor = at + CFG_TEST.len();
        let mut open = None;
        while cursor < bytes.len() {
            match bytes[cursor] {
                b'{' => {
                    open = Some(cursor);
                    break;
                }
                b';' => break,
                _ => cursor += 1,
            }
        }
        let Some(open) = open else {
            search = at + CFG_TEST.len();
            continue;
        };
        let mut depth = 0usize;
        let mut end = open;
        while end < bytes.len() {
            match bytes[end] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            end += 1;
        }
        let stop = end.min(bytes.len().saturating_sub(1));
        for slot in &mut out[at..=stop] {
            if *slot != b'\n' {
                *slot = b' ';
            }
        }
        search = stop + 1;
    }
    String::from_utf8(out).expect("blanking whole byte ranges leaves valid UTF-8")
}

/// Every needle left in `source` once comments, literals and `#[cfg(test)]`
/// items are gone.
fn production_panic_sites(source: &str) -> usize {
    let stripped = strip_comments_and_literals(source);
    if file_is_test_only(&stripped) {
        return 0;
    }
    let code = blank_cfg_test_items(&stripped);
    NEEDLES.iter().map(|n| code.matches(n).count()).sum()
}

/// Only leading inner attributes can fence the complete file. Searching for the
/// text anywhere would let a nested module hide its production siblings.
fn file_is_test_only(stripped: &str) -> bool {
    let mut rest = stripped.trim_start();
    while rest.starts_with("#![") {
        let mut depth = 1usize;
        let Some(end) = rest.char_indices().skip(3).find_map(|(at, ch)| {
            match ch {
                '[' => depth += 1,
                ']' => depth -= 1,
                _ => {}
            }
            (depth == 0).then_some(at)
        }) else {
            return false;
        };
        let attribute: String = rest[..=end]
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        if attribute == "#![cfg(test)]" {
            return true;
        }
        rest = rest[end + 1..].trim_start();
    }
    false
}

/// Every `.rs` file of one crate that a production build can reach, sorted so
/// the walk is identical on both CI legs.
fn scanned_files(crate_dir: &Path, excluded: &[PathBuf]) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    let Ok(entries) = fs::read_dir(crate_dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() {
            // Only the crate's own roots are excluded; a `src/tests/` module
            // would be production code under a misleading name.
            if name == "tests" || name == "benches" || name == "target" {
                continue;
            }
            walk(&path, &mut files);
        } else if name == "build.rs" {
            continue;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
    files.retain(|path| !excluded.iter().any(|skip| skip == path));
    files.sort();
    files
}

/// The measured production panic surface of every crate in the workspace.
fn measure(root: &Path) -> BTreeMap<String, usize> {
    let excluded: Vec<PathBuf> = EXCLUDED_FILES.iter().map(|rel| root.join(rel)).collect();
    let mut measured = BTreeMap::new();
    let crates_dir = root.join("crates");
    let mut names: Vec<String> = fs::read_dir(&crates_dir)
        .expect("the workspace has a crates/ directory")
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| !EXCLUDED_CRATES.contains(&name.as_str()))
        .collect();
    names.sort();
    for name in names {
        let total = scanned_files(&crates_dir.join(&name), &excluded)
            .iter()
            .filter_map(|path| fs::read_to_string(path).ok())
            .map(|source| production_panic_sites(&source))
            .sum();
        measured.insert(name, total);
    }
    measured
}

// ---------------------------------------------------------------------------
// The budget files
// ---------------------------------------------------------------------------

/// Which rule a crate's budget obeys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tier {
    /// Down only; a raise must be justified in the file itself.
    Ratchet,
    /// `actual <= budget`; a raise is an ordinary reviewable diff.
    Ceiling,
}

impl Tier {
    const fn label(self) -> &'static str {
        match self {
            Self::Ratchet => "ratchet",
            Self::Ceiling => "ceiling",
        }
    }

    fn of(crate_name: &str) -> Self {
        if RATCHET_CRATES.contains(&crate_name) {
            Self::Ratchet
        } else {
            Self::Ceiling
        }
    }
}

/// One parsed `panic_budget/<crate>.txt`.
#[derive(Debug, Clone)]
struct Budget {
    tier: Tier,
    /// The number the gate compares against.
    budget: usize,
    /// Ratchet crates only: the lowest budget this crate has ever held.
    low_water: Option<usize>,
    /// Ratchet crates only: the justification for a budget above `low_water`.
    raise: Option<(usize, String)>,
}

impl Budget {
    /// A crate with no file at all.
    fn zero(tier: Tier) -> Self {
        Self {
            tier,
            budget: 0,
            low_water: (tier == Tier::Ratchet).then_some(0),
            raise: None,
        }
    }
}

/// Parse a budget file, or say why it is not one.
fn parse_budget(text: &str) -> Result<Budget, String> {
    let mut tier = None;
    let mut low_water = None;
    let mut raise = None;
    let mut budget = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(comment) = line.strip_prefix('#') {
            let comment = comment.trim();
            if let Some(value) = comment.strip_prefix("tier:") {
                tier = match value.trim() {
                    "ratchet" => Some(Tier::Ratchet),
                    "ceiling" => Some(Tier::Ceiling),
                    other => return Err(format!("unknown tier `{other}`")),
                };
            } else if let Some(value) = comment.strip_prefix("low-water:") {
                low_water = Some(
                    value
                        .trim()
                        .parse::<usize>()
                        .map_err(|err| format!("low-water is not a number: {err}"))?,
                );
            } else if let Some(value) = comment.strip_prefix("raised to ") {
                let (number, reason) = value
                    .split_once(':')
                    .ok_or_else(|| "a raise reads `# raised to <n>: <reason>`".to_string())?;
                let number = number
                    .trim()
                    .parse::<usize>()
                    .map_err(|err| format!("the raised-to number is not a number: {err}"))?;
                raise = Some((number, reason.trim().to_string()));
            }
            continue;
        }
        if budget.is_some() {
            return Err("a budget file holds exactly one number".to_string());
        }
        budget = Some(
            line.parse::<usize>()
                .map_err(|err| format!("`{line}` is not a budget: {err}"))?,
        );
    }
    let tier = tier.ok_or_else(|| "no `# tier:` line".to_string())?;
    let budget = budget.ok_or_else(|| "no budget number".to_string())?;
    Ok(Budget {
        tier,
        budget,
        low_water,
        raise,
    })
}

/// Render a budget file, LF-terminated (`.gitattributes` normalises the tree).
fn render_budget(crate_name: &str, budget: &Budget) -> String {
    let mut text = format!(
        "# {crate_name} — production panic-surface budget (WS01-S11).\n\
         #\n\
         # Regenerate with:\n\
         #   {BLESS_VAR}=1 cargo test -p mush-contract-tests --test panic_budget\n\
         # A raise is a reviewed diff, not a rounding error.\n\
         # tier: {}\n",
        budget.tier.label()
    );
    if let Some(low) = budget.low_water {
        text.push_str(&format!("# low-water: {low}\n"));
    }
    if let Some((to, reason)) = &budget.raise {
        text.push_str(&format!("# raised to {to}: {reason}\n"));
    }
    text.push_str(&format!("{}\n", budget.budget));
    text
}

/// Read every budget file under `dir`, keyed by crate name.
fn read_budgets(dir: &Path) -> Result<BTreeMap<String, Budget>, String> {
    let mut budgets = BTreeMap::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(budgets);
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "txt"))
        .collect();
    files.sort();
    for path in files {
        let name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .ok_or_else(|| format!("{} has no crate name", path.display()))?;
        let text = fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
        let budget = parse_budget(&text).map_err(|err| format!("{}: {err}", path.display()))?;
        budgets.insert(name, budget);
    }
    Ok(budgets)
}

/// The whole gate as a pure function, so the fixtures below can exercise the
/// failure message and not only the happy path.
///
/// `Ok(())` when every crate is within its budget and every budget file is
/// well formed; `Err` carries the operator-facing report.
fn enforce(
    measured: &BTreeMap<String, usize>,
    budgets: &BTreeMap<String, Budget>,
) -> Result<(), String> {
    let mut faults: Vec<String> = Vec::new();
    for (name, budget) in budgets {
        if !measured.contains_key(name.as_str()) {
            faults.push(format!(
                "{BUDGET_DIR}/{name}.txt budgets a crate that does not exist — delete it"
            ));
            continue;
        }
        let expected = Tier::of(name);
        if budget.tier != expected {
            faults.push(format!(
                "{BUDGET_DIR}/{name}.txt says `tier: {}`; {name} is a {} crate",
                budget.tier.label(),
                expected.label()
            ));
        }
        if expected == Tier::Ratchet {
            let Some(low) = budget.low_water else {
                faults.push(format!(
                    "{BUDGET_DIR}/{name}.txt is a ratchet and carries no `# low-water:` line"
                ));
                continue;
            };
            if budget.budget > low {
                match &budget.raise {
                    Some((to, reason)) if *to == budget.budget && !reason.is_empty() => {}
                    _ => faults.push(format!(
                        "{BUDGET_DIR}/{name}.txt raises a ratchet budget from {low} to {} with no \
                         `# raised to {}: <reason>` line — the pre-auth half only goes down, and \
                         a raise is a decision somebody signs",
                        budget.budget, budget.budget
                    )),
                }
            }
        }
    }
    for (name, actual) in measured {
        let budget = budgets
            .get(name)
            .cloned()
            .unwrap_or_else(|| Budget::zero(Tier::of(name)));
        if *actual > budget.budget {
            let delta = actual - budget.budget;
            faults.push(format!(
                "{name}: {actual} production panic sites, budget {} (+{delta}) — this crate's \
                 unwrap/expect/panic!/unreachable!/todo!/unimplemented! surface grew. Remove the \
                 new sites, or raise {BUDGET_DIR}/{name}.txt in this same change and say why",
                budget.budget
            ));
        }
    }
    if faults.is_empty() {
        return Ok(());
    }
    faults.sort();
    Err(format!(
        "the production panic surface is budgeted per crate ({BUDGET_DIR}/<crate>.txt); \
         regenerate with `{BLESS_VAR}=1 cargo test -p mush-contract-tests --test panic_budget`:\n  {}",
        faults.join("\n  ")
    ))
}

/// The generator half. Rewrites the budget files to the measured surface,
/// refusing to raise a ratchet crate without a reason.
fn bless(dir: &Path, measured: &BTreeMap<String, usize>, raise_reason: Option<&str>) {
    fs::create_dir_all(dir).expect("the budget directory is creatable");
    for (name, actual) in measured {
        let tier = Tier::of(name);
        let path = dir.join(format!("{name}.txt"));
        let existing = fs::read_to_string(&path)
            .ok()
            .and_then(|text| parse_budget(&text).ok());
        // A ceiling crate at zero keeps no file: a missing file *is* zero, so a
        // new crate starts clean without anyone writing it down.
        if tier == Tier::Ceiling && *actual == 0 {
            let _ = fs::remove_file(&path);
            continue;
        }
        // A file being born records today's surface as its low-water mark: the
        // ratchet starts where the crate is, not at a zero it never held.
        let mut next = existing.clone().unwrap_or(Budget {
            tier,
            budget: *actual,
            low_water: None,
            raise: None,
        });
        next.tier = tier;
        if tier == Tier::Ratchet {
            let low = next.low_water.unwrap_or(*actual).min(*actual);
            if *actual > next.budget && existing.is_some() {
                let reason = raise_reason.unwrap_or_else(|| {
                    panic!(
                        "{name} is a ratchet crate: its panic surface rose from {} to {actual}. \
                         Remove the new sites, or re-run with {RAISE_VAR}=\"<why this one stays>\"",
                        next.budget
                    )
                });
                next.raise = Some((*actual, reason.to_string()));
            }
            next.low_water = Some(low);
            next.budget = *actual;
            if next.budget <= low {
                next.raise = None;
            } else if let Some((to, _)) = &mut next.raise {
                *to = next.budget;
            }
        } else {
            next.budget = *actual;
            next.low_water = None;
            next.raise = None;
        }
        let rendered = render_budget(name, &next);
        if fs::read_to_string(&path).ok().as_deref() != Some(rendered.as_str()) {
            fs::write(&path, rendered).expect("the budget file is writable");
        }
    }
}

// ---------------------------------------------------------------------------
// The gates
// ---------------------------------------------------------------------------

#[test]
fn no_crate_exceeds_its_production_panic_budget() {
    let root = repo_root();
    for name in EXCLUDED_CRATES {
        let path = root.join("crates").join(name);
        assert!(
            path.is_dir(),
            "{} is excluded from the panic budget by name and no longer exists — a rename widened \
             the gate silently",
            path.display()
        );
    }
    for rel in EXCLUDED_FILES {
        let path = root.join(rel);
        assert!(
            path.is_file(),
            "{} is excluded from the panic budget by name and no longer exists — a rename widened \
             the gate silently",
            path.display()
        );
    }

    let measured = measure(&root);
    assert!(
        measured.len() > 30,
        "the scanner found only {} crates — it is broken, not the tree",
        measured.len()
    );
    let total: usize = measured.values().sum();
    assert!(
        total > 100,
        "the scanner found only {total} panic sites across the workspace — it is broken, not the \
         tree"
    );

    let dir = root.join(BUDGET_DIR);
    if std::env::var(BLESS_VAR).is_ok() {
        let reason = std::env::var(RAISE_VAR).ok();
        bless(&dir, &measured, reason.as_deref());
    }

    let budgets = read_budgets(&dir).unwrap_or_else(|err| panic!("{err}"));
    if let Err(report) = enforce(&measured, &budgets) {
        panic!("{report}");
    }
}

/// The sibling half: no profile may turn the unwind WS01-S4 catches into a
/// `SIGABRT` that skips the writer drain and the quarantine fsync.
#[test]
fn no_profile_turns_a_world_panic_into_an_abort() {
    fn offenders(value: &toml::Value, path: &str, out: &mut Vec<String>) {
        let toml::Value::Table(table) = value else {
            return;
        };
        for (key, child) in table {
            if key == "panic" && child.as_str() == Some("abort") {
                out.push(format!("{path}.panic"));
                continue;
            }
            offenders(child, &format!("{path}.{key}"), out);
        }
    }

    let root = repo_root();
    let mut manifests = vec![root.join("Cargo.toml")];
    let mut crate_manifests: Vec<PathBuf> = fs::read_dir(root.join("crates"))
        .expect("the workspace has a crates/ directory")
        .flatten()
        .map(|entry| entry.path().join("Cargo.toml"))
        .filter(|path| path.is_file())
        .collect();
    crate_manifests.sort();
    manifests.append(&mut crate_manifests);

    let mut found = Vec::new();
    for manifest in &manifests {
        let text = fs::read_to_string(manifest)
            .unwrap_or_else(|err| panic!("{} is unreadable: {err}", manifest.display()));
        let parsed: toml::Value = toml::from_str(&text)
            .unwrap_or_else(|err| panic!("{} does not parse: {err}", manifest.display()));
        if let Some(profiles) = parsed.get("profile") {
            let mut hits = Vec::new();
            offenders(profiles, "profile", &mut hits);
            for hit in hits {
                found.push(format!("{}: [{hit}]", manifest.display()));
            }
        }
    }

    // The other way to set it: a blanket rustflag.
    let cargo_config = root.join(".cargo/config.toml");
    if let Ok(text) = fs::read_to_string(&cargo_config)
        && text.contains("panic=abort")
    {
        found.push(format!("{}: rustflags", cargo_config.display()));
    }

    assert!(
        found.is_empty(),
        "`panic = \"abort\"` deletes WS01-S4's boundary: `catch_unwind` never runs, the world task \
         dies as SIGABRT instead of an orderly non-zero exit, and the store-writer drain and the \
         quarantine fsync it holds go with it. Remove it from:\n  {}",
        found.join("\n  ")
    );
}

// ---------------------------------------------------------------------------
// The scanner's own proofs
// ---------------------------------------------------------------------------

#[test]
fn only_a_leading_file_test_fence_exempts_an_out_of_line_module() {
    assert_eq!(
        production_panic_sites(
            "//! Fixture.\n#![allow(dead_code)]\n#![ cfg ( test ) ]\nfn f() { x.unwrap(); }"
        ),
        0
    );
    assert_eq!(
        production_panic_sites("#![cfg(any(test, feature = \"testkit\"))]\nfn f() { x.unwrap(); }"),
        1
    );
    assert_eq!(
        production_panic_sites(
            "fn production() { x.unwrap(); }\nmod nested { #![cfg(test)] fn f() { x.expect(\"test\"); } }"
        ),
        2,
        "an inner nested attribute must not exempt production siblings"
    );
    assert_eq!(
        production_panic_sites("// #![cfg(test)]\nfn f() { x.unwrap(); }"),
        1
    );
    assert_eq!(
        production_panic_sites("fn f() { let text = \"#![cfg(test)]\"; x.unwrap(); }"),
        1
    );
    assert_eq!(
        production_panic_sites("#![cfg(not(test))]\nfn f() { x.unwrap(); }"),
        1
    );
}

/// A fixture tree, cleaned up by the caller.
fn fixture_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rustmush-ws01-s11-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("the fixture tree is creatable under the temp dir");
    dir
}

/// Each of the four places an `unwrap()` does not count, and the one place it
/// does. A gate that cannot tell them apart is a gate nobody can satisfy.
#[test]
fn only_production_code_counts_toward_the_budget() {
    assert_eq!(production_panic_sites("fn f() { x.unwrap() }"), 1);
    assert_eq!(
        production_panic_sites("#[cfg(test)]\nmod tests {\n    fn t() { x.unwrap(); }\n}\n"),
        0,
        "a `#[cfg(test)]` block is not in a production build"
    );
    assert_eq!(
        production_panic_sites("/// Prefer this to `x.unwrap()`.\nfn f() {}\n"),
        0,
        "a doc comment naming unwrap is prose"
    );
    assert_eq!(
        production_panic_sites("fn f() { log(\"never x.unwrap() here\"); }"),
        0,
        "a string literal naming unwrap is data"
    );
    assert_eq!(
        production_panic_sites("fn f() { log(r#\"x.unwrap() in raw\"#); }"),
        0,
        "a raw string literal naming unwrap is data"
    );
    assert_eq!(
        production_panic_sites("/* x.unwrap() */ fn f() {}"),
        0,
        "a block comment naming unwrap is prose"
    );
    // The trap a naive scanner falls into: `'\"'` is a character, not the start
    // of a string, and everything after it must still be scanned as code.
    assert_eq!(
        production_panic_sites("fn f(c: char) { if c == '\"' { x.unwrap(); } }"),
        1,
        "a quote character literal must not swallow the code after it"
    );
    assert_eq!(
        production_panic_sites(
            "#[cfg(any(test, feature = \"testkit\"))]\nmod shim {\n    fn t() { x.unwrap(); }\n}\n"
        ),
        1,
        "an item a feature can compile into production is production"
    );
    assert_eq!(
        production_panic_sites(
            "fn f() {\n    a.expect(\"why\");\n    panic!(\"no\");\n    unreachable!();\n    todo!();\n    unimplemented!();\n}\n"
        ),
        5,
        "all six needles count, not just unwrap"
    );
    assert_eq!(
        production_panic_sites("fn f() { let s = \"a \\\" b\"; x.unwrap(); }"),
        1,
        "an escaped quote does not end the string early"
    );
}

/// The gate must trip, and the message must say which crate and by how much.
#[test]
fn the_gate_names_the_crate_and_the_delta() {
    let measured = BTreeMap::from([("mush-core".to_string(), 122), ("mush-jobs".to_string(), 2)]);
    let budgets = BTreeMap::from([
        (
            "mush-core".to_string(),
            Budget {
                tier: Tier::Ceiling,
                budget: 121,
                low_water: None,
                raise: None,
            },
        ),
        (
            "mush-jobs".to_string(),
            Budget {
                tier: Tier::Ceiling,
                budget: 2,
                low_water: None,
                raise: None,
            },
        ),
    ]);
    let report = enforce(&measured, &budgets).expect_err("122 sites against a budget of 121 fails");
    assert!(report.contains("mush-core: 122"), "{report}");
    assert!(report.contains("(+1)"), "{report}");
    assert!(!report.contains("mush-jobs"), "{report}");

    let at_budget = BTreeMap::from([("mush-core".to_string(), 121), ("mush-jobs".to_string(), 2)]);
    assert!(enforce(&at_budget, &budgets).is_ok());

    // A removal is never a failure: the ratchet only turns one way.
    let smaller = BTreeMap::from([("mush-core".to_string(), 3), ("mush-jobs".to_string(), 0)]);
    assert!(enforce(&smaller, &budgets).is_ok());

    // A crate with no file at all is budgeted at zero.
    let newcomer = BTreeMap::from([("mush-newthing".to_string(), 1)]);
    let report = enforce(&newcomer, &BTreeMap::new())
        .expect_err("a brand new crate is born with a budget of zero");
    assert!(report.contains("mush-newthing: 1"), "{report}");
}

/// The pre-auth half only goes down, and a raise must carry its reason.
#[test]
fn a_ratchet_crate_cannot_be_raised_without_a_reason() {
    let measured = BTreeMap::from([("mush-gateway".to_string(), 56)]);
    let silent = BTreeMap::from([(
        "mush-gateway".to_string(),
        Budget {
            tier: Tier::Ratchet,
            budget: 56,
            low_water: Some(54),
            raise: None,
        },
    )]);
    let report = enforce(&measured, &silent).expect_err("a silent ratchet raise is refused");
    assert!(
        report.contains("raises a ratchet budget from 54 to 56"),
        "{report}"
    );

    let justified = BTreeMap::from([(
        "mush-gateway".to_string(),
        Budget {
            tier: Tier::Ratchet,
            budget: 56,
            low_water: Some(54),
            raise: Some((56, "russh 0.64 handshake precondition".to_string())),
        },
    )]);
    assert!(enforce(&measured, &justified).is_ok());

    // A justification that names a different number is a stale one: the file
    // was hand-edited upward after the reason was written.
    let stale = BTreeMap::from([(
        "mush-gateway".to_string(),
        Budget {
            tier: Tier::Ratchet,
            budget: 56,
            low_water: Some(54),
            raise: Some((55, "an older, smaller raise".to_string())),
        },
    )]);
    assert!(enforce(&measured, &stale).is_err());

    // And a gateway crate cannot demote itself to the ceiling tier.
    let demoted = BTreeMap::from([(
        "mush-gateway".to_string(),
        Budget {
            tier: Tier::Ceiling,
            budget: 56,
            low_water: None,
            raise: None,
        },
    )]);
    let report = enforce(&measured, &demoted).expect_err("the tier is not the file's to choose");
    assert!(report.contains("is a ratchet crate"), "{report}");
}

/// `--bless` round-trips: what it writes, the gate accepts; what it lowers, it
/// lowers the low-water mark with; and what it would raise, it refuses.
#[test]
fn bless_writes_files_the_gate_accepts_and_refuses_a_silent_ratchet_raise() {
    let dir = fixture_dir("bless");
    let measured = BTreeMap::from([
        ("mush-core".to_string(), 121),
        ("mush-gateway".to_string(), 54),
        ("mush-ansi".to_string(), 0),
        ("mush-dice".to_string(), 0),
    ]);
    bless(&dir, &measured, None);

    let budgets = read_budgets(&dir).expect("bless writes parseable files");
    assert!(
        enforce(&measured, &budgets).is_ok(),
        "the gate must accept its own generator's output"
    );
    assert!(
        !dir.join("mush-dice.txt").exists(),
        "a ceiling crate at zero keeps no file"
    );
    assert!(
        dir.join("mush-ansi.txt").exists(),
        "a ratchet crate at zero keeps its file, so its first site needs a reason"
    );

    // A removal lowers the low-water mark, so the surface cannot creep back.
    let lowered = BTreeMap::from([
        ("mush-core".to_string(), 121),
        ("mush-gateway".to_string(), 50),
        ("mush-ansi".to_string(), 0),
        ("mush-dice".to_string(), 0),
    ]);
    bless(&dir, &lowered, None);
    let budgets = read_budgets(&dir).expect("bless writes parseable files");
    assert_eq!(budgets["mush-gateway"].low_water, Some(50));
    assert!(
        enforce(&measured, &budgets).is_err(),
        "50 is the new ceiling"
    );

    let dir2 = fixture_dir("bless-raise");
    bless(
        &dir2,
        &BTreeMap::from([("mush-gateway".to_string(), 54)]),
        None,
    );
    let raised = BTreeMap::from([("mush-gateway".to_string(), 55)]);
    let refused = std::panic::catch_unwind(|| bless(&dir2, &raised, None));
    assert!(
        refused.is_err(),
        "a ratchet raise without a reason is refused"
    );
    bless(&dir2, &raised, Some("a documented reason"));
    let budgets = read_budgets(&dir2).expect("bless writes parseable files");
    assert_eq!(budgets["mush-gateway"].budget, 55);
    assert!(enforce(&raised, &budgets).is_ok());

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&dir2);
}

/// The scan is a directory walk, so what it walks is part of the contract.
#[test]
fn the_walk_skips_tests_benches_and_build_scripts() {
    let dir = fixture_dir("walk");
    let crate_dir = dir.join("pkg");
    for sub in ["src", "tests", "benches", "src/deep"] {
        fs::create_dir_all(crate_dir.join(sub)).expect("the fixture tree is creatable");
    }
    fs::write(crate_dir.join("src/lib.rs"), "fn f() { x.unwrap(); }\n").expect("writable");
    fs::write(
        crate_dir.join("src/deep/inner.rs"),
        "fn g() { x.unwrap(); }\n",
    )
    .expect("writable");
    fs::write(crate_dir.join("tests/it.rs"), "fn t() { x.unwrap(); }\n").expect("writable");
    fs::write(crate_dir.join("benches/b.rs"), "fn b() { x.unwrap(); }\n").expect("writable");
    fs::write(crate_dir.join("build.rs"), "fn main() { x.unwrap(); }\n").expect("writable");
    fs::write(crate_dir.join("src/skipped.rs"), "fn s() { x.unwrap(); }\n").expect("writable");
    fs::write(
        crate_dir.join("src/test_fixture.rs"),
        "#![cfg(test)]\nfn fixture() { x.unwrap(); }\n",
    )
    .expect("writable");

    let files = scanned_files(&crate_dir, &[crate_dir.join("src/skipped.rs")]);
    let total: usize = files
        .iter()
        .filter_map(|path| fs::read_to_string(path).ok())
        .map(|source| production_panic_sites(&source))
        .sum();
    let _ = fs::remove_dir_all(&dir);

    assert_eq!(
        total, 2,
        "src/lib.rs and src/deep/inner.rs count; tests/, benches/, build.rs and the excluded file \
         do not: {files:?}"
    );
}
