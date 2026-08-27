//! # mush-core
//!
//! The synchronous heart of rustmush: the in-RAM [`World`], the object
//! model (objects, attributes, flags, locks, zones), and the IC game clock.
//!
//! Design ground rules (see `docs/PLAN.md`):
//!
//! - **No I/O, no async, no locks.** `World` is plain data plus methods; at
//!   runtime a single task owns it outright, and in tests it is driven
//!   directly. Nothing in this crate's public API mentions tokio.
//! - **All mutation goes through `World` methods**, which maintain every
//!   inverse index (contents/exits, player names, zone membership), stamp
//!   `modified`, and record dirty ids for the persistence layer.
//! - **Ids are never recycled** ([`obj::ObjId`] is monotonic; destroyed
//!   objects leave tombstones), so a stale dbref can never alias a new
//!   object.
//!
//! On top of the foundation sit the command layer:
//!
//! - [`perm`] — centralized permission checks (`controls`, `can_examine`,
//!   power/role gating); handlers never inline flag math.
//! - [`output`] — [`Audience`]/[`Outbox`] plus view builders emitting
//!   `mush-proto` types **directly** (treaty ruling 1: no parallel event
//!   enum in core).
//! - [`matchit`] — the single classic-precedence name resolver
//!   (`me`/`here`/`#dbref`/`*player`/inventory/location/exits/prefix/
//!   `2nd sword`).
//! - [`cmd`] — [`CommandSpec`]/[`Registry`]/[`dispatch`], implementing the
//!   treaty ruling 5 pipeline (prefix chars → alias stage → HOME → exits →
//!   command table → Huh).
//! - [`commands`] — the core MUSH command set, registered via
//!   [`register_core_commands`].

pub mod attr;
pub mod clock;
pub mod cmd;
pub mod commands;
pub mod flags;
pub mod lock;
pub mod matchit;
pub mod obj;
pub mod output;
pub mod perm;
pub mod world;

pub use attr::{Attr, AttrFlags, AttrMap, AttrValue, UncasedName};
pub use clock::{GameClock, Season, TimeOfDay};
pub use cmd::{
    AliasOutcome, AliasStage, ArgShape, CmdError, CommandHandler, CommandSpec, Ctx, LookupErr,
    ParsedArgs, Registry, RegistryError, SwitchSet, dispatch,
};
pub use commands::register_core_commands;
pub use flags::{Flags, Powers, Role};
pub use lock::{LockExpr, LockParseError, LockType, eval_lock, glob_match, parse_lock};
pub use matchit::{MatchErr, match_object};
pub use obj::{GOD, LIMBO, Obj, ObjId, ObjKind};
pub use output::{Audience, ConnId, Outbox, name_span, obj_ref, room_view};
pub use world::{World, WorldError};
