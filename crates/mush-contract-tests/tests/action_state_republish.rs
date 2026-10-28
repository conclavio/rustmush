//! Architectural tripwire for interface cut I13 (docs/interface/02 §3.13): the
//! production commit seam installs typed action-state post-images through
//! `Engine::install_composed_action_announcing`, which emits one
//! `WorldEvent::ActionStateChanged` per touched family once the images are
//! live, after all registered owner images also apply — the event the
//! `<ruleset>.vitals` republish subscribes to beside `AttrSet`. A silent typed
//! or composed install on a production path is a
//! commit nobody hears about: a Willpower spend or Condition-Monitor damage
//! lands, and every client keeps a stale sheet and stale vitals until its next
//! poll. So this probe refuses it outside test code and fixture crates.
//!
//! Both silent install helpers stay legitimate where there is no
//! audience — test fixtures, the contract-test toy bootstraps, boot-time
//! hydration — which is why `tests/` directories, `#[cfg(test)] mod … { … }`
//! bodies (inline test modules, wherever they sit in a file), and the two
//! fixture crates' `src/` are skipped. A `#[cfg(test)] mod name;` declaration
//! is NOT a cut: the external file is scanned on its own and production code
//! after the declaration is still production code.
//!
//! The match is on the bare identifier followed by `(` — a method call, a
//! UFCS call (`Engine::install_prepared_action(&mut e, i)`) and a path-valued
//! call all match; only the method's own definition (`fn
//! install_prepared_action(`) is exempt. Both announcing names are distinct
//! identifiers and never match.

use std::path::{Path, PathBuf};

const SILENT_INSTALLS: [&str; 2] = ["install_prepared_action", "install_composed_action"];
const REQUIRED_SEAM: &str = "install_composed_action_announcing(";

/// Crates whose `src/` is test support. `mush-contract-tests` is dev-only
/// everywhere; `mush-testkit` is a `[dependencies]` entry of the two
/// production binaries for its text renderer (`text_of` in the `mushd` REPL)
/// and nothing else — its `TestWorld` installs fixture images with no audience
/// by design, and its announcing helper wraps the seam this probe requires.
const FIXTURE_CRATES: &[&str] = &["mush-contract-tests", "mush-testkit"];

fn rust_sources(root: &Path, files: &mut Vec<PathBuf>) {
    let mut entries = std::fs::read_dir(root)
        .unwrap_or_else(|error| panic!("cannot scan {}: {error}", root.display()))
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|error| panic!("cannot read entry in {}: {error}", root.display()));
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let kind = entry
            .file_type()
            .unwrap_or_else(|error| panic!("cannot inspect {}: {error}", path.display()));
        if kind.is_dir() {
            if entry.file_name() == "tests" || entry.file_name() == "target" {
                continue;
            }
            rust_sources(&path, files);
        } else if kind.is_file() && path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

/// Whether `line` (already trimmed at the start) opens an inline module body:
/// `mod name {` / `pub mod name {` / `pub(crate) mod name {`, with the brace
/// on the same line (rustfmt's only shape). `mod name;` is a declaration.
fn opens_inline_mod(line: &str) -> bool {
    let rest = line
        .strip_prefix("pub(crate) ")
        .or_else(|| line.strip_prefix("pub(super) "))
        .or_else(|| line.strip_prefix("pub "))
        .unwrap_or(line);
    rest.starts_with("mod ") && rest.trim_end().ends_with('{')
}

/// Count the net brace depth change of one source line. Braces inside string
/// literals and comments are not tracked: rustfmt'd Rust with balanced code
/// braces per statement keeps this exact for module bodies in practice, and
/// the probe's self-tests pin the shapes that matter.
fn brace_delta(line: &str) -> i64 {
    let mut delta = 0;
    for byte in line.bytes() {
        match byte {
            b'{' => delta += 1,
            b'}' => delta -= 1,
            _ => {}
        }
    }
    delta
}

/// The production lines of one source file, as `(1-based line number,
/// text)`: everything outside the bodies of `#[cfg(test)]`-gated inline
/// modules. A `#[cfg(test)]` attribute on a `fn`, an `impl`, a `const` or a
/// `mod name;` declaration ends nothing — only an inline body is skipped, and
/// scanning resumes after its closing brace.
fn production_lines(source: &str) -> Vec<(usize, &str)> {
    // An external test module carries its own explicit file-level fence. Its
    // parent's cfg attribute is unavailable when scanning this file alone.
    if source.lines().any(|line| line.trim() == "#![cfg(test)]") {
        return Vec::new();
    }
    let lines: Vec<&str> = source.lines().collect();
    let mut kept = Vec::with_capacity(lines.len());
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if line.trim() == "#[cfg(test)]" {
            let next = lines[index + 1..]
                .iter()
                .position(|candidate| !candidate.trim().is_empty())
                .map(|offset| index + 1 + offset);
            if let Some(mod_index) = next
                && opens_inline_mod(lines[mod_index].trim_start())
            {
                // Skip the attribute, the `mod … {` line and the body up to
                // and including the brace that closes it.
                let mut depth = brace_delta(lines[mod_index]);
                index = mod_index + 1;
                while index < lines.len() && depth > 0 {
                    depth += brace_delta(lines[index]);
                    index += 1;
                }
                continue;
            }
        }
        kept.push((index + 1, line));
        index += 1;
    }
    kept
}

