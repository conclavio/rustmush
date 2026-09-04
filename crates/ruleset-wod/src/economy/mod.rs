//! **E5c — the WoD economic layer** (08d §5).
//!
//! What a character's background quietly does for them, and what it quietly asks of
//! them in return: a support level derived from `backgrounds:Resources` and a stance,
//! submitted to [`mush_housing`] as an opaque plan every period; four deterministic
//! career stances that alter *pressure and opportunity* and never income; at most one
//! unresolved prompt per character; a bounded per-faction pool of off-screen
//! operations; and — only in `personal_mode = "exact"` — a real `wod.usd` account
//! with a faucet-funded income line.
//!
//! ## The three rules this whole module is shaped by
//!
//! - **The ruleset interprets dots; housing and the economy stay sheet-blind**
//!   (D-E5-5). The one sheet read in the layer is [`sheet_resources`], and there is
//!   no sheet **write** anywhere: the economy never changes a trait and never files a
//!   trait-loss job because a scheduled process ran (D-E5-3). A permanent Resources
//!   change goes through `+xp/spend`.
//! - **All money goes through E4.** `create_system_obligation` under a registered
//!   faucet, `issue_title` for a property's title, contracts for a lease. There is no
//!   direct post anywhere in this crate.
//! - **Existence hiding on every render.** A property, tenancy or account the actor
//!   may not view is answered the way `mush-housing` and `mush-economy` answer it,
//!   through their own public predicates, never with a placeholder.
//!
//! ## Layout
//!
//! | module | what |
//! |---|---|
//! | [`config`] | `[wod.economy]`, frozen at wiring |
//! | [`record`] | the profile, the career, the stances, fronts, the prompt |
//! | [`store`] | the module state and its four documents |
//! | [`boot`] | the clock, the instrument, the faucet accounts |
//! | [`evaluate`] | the period evaluation (the tick) |
//! | [`prompt`] | the attention budget, preemption, the pool, non-response |
//! | [`chargen`] | the Residence step and the approval artifacts |
//! | [`migrate`] | `@wod/economy migrate`, the one-shot mode reconciliation |
//! | [`commands`] | `+career`, `+prompt`, `+resources`, `@wod/economy`, `@career/set` |

pub mod boot;
pub mod chargen;
pub mod commands;
pub mod config;
pub mod evaluate;
pub mod migrate;
pub mod prompt;
pub mod record;
pub mod store;

use mush_core::{ObjId, World};
use mush_ruleset::SheetDoc;

pub use config::{Engagement, PersonalMode, WodEconomyConfig};
pub use record::{
    CareerProfile, FrontRef, HavenQualities, Prompt, PromptDefault, PromptKind, PromptOption,
    PromptOutcome, SpendingProfile, Stance, SupportSource,
};
pub use store::{WodEconomyStore, load as load_store};

/// The sheet attribute every WoD sheet lives at. Named here so the "the economy never
/// writes a sheet" claim is checkable by grep: the whole `economy` module mentions it
/// **once**, in [`sheet_resources`], on the read path, and `World::set_attr` appears
/// in it nowhere at all.
pub const SHEET_ATTR: &str = "sheet:wod";

/// `backgrounds:Resources`, clamped into the `0..=5` band.
///
/// `backgrounds` is a **Map** field (`schema.rs`), so `SheetDoc::dots` does not apply
/// to it; this is the case-insensitive map lookup the splats already use
/// (`splats/vampire.rs`'s `background_dots`). A character with no sheet, no
/// backgrounds map, or no Resources entry reads 0 — the same answer the sheet itself
/// gives, and the one that supports nothing.
#[must_use]
pub fn sheet_resources(w: &World, who: ObjId) -> Option<u8> {
    let doc = w
        .attr(who, SHEET_ATTR)
        .and_then(mush_core::Attr::as_json)
        .and_then(|v| serde_json::from_value::<SheetDoc>(v.clone()).ok())?;
    Some(background_dots(&doc, "Resources"))
}

/// `backgrounds:Haven`, when the character has any (D-E5-7).
///
/// Read for exactly one purpose — to write [`HavenQualities`](record::HavenQualities)
/// onto the profile, which nothing then reads back. Haven never selects a property,
/// never discounts rent, and never becomes a `PropertyTier`.
#[must_use]
pub fn sheet_haven(w: &World, who: ObjId) -> Option<u8> {
    let doc = w
        .attr(who, SHEET_ATTR)
        .and_then(mush_core::Attr::as_json)
        .and_then(|v| serde_json::from_value::<SheetDoc>(v.clone()).ok())?;
    let dots = background_dots(&doc, "Haven");
    (dots > 0).then_some(dots)
}

/// One named background's dots, case-insensitively, clamped into the band.
#[must_use]
pub fn background_dots(doc: &SheetDoc, name: &str) -> u8 {
    doc.map("backgrounds")
        .and_then(|m| {
            m.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .and_then(|(_, v)| v.as_i64())
        })
        .unwrap_or(0)
        .clamp(0, i64::from(record::MAX_LEVEL)) as u8
}
