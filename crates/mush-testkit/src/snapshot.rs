//! A committed text snapshot of what a protocol persists (doc 46 unit CT.0).
//!
//! Doc 46 §4.4 found that no test pinned a digest, a key or a stored byte as a literal: every
//! test recomputed them from the strings the code uses, so a refactor could move a persisted
//! byte unseen. A [`Snapshot`] is the pin. A test walks a protocol family to each of its
//! terminal states, writes every key, value, digest and identity string it met as one line of
//! text, and [`Snapshot::check`] compares the whole with a file committed beside the crate's
//! tests.
//!
//! The file is text a reviewer reads in a diff: a header, then `[section]` lines and
//! `label = value` lines in the order the walk wrote them. JSON is written as it was encoded;
//! bytes that are not text are written as hex. It is stored with LF on every platform, and a
//! CRLF checkout is read as LF.
//!
//! **Re-blessing.** `COMMERCE_SNAPSHOT_BLESS=1` rewrites the file from the run. Doc 46's
//! ruling Q-46-2 says no persisted byte, identity string, key or player-facing sentence moves:
//! no unit of doc 46 may re-bless, and a diff in a snapshot file is a stopped unit. A new file
//! is blessed once, by the unit that adds the walk. A change outside doc 46 that means to move
//! a line (a new field, a reworded sentence, a new registration) re-blesses in its own pull
//! request, with the file's diff read line by line and named in its cut note.
//!
//! Apart from [`Snapshot::replay_writes`] this module names no `mush-core` type: it takes
//! text, bytes and anything `serde` can encode, so `mush-core`'s own unit tests (where the
//! crate under test is a second copy of the one this crate links) can use it too.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The variable that rewrites a snapshot file from the run instead of comparing with it.
pub const BLESS_ENV: &str = "COMMERCE_SNAPSHOT_BLESS";

const HEADER: &str = "\
# Doc 46 CT.0 snapshot. Every line is something a store or a player sees: a persisted key,
# a stored value, a digest, a family id, an action id, a protocol name, a printed sentence.
#
# NO UNIT OF DOC 46 MAY RE-BLESS THIS FILE (ruling Q-46-2: bytes do not move).
# A diff in this file is a stopped unit: find the change that moved the byte and undo it.
#
# The walk that writes it is named below. Blessing is for the unit that adds a walk, once:
#   COMMERCE_SNAPSHOT_BLESS=1 cargo test -p <crate> ct0_
";

/// The text of one walk: sections and labelled lines, in the order they were written.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    body: String,
}

impl Snapshot {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a section: one state of a walk, or one type of a codec pin.
    pub fn section(&mut self, title: &str) {
        self.body.push_str("\n[");
        escape_into(&mut self.body, title);
        self.body.push_str("]\n");
    }

    /// One labelled value, on one line. Control characters and backslashes in either are
    /// escaped, so a value never spans lines and a line never ends in a carriage return.
    pub fn line(&mut self, label: &str, value: &str) {
        escape_into(&mut self.body, label);
        self.body.push_str(" = ");
        escape_into(&mut self.body, value);
        self.body.push('\n');
    }

    /// Stored bytes: as text where they are UTF-8 (JSON is), as hex where they are not.
    pub fn bytes(&mut self, label: &str, bytes: &[u8]) {
        match std::str::from_utf8(bytes) {
            Ok(text) => self.line(label, text),
            Err(_) => self.hex(label, bytes),
        }
    }

    /// Bytes as lower-case hex: a digest, a hash, a token.
    pub fn hex(&mut self, label: &str, bytes: &[u8]) {
        let mut hex = String::with_capacity(4 + bytes.len() * 2);
        hex.push_str("hex:");
        for byte in bytes {
            // Writing to a String cannot fail.
            let _ = write!(hex, "{byte:02x}");
        }
        self.line(label, &hex);
    }

