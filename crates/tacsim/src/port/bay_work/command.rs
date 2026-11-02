//! **`+bay`** (doc 31 UD.4c's player surface, as doc 36 cut GC.8a builds it): the quote, the
//! order, its cancel, berth and stow. The doll is GC.8b's.
//!
//! - `+bay <hull>` (or `/quote`): where the hull stands, the work there and its price, the work
//!   that needs a workshop, and its open or last order.
//! - `+bay/order <hull>=<work>`: `all`, a kind (`armor`, `rebuild`, `mend`, `unjam`,
//!   `replace`, `rearm`) or a section or part, comma-separated.
//! - `+bay/cancel <hull>`, `+bay/berth <hull>`, `+bay/stow <hull>`.
//!
//! A hull is named by `#<id>` or by its name in the room the actor stands in.

use mush_core::{CmdError, Ctx, ObjId};

use super::{BayHullState, BayQuote, FacilityClass, OrderStatus, quote_here};
use crate::port::coop::{money_minor, say};
use crate::store::TacsimStore;

/// The verb.
pub const BAY_VERB: &str = "+bay";
/// Its switches.
pub const BAY_SWITCHES: [&str; 5] = ["quote", "order", "cancel", "berth", "stow"];
/// The grammar.
pub const BAY_USAGE: &str = "Usage: +bay[/quote] <hull>, +bay/order <hull>=<work>, \
                             +bay/cancel <hull>, +bay/berth <hull>, +bay/stow <hull>";
/// The help text.
pub const BAY_HELP: &str = "+bay <hull> quotes the work on a hull where it stands: a carrier's \
    vehicle bay does armour, mending, unjamming and recharging at twice the time; a port \
    workshop does everything, rebuilt sections and new parts too. +bay/order <hull>=<work> pays \
    the fee into escrow and holds the hull until the work is done on the IC clock \
    (all, armor, rebuild, mend, unjam, replace, rearm, or a section or part). +bay/cancel \
    releases the fee. At a port's workshop, +bay/berth <hull> takes a hull out of your docked \
    carrier's bay and +bay/stow <hull> puts it back.";

/// `#<id>`, or a thing of that name in the actor's room.
fn hull_named(ctx: &Ctx<'_>, word: &str) -> Result<ObjId, String> {
    let word = word.trim();
    if let Some(id) = word.strip_prefix('#').and_then(|n| n.parse::<u32>().ok()) {
        return Ok(ObjId(id));
    }
    let here = ctx
        .w
        .get(ctx.actor)
        .and_then(|o| o.location)
        .ok_or(super::NO_HULL)?;
    ctx.w
        .contents_of(here)
        .iter()
        .copied()
        .find(|id| {
            ctx.w
                .get(*id)
                .is_some_and(|o| !o.kind.is_actor() && o.name.eq_ignore_ascii_case(word))
        })
        .ok_or_else(|| super::NO_HULL.to_owned())
}

fn hours(secs: i64) -> String {
    let minutes = secs.div_euclid(60);
    format!("{}h{:02}", minutes.div_euclid(60), minutes.rem_euclid(60))
}

