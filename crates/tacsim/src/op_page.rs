//! **Following the Isidis operation: `+operation` and the notices** (doc 50 cut OP.7; Q-50-30,
//! Q-50-27, Q-45-17).
//!
//! - **`+operation`** (Q-50-30 as corrected), one page:
//!   - who declared against whom, and the stake as what each side's win changes in the terms
//!     at Isidis, read from the lane's terms rows ([`stake`]);
//!   - the notice and the window, in IC dates and real hours from now;
//!   - how it is decided, and the tank farm's level against its threshold;
//!   - both clocks, the Concession Review and Hold the Concession;
//!   - the work open for it: the Freeport arm's hauls and escorts still untaken;
//!   - who holds Isidis now and what that puts on a sale there, and past operations with their
//!     result.
//!
//!   TacSim reads the operation only through the frame's seam (`crate::isidis_op::status`); the
//!   page joins it with the clocks, the farm's reading and the job records, and writes nothing.
//! - **The job board tags the operation's jobs** ([`job_tag`]): while one is in notice or in its
//!   window, an arm's haul or escort on `+market/jobs` says it is the Freeport's work for it.
//! - **The notices** (Q-50-30, Q-45-17's channel): an official notice, at once, to every player
//!   with a hull on the lane, at the declaration (`tell_declared`) and at the resolution
//!   (`tell_resolved`). A connected player hears it; an absent one has it queued for login. The
//!   defenders' own notice is the campaign's (OP.0), so at the declaration the defender
//!   organization's active members are left out here.

use mush_core::{CanonicalId, CmdError, Ctx, Flags, ObjId, ObjKind, World};
use mush_org::PrincipalId;

use crate::isidis_op::{ISIDIS_PLACE, IsidisOp};
use crate::operations::{OperationOutcome, OperationPhase, OperationStatus};

/// The verb.
pub const OPERATION_VERB: &str = "+operation";
/// The verb's help.
pub const OPERATION_HELP: &str = "+operation   the Isidis operation: who declared against \
     whom, what each side's win changes in the terms at Isidis, the notice and the window in IC \
     and real time, both clocks, the tank farm's level against its target, the work open for it, \
     who holds Isidis now, and past operations with their result.";
/// The page's channel.
pub const CHANNEL: &str = "tacsim.operation";
/// What a board row of the operation's work carries (Q-50-30: the job board tags its jobs).
pub const JOB_TAG: &str = " (work for the Isidis operation, the Freeport's side)";
/// The declaration's notice: `{declarer}`, `{side}`, `{defender}`, `{open}`, `{open_h}`,
/// `{days}`.
pub const DECLARED_NOTICE: &str = "Official notice: {declarer} has declared an operation for \
     {side} to assume the administration of the Isidis Concession, against {defender}. Its window \
     opens {open} IC (in about {open_h} real hours) and runs {days} IC days. +operation shows \
     what each side's win changes and the work open for it.";
/// The resolution's notice: `{result}`.
pub const RESOLVED_NOTICE: &str =
    "Official notice: the Isidis operation is decided: {result} +operation shows it.";
/// The most past operations the record keeps.
pub const MAX_PAST: usize = 16;
/// **The Republic's bid** (Q-50-27): where it is posted, the port entry.
pub const BID_ENTRY: &str = "stickney_port";
/// What it buys: the stockpile's good, so every tonne sold to the Republic is one not landed
/// at Isidis.
pub const BID_GOOD: &str = crate::stockpile::STOCKPILE_GOOD;
/// What it pays, whole credits a tonne (provisional): over the tank farm's 50 at Isidis, and
/// under Kasei's 60 ask at Stickney, so nobody buys at Stickney to sell back to it there.
pub const BID_CR: i64 = 56;
/// How much it takes, tonnes (provisional): 11,200 cr at [`BID_CR`], inside what the customs
/// office holds beside its bond.
pub const BID_T: u64 = 200;