    /// Player-facing output of several lines: one snapshot line for each, numbered, so a
    /// changed sentence is one changed line.
    pub fn text(&mut self, label: &str, text: &str) {
        for (number, line) in text.split('\n').enumerate() {
            self.line(&format!("{label} | {:02}", number + 1), line);
        }
    }

    /// Every replay write of one transition, in the order the owner emitted them, each as the
    /// tagged JSON a bundle carries: its kind, its namespace, its key and its value.
    pub fn replay_writes(
        &mut self,
        writes: &[mush_core::authoritative::ReplayWrite],
    ) -> Result<(), String> {
        for write in writes {
            let json = serde_json::to_string(write).map_err(|error| error.to_string())?;
            self.line("write", &json);
        }
        Ok(())
    }

    /// One typed record as its `Serialize` writes it: the struct's own field order, which the
    /// stored JSON value (sorted keys) does not show. Doc 46 §4.4: a field reorder moves no
    /// digest, but it moves these bytes.
    pub fn wire<T: serde::Serialize + ?Sized>(
        &mut self,
        label: &str,
        value: &T,
    ) -> Result<(), String> {
        let json = serde_json::to_string(value).map_err(|error| error.to_string())?;
        self.line(label, &json);
        Ok(())
    }

    /// A codec pin of one strict wire type (doc 46 §4.5: the `checked!` / `record!` / `data!`
    /// macro's structs): a section holding the fixture's encoded bytes in the struct's field
    /// order, and the refusal a decode answers to an unknown field and to a failed validation.
    ///
    /// It fails, without writing the refusal, if the bytes do not decode back to `value`, if
    /// a JSON object carrying one more field decodes, or if the object `invalidate` edited
    /// decodes. `invalidate` receives the fixture as a JSON value and must break one rule the
    /// type's `validate` holds (a `format` nobody wrote, a count over its bound).
    pub fn codec<T>(
        &mut self,
        name: &str,
        value: &T,
        invalidate: impl FnOnce(&mut serde_json::Value),
    ) -> Result<(), String>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + PartialEq,
    {
        self.codec_shape(name, value)?;
        let mut invalid = serde_json::to_value(value).map_err(|error| error.to_string())?;
        let before = invalid.clone();
        invalidate(&mut invalid);
        if invalid == before {
            return Err(format!("{name}: the invalidating edit changed nothing"));
        }
        match serde_json::from_str::<T>(&invalid.to_string()) {
            Ok(_) => Err(format!("{name}: an invalid value decoded")),
            Err(error) => {
                self.line("invalid refused", &error.to_string());
                Ok(())
            }
        }
    }

    /// [`Snapshot::codec`] for a strict wire type whose validation no JSON edit can fail
    /// (every well-typed value is valid): the bytes and the unknown-field refusal only.
    pub fn codec_shape<T>(&mut self, name: &str, value: &T) -> Result<(), String>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + PartialEq,
    {
        self.section(name);
        let json = serde_json::to_string(value).map_err(|error| error.to_string())?;
        self.line("wire", &json);
        let back: T = serde_json::from_str(&json)
            .map_err(|error| format!("{name}: its own bytes do not decode: {error}"))?;
        if back != *value {
            return Err(format!("{name}: its bytes decode to another value"));
        }
        let mut extra: serde_json::Value =
            serde_json::from_str(&json).map_err(|error| error.to_string())?;
        let serde_json::Value::Object(fields) = &mut extra else {
            return Err(format!("{name}: not a JSON object"));
        };
        fields.insert(
            "ct0_unknown_field".to_owned(),
            serde_json::Value::Bool(true),
        );
        match serde_json::from_str::<T>(&extra.to_string()) {
            Ok(_) => Err(format!("{name}: an unknown field decoded")),
            Err(error) => {
                self.line("unknown field refused", &error.to_string());
                Ok(())
            }
        }
    }

