//! `mush-housing` (E5a) — the pure, game-neutral housing library.
//!
//! This crate is **world-free**: property and tenancy records, the tenancy state
//! machine, the multi-resident support assessment, the access predicate, and the
//! `Obj.home` invariant helpers — all unit-tested on hand-built records, exactly the
//! shipped E2a/E3a/E4a pure-library shape. The `HousingModule` that attaches this to
//! the world (persistence and boot, the `MoveAttempt`/`CarryAttempt` hooks, the
//! destroy gate, the cadence-driven assessment tick, the story-window job, the
//! `@property/*` and `@tenancy/*` surface) is **E5b**.
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
//! No ruleset edge (`SupportLevel` is opaque), no `mush-scenes` edge ([`SceneRef`] is
//! an opaque handle E5b maps), no `mush-jobs` edge (a story-window job is a bare
//! `u64`), and no registration with doc 09's `action-state` families or the
//! authoritative store lane — 08c §12's "K3 review" decision keeps the E-series
//! records conventional for now (08d §3.5).

mod access;
mod assess;
mod error;
mod home;
mod ids;
mod record;
mod tenancy;

pub use access::{Access, may_enter, may_enter_with, may_take, may_take_with};
pub use assess::{Assessment, UNKNOWN_GRACE, assess, best_level, harden};
pub use error::HousingError;
pub use home::{homed_inside, homed_on_activate, to_unhome_on_end};
pub use ids::{PropertyId, SceneRef, TenancyId};
pub use record::{
    AccessTerms, Feature, Funding, InspectionTerm, InspectionWhen, MAX_FEATURE_KEY_BYTES,
    MAX_FEATURES, MAX_GUESTS, MAX_LEVEL, MAX_PLAN_SOURCES, MAX_PROPERTY_NAME_BYTES,
    MAX_REASON_BYTES, MAX_ROOMS_PER_PROPERTY, MAX_TENANCY_HISTORY, MAX_TENANTS, Property,
    PropertyStatus, PropertyTier, Resolution, StoryWindow, SupportLevel, SupportPlan, SupportSource,
    Tenancy, TenancyEvent, TenancyStatus, TenancyStatusKind, submit_plan,
};
pub use tenancy::{
    DELINQUENT_PERIODS, GRACE_PERIODS, MAX_WINDOW_NOTICES, WINDOW_ESCALATE,
    WINDOW_ESCALATED_RENOTIFY, WINDOW_RENOTIFY, WindowAction, accept, advance, attach_window,
    delinquent_elapsed, grace_elapsed, period_missed, period_supported, periods_since, resolve,
    window_due, window_elapsed,
};