/// The quote and the record, as lines.
fn show(ctx: &mut Ctx<'_>, hull: ObjId) -> Result<(), String> {
    let name = crate::port::coop::name_of(ctx.w, hull);
    let mut lines = Vec::new();
    match quote_here(ctx.w, hull) {
        Ok((facility, quote)) => {
            let factor = match facility.class {
                FacilityClass::Workshop => String::new(),
                FacilityClass::CarrierBay => {
                    format!(" (at {}x the workshop's time)", super::CARRIER_TIME_FACTOR)
                }
            };
            lines.push(format!(
                "{name} (#{}) stands in a {}{factor}.",
                hull.0,
                facility.class.word()
            ));
            if quote.jobs.is_empty() && quote.elsewhere.is_empty() {
                lines.push("  Nothing needs doing.".to_owned());
            }
            for job in &quote.jobs {
                let secs = BayQuote::secs(std::slice::from_ref(job), facility.class);
                lines.push(format!(
                    "  {:<34} {:>12}  {}",
                    job.job.describe(),
                    money_minor(ctx, i64::try_from(job.cost_minor).unwrap_or(i64::MAX)),
                    hours(secs)
                ));
            }
            if !quote.jobs.is_empty() {
                lines.push(format!(
                    "  {:<34} {:>12}  {}",
                    "all",
                    money_minor(
                        ctx,
                        i64::try_from(BayQuote::cost_minor(&quote.jobs)).unwrap_or(i64::MAX)
                    ),
                    hours(BayQuote::secs(&quote.jobs, facility.class))
                ));
            }
            for job in &quote.elsewhere {
                lines.push(format!(
                    "  {:<34} needs a {}",
                    job.job.describe(),
                    job.job.needs().word()
                ));
            }
        }
        Err(why) => lines.push(format!("{name} (#{}): {why}", hull.0)),
    }
    lines.push(match super::right_of(ctx, hull) {
        Some(super::Right::Title) => "  Your right: title (everything).".to_owned(),
        Some(super::Right::Grant) => {
            "  Your right: a maintain/refit grant (repair, rearm, refit).".to_owned()
        }
        Some(super::Right::Lessee) => "  Your right: lessee (repair and rearm).".to_owned(),
        Some(super::Right::Salvor) => {
            "  Your right: salvage custody (repair at your own risk).".to_owned()
        }
        None => "  You hold no right to have this hull worked on.".to_owned(),
    });
    if let Some(BayHullState::Present(row)) = ctx
        .w
        .module_state::<TacsimStore>()
        .and_then(|s| s.bay_hull(hull))
    {
        if row.berth.is_some() {
            lines.push("  Berthed in this workshop; +bay/stow returns her to her carrier.".into());
        }
        if let Some(order) = &row.order {
            let state = match order.status {
                OrderStatus::Open => format!(
                    "open, done in {}",
                    hours(order.due_ic.saturating_sub(ctx.w.world_ic_now().at).max(0))
                ),
                OrderStatus::Done => "done".to_owned(),
                OrderStatus::Cancelled => format!(
                    "cancelled ({})",
                    order.note.as_deref().unwrap_or("no reason recorded")
                ),
            };
            lines.push(format!(
                "  Order {}: {} for {}, {state}.",
                order.seq,
                order.class.word(),
                money_minor(ctx, order.fee_minor)
            ));
        }
    }
    say(ctx, lines.join("\n"));
    Ok(())
}

/// **`+bay`**.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line said to the actor.
pub(crate) fn bay_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    crate::commands::require_enabled(ctx)?;
    let raw = ctx.args.raw().trim().to_owned();
    if raw.is_empty() {
        return Err(CmdError::msg(BAY_USAGE));
    }
    let run = |ctx: &mut Ctx<'_>| -> Result<(), String> {
        if ctx.switches.has("order") {
            let (hull, work) = raw.split_once('=').ok_or(BAY_USAGE)?;
            let hull = hull_named(ctx, hull)?;
            let taken = super::take_order(ctx, hull, work.trim())?;
            let order = taken.order;
            let fee = money_minor(ctx, order.fee_minor);
            let words: Vec<String> = order.jobs.iter().map(|j| j.job.describe()).collect();
            say(
                ctx,
                format!(
                    "Order {} taken in the {}: {}. {fee} is held until the work is done, in {} \
                     IC time.",
                    order.seq,
                    order.class.word(),
                    words.join(", "),
                    hours(order.due_ic.saturating_sub(order.started_ic))
                ),
            );
            return Ok(());
        }
        let hull = hull_named(ctx, &raw)?;
        if ctx.switches.has("cancel") {
            super::cancel_order(ctx, hull)?;
            return Ok(());
        }
        if ctx.switches.has("berth") {
            super::berth(ctx, hull)?;
            let name = crate::port::coop::name_of(ctx.w, hull);
            say(
                ctx,
                format!("{name} is towed out of her carrier's bay and berthed in the workshop."),
            );
            return Ok(());
        }
        if ctx.switches.has("stow") {
            super::stow(ctx, hull)?;
            let name = crate::port::coop::name_of(ctx.w, hull);
            say(ctx, format!("{name} is stowed back in her carrier's bay."));
            return Ok(());
        }
        show(ctx, hull)
    };
    run(ctx).map_err(CmdError::msg)
}