    /// The whole file this walk would commit: the header, the walk's name, the body.
    pub fn render(&self, relative: &str) -> String {
        let mut out = String::with_capacity(HEADER.len() + self.body.len() + 64);
        out.push_str(HEADER);
        out.push_str("# File: ");
        escape_into(&mut out, relative);
        out.push('\n');
        out.push_str(&self.body);
        out
    }

    /// Compares the walk with the committed file `relative` under `manifest_dir` (pass
    /// `env!("CARGO_MANIFEST_DIR")`). With [`BLESS_ENV`] set to `1` it writes the file instead.
    ///
    /// The error names the first line that differs, with the committed and the produced text.
    pub fn check(&self, manifest_dir: &str, relative: &str) -> Result<(), String> {
        let path: PathBuf = Path::new(manifest_dir).join(relative);
        let produced = self.render(relative);
        if std::env::var(BLESS_ENV).is_ok_and(|value| value == "1") {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            return std::fs::write(&path, produced.as_bytes()).map_err(|error| error.to_string());
        }
        let committed = std::fs::read_to_string(&path).map_err(|error| {
            format!(
                "no committed snapshot at {}: {error}. A new walk is blessed once, with \
                 {BLESS_ENV}=1; an existing one is never re-blessed by a doc 46 unit.",
                path.display()
            )
        })?;
        // A CRLF checkout is the same file: no produced line holds a raw carriage return.
        let committed = committed.replace("\r\n", "\n");
        if committed == produced {
            return Ok(());
        }
        let report = difference(&path, &committed, &produced);
        // A test that returns this error prints it through `Debug`, on one escaped line:
        // say it once in the clear as well.
        eprintln!("{report}");
        Err(report)
    }
}

fn difference(path: &Path, committed: &str, produced: &str) -> String {
    let old: Vec<&str> = committed.split('\n').collect();
    let new: Vec<&str> = produced.split('\n').collect();
    let mut differing = old.len().abs_diff(new.len());
    let mut first = None;
    for (index, (left, right)) in old.iter().zip(new.iter()).enumerate() {
        if left != right {
            differing += 1;
            if first.is_none() {
                first = Some((index + 1, *left, *right));
            }
        }
    }
    let mut out = format!(
        "SNAPSHOT MOVED: {} differs from this run ({differing} of {} lines; committed {} \
         lines, produced {}).\n",
        path.display(),
        old.len().max(new.len()),
        old.len(),
        new.len()
    );
    match first {
        Some((number, left, right)) => {
            let _ = write!(
                out,
                "first difference, line {number}:\n  committed: {}\n  produced:  {}\n",
                clip(left),
                clip(right)
            );
            if let Some(at) = left
                .char_indices()
                .zip(right.char_indices())
                .find_map(|((at, a), (_, b))| (a != b).then_some(at))
            {
                let _ = writeln!(
                    out,
                    "  at byte {at}: committed ..{}  produced ..{}",
                    clip(left.get(at..).unwrap_or_default()),
                    clip(right.get(at..).unwrap_or_default())
                );
            }
        }
        None => out.push_str("the files agree up to the shorter one's end.\n"),
    }
    out.push_str(
        "A persisted byte, key, digest, identity string or printed sentence changed (doc 46 \
         Q-46-2). No doc 46 unit may re-bless: undo the change that moved it. A change outside \
         doc 46 that means to move this line re-blesses in its own pull request \
         (COMMERCE_SNAPSHOT_BLESS=1), with the file's diff named in its cut note.\n",
    );
    out
}

/// At most 240 characters of a line, so one long record does not bury the report.
fn clip(line: &str) -> String {
    const MAX: usize = 240;
    if line.chars().count() <= MAX {
        return line.to_owned();
    }
    let mut out: String = line.chars().take(MAX).collect();
    out.push_str("...");
    out
}

