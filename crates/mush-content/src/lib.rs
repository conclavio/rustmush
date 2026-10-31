//! # mush-content
//!
//! **Content packs** (doc 14 §2.5; plan §3.1, cut S1): the JSON pack format and its
//! digest ([`pack`]), the once-per-pack apply latch with create / update / retire
//! ([`latch`]), and the registry of entry kinds the composed server can decode.
//!
//! This crate knows no world object kind and no game system. An owner — `mush-atlas`
//! for places, `mush-npc` for people — declares the entry kinds it decodes and does the
//! decoding and the object-making at plan and apply time; this crate supplies the
//! discipline around it: strict bytes, a content-addressed identity, dependency pinning
//! by version and by digest, and a latch that maps every applied entry to the object it
//! made and the revision it was made from, so an unchanged pack applied twice does
//! nothing and a retire never orphans a reference.
//!
//! Boot spends nothing here: loading and applying a pack makes no call outside the
//! world task (doc 14 P9).

// WS01-S12: a new crate is born with every panic-shaped restriction lint denied,
// and `crates/mush-contract-tests/tests/restriction_lints.rs` holds it there.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

pub mod apply;
pub mod latch;
pub mod module;
pub mod pack;

pub use apply::{
    Appliers, ApplyError, ApplyReport, EntryApplier, PLAN_FINGERPRINT_CHARS,
    PLAN_FINGERPRINT_DOMAIN, Plan, PlannedEntry, Step, apply, check_pack as check_pack_with, plan,
    retire_pack,
};
pub use latch::{
    Applied, AppliedEntry, AppliedEntryWire, AppliedWire, LatchError, Latches, NS, applied_key,
};
pub use module::{ContentModule, ContentState, apply_pack, check_pack};
pub use pack::{
    ENTRY_DIGEST_DOMAIN, EntryKinds, LoadedPack, MAX_ENTRY_BYTES, MAX_PACK_BYTES, MAX_PACK_ENTRIES,
    MAX_REQUIRES, PACK_DIGEST_DOMAIN, Pack, PackError, PackKind, PackManifest, PackManifestWire,
    PackRequirement, PackWire, RawEntry, RawEntryWire, check_requires, load_pack, pack_digest,
    parse_pack,
};
