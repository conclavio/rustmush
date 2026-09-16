//! `mush-housing` — the game-neutral housing library (E5a) and the module that
//! attaches it to a world (E5b).
//!
//! **E5a** is world-free: property and tenancy records, the tenancy state machine, the
//! multi-resident support assessment, the access predicate, and the `Obj.home`
//! invariant helpers — all unit-tested on hand-built records, exactly the shipped
//! E2a/E3a/E4a pure-library shape.
//!
//! **E5b** is [`HousingModule`]: persistence and boot reconciliation
//! (`src/module.rs`, `src/boot.rs`), the `MoveAttempt`/`CarryAttempt` hook, the
//! destroy gate and the `ObjDestroyed` backstop (`src/hooks.rs`), the
//! cadence-driven assessment tick (`src/tick.rs`), the exact-mode bridge to E4
//! contracts and obligations (`src/econ.rs`), the story window and the `Obj.home`
//! invariant (`src/flow.rs`), the bounded offline notices (`src/notice.rs`), and
//! the `@property/*`, `@tenancy/*`, `+home`, `+guest` surface (`src/commands.rs`).
//!
//! ## Design spine (08d §3, decisions D-E5-1 … D-E5-14)
//!
//! - **A home never disappears on its own** (D-E5-1). Automatic processing stops at
//!   `EvictionPending` with a bounded story window. [`TenancyStatus::Ended`] *carries*
//!   a [`Resolution`], and [`resolve`] is the only function that produces it, so
//!   "no path to `Ended` without an explicit act" is a property of the type, not of a
//!   code review.
//! - **Game-neutral and sheet-blind** (D-E5-5). No World of Darkness vocabulary and
//!   no dots: a [`SupportLevel`] is an opaque `0..=5` the ruleset derives, a
//!   [`PropertyTier`] is an opaque `0..=5` physical band, and [`SupportSource`] is an
//!   attribution label. Housing never reads a sheet; `ruleset-wod` submits plans
//!   through [`submit_plan`].
//! - **Best-of across residents** (D-E5-13). Any one paying tenant covers a shared
//!   lease; plans are keyed by resident and cannot overwrite one another.
//! - **Access fails closed** (D-E5-4). [`may_enter`] returns [`Access::Refused`] for
//!   every case it does not positively recognise; tenants and guests are admitted at
//!   every live status, including `Delinquent`, because pressure is strain, status and
//!   a story hook, never a locked door. [`may_take`] *is* [`may_enter`].
//! - **`Obj.home` follows the tenancy 1:1** (D-E5-11), because core's destroy
//!   dispersal honours no hook: [`homed_on_activate`] and [`to_unhome_on_end`].
//! - **No clock is read here** (D-E5-14). Every `now` is a position on the tenancy's
//!   own [`mush_economy::CadenceId`] clock, handed in by the caller; a clock that
//!   moved backwards yields zero elapsed periods.
//! - **Caps are enforced, never asserted** (§9). Fallible constructors and fallible
//!   transitions; no `unwrap`, no `expect`, no panicking index on record data, and
//!   checked arithmetic on every period count.
//!
//! ## What is deliberately not here
//!
//! No ruleset edge (`SupportLevel` stays opaque — D-E5-5), no `mush-scenes` edge
//! ([`SceneRef`] is an opaque handle the module maps), no `mush-sim` edge (D-E5-12's
//! sim-space rule is enforced by the thoroughfare test and a wiring-frozen reserved
//! list, not by asking the simulation), no `mush-config` edge (the host wires what it
//! knows), and no registration with doc 09's `action-state` families or the
//! authoritative store lane — 08c §12's "K3 review" decision keeps the E-series
//! records conventional for now (08d §3.5).
//!
//! E5b adds exactly one dependency E5a did not have: **`mush-jobs`**, because the
//! bare `u64` a story window carries has to become a real ticket in the `tacsim-story`
//! queue for an ST to answer it.

// WS01-S12: the panic-shaped restriction lints this crate measures zero of,
// held at zero. `crates/mush-contract-tests/tests/restriction_lints.rs` carries
// the measurement this set was cut from — including the counts of the lints
// NOT named here — and fails if this attribute loses one of them.
#![deny(
    clippy::unwrap_used,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

// E5a — the pure library.
mod access;
mod assess;
mod error;
mod home;
mod ids;
mod record;
mod tenancy;

// E5b — the world-attached module.
mod boot;
mod commands;
mod econ;
mod flow;
mod hooks;
mod module;
mod notice;
mod quarantine;
mod tick;

pub use access::{Access, may_enter, may_enter_with, may_take, may_take_with};
pub use assess::{
    Assessment, PLAN_MAX_AGE_PERIODS, UNKNOWN_GRACE, assess, assess_at, best_level, harden,
};
pub use error::HousingError;
pub use home::{homed_inside, homed_on_activate, to_unhome_on_end};
pub use ids::{PropertyId, SceneRef, TenancyId};
pub use record::{
    AccessTerms, Feature, Funding, InspectionTerm, InspectionWhen, MAX_FEATURE_KEY_BYTES,
    MAX_FEATURES, MAX_GUESTS, MAX_LEVEL, MAX_PLAN_SOURCES, MAX_PROPERTY_NAME_BYTES,
    MAX_REASON_BYTES, MAX_ROOMS_PER_PROPERTY, MAX_TENANCY_HISTORY, MAX_TENANTS, Property,
    PropertyStatus, PropertyTier, Resolution, StoryWindow, SupportLevel, SupportPlan,
    SupportSource, Tenancy, TenancyEvent, TenancyStatus, TenancyStatusKind, submit_plan,
};
pub use tenancy::{
    DELINQUENT_PERIODS, GRACE_PERIODS, MAX_WINDOW_NOTICES, RELAPSE_WINDOW_PERIODS, WINDOW_ESCALATE,
    WINDOW_ESCALATED_RENOTIFY, WINDOW_RENOTIFY, WindowAction, accept, advance, attach_window,
    delinquent_elapsed, grace_elapsed, period_missed, period_supported, periods_since, resolve,
    window_due, window_elapsed,
};

// E5b — the module surface.
pub use commands::officer_of;
pub use commands::{
    LIST_CAP, NO_SUCH_PROPERTY, NO_SUCH_TENANCY, may_view_property, may_view_tenancy,
};
pub use econ::{BindEdge, HousingEconomyObserver, PENDING_OBLIGATION};
pub use flow::{create_tenancy, submit_support_plan};
pub use hooks::{NO_DESTROY, REFUSED, REFUSED_CARRY, may_reach};
pub use module::{
    AssessState, CAPABILITIES, HousingModule, HousingStore, LiveEntry, MAX_ASSESSMENTS_PER_FIRE,
    MAX_NOTICE_BYTES, MAX_PENDING_NOTICES, MAX_VISITED_PER_FIRE, MAX_WORKED_PER_FIRE, META_KEY,
    NO_PROPERTY_MARKER, NS, PendingNotice, RoomEntry, STORY_QUEUE, TICK_PERIOD_SECS, TICK_TAG,
    TickStats, assess_key, cadence_now, is_staff, notice_key, property_key, render_principal,
    tenancy_key,
};
pub use quarantine::{Conflict, MAX_QUARANTINE_PAGES, QuarantineKey};
