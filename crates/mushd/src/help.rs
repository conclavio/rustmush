//! Boot-time help-topic loader: reads `docs/help/*.md` (the `[server].
//! help_dir` config key) and feeds the sync help registry inside
//! `mush-core`'s [`Registry`] — the I/O lives here, the serving stays in
//! core (`help <topic>`, with exact command names always winning).
//!
//! Topic naming: the file stem, lowercased (`getting-started.md` →
//! `help getting-started`). The title is the file's first `# heading` line
//! (which is stripped from the served body); a file with no heading uses
//! its stem as the title and is served whole.

use std::path::Path;

use mush_core::Registry;

/// Split a topic file into `(title, body)`: a leading `# heading` becomes
/// the title and is removed from the body; otherwise the fallback title is
/// used and the text is served as-is.
fn split_topic(text: &str, fallback_title: &str) -> (String, String) {
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    if let Some(rest) = trimmed.strip_prefix("# ") {
        let (title, body) = rest.split_once('\n').unwrap_or((rest, ""));
        return (title.trim().to_string(), body.trim().to_string());
    }
    (fallback_title.to_string(), text.trim().to_string())
}

/// Load every `*.md` file in `dir` as a help topic. Returns how many were
/// loaded. A missing directory is not an error (the server may run from an
/// install location without docs) — it loads zero topics with a warning.
pub fn load_help_topics(reg: &mut Registry, dir: &Path) -> std::io::Result<usize> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::warn!(dir = %dir.display(), "help directory missing; no topics loaded");
            return Ok(0);
        }
        Err(e) => return Err(e),
    };
    let mut count = 0usize;
    let mut files: Vec<std::path::PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| x.eq_ignore_ascii_case("md"))
        })
        .collect();
    files.sort();
    for path in files {
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let name = stem.to_lowercase();
        let text = std::fs::read_to_string(&path)?;
        let (title, body) = split_topic(&text, &name);
        reg.add_help_topic(&name, title, body);
        count += 1;
    }
    tracing::info!(count, dir = %dir.display(), "help topics loaded");
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_extracts_heading_title() {
        let (t, b) = split_topic("# Getting Started\n\nWelcome to the game.\n", "x");
        assert_eq!(t, "Getting Started");
        assert_eq!(b, "Welcome to the game.");
        // No heading: fallback title, body served whole.
        let (t, b) = split_topic("Just text.\n", "locks");
        assert_eq!(t, "locks");
        assert_eq!(b, "Just text.");
        // Heading only.
        let (t, b) = split_topic("# Lone Title", "x");
        assert_eq!(t, "Lone Title");
        assert_eq!(b, "");
    }

    #[test]
    fn loads_repo_help_dir() {
        // The real docs/help shipped with the repo must load and serve.
        let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/help"));
        let mut reg = Registry::new();
        let n = load_help_topics(&mut reg, dir).unwrap();
        assert!(n >= 8, "expected at least 8 shipped topics, got {n}");
        for topic in [
            "getting-started",
            "building",
            "locks",
            "channels",
            "mail",
            "jobs",
            "admin",
            "movement",
            "communication",
            "rules-of-conduct",
        ] {
            let t = reg
                .help_topic(topic)
                .unwrap_or_else(|| panic!("missing shipped topic {topic:?}"));
            assert!(!t.body.is_empty(), "{topic}: empty body");
        }
        // Case-insensitive lookup.
        assert!(reg.help_topic("Getting-Started").is_some());
    }

    #[test]
    fn missing_dir_loads_zero() {
        let mut reg = Registry::new();
        let n = load_help_topics(&mut reg, Path::new("Z:/no/such/dir/help")).unwrap();
        assert_eq!(n, 0);
    }
}