fn escape_into(out: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other if other.is_control() => {
                let _ = write!(out, "\\u{{{:x}}}", u32::from(other));
            }
            other => out.push(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ensure(holds: bool, what: &str, text: &str) -> Result<(), String> {
        if holds {
            Ok(())
        } else {
            Err(format!("{what}:\n{text}"))
        }
    }

    fn sample() -> Snapshot {
        let mut snapshot = Snapshot::new();
        snapshot.section("drafted");
        snapshot.line("key", "market.mind.renewal.v1:00");
        snapshot.bytes("stored", br#"{"format":1}"#);
        snapshot.bytes("raw", &[0xff, 0x00]);
        snapshot.text("printed", "one\ntwo\r");
        snapshot
    }

    #[test]
    fn a_walk_renders_one_escaped_line_a_value() -> Result<(), String> {
        let text = sample().render("tests/ct0/sample.snap");
        ensure(text.starts_with(HEADER), "the header leads", &text)?;
        ensure(
            text.ends_with(
                "# File: tests/ct0/sample.snap\n\n[drafted]\nkey = market.mind.renewal.v1:00\n\
                 stored = {\"format\":1}\nraw = hex:ff00\nprinted | 01 = one\n\
                 printed | 02 = two\\r\n",
            ),
            "one line a value",
            &text,
        )?;
        ensure(!text.contains('\r'), "no raw carriage return", &text)
    }

    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Strict {
        format: u16,
        name: String,
    }

    #[test]
    fn a_codec_pin_holds_the_field_order_and_both_refusals() -> Result<(), String> {
        let value = Strict {
            format: 1,
            name: "a".into(),
        };
        let mut snapshot = Snapshot::new();
        snapshot.codec("Strict", &value, |json| json["format"] = "one".into())?;
        let text = snapshot.render("x.snap");
        ensure(
            text.contains("\n[Strict]\nwire = {\"format\":1,\"name\":\"a\"}\n"),
            "the struct's own field order",
            &text,
        )?;
        ensure(
            text.contains("unknown field refused = unknown field `ct0_unknown_field`, expected")
                && text.contains("invalid refused = invalid type: string \"one\""),
            "both refusals",
            &text,
        )?;
        let unchanged = Snapshot::new().codec("Strict", &value, |_| {});
        ensure(
            unchanged.is_err(),
            "an edit that changes nothing is an error",
            "",
        )?;
        let accepted = Snapshot::new().codec("Strict", &value, |json| json["format"] = 2.into());
        ensure(
            accepted.is_err(),
            "an edit that still decodes is an error",
            "",
        )
    }

    #[test]
    fn a_moved_byte_is_reported_with_its_line_and_a_crlf_checkout_is_not() -> Result<(), String> {
        let dir =
            std::env::temp_dir().join(format!("mush-testkit-snapshot-{}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
        let root = dir.to_str().ok_or("a text path")?;
        let snapshot = sample();
        let committed = snapshot.render("same.snap");
        std::fs::write(dir.join("same.snap"), committed.replace('\n', "\r\n"))
            .map_err(|error| error.to_string())?;
        snapshot.check(root, "same.snap")?;

        let moved = snapshot
            .render("moved.snap")
            .replace("renewal.v1", "renewal.v2");
        std::fs::write(dir.join("moved.snap"), moved).map_err(|error| error.to_string())?;
        let report = snapshot
            .check(root, "moved.snap")
            .err()
            .ok_or("a moved byte passes")?;
        ensure(
            report.starts_with("SNAPSHOT MOVED"),
            "the report's lead",
            &report,
        )?;
        ensure(
            report.contains("first difference, line 12"),
            "the line is named",
            &report,
        )?;
        ensure(
            report.contains("committed: key = market.mind.renewal.v2:00")
                && report.contains("produced:  key = market.mind.renewal.v1:00"),
            "both sides are shown",
            &report,
        )?;

        let missing = snapshot
            .check(root, "absent.snap")
            .err()
            .ok_or("a missing file passes")?;
        ensure(
            missing.contains("no committed snapshot"),
            "a missing file is said",
            &missing,
        )?;
        std::fs::remove_dir_all(&dir).map_err(|error| error.to_string())
    }
}
