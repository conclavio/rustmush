//! **One scanner** (doc 43 cut TS.8): the workspace root, the directory walk, the
//! comment-and-literal stripper and the `#[cfg(test)]` blanker every source gate
//! in `tests/` reads the tree with.
//!
//! Before TS.8 each gate carried its own copy: fourteen `repo_root`s, a private
//! walker per file, and two copies of the stripper (`panic_budget.rs`,
//! `matcher_call_sites.rs`) that had drifted apart. A fix to one never reached
//! the other, so two gates could read the same file differently. These are the
//! one copy; a gate that needs another rule says so at its call site.

use std::fs;
use std::path::{Path, PathBuf};

/// The workspace root, two levels above this crate's manifest directory.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<pkg> has a workspace root two levels up")
        .to_path_buf()
}

/// Every workspace member directory, `crates/*` with a `Cargo.toml`, sorted.
pub fn crate_dirs(root: &Path) -> Vec<PathBuf> {
    let crates = root.join("crates");
    let Ok(entries) = fs::read_dir(&crates) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|dir| dir.join("Cargo.toml").is_file())
        .collect();
    dirs.sort();
    dirs
}

/// Every file under `dir` whose extension is one of `extensions`, recursively,
/// in path order (each directory's entries sorted, so the walk is identical on
/// every host). A directory whose name is in `skip` is not entered, at any
/// depth; an unreadable or missing directory contributes nothing.
pub fn files(dir: &Path, extensions: &[&str], skip: &[&str]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(dir, extensions, skip, &mut out);
    out
}

fn walk(dir: &Path, extensions: &[&str], skip: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            let skipped = path
                .file_name()
                .is_some_and(|name| skip.iter().any(|skip| name == *skip));
            if !skipped {
                walk(&path, extensions, skip, out);
            }
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| extensions.contains(&ext))
        {
            out.push(path);
        }
    }
}

/// Every `.rs` file under `dir`, in path order ([`files`] with no skips).
pub fn rust_files(dir: &Path) -> Vec<PathBuf> {
    files(dir, &["rs"], &[])
}

/// `path` relative to `root`, with forward slashes on every host.
pub fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Blank every comment and every literal interior, keeping each newline and
/// every byte offset, so a brace scan or a line number afterwards still lines
/// up with the source.
///
/// Not a lexer for Rust — a lexer for the four things that make a grep lie:
/// `//` and `/* */` comments (doc comments included, since they are comments;
/// block comments nest), `"…"` and `r#"…"#` strings (with their `b` prefixes),
/// and `'c'` character literals, which must be told from lifetimes or `'"'`
/// would open a string that never closes. A `b` or `r` that ends an identifier
/// is not a literal prefix; the `r` after a `b` prefix is.
pub fn strip_comments_and_literals(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = bytes.to_vec();
    let len = bytes.len();
    let mut i = 0;
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for slot in &mut out[from..to] {
            if *slot != b'\n' {
                *slot = b' ';
            }
        }
    };
    while i < len {
        // A line comment runs to the newline.
        if bytes[i] == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            let start = i;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            blank(&mut out, start, i);
            continue;
        }
        // Block comments nest in Rust.
        if bytes[i] == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            let start = i;
            let mut depth = 0usize;
            while i < len {
                if bytes[i] == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                    continue;
                }
                if bytes[i] == b'*' && i + 1 < len && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                    continue;
                }
                i += 1;
            }
            blank(&mut out, start, i.min(len));
            continue;
        }
        // Read from `out`, not `bytes`: a `b` prefix blanked one step earlier is
        // not an identifier, so `br#"…"#` still opens its raw string.
        let ident_before = i > 0 && (out[i - 1].is_ascii_alphanumeric() || out[i - 1] == b'_');
        // `b"…"`, `b'c'`, `br"…"`: blank the prefix and let the arms below take
        // the literal itself.
        if !ident_before
            && bytes[i] == b'b'
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
        if !ident_before
            && bytes[i] == b'r'
            && i + 1 < len
            && (bytes[i + 1] == b'"' || bytes[i + 1] == b'#')
        {
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
                blank(&mut out, i, end);
                i = end;
                continue;
            }
        }
        // An ordinary string, whose escapes may hide the closing quote.
        if bytes[i] == b'"' {
            let start = i;
            i += 1;
            while i < len {
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                let closing = bytes[i] == b'"';
                i += 1;
                if closing {
                    break;
                }
            }
            blank(&mut out, start, i.min(len));
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
                blank(&mut out, i, end);
                i = end;
                continue;
            }
            if let Some(c) = src[i + 1..].chars().next() {
                let after = i + 1 + c.len_utf8();
                if after < len && bytes[after] == b'\'' {
                    blank(&mut out, i, after + 1);
                    i = after + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    String::from_utf8(out).expect("blanking whole byte ranges leaves valid UTF-8")
}

/// The attribute that puts an item out of a production build.
pub const CFG_TEST: &str = "#[cfg(test)]";

/// Blank each `#[cfg(test)]` item, brace-counted from the attribute to its
/// matching close — a `mod tests { … }` at the end of a file, or one in the
/// middle of it. An attribute whose item is not brace-delimited (`#[cfg(test)]
/// use super::*;`) reaches its `;` first and is left alone; it holds no needles
/// either way. Input must already be [`strip_comments_and_literals`]ed, so a
/// brace inside a string cannot throw the depth off.
pub fn blank_cfg_test_items(src: &str) -> String {
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

/// Whether a [`strip_comments_and_literals`]ed file is fenced out of a
/// production build whole by a leading `#![cfg(test)]`. Only leading inner
/// attributes can fence the complete file; searching for the text anywhere
/// would let a nested module hide its production siblings.
pub fn file_is_test_only(stripped: &str) -> bool {
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