/// A side's name: the Republic or the Freeport.
fn side_name(tag: &str) -> String {
    format!("the {}", crate::polity::polity_name(tag))
}

/// A whole number of real hours from now to IC second `at` (never negative).
fn real_hours(w: &World, at: i64) -> i64 {
    let rate = i64::from(w.world_timeline().current().rate.max(1));
    at.saturating_sub(w.world_ic_now().at).max(0) / rate / 3_600
}

/// A good's label, as the catalog names it.
fn good_label(w: &World, good: &CanonicalId) -> String {
    crate::services::haul::label(w, good)
}

/// A per-mille rate as a percent: `5%`, `2.5%`.
fn percent(permille: u16) -> String {
    let (whole, tenth) = (permille / 10, permille % 10);
    if tenth == 0 {
        format!("{whole}%")
    } else {
        format!("{whole}.{tenth}%")
    }
}

/// A list in words: `a`, `a and b`, `a, b and c`; with semicolons when an item has a comma of
/// its own (`metal ore, run-of-mine; ice ore, run-of-mine and water ice`).
fn words(items: &[String]) -> String {
    let sep = if items.iter().any(|i| i.contains(',')) {
        "; "
    } else {
        ", "
    };
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [head @ .., last] => format!("{} and {last}", head.join(sep)),
    }
}

/// **What `tag`'s hold puts on a sale at Isidis** (Q-50-29), from its terms row there: the
/// import duty, its goods and its payee, or none.
#[must_use]
pub fn terms_words(w: &World, tag: &str) -> String {
    let Ok(place) = CanonicalId::new(ISIDIS_PLACE) else {
        return String::new();
    };
    match crate::polity::terms_of(w, &place, tag) {
        Some(row) if row.import_duty_permille > 0 && !row.import_goods.is_empty() => {
            let goods: Vec<String> = row.import_goods.iter().map(|g| good_label(w, g)).collect();
            let payee = crate::polity::lane_org_account(w, &row.payee).map_or_else(
                || row.payee.to_string(),
                |(org, _)| {
                    crate::surface::retrieval::principal_name(w, &PrincipalId::Organization(org))
                },
            );
            format!(
                "{} sold at Isidis pay an import duty of {} of their catalog value to {payee}",
                words(&goods),
                percent(row.import_duty_permille)
            )
        }
        _ => "cargo sold at Isidis pays no import duty".to_owned(),
    }
}

/// **The stake, in plain words** (Q-50-30): what each side's win changes in the terms.
#[must_use]
pub fn stake(w: &World) -> String {
    format!(
        "If the Republic wins, it administers the Isidis Concession, and {}. If the Freeport \
         holds, it keeps the Concession, and {}.",
        terms_words(w, "republic"),
        terms_words(w, "freeport")
    )
}

/// The polity tag that holds Isidis de facto, when it is the Republic or the Freeport.
fn holder_tag(w: &World) -> Option<&'static str> {
    let place = CanonicalId::new(ISIDIS_PLACE).ok()?;
    let holder = crate::polity::control_at(w, &place)?.de_facto?;
    ["republic", "freeport"]
        .into_iter()
        .find(|tag| crate::polity::polity(w, tag) == Some(holder))
}

/// **Who holds Isidis now, and what that puts on a sale there**: the line a newcomer reads.
#[must_use]
pub fn holder_line(w: &World) -> String {
    let place = CanonicalId::new(ISIDIS_PLACE).ok();
    let since = place
        .as_ref()
        .and_then(|p| crate::polity::control_at(w, p))
        .and_then(|c| c.de_facto_since_ic);
    match holder_tag(w) {
        Some(tag) => format!(
            "Isidis now: {} administers the Concession{}, and {}.",
            side_name(tag),
            since.map_or_else(String::new, |at| format!(
                " (since {} IC)",
                crate::port::licence::ic_stamp(at)
            )),
            terms_words(w, tag)
        ),
        None => "Isidis now: nobody administers the Concession.".to_owned(),
    }
}

