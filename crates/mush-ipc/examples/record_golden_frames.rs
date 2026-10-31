//! Record the golden frame set of this build's `IPC_VERSION` (doc 33 §4.1, cut
//! GG.0): every sample of `tests/samples/mod.rs`, one length-prefixed frame
//! each, into `tests/golden/v<IPC_VERSION>/`, replacing what was there.
//!
//! ```text
//! cargo run -p mush-ipc --example record_golden_frames
//! ```
//!
//! Run it at the version being recorded: BEFORE a bump when the old version's
//! set is missing, and again after the bump. A set is only ever written by a
//! build AT that version and never edited by hand; `tests/golden.rs` then holds
//! the current set to byte equality and decodes the previous one.
//!
//! A tool, not a test: a program that writes into the tree proves nothing by
//! passing, and `mush-contract-tests`'s gated-proof inventory reads an
//! `#[ignore]`d test as a proof some CI job must run.

#[path = "../tests/samples/mod.rs"]
mod samples;
// The recorder uses the writing half of the shared module; `tests/golden.rs`
// the reading half. Each root sees the other half as dead code.
#[path = "../tests/golden_set/mod.rs"]
#[allow(dead_code)]
mod golden_set;

use golden_set::{current_encoding, decode_set, set_dir, write_set};
use mush_ipc::IPC_VERSION;

fn main() {
    let set = current_encoding();
    write_set(IPC_VERSION, &set);
    let (to_server, to_gateway) = decode_set(IPC_VERSION, &set);
    println!(
        "recorded {to_server} ToServer and {to_gateway} ToGateway frames to {}",
        set_dir(IPC_VERSION).display()
    );
}
