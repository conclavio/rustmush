//! The recorded wire contract (doc 33 §4.1, cut GG.0).
//!
//! `roundtrip.rs` proves that the codec of ONE build agrees with itself. The
//! compatibility window needs more: a server at version N must decode every
//! frame a gateway at N-1 sends, and a gateway at N-1 must decode what an N
//! server sends it (what the per-link gate lets through, which is the N-1
//! shape). The only trustworthy witness to "what an N-1 build put on the wire"
//! is bytes that build wrote, so each protocol version has a recorded set under
//! `tests/golden/v<N>/`: one length-prefixed frame per sample of
//! [`samples::every_to_server`] and [`samples::every_to_gateway`], recorded by
//! the build whose `IPC_VERSION` was N and never edited by hand.
//!
//! Three suites read the sets:
//!
//! - [`the_current_version_set_is_recorded_and_matches_the_codec`] proves the
//!   set for the current `IPC_VERSION` exists and is byte-for-byte what this
//!   build encodes. Any wire change under a fixed version — a new sample, an
//!   added field, a changed dependency type — fails it until the set is
//!   re-recorded, so the diff of `tests/golden/` is the record of every change
//!   to the wire.
//! - [`the_previous_version_set_decodes_under_the_current_codec`] proves the
//!   window's promise, both directions. A removal that breaks it waits one
//!   release (protocol.rs, "Inside the window, additive only").
//! - `crates/mush-contract-tests/tests/ipc_golden_frames.rs` is the bump lint,
//!   which fails when `IPC_VERSION` is newer than the newest recorded set.
//!
//! Recording (the `#[ignore]`d test; it replaces the current version's set):
//!
//! ```text
//! cargo test -p mush-ipc --test golden record_golden_frames -- --ignored
//! ```
//!
//! Record BEFORE bumping when the old set is missing, and again after the bump:
//! the set for a version is only ever written by a build at that version.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use mush_ipc::codec::decode_frame;
use mush_ipc::{IPC_VERSION, ToGateway, ToServer, encode_frame};

mod samples;
use samples::{every_to_gateway, every_to_server};

/// The first version with a recorded set. The window reaches one step back,
/// so there is nothing to decode before it.
const FIRST_GOLDEN_VERSION: u32 = 12;

/// The gateway → server direction's directory inside a set.
const TO_SERVER: &str = "to_server";
/// The server → gateway direction's directory inside a set.
const TO_GATEWAY: &str = "to_gateway";

/// The command that re-records the current version's set.
const RECORD_COMMAND: &str =
    "cargo test -p mush-ipc --test golden record_golden_frames -- --ignored";

/// `crates/mush-ipc/tests/golden/`.
fn golden_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
}

/// `crates/mush-ipc/tests/golden/v<version>/`.
fn set_dir(version: u32) -> PathBuf {
    golden_root().join(format!("v{version}"))
}