/// A finished operation's result, in words.
fn result_words(outcome: &str) -> &'static str {
    match outcome {
        "republic" => "the Republic won, and assumed the Concession's administration",
        "freeport" => "the Freeport held the Concession",
        _ => "it ended with no change at Isidis",
    }
}

/// The line for a decided outcome, from the frame.
fn outcome_word(outcome: OperationOutcome) -> &'static str {
    match outcome {
        OperationOutcome::DeclarerSucceeded => "republic",
        OperationOutcome::DeclarerFailed => "freeport",
        OperationOutcome::Partial | OperationOutcome::Abandoned => "none",
    }
}

/// **Whether an operation is in notice or in its window** (the board's tag and the page's work).
#[must_use]
pub fn live(w: &World) -> Option<OperationStatus> {
    crate::isidis_op::status(w)
        .filter(|s| matches!(s.phase, OperationPhase::Notice | OperationPhase::Running))
}

/// **The board's tag for contract `contract`** (Q-50-30): [`JOB_TAG`] for one of the Freeport
/// arm's hauls or escorts while the operation is live; otherwise nothing.
#[must_use]
pub fn job_tag(w: &World, contract: u64) -> &'static str {
    let ours = crate::stockpile::arm(w).is_some_and(|arm| {
        arm.posts
            .iter()
            .any(|p| p.haul == contract || p.escort == Some(contract))
    });
    if ours && live(w).is_some() {
        JOB_TAG
    } else {
        ""
    }
}

/// The arm's work still open: one line a haul or escort waiting for a taker.
fn work_lines(ctx: &Ctx<'_>) -> Vec<String> {
    let w: &World = ctx.w;
    let Some(arm) = crate::stockpile::arm(w) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for post in &arm.posts {
        if let Some(job) = crate::services::haul::haul_of(w, post.haul)
            .filter(|j| j.stage == crate::services::haul::HaulStage::Posted)
        {
            out.push(format!(
                "  Haul {} for the Freeport: carry {} t of {} from {} to {}; pays {}. Take it at \
                 {}: +haul/accept {}",
                job.job(),
                job.quantity,
                good_label(w, &job.good),
                crate::services::venue_name(w, &job.origin),
                crate::services::venue_name(w, &job.destination),
                crate::port::coop::money_minor(ctx, job.fee_minor),
                crate::services::venue_name(w, &job.origin),
                job.job()
            ));
        }
        if let Some(job) = post
            .escort
            .and_then(|e| crate::services::escort::escort_of(w, e))
            .filter(|j| j.stage == crate::services::escort::EscortStage::Posted)
        {
            out.push(format!(
                "  Escort {} for the Freeport: guard the hauler of haul {}; pays {}. Take it at \
                 {}: +escort/accept {} with=<hull>",
                job.job(),
                post.haul,
                crate::port::coop::money_minor(ctx, job.fee_minor),
                crate::services::venue_name(w, &job.venue),
                job.job()
            ));
        }
    }
    out
}

/// A front's line: its title, whose it is, and where its clock stands.
fn clock_line(clock: Option<mush_faction::Clock>, title: &str, side: &str) -> String {
    match clock {
        Some(c) => format!(
            "  {title} ({side}): {} of {}{}.",
            c.filled,
            c.segments,
            if c.completed { ", filled" } else { "" }
        ),
        None => format!("  No clock is kept for {side} at Isidis."),
    }
}

