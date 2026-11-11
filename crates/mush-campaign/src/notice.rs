//! **The operation's notice, pushed** (doc 45 cut AP.1; Q-45-15, Q-45-17).
//!
//! T6's rule is that a declared operation gives every party entitled to defend sufficient
//! notice of its window. Until this cut the notice was a lens only: a defender learned of it by
//! typing `@op/show` or `@op/list` ([`crate::render`]). Now the campaign tells them.
//!
//! - **When.** Once, when the operation's terms freeze (`@op/activate`): the audience and the
//!   window are final then, and the notice period is counted from that moment
//!   (`commands::op_activate` moves the notice stamp and the window with it, so what a defender
//!   is told is never less than [`crate::OPERATION_NOTICE_MIN`] ahead). The tick makes the push
//!   ([`crate::tick`]) and marks the runtime ([`crate::OperationRuntime::noticed`]) in the same
//!   batch, so a restart never tells anyone twice.
//! - **Whom.** Each eligible defender the terms name as an object, when [`crate::render::lens_for`]
//!   shows them the operation at all. A covert operation (`visibility=covert`) pushes nothing,
//!   as it shows nothing: the push never tells a defender more than `@op/show` would.
//! - **What.** The notice lens and nothing else: the window, and the issuer only when the terms
//!   announce it. Never the objective, the stakes, the roster or the bond.
//! - **How.** Through a composition seam ([`DefenderNotice`]), on
//!   [`crate::CampaignModule::with_formation_kinds`]' shape: the composition root hands in the
//!   function that queues the line for a player who is not connected and posts the letter. This
//!   crate takes no edge to a mail module, to the reconnect queue or to any field host: nothing
//!   here reads or gates a fight (Q-45-15: fights need no operation). Unwired, the line is said
//!   to a connected defender and nothing is kept.

use std::sync::Arc;

use mush_core::{Ctx, Flags, ObjId};
use mush_org::PrincipalId;

use crate::record::OperationDoc;
use crate::render::{Lens, Verbosity, lens_for, render};

/// **The line a defender is told** (`{id}` the operation, `{days}` and `{hours}` how far ahead
/// its window opens in IC time, `{length}` how many IC hours it runs).
pub const DEFENDER_NOTICE: &str = "Operation {id} names you an eligible defender: its window opens in {days} IC days {hours} hours and runs {length} IC hours. @op/show {id} has the notice.";
/// The letter's subject (`{id}`).
pub const DEFENDER_SUBJECT: &str = "Operation {id}: notice to its eligible defenders";

/// One defender's notice, as the seam delivers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefenderLetter {
    /// The one line: said to a connected defender, queued for one who is not.
    pub line: String,
    /// The letter's subject.
    pub subject: String,
    /// The letter's body: the line, then the notice as `@op/show` renders it.
    pub body: String,
}

/// The seam a defender's notice goes through: `(ctx, defender, letter)`.
pub type DefenderNoticeFn = dyn Fn(&mut Ctx<'_>, ObjId, &DefenderLetter) + Send + Sync;

/// **The notice's way out** (world state; this module's head, "How"): the composition root's
/// function, or nothing.
#[derive(Clone, Default)]
pub struct DefenderNotice(Option<Arc<DefenderNoticeFn>>);

impl DefenderNotice {
    /// The seam, wired.
    #[must_use]
    pub fn new(post: Arc<DefenderNoticeFn>) -> DefenderNotice {
        DefenderNotice(Some(post))
    }
}

impl std::fmt::Debug for DefenderNotice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("DefenderNotice")
            .field(&self.0.is_some())
            .finish()
    }
}

/// **What `doc`'s defenders are told now** (this module's head, "What"). `now_ic` is the world's
/// IC second on the window's own clock.
#[must_use]
pub fn letter(doc: &OperationDoc, now_ic: i64) -> DefenderLetter {
    let id = doc.id().to_string();
    let ahead = doc.terms.window.0.at.saturating_sub(now_ic).max(0);
    let length = doc
        .terms
        .window
        .1
        .at
        .saturating_sub(doc.terms.window.0.at)
        .max(0);
    let line = DEFENDER_NOTICE
        .replace("{id}", &id)
        .replace("{days}", &ahead.div_euclid(86_400).to_string())
        .replace(
            "{hours}",
            &ahead.rem_euclid(86_400).div_euclid(3_600).to_string(),
        )
        .replace("{length}", &length.div_euclid(3_600).to_string());
    let mut body = vec![line.clone()];
    body.extend(render(doc, Lens::Notice, Verbosity::Normal));
    DefenderLetter {
        line,
        subject: DEFENDER_SUBJECT.replace("{id}", &id),
        body: body.join("\n"),
    }
}

/// **The defenders `doc` tells** (this module's head, "Whom"), in the terms' order, each once.
#[must_use]
pub fn defenders(w: &mush_core::World, doc: &OperationDoc) -> Vec<ObjId> {
    let mut out: Vec<ObjId> = Vec::new();
    for member in &doc.terms.eligible_defenders.members {
        let PrincipalId::Object(who) = member else {
            continue;
        };
        let live = w
            .get(*who)
            .is_some_and(|object| !object.flags.contains(Flags::GOING));
        if live && !out.contains(who) && lens_for(w, doc, *who) != Lens::Hidden {
            out.push(*who);
        }
    }
    out
}

/// **Push `doc`'s notice** to its defenders, answering how many were told.
pub(crate) fn push(ctx: &mut Ctx<'_>, doc: &OperationDoc) -> usize {
    let to = defenders(ctx.w, doc);
    if to.is_empty() {
        return 0;
    }
    let now_ic = ctx.w.world_timeline().stamp_at(ctx.w.now()).at;
    let letter = letter(doc, now_ic);
    let post = ctx
        .w
        .module_state::<DefenderNotice>()
        .and_then(|slot| slot.0.clone());
    for who in &to {
        match &post {
            Some(post) => post(ctx, *who, &letter),
            None => ctx.out.sys_to_actor(*who, "campaign", letter.line.clone()),
        }
    }
    to.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::OperationId;
    use crate::record::OperationRuntime;
    use crate::terms::operation_consent_hash;
    use crate::testfix::terms_fixture;

    /// The pushed notice is the notice lens: the window, never the plan.
    #[test]
    fn ap1_a_defenders_letter_carries_the_window_and_never_the_plan() {
        let id = OperationId(4);
        let mut terms = terms_fixture(id);
        terms.eligible_defenders.members = vec![PrincipalId::Object(ObjId(200))];
        terms.visibility = mush_economy::EconomicVisibility::Audience {
            members: terms.eligible_defenders.members.clone(),
            announce_issuer: false,
        };
        terms.consent_hash = operation_consent_hash(&terms);
        let doc = OperationDoc {
            terms,
            runtime: OperationRuntime::declared(id),
        };
        let ahead = 28 * 86_400 + 5 * 3_600;
        let now = doc.terms.window.0.at - ahead;
        let letter = letter(&doc, now);
        assert!(
            letter.line.contains("opens in 28 IC days 5 hours"),
            "{}",
            letter.line
        );
        assert!(letter.line.len() <= 240, "one row of the reconnect queue");
        assert!(letter.body.starts_with(&letter.line));
        assert!(letter.body.contains("You are entitled to defend"));
        for leak in [
            doc.terms.objective.summary.as_str(),
            "Bond",
            "Participants",
            "declared by",
            "State:",
        ] {
            assert!(!letter.body.contains(leak), "the notice leaked {leak:?}");
        }
    }
}
