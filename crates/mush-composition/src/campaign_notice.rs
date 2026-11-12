//! **How an operation's notice reaches its eligible defenders** (doc 45 cut AP.1; Q-45-15,
//! Q-45-17): the composition root's half of `mush_campaign::CampaignModule::with_defender_notice`.
//!
//! `mush-campaign` decides who is told, when and what; it has no edge to a mail module or to
//! TacSim. This function is the two paths the game already has, joined here where both crates
//! are in reach: the reconnect queue (`tacsim::port_tick::notify_budget`, trip level: said to a
//! defender who is connected, queued for one who is not) and an official letter
//! (`mush_mail::post_notice`), so the record outlives a login. Nothing here reads a field.

use std::sync::Arc;

use mush_campaign::{DefenderLetter, DefenderNoticeFn};
use mush_core::{Ctx, ObjId};

/// The seam's function.
#[must_use]
pub fn defender_notice() -> Arc<DefenderNoticeFn> {
    Arc::new(|ctx: &mut Ctx<'_>, who: ObjId, letter: &DefenderLetter| {
        tacsim::port_tick::notify_budget(
            ctx,
            who,
            &letter.line,
            tacsim::port_tick::NoticeLevel::Trip,
        );
        // The letter goes under the name of whoever the tick runs as: the campaign names no
        // clerk, and the issuer is never the sender (the notice may not disclose them).
        let from = ctx.actor;
        if let Err(error) = mush_mail::post_notice(
            ctx,
            from,
            std::slice::from_ref(&who),
            &letter.subject,
            &letter.body,
        ) {
            tracing::warn!(%error, to = who.0, "composition: an operation's notice was not mailed");
        }
    })
}
