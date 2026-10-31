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
//! Recording replaces the current version's set. It is an example binary, not an
//! `#[ignore]`d test: a tool that writes into the tree proves nothing by
//! passing, and `mush-contract-tests`'s gated-proof inventory reads an ignored
//! test as a proof some CI job must run.
//!
//! ```text
//! cargo run -p mush-ipc --example record_golden_frames
//! ```
//!
//! Record BEFORE bumping when the old set is missing, and again after the bump:
//! the set for a version is only ever written by a build at that version.

use mush_ipc::IPC_VERSION;

mod samples;
// The suite reads sets; the recorder example writes them. Each root uses one
// half of the shared module and sees the other half as dead code.
#[allow(dead_code)]
mod golden_set;
use golden_set::{
    FIRST_GOLDEN_VERSION, RECORD_COMMAND, current_encoding, decode_set, read_set, set_dir,
    variant_name,
};

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