/// The variant a frame's `Debug` rendering names: `Inbound(Cmd { .. })` is
/// `Inbound`, `Deliver { .. }` is `Deliver`, `Bye` is `Bye`.
fn variant_name(debug: &str) -> &str {
    debug
        .split([' ', '(', '{'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(debug)
}

/// Encode one direction's samples, named `<direction>/<Variant>-<nn>.frame`
/// with `nn` counting the samples of that variant in list order.
fn encode_direction<T: std::fmt::Debug + serde::Serialize>(
    direction: &str,
    samples: &[T],
    into: &mut BTreeMap<String, Vec<u8>>,
) {
    let mut per_variant: BTreeMap<String, usize> = BTreeMap::new();
    for sample in samples {
        let variant = variant_name(&format!("{sample:?}")).to_string();
        let index = per_variant.entry(variant.clone()).or_insert(0);
        let name = format!("{direction}/{variant}-{index:02}.frame");
        *index += 1;
        let bytes = encode_frame(sample).expect("a sample frame encodes");
        let previous = into.insert(name.clone(), bytes);
        assert!(previous.is_none(), "duplicate sample name {name}");
    }
}

/// Every sample of both directions as this build encodes it.
fn current_encoding() -> BTreeMap<String, Vec<u8>> {
    let mut set = BTreeMap::new();
    encode_direction(TO_SERVER, &every_to_server(), &mut set);
    encode_direction(TO_GATEWAY, &every_to_gateway(), &mut set);
    set
}

/// Read a recorded set as `<direction>/<file>` → bytes. `None` when the set's
/// directory does not exist.
fn read_set(version: u32) -> Option<BTreeMap<String, Vec<u8>>> {
    let dir = set_dir(version);
    if !dir.is_dir() {
        return None;
    }
    let mut set = BTreeMap::new();
    for direction in [TO_SERVER, TO_GATEWAY] {
        let Ok(entries) = fs::read_dir(dir.join(direction)) else {
            continue;
        };
        for entry in entries {
            let path = entry.expect("a readable golden entry").path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("frame") {
                continue;
            }
            let file = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("a utf-8 golden file name")
                .to_string();
            let bytes = fs::read(&path).expect("a readable golden frame");
            set.insert(format!("{direction}/{file}"), bytes);
        }
    }
    Some(set)
}

/// Write `set` as the recorded set for `version`, replacing what was there.
fn write_set(version: u32, set: &BTreeMap<String, Vec<u8>>) {
    let dir = set_dir(version);
    if dir.exists() {
        fs::remove_dir_all(&dir).expect("the old set is removable");
    }
    for (name, bytes) in set {
        let path = dir.join(name.replace('/', std::path::MAIN_SEPARATOR_STR));
        fs::create_dir_all(path.parent().expect("a frame has a directory"))
            .expect("the set's directories are creatable");
        fs::write(&path, bytes).expect("a golden frame is writable");
    }
}

/// Decode every frame of a set with this build's codec, each as the type its
/// direction carries, and insist each frame is exactly one whole message.
fn decode_set(version: u32, set: &BTreeMap<String, Vec<u8>>) -> (usize, usize) {
    let (mut to_server, mut to_gateway) = (0, 0);
    for (name, bytes) in set {
        let consumed = if name.starts_with(TO_SERVER) {
            to_server += 1;
            let (_, consumed): (ToServer, usize) = decode_frame(bytes)
                .unwrap_or_else(|e| panic!("v{version} {name} does not decode as ToServer: {e}"))
                .unwrap_or_else(|| panic!("v{version} {name} is a truncated frame"));
            consumed
        } else {
            to_gateway += 1;
            let (_, consumed): (ToGateway, usize) = decode_frame(bytes)
                .unwrap_or_else(|e| panic!("v{version} {name} does not decode as ToGateway: {e}"))
                .unwrap_or_else(|| panic!("v{version} {name} is a truncated frame"));
            consumed
        };
        assert_eq!(
            consumed,
            bytes.len(),
            "v{version} {name} holds exactly one frame"
        );
    }
    (to_server, to_gateway)
}

/// Re-record the current version's set. Ignored: it writes into the tree.
#[test]
#[ignore = "writes tests/golden/v<IPC_VERSION>/; run it on purpose after a wire change"]
fn record_golden_frames() {
    let set = current_encoding();
    write_set(IPC_VERSION, &set);
    let (to_server, to_gateway) = decode_set(IPC_VERSION, &set);
    eprintln!(
        "recorded {} ToServer and {} ToGateway frames to {}",
        to_server,
        to_gateway,
        set_dir(IPC_VERSION).display()
    );
}

/// The set for this build's `IPC_VERSION` exists and is exactly what this build
/// encodes. A difference is a wire change: re-record, and let the diff of
/// `tests/golden/` say what moved.
#[test]
fn the_current_version_set_is_recorded_and_matches_the_codec() {
    let recorded = read_set(IPC_VERSION).unwrap_or_else(|| {
        panic!(
            "no recorded set for IPC_VERSION {IPC_VERSION} at {}; record it with `{RECORD_COMMAND}`",
            set_dir(IPC_VERSION).display()
        )
    });
    let current = current_encoding();
    let mut differences = Vec::new();
    for (name, bytes) in &current {
        match recorded.get(name) {
            None => differences.push(format!("missing from the recorded set: {name}")),
            Some(old) if old != bytes => differences.push(format!(
                "encoding changed: {name} ({} recorded bytes, {} now)",
                old.len(),
                bytes.len()
            )),
            Some(_) => {}
        }
    }
    for name in recorded.keys() {
        if !current.contains_key(name) {
            differences.push(format!("recorded but no longer a sample: {name}"));
        }
    }
    assert!(
        differences.is_empty(),
        "the wire changed under IPC_VERSION {IPC_VERSION} without a re-record:\n  {}\n\
         If the change is deliberate and additive (protocol.rs, \"Inside the window, additive \
         only\"), run `{RECORD_COMMAND}` and commit tests/golden/; a new variant or a changed \
         meaning is a bump first.",
        differences.join("\n  ")
    );
    let (to_server, to_gateway) = decode_set(IPC_VERSION, &recorded);
    assert!(
        to_server > 0 && to_gateway > 0,
        "both directions are recorded"
    );
}

/// The window's promise: every frame the previous version's build put on the
/// wire decodes under this build. The gateway → server half is the server at N
/// serving a gateway at N-1; the server → gateway half is the N-1 shape that
/// the per-link gate emits to such a gateway, which an N decoder must still
/// read so that a removal waits one release.
#[test]
fn the_previous_version_set_decodes_under_the_current_codec() {
    let Some(previous) = IPC_VERSION.checked_sub(1) else {
        return;
    };
    if previous < FIRST_GOLDEN_VERSION {
        // Nothing was recorded before the window opened.
        return;
    }
    let set = read_set(previous).unwrap_or_else(|| {
        panic!(
            "no recorded set for the previous version {previous} at {}; the window is one step \
             and the previous version's set is the contract this build must still honour (it is \
             recorded by a build AT that version, before the bump)",
            set_dir(previous).display()
        )
    });
    let (to_server, to_gateway) = decode_set(previous, &set);
    assert!(
        to_server > 0 && to_gateway > 0,
        "v{previous} records both directions ({to_server} ToServer, {to_gateway} ToGateway)"
    );
}

#[test]
fn variant_names_come_from_the_debug_rendering() {
    assert_eq!(variant_name("Inbound(Cmd { conn: 7 })"), "Inbound");
    assert_eq!(variant_name("Deliver { target: All }"), "Deliver");
    assert_eq!(variant_name("Bye"), "Bye");
    let names: std::collections::BTreeSet<String> = current_encoding()
        .keys()
        .map(|name| name.split(['/', '-']).nth(1).unwrap_or("").to_string())
        .collect();
    assert!(
        names.contains("Hello"),
        "the handshake is sampled: {names:?}"
    );
    assert!(names.contains("Request"), "requests are sampled: {names:?}");
}
