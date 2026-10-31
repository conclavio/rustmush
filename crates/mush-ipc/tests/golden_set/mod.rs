//! The recorded set's layout and codec, shared by `golden.rs` (the suite that
//! reads the sets) and `examples/record_golden_frames.rs` (the tool that writes
//! the current version's). Doc 33 §4.1, cut GG.0.
//!
//! A set is `tests/golden/v<N>/<direction>/<Variant>-<nn>.frame`: one
//! length-prefixed frame per sample of `samples::every_to_server` (under
//! `to_server/`) and `samples::every_to_gateway` (under `to_gateway/`), `nn`
//! counting that variant's samples in list order.
//!
//! Not a target of its own: each root declares `mod samples;` and
//! `mod golden_set;` (the example through `#[path]`), and this module reaches
//! the samples through `super::samples`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use mush_ipc::codec::decode_frame;
use mush_ipc::{ToGateway, ToServer, encode_frame};

use super::samples::{every_to_gateway, every_to_server};

/// The first version with a recorded set. The window reaches one step back,
/// so there is nothing to decode before it.
pub const FIRST_GOLDEN_VERSION: u32 = 12;

/// The gateway → server direction's directory inside a set.
pub const TO_SERVER: &str = "to_server";
/// The server → gateway direction's directory inside a set.
pub const TO_GATEWAY: &str = "to_gateway";

/// The command that re-records the current version's set. An example binary,
/// not an `#[ignore]`d test: a tool that writes into the tree is not a proof,
/// and `mush-contract-tests`'s gated-proof inventory reads an ignored test as
/// a proof some CI job must run.
pub const RECORD_COMMAND: &str = "cargo run -p mush-ipc --example record_golden_frames";

/// `crates/mush-ipc/tests/golden/`.
pub fn golden_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
}

/// `crates/mush-ipc/tests/golden/v<version>/`.
pub fn set_dir(version: u32) -> PathBuf {
    golden_root().join(format!("v{version}"))
}

/// The variant a frame's `Debug` rendering names: `Inbound(Cmd { .. })` is
/// `Inbound`, `Deliver { .. }` is `Deliver`, `Bye` is `Bye`.
pub fn variant_name(debug: &str) -> &str {
    debug
        .split([' ', '(', '{'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(debug)
}

/// Encode one direction's samples, named `<direction>/<Variant>-<nn>.frame`
/// with `nn` counting the samples of that variant in list order.
pub fn encode_direction<T: std::fmt::Debug + serde::Serialize>(
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
pub fn current_encoding() -> BTreeMap<String, Vec<u8>> {
    let mut set = BTreeMap::new();
    encode_direction(TO_SERVER, &every_to_server(), &mut set);
    encode_direction(TO_GATEWAY, &every_to_gateway(), &mut set);
    set
}

/// Read a recorded set as `<direction>/<file>` → bytes. `None` when the set's
/// directory does not exist.
pub fn read_set(version: u32) -> Option<BTreeMap<String, Vec<u8>>> {
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
pub fn write_set(version: u32, set: &BTreeMap<String, Vec<u8>>) {
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
pub fn decode_set(version: u32, set: &BTreeMap<String, Vec<u8>>) -> (usize, usize) {
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
