//! **Services on the exchange** (doc 48 cut EX.5; Q-48-3, -11, -12, -58).
//!
//! A venue's job board is the projection of its open services. Each kind's host keeps the
//! truth (a retrieval's standing offer, a survey commission, a traveller waiting for passage)
//! and answers [`ServiceBoard::services`] from it each time the board is shown, so the market
//! stores nothing and nothing drifts. `+market/jobs` prints the board; each row names the kind's
//! own verb that takes it, and a job a stranger joins goes through the economy's generalised
//! join (`mush_economy::join_job`), which escrows the provider's bond and asks both budgets.
//!
//! The row is typed ([`crate::Service`]): the kind, the side, the pay, the bond and the term
//! are fields, so a filter and a render never parse prose. The subject alone is the host's
//! words.

use mush_core::{CmdError, Ctx, World};
use mush_economy::Amount;

use crate::ids::MarketId;
use crate::record::{MAX_SERVICES_SHOWN, Service, ServiceKind, ServiceTerm};

/// **The board seam** (doc 48 cut EX.5): what a composition's hosts answer for a venue's open
/// services. Read-only; a host answers from its own records and never writes.
pub trait ServiceBoard: Send + Sync {
    /// The open services `venue`'s board shows, in the host's order, at most
    /// [`MAX_SERVICES_SHOWN`] (the market truncates a longer answer).
    fn services(&self, w: &World, venue: &MarketId) -> Vec<Service>;
}

/// `+market/jobs` usage.
pub const JOBS_USAGE: &str = "Usage: +market/jobs [retrieval|passage|survey] [market=<venue>]";
/// No venue here and none named.
pub const JOBS_NO_VENUE: &str = "You are not at a market here; name one with market=<venue>.";
/// An empty board.
pub const JOBS_NONE: &str = "No jobs are posted on this board.";
/// The board's heading: `{n}` jobs.
pub const JOBS_HEAD: &str = "Jobs on this board ({n}):";
/// A job's first line: `{kind}`, `{subject}`, `{pay}` and `{bond}`.
pub const JOB_LINE: &str = "{kind}: {subject}. Pays {pay}{bond}.";
/// A negotiable pay's mark, after the amount.
pub const JOB_NEGOTIABLE: &str = " to open";
/// A bond's clause, inside [`JOB_LINE`]: `{bond}`.
pub const JOB_BOND: &str = ", bond {bond}";
/// A job's second line: `{term}`, `{poster}` and `{take}`.
pub const JOB_TAKE: &str = "  {term}; posted by {poster}. Take it: {take}";
/// A term from acceptance: `{days}` IC days.
pub const TERM_FROM_ACCEPTANCE: &str = "{days} IC days from acceptance";
/// An absolute deadline still ahead: `{days}` IC days.
pub const TERM_BY: &str = "due within {days} IC days";
/// A deadline today.
pub const TERM_TODAY: &str = "due today";
/// Settled on performance.
pub const TERM_ON_PERFORMANCE: &str = "paid on arrival";
/// More than the board shows.
pub const JOBS_MORE: &str = "(More are posted than one board shows.)";

/// **The open services on `venue`'s board** (doc 48 cut EX.5): the composition's board, at most
/// [`MAX_SERVICES_SHOWN`], filtered to `kind` when one is named. Empty when no board is
/// composed.
#[must_use]
pub fn services_at(w: &World, venue: &MarketId, kind: Option<ServiceKind>) -> Vec<Service> {
    let Some(board) = crate::module::service_board(w) else {
        return Vec::new();
    };
    board
        .services(w, venue)
        .into_iter()
        .filter(|s| &s.venue == venue && kind.is_none_or(|k| s.kind == k))
        .take(MAX_SERVICES_SHOWN + 1)
        .collect()
}

/// The term, in the board's words.
fn term_words(w: &World, term: ServiceTerm) -> String {
    const DAY: i64 = 86_400;
    match term {
        ServiceTerm::FromAcceptance { ic_secs } => {
            TERM_FROM_ACCEPTANCE.replace("{days}", &(ic_secs / DAY).to_string())
        }
        ServiceTerm::By { ic_at } => {
            let left = ic_at.saturating_sub(w.world_ic_now().at);
            if left < DAY {
                TERM_TODAY.to_string()
            } else {
                TERM_BY.replace("{days}", &(left / DAY).to_string())
            }
        }
        ServiceTerm::OnPerformance => TERM_ON_PERFORMANCE.to_string(),
    }
}

/// **A service's two board lines** (doc 48 cut EX.5).
#[must_use]
pub fn service_lines(w: &World, s: &Service) -> [String; 2] {
    let money = |a: Amount| crate::commands::money(w, s.instrument.as_ref(), a);
    let mut pay = money(s.pay);
    if s.negotiable {
        pay.push_str(JOB_NEGOTIABLE);
    }
    let bond = s
        .bond
        .map(|b| JOB_BOND.replace("{bond}", &money(b)))
        .unwrap_or_default();
    [
        JOB_LINE
            .replace("{kind}", s.kind.label())
            .replace("{subject}", &s.subject)
            .replace("{pay}", &pay)
            .replace("{bond}", &bond),
        JOB_TAKE
            .replace("{term}", &term_words(w, s.term))
            .replace("{poster}", &s.poster_name)
            .replace("{take}", &s.take),
    ]
}

/// `+market/jobs [kind] [market=<venue>]`.
pub(crate) fn jobs(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let arg = ctx.args.raw().trim().to_string();
    let mut kind = None;
    let mut venue = None;
    for word in arg.split_whitespace() {
        if let Some(rest) = word.strip_prefix("market=") {
            venue = Some(MarketId::new(rest).ok_or_else(|| CmdError::msg(JOBS_USAGE))?);
        } else if let Some(k) = ServiceKind::parse(word) {
            kind = Some(k);
        } else {
            return Err(CmdError::msg(JOBS_USAGE));
        }
    }
    let venue = match venue.or_else(|| crate::commands::gate_venue(ctx)) {
        Some(v) => v,
        None => return Err(CmdError::msg(JOBS_NO_VENUE)),
    };
    let services = services_at(ctx.w, &venue, kind);
    let actor = ctx.actor;
    if services.is_empty() {
        ctx.out.sys_to_actor(actor, "market", JOBS_NONE.to_string());
        return Ok(());
    }
    let shown = services.len().min(MAX_SERVICES_SHOWN);
    let mut out = vec![JOBS_HEAD.replace("{n}", &shown.to_string())];
    for s in services.iter().take(MAX_SERVICES_SHOWN) {
        out.extend(service_lines(ctx.w, s));
    }
    if services.len() > MAX_SERVICES_SHOWN {
        out.push(JOBS_MORE.to_string());
    }
    for line in out {
        ctx.out.sys_to_actor(actor, "market", line);
    }
    Ok(())
}