/// **`+operation`'s page** (Q-50-30).
#[must_use]
pub fn page(ctx: &Ctx<'_>) -> String {
    let w: &World = ctx.w;
    let record: Option<&IsidisOp> = crate::isidis_op::record(w);
    let status = crate::isidis_op::status(w);
    let mut out = Vec::new();
    match (record.and_then(|r| r.operation), &status) {
        (Some(id), Some(s)) => {
            let declarer = crate::surface::retrieval::principal_name(w, &s.declarer);
            let defenders: Vec<String> = s
                .defenders
                .iter()
                .map(|d| crate::surface::retrieval::principal_name(w, d))
                .collect();
            out.push(format!(
                "The Isidis operation (operation {id}): {declarer}, for the Republic, against {}, \
                 for the Freeport, defending.",
                words(&defenders)
            ));
            out.push(format!("  The stake: {}", stake(w)));
            let (open, close) = s.window;
            let declared = record
                .and_then(|r| r.declared_ic)
                .map_or_else(String::new, |at| {
                    format!("Declared {} IC. ", crate::port::licence::ic_stamp(at))
                });
            let when = match s.phase {
                OperationPhase::Declared | OperationPhase::Notice => format!(
                    "  {declared}Notice runs: the window opens {} IC (in about {} real hours) and \
                     closes {} IC (in about {} real hours).",
                    crate::port::licence::ic_stamp(open),
                    real_hours(w, open),
                    crate::port::licence::ic_stamp(close),
                    real_hours(w, close)
                ),
                OperationPhase::Running => format!(
                    "  {declared}The window is open: it closes {} IC (in about {} real hours).",
                    crate::port::licence::ic_stamp(close),
                    real_hours(w, close)
                ),
                OperationPhase::Settling | OperationPhase::Settled => format!(
                    "  {declared}Decided: {}.",
                    result_words(s.outcome.map_or("none", outcome_word))
                ),
            };
            out.push(when);
        }
        _ => out.push(
            "No operation is declared at Isidis. When the Concession Review fills, the Republic \
             declares one."
                .to_owned(),
        ),
    }
    let level = crate::stockpile::arm(w).and_then(|a| a.level);
    out.push(format!(
        "  How it is decided: the Freeport holds if Occator's tank farm at Isidis stands at {} t \
         of {} when the window closes. {}",
        crate::stockpile::STOCKPILE_THRESHOLD_T,
        CanonicalId::new(crate::stockpile::STOCKPILE_GOOD)
            .map_or_else(|_| "propellant".to_owned(), |g| good_label(w, &g)),
        level.map_or_else(
            || "It has not been read yet.".to_owned(),
            |t| format!("It holds {t} t now.")
        )
    ));
    out.push("Clocks:".to_owned());
    out.push(clock_line(
        crate::concession::review_clock(w),
        crate::concession::CONCESSION_REVIEW_TITLE,
        "the Republic",
    ));
    out.push(clock_line(
        crate::concession::hold_clock(w),
        crate::concession::HOLD_THE_CONCESSION_TITLE,
        "the Freeport",
    ));
    if live(w).is_some() {
        out.push("Work for it:".to_owned());
        let mut work = work_lines(ctx);
        work.extend(bid_line(ctx));
        if work.is_empty() {
            out.push("  No job is posted for it now.".to_owned());
        } else {
            out.extend(work);
        }
    }
    out.push(holder_line(w));
    let past: Vec<String> = record
        .map(|r| {
            r.past
                .iter()
                .map(|p| {
                    format!(
                        "  Operation {}, declared {} IC: {}.",
                        p.operation,
                        crate::port::licence::ic_stamp(p.declared_ic),
                        result_words(&p.outcome)
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    if !past.is_empty() {
        out.push("Past operations:".to_owned());
        out.extend(past);
    }
    out.join("\n")
}

/// `+operation`'s spec.
#[must_use]
pub(crate) fn operation_spec() -> mush_core::CommandSpec {
    mush_core::CommandSpec {
        name: OPERATION_VERB.into(),
        shape: mush_core::ArgShape::Raw,
        help: OPERATION_HELP.into(),
        ..mush_core::CommandSpec::default()
    }
}

/// **`+operation`** (doc 50 cut OP.7): a pure read.
///
/// # Errors
///
/// None in practice; the verb is a read.
pub(crate) fn operation_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    crate::commands::require_enabled(ctx)?;
    let page = page(ctx);
    ctx.out.sys_to_actor(ctx.actor, CHANNEL, page);
    Ok(())
}

/// **Every player with a hull on the lane** (Q-50-30's audience): a live player, not the God
/// object, who may trade from some hull's hold (`crate::port::coop::trade_hulls`), in id order.
#[must_use]
pub fn lane_audience(w: &World) -> Vec<ObjId> {
    w.iter()
        .filter(|o| o.kind == ObjKind::Player && !o.flags.contains(Flags::GOING))
        .map(|o| o.id)
        .filter(|who| *who != mush_core::GOD)
        .filter(|who| !crate::port::coop::trade_hulls(w, *who).is_empty())
        .collect()
}

/// **The declaration's notice** (Q-50-30): to every player with a hull on the lane but those the
/// campaign's own notice tells (doc 50b item 9: [`OperationStatus::told`], so a defender's member
/// past the campaign's recipient cap still hears the lane's).
pub(crate) fn tell_declared(ctx: &mut Ctx<'_>, status: &OperationStatus) {
    let w: &World = ctx.w;
    let declarer = crate::surface::retrieval::principal_name(w, &status.declarer);
    let defenders: Vec<String> = status
        .defenders
        .iter()
        .map(|d| crate::surface::retrieval::principal_name(w, d))
        .collect();
    let (open, close) = status.window;
    let line = DECLARED_NOTICE
        .replace("{declarer}", &declarer)
        .replace("{side}", "the Republic")
        .replace("{defender}", &words(&defenders))
        .replace("{open}", &crate::port::licence::ic_stamp(open))
        .replace("{open_h}", &real_hours(w, open).to_string())
        .replace(
            "{days}",
            &(close.saturating_sub(open) / crate::port::stock::IC_DAY).to_string(),
        );
    for who in lane_audience(w) {
        if !status.told.contains(&who) {
            crate::port_tick::notify(ctx, who, &line);
        }
    }
}

/// **The resolution's notice** (Q-50-30): to every player with a hull on the lane. `outcome` is
/// the record's word.
pub(crate) fn tell_resolved(ctx: &mut Ctx<'_>, outcome: &str) {
    let result = match outcome {
        "republic" | "freeport" => format!(
            "{}; {}.",
            result_words(outcome),
            terms_words(ctx.w, outcome)
        ),
        _ => format!("{}.", result_words(outcome)),
    };
    let mut result = result;
    if let Some(first) = result.get(..1) {
        result = first.to_uppercase() + result.get(1..).unwrap_or_default();
    }
    let line = RESOLVED_NOTICE.replace("{result}", &result);
    for who in lane_audience(ctx.w) {
        crate::port_tick::notify(ctx, who, &line);
    }
}

/// The customs office's vendor and storefront vault at Stickney (lane v51's row).
fn customs_vendor(w: &World) -> Option<(mush_market::VendorId, mush_economy::VaultId)> {
    let org = CanonicalId::new(crate::isidis_op::DECLARER_ORG).ok()?;
    crate::port::market::ports(w)
        .into_iter()
        .find(|(entry, _, _)| entry.as_str() == BID_ENTRY)
        .and_then(|(_, _, map)| {
            let row = map?.vendors.get(&org)?;
            Some((
                mush_market::VendorId(row.vendor?),
                mush_economy::VaultId(row.vault),
            ))
        })
}

/// **Post the Republic's bid** (Q-50-27) at the declaration: [`BID_T`] t of [`BID_GOOD`] at
/// [`BID_CR`], into the customs office's storefront at Stickney. The listing, or `None` when the
/// lane has no such vendor or the market refuses (logged).
pub(crate) fn post_republic_bid(ctx: &mut Ctx<'_>) -> Option<u64> {
    let (vendor, vault) = customs_vendor(ctx.w)?;
    let good = CanonicalId::new(BID_GOOD).ok()?;
    let scale = 10_i64.pow(u32::from(crate::port::apply::credit_scale(ctx.w)));
    let spec = mush_market::WantedSpec {
        vendor,
        catalog: crate::port::vendor::catalog_of(&good),
        price: mush_economy::Amount::from_minor(BID_CR.saturating_mul(scale)),
        remaining: BID_T,
        vault,
        min_grade: 0,
        ttl_secs: mush_market::MAX_LISTING_TTL_SECS,
    };
    match mush_market::post_wanted(ctx, spec) {
        Ok(listing) => Some(listing.0),
        Err(error) => {
            tracing::warn!(?error, "tacsim: the Republic's bid could not be posted");
            None
        }
    }
}

/// **Withdraw the Republic's bid** at the resolution. A listing already gone is nothing to do.
pub(crate) fn withdraw_republic_bid(ctx: &mut Ctx<'_>, listing: u64) {
    let id = mush_market::ListingId(listing);
    let live = ctx
        .w
        .module_state::<mush_market::MarketStore>()
        .is_some_and(|s| s.listing(id).is_some());
    if live && let Err(error) = mush_market::withdraw_listing(ctx, id) {
        tracing::warn!(?error, "tacsim: the Republic's bid could not be withdrawn");
    }
}

/// The Republic's bid, while it stands: its line on the page.
fn bid_line(ctx: &Ctx<'_>) -> Option<String> {
    let listing = crate::isidis_op::record(ctx.w)?.bid?;
    let l = ctx
        .w
        .module_state::<mush_market::MarketStore>()?
        .listing(mush_market::ListingId(listing))?
        .clone();
    Some(format!(
        "  The Republic's bid: Republic Customs Office buys up to {} t of {} at {} a tonne at \
         {}. Sell into it there: +market/sell {listing}=<qty>",
        l.quantity,
        CanonicalId::new(BID_GOOD).map_or_else(|_| BID_GOOD.to_owned(), |g| good_label(ctx.w, &g)),
        crate::port::coop::money_minor(ctx, l.price.minor()),
        crate::services::venue_name(ctx.w, "stickney")
    ))
}

/// **Push a finished operation onto the record's past** (at most [`MAX_PAST`], oldest dropped).
pub(crate) fn keep_past(doc: &mut IsidisOp) {
    if let (Some(operation), Some(declared_ic), Some(outcome)) =
        (doc.operation, doc.declared_ic, doc.outcome.clone())
    {
        doc.past.push(crate::isidis_op::PastOp {
            operation,
            declared_ic,
            outcome,
        });
        if doc.past.len() > MAX_PAST {
            doc.past.remove(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Doc 50 cut OP.7**: a list reads as words.
    #[test]
    fn op7_a_list_reads_as_words() {
        let s = |v: &[&str]| words(&v.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>());
        assert_eq!(s(&[]), "");
        assert_eq!(s(&["ore"]), "ore");
        assert_eq!(s(&["ore", "ice"]), "ore and ice");
        assert_eq!(s(&["ore", "ice", "fuel"]), "ore, ice and fuel");
        assert_eq!(
            s(&["ore, raw", "ice, raw", "fuel"]),
            "ore, raw; ice, raw and fuel"
        );
    }

    /// **Doc 50 cut OP.7**: a past operation is kept once, and the record keeps at most
    /// [`MAX_PAST`].
    #[test]
    fn op7_the_past_is_bounded() {
        let mut doc = IsidisOp {
            operation: Some(1),
            declared_ic: Some(10),
            outcome: Some("freeport".to_owned()),
            ..IsidisOp::default()
        };
        for _ in 0..(MAX_PAST + 3) {
            keep_past(&mut doc);
        }
        assert_eq!(doc.past.len(), MAX_PAST);
        let mut open = IsidisOp {
            operation: Some(2),
            declared_ic: Some(10),
            ..IsidisOp::default()
        };
        keep_past(&mut open);
        assert!(open.past.is_empty(), "an undecided operation is not past");
    }
}