fn crate_name_of(path: &Path, crates: &Path) -> String {
    path.strip_prefix(crates)
        .ok()
        .and_then(|rel| rel.components().next())
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn is_forbidden(line: &str) -> bool {
    SILENT_INSTALLS
        .iter()
        .any(|name| line.contains(&format!("{name}(")) && !line.contains(&format!("fn {name}(")))
}

#[test]
fn production_commit_seams_install_prepared_actions_announcing() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    rust_sources(&crates, &mut files);
    assert!(
        files.len() > 100,
        "the probe found only {} sources under {}",
        files.len(),
        crates.display()
    );

    let mut offenders = Vec::new();
    let mut seam_sites = 0_usize;
    for path in &files {
        let crate_name = crate_name_of(path, &crates);
        if FIXTURE_CRATES.contains(&crate_name.as_str()) {
            continue;
        }
        let source = std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        for (number, line) in production_lines(&source) {
            if line.contains(REQUIRED_SEAM) {
                seam_sites += 1;
            }
            if is_forbidden(line) {
                offenders.push(format!("{}:{number}: {}", path.display(), line.trim()));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "production code installs prepared action state silently (use \
         `install_composed_action_announcing` so the world's observers hear the \
         commit — interface plan I13):\n{}",
        offenders.join("\n")
    );
    // The world task's commit completion is the one production seam; the
    // engine defines the method. Both must still exist by name.
    assert!(
        seam_sites >= 2,
        "expected the announcing seam in mush-core (definition) and mush-server \
         (the commit completion); found {seam_sites} production sites"
    );
}

#[test]
fn the_world_task_commit_completion_announces() {
    let server_src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("mush-server")
        .join("src");
    let mut production = Vec::new();
    for rel in ["world_task.rs", "world_task/commit_pipeline.rs"] {
        let path = server_src.join(rel);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        production.extend(
            production_lines(&source)
                .into_iter()
                .map(|(n, line)| (n, line.to_owned())),
        );
    }
    assert!(
        production
            .iter()
            .any(|(_, line)| line.contains(REQUIRED_SEAM)),
        "world_task's production code no longer calls the announcing install"
    );
    assert!(
        !production.iter().any(|(_, line)| is_forbidden(line)),
        "world_task's production code installs prepared action state silently"
    );
}

fn kept_text(source: &str) -> String {
    production_lines(source)
        .into_iter()
        .map(|(_, line)| line)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_test_module_body_is_skipped_but_a_test_fn_is_not() {
    let source = "fn a() {}\n#[cfg(test)]\nfn helper() {}\nfn b() {}\n#[cfg(test)]\nmod tests {\n    fn c() {}\n}\n";
    let kept = kept_text(source);
    assert!(kept.contains("fn helper()"));
    assert!(kept.contains("fn b()"));
    assert!(!kept.contains("fn c()"));
}

#[test]
fn a_test_module_declaration_ends_nothing() {
    // mush-campaign's shape: `#[cfg(test)] mod testfix;` at the top, then the
    // whole production body.
    let source = "#[cfg(test)]\nmod testfix;\n\nfn prod(e: &mut E, i: I) {\n    e.install_prepared_action(i);\n}\n";
    let kept = kept_text(source);
    assert!(kept.contains("fn prod("));
    assert!(kept.lines().any(is_forbidden), "{kept}");
}

#[test]
fn production_code_after_a_mid_file_test_module_is_scanned() {
    let source = "fn before() {}\n#[cfg(test)]\nmod t {\n    fn c() { e.install_prepared_action(i); }\n    mod inner { fn d() {} }\n}\nfn after(e: &mut E, i: I) {\n    e.install_prepared_action(i);\n}\n#[cfg(test)]\npub(crate) mod more {\n    fn z() {}\n}\n";
    let kept = kept_text(source);
    assert!(kept.contains("fn before()"));
    assert!(!kept.contains("fn c()"));
    assert!(!kept.contains("fn d()"));
    assert!(kept.contains("fn after("));
    assert!(!kept.contains("fn z()"));
    let offenders: Vec<usize> = production_lines(source)
        .into_iter()
        .filter(|(_, line)| is_forbidden(line))
        .map(|(number, _)| number)
        .collect();
    assert_eq!(offenders, vec![8], "only the production call is reported");
}

#[test]
fn the_match_is_the_bare_identifier_not_the_method_syntax() {
    assert!(is_forbidden(
        "    engine.install_prepared_action(installation);"
    ));
    assert!(is_forbidden(
        "    Engine::install_prepared_action(&mut engine, installation);"
    ));
    assert!(is_forbidden(
        "    install(&mut engine, installation); install_prepared_action(x)"
    ));
    assert!(!is_forbidden(
        "    pub fn install_prepared_action(&mut self, installation: PreparedInstallation) {"
    ));
    assert!(!is_forbidden(
        "    engine.install_prepared_action_announcing(installation, &mut out, &mut fx);"
    ));
    assert!(!is_forbidden(
        "    /// [`Engine::install_prepared_action`] stays for fixtures."
    ));
    assert!(is_forbidden(
        "    engine.install_composed_action(installation);"
    ));
    assert!(is_forbidden(
        "    Engine::install_composed_action(&mut engine, installation);"
    ));
    assert!(!is_forbidden(
        "    pub fn install_composed_action(&mut self, installation: I) {"
    ));
    assert!(!is_forbidden(
        "    engine.install_composed_action_announcing(installation, out, fx);"
    ));
}

#[test]
fn an_explicit_external_test_file_is_excluded_but_an_outer_attribute_is_not() {
    let test = "#![cfg(test)]\nfn fixture() { e.install_composed_action(i); }\n";
    assert!(production_lines(test).is_empty());
    let production =
        "#[cfg(test)]\nmod tests;\nfn production() { e.install_composed_action(i); }\n";
    assert!(
        production_lines(production)
            .iter()
            .any(|(_, line)| is_forbidden(line))
    );
}
