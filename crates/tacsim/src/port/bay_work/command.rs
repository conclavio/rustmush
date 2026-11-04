//! **`+bay`** (doc 31 UD.4c's player surface, as doc 36 cuts GC.8a and GC.8b build it): the
//! doll, the quote, the order, its cancel, berth and stow, fitting a part from the hold,
//! stripping a part and scrapping a hull.
//!
//! - `+bay <hull>` (or `/quote`): the doll (each section's condition and points, each part's
//!   condition and rounds), where the hull stands, the work there and its price with the goods
//!   it draws from the carrier's hold, the work that needs a workshop, and its open or last
//!   order.
//! - `+bay/order <hull>=<work>`: `all`, a kind (`armor`, `rebuild`, `mend`, `unjam`,
//!   `replace`, `rearm`) or a section or part, comma-separated.
//! - `+bay/fit <hull>=<slot> <lot>`: replace a part from a lot in the carrier's hold.
//! - `+bay/strip <hull>=<part>`, `+bay/scrap <hull>`.
//! - `+bay/cancel <hull>`, `+bay/berth <hull>`, `+bay/stow <hull>`.
//!
//! A hull is named by `#<id>` or by its name in the room the actor stands in.

use mush_core::{CmdError, Ctx, ObjId};
use mush_tactics::CellAddr;

use super::{BayHullState, BayJob, BayQuote, FacilityClass, OrderStatus, quote_here};
use crate::port::coop::{money_minor, say};
use crate::store::TacsimStore;

/// The verb.
pub const BAY_VERB: &str = "+bay";
/// Its switches.
pub const BAY_SWITCHES: [&str; 8] = [
    "quote", "order", "cancel", "berth", "stow", "fit", "strip", "scrap",
];
/// The grammar.
pub const BAY_USAGE: &str = "Usage: +bay[/quote] <hull>, +bay/order <hull>=<work>, \
                             +bay/fit <hull>=<slot> <lot>, +bay/strip <hull>=<part>, \
                             +bay/scrap <hull>, +bay/cancel <hull>, +bay/berth <hull>, \
                             +bay/stow <hull>[=<carrier>]";
/// The help text.
pub const BAY_HELP: &str = "+bay <hull> quotes the work on a hull where it stands: a carrier's \
    vehicle bay does armour, mending, unjamming and recharging at twice the time; a port \
    workshop does everything, rebuilt sections and new parts too. +bay/order <hull>=<work> pays \
    the fee into escrow and holds the hull until the work is done on the IC clock \
    (all, armor, rebuild, mend, unjam, replace, rearm, or a section or part). +bay/cancel \
    releases the fee. At a port's workshop, +bay/berth <hull> takes a hull out of your docked \
    carrier's bay and +bay/stow <hull> puts it back; +bay/stow <hull>=<carrier> puts it in a     free slot of another carrier of yours docked here, as a hull the yard built for you. Munitions come from the carrier's hold \
    when the work is done; armour and rebuilds draw from the hull's bale, which the first repair \
    opens with a tonne each of structural alloys and industrial parts from the hold, topped up \
    by the tonne when it runs short. Away from port a carrier's bay works from her own hold for \
    no fee, and waits while a field is open on her site. In a workshop, or in a carrier's bay \
    away from port, +bay/fit <hull>=<slot> <lot> fits a traced part from the hold into any slot \
    of its kind (a hot part is refused). In a workshop, +bay/strip \
    <hull>=<part> takes a part \
    off into the hold, and the title holder's +bay/scrap <hull> breaks the hull up into \
    structural alloys and parts.";

/// `#<id>`, or a thing of that name in the actor's room — never a scrapped shell by name (doc
/// 36a P1-7).
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
    if let Some(found) = ctx.w.contents_of(here).iter().copied().find(|id| {
        ctx.w
            .get(*id)
            .is_some_and(|o| !o.kind.is_actor() && o.name.eq_ignore_ascii_case(word))
            && !super::scrapped(ctx.w, *id)
    }) {
        return Ok(found);
    }
    if let Some(helm) = crate::port::coop::helm_of(ctx.w, ctx.actor)
        .map(|h| h.hull)
        .or_else(|| crate::space::commands::hull_aboard_for_actor(ctx.w, ctx.actor))
        && !super::scrapped(ctx.w, helm)
        && (word.eq_ignore_ascii_case("ship")
            || word.eq_ignore_ascii_case("dray")
            || word.eq_ignore_ascii_case(&crate::port::coop::name_of(ctx.w, helm)))
    {
        return Ok(helm);
    }
    Err(super::NO_HULL.to_owned())
}

/// **A carrier by `#<id>`, her name or her mark** (doc 39 cut IN.9): any hull the bay's carrier
/// scan names (programme, yard and commissioned rows); the stow checks she is docked here.
fn carrier_named(ctx: &Ctx<'_>, word: &str) -> Result<ObjId, String> {
    let word = word.trim();
    if let Some(id) = word.strip_prefix('#').and_then(|n| n.parse::<u32>().ok()) {
        return Ok(ObjId(id));
    }
    super::bay_carriers(ctx.w)
        .into_iter()
        .find(|hull| {
            ctx.w.get(*hull).is_some_and(|o| {
                o.name.eq_ignore_ascii_case(word)
                    || o.aliases
                        .iter()
                        .any(|alias| alias.eq_ignore_ascii_case(word))
            })
        })
        .ok_or_else(|| "There is no such carrier.".to_owned())
}

/// **The doll** (UD.4c's damage view): each section's condition and points, and under it each
/// part it holds, with its condition and, for a part with a magazine, its rounds.
fn doll(w: &mush_core::World, hull: ObjId) -> Vec<String> {
    let Ok((unit, _, anatomy)) = super::unit_parts(w, hull) else {
        return Vec::new();
    };
    let assessment = mush_tactics::field_ops::assess_tactical_unit(&unit, &anatomy);
    let state = format!("{:?}", assessment.state).to_lowercase();
    let mut lines = vec![format!("  Condition: {state}.")];
    for (section, condition) in &assessment.section_conditions {
        let (current, max) = unit
            .cells()
            .filter(|(addr, _)| match addr {
                CellAddr::Section { cell: cid, .. }
                    if cid.as_str() == crate::definitions::unit::HEAT_CELL
                        || cid.as_str() == crate::definitions::unit::HEAT_SINK_CELL =>
                {
                    false
                }
                CellAddr::Section { section: s, .. } | CellAddr::Facet { section: s, .. } => {
                    s == section
                }
                _ => false,
            })
            .fold((0i64, 0i64), |(c, m), (_, cell)| {
                (
                    c.saturating_add(cell.current.saturating_sub(cell.min)),
                    m.saturating_add(cell.max.saturating_sub(cell.min)),
                )
            });
        lines.push(format!(
            "    {:<28} {:<10} {current:>5}/{max}",
            section.as_str(),
            condition.as_str()
        ));
        for (component, state) in unit.components() {
            if mush_tactics::field_ops::install_point_section(&state.install) != Some(section) {
                continue;
            }
            let condition = assessment
                .component_conditions
                .get(component)
                .map_or("unknown", |c| c.as_str());
            let rounds = anatomy
                .weapon_ammo_cell
                .as_ref()
                .filter(|_| anatomy.weapon_components.contains(component))
                .and_then(|cell| {
                    unit.cell(&CellAddr::Component {
                        component: component.clone(),
                        cell: cell.clone(),
                    })
                })
                .map(|cell| format!(" {}/{} rounds", cell.current, cell.max))
                .unwrap_or_default();
            lines.push(format!(
                "      {:<26} {condition}{rounds}",
                component.as_str()
            ));
        }
    }
    lines
}

fn hours(secs: i64) -> String {
    let minutes = secs.div_euclid(60);
    format!("{}h{:02}", minutes.div_euclid(60), minutes.rem_euclid(60))
}

/// **What a taken order says**: its jobs, the fee held and the IC time — no fee away from port
/// (doc 41 cut RS.2, Q-41-7).
fn taken_line(ctx: &Ctx<'_>, order: &super::BayOrder) -> String {
    let words: Vec<String> = order.jobs.iter().map(|j| j.job.describe()).collect();
    let fee = if order.contract.is_some() {
        format!(
            "{} is held until the work is done",
            money_minor(ctx, order.fee_minor)
        )
    } else {
        "No fee away from port: her crew does the work".to_owned()
    };
    format!(
        "Order {} taken in the {}: {}. {fee}, in {} IC time.",
        order.seq,
        order.class.word(),
        words.join(", "),
        hours(order.due_ic.saturating_sub(order.started_ic))
    )
}

/// The quote and the record, as lines.
fn show(ctx: &mut Ctx<'_>, hull: ObjId) -> Result<(), String> {
    let name = crate::port::coop::name_of(ctx.w, hull);
    let mut lines = Vec::new();
    // Doc 41 Q-41-20 (Q-S14): a salvor reads the risk before anything else.
    if super::right_of(ctx, hull) == Some(super::Right::Salvor) {
        lines.push(crate::surface::claim::AT_RISK_QUOTE.to_owned());
    }
    match quote_here(ctx.w, hull) {
        Ok((facility, quote)) => {
            let factor = match facility.class {
                FacilityClass::Workshop => String::new(),
                // Doc 41 cut RS.2 (Q-41-7): away from port the crew's labour is unpaid.
                FacilityClass::CarrierBay if facility.away() => format!(
                    " away from port (at {}x the workshop's time, from her hold; no fee)",
                    super::CARRIER_TIME_FACTOR
                ),
                FacilityClass::CarrierBay => {
                    format!(" (at {}x the workshop's time)", super::CARRIER_TIME_FACTOR)
                }
            };
            lines.push(format!(
                "{name} (#{}) stands in a {}{factor}.",
                hull.0,
                facility.class.word()
            ));
            let price = |ctx: &Ctx<'_>, minor: u64| {
                if facility.away() {
                    "no fee".to_owned()
                } else {
                    money_minor(ctx, i64::try_from(minor).unwrap_or(i64::MAX))
                }
            };
            lines.extend(doll(ctx.w, hull));
            if quote.jobs.is_empty() && quote.elsewhere.is_empty() {
                lines.push("  Nothing needs doing.".to_owned());
            }
            for job in &quote.jobs {
                let secs = BayQuote::secs(std::slice::from_ref(job), facility.class);
                lines.push(format!(
                    "  {:<34} {:>12}  {}",
                    job.job.describe(),
                    price(ctx, job.cost_minor),
                    hours(secs)
                ));
                // Doc 39 cut IN.6 (Q-39-11): the part from the stores, at its catalogue price,
                // beside the labour — or `NO_PART` when the stores hold none.
                if let BayJob::Replace {
                    component,
                    lot: None,
                } = &job.job
                {
                    lines.push(match super::store_part(ctx, hull, &facility, component) {
                        Ok(_) => format!(
                            "    {:<32} {:>12}  from the workshop's stores",
                            "the part",
                            money_minor(ctx, i64::try_from(job.part_minor).unwrap_or(i64::MAX))
                        ),
                        Err(why) => format!("    {why}"),
                    });
                }
            }
            if !quote.jobs.is_empty() {
                lines.push(format!(
                    "  {:<34} {:>12}  {}",
                    "all",
                    price(ctx, BayQuote::fee_minor(&quote.jobs)),
                    hours(BayQuote::secs(&quote.jobs, facility.class))
                ));
                // Doc 39 cut IN.4 (Q-39-9): the workshop's berth time at its slot-hour, before
                // margin, which the operator's multiplier may not take the labour under.
                if let Some((cost, berths)) = facility.cost {
                    let secs = BayQuote::secs(&quote.jobs, FacilityClass::Workshop);
                    let floor = cost.floor_minor_for_secs(u64::try_from(secs).unwrap_or(0), berths);
                    lines.push(format!(
                        "  {:<34} {:>12}  the berth's cost, before margin",
                        "floor",
                        money_minor(ctx, i64::try_from(floor).unwrap_or(i64::MAX))
                    ));
                }
                let bale = super::record_of(ctx.w, hull).ok().and_then(|r| r.bale);
                let (goods, after) = super::order_goods(&quote.jobs, bale);
                if !goods.is_empty() {
                    let words: Vec<String> = goods
                        .iter()
                        .map(|g| format!("{} {}", g.units, g.good))
                        .collect();
                    lines.push(format!(
                        "  From the carrier's hold, all of it: {}.",
                        words.join(", ")
                    ));
                }
                // Doc 39 cut IN.1: what the hull's bale holds once the work is done.
                if let Some(after) = after {
                    lines.push(format!(
                        "  Her bale after it: {} kg of alloys and {} kg of fittings left.",
                        after.alloys_kg, after.fittings_kg
                    ));
                }
            }
            for job in &quote.elsewhere {
                // Doc 41 cut RS.2: away from port the line names the workshops, nearest first.
                lines.push(if facility.away() {
                    format!(
                        "  {}",
                        super::workshop_refusal(ctx.w, hull, &facility, &job.job)
                    )
                } else {
                    format!(
                        "  {:<34} needs a {}",
                        job.job.describe(),
                        job.job.needs().word()
                    )
                });
            }
        }
        Err(why) => {
            lines.push(format!("{name} (#{}): {why}", hull.0));
            lines.extend(doll(ctx.w, hull));
        }
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
        match &row.berth {
            // Doc 39 cut IN.9: the yard's berth names no carrier.
            Some(berth) if berth.carrier.is_none() => lines.push(
                "  Built on this floor; +bay/stow <hull>=<carrier> puts her in a carrier's bay."
                    .into(),
            ),
            Some(_) => {
                lines.push(
                    "  Berthed in this workshop; +bay/stow returns her to her carrier.".into(),
                );
            }
            None => {}
        }
        if row.scrapped_ic.is_some() {
            lines.push("  Scrapped: an irrecoverable shell.".into());
        }
        if row.owed.is_some() {
            lines.push("  The bay's stores are still settling her last work.".into());
        }
        if let Some(order) = &row.order {
            let state = match order.status {
                OrderStatus::Open if order.paused.is_some() => format!(
                    "open, waiting under a field with {} left",
                    hours(order.paused.unwrap_or(0).max(0))
                ),
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
            let fee = if order.contract.is_some() {
                money_minor(ctx, order.fee_minor)
            } else {
                "no fee".to_owned()
            };
            lines.push(format!(
                "  Order {}: {} for {fee}, {state}.",
                order.seq,
                order.class.word(),
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
    // Doc 42 cut CR.0 (Q-42-2): every bay act is the held gate's; the quote, a read, passes.
    if ctx.switches.iter().any(|switch| switch != "quote") {
        crate::field::crew::gate(ctx, crate::field::crew::Gate::Operate)?;
    }
    let raw = ctx.args.raw().trim().to_owned();
    if raw.is_empty() {
        return Err(CmdError::msg(BAY_USAGE));
    }
    let run = |ctx: &mut Ctx<'_>| -> Result<(), String> {
        if ctx.switches.has("order") {
            let (hull, work) = raw.split_once('=').ok_or(BAY_USAGE)?;
            let hull = hull_named(ctx, hull)?;
            let taken = super::take_order(ctx, hull, work.trim())?;
            let line = taken_line(ctx, &taken.order);
            say(ctx, line);
            return Ok(());
        }
        if ctx.switches.has("fit") || ctx.switches.has("strip") {
            let (hull, rest) = raw.split_once('=').ok_or(BAY_USAGE)?;
            let hull = hull_named(ctx, hull)?;
            if ctx.switches.has("strip") {
                let lot = super::strip::strip(ctx, hull, rest)?;
                say(
                    ctx,
                    format!(
                        "{} comes off #{} and goes to the carrier's hold as {lot}.",
                        rest.trim(),
                        hull.0
                    ),
                );
                return Ok(());
            }
            let (slot, lot) = rest.trim().split_once(' ').ok_or(BAY_USAGE)?;
            let lot = mush_inventory::LotId::parse(lot).ok_or("That is not a lot.")?;
            let order = super::fit_order(ctx, hull, slot, lot)?.order;
            let line = taken_line(ctx, &order);
            say(ctx, line);
            return Ok(());
        }
        // Doc 39 cut IN.9 (§4.13): `+bay/stow <hull>=<carrier>`.
        if ctx.switches.has("stow")
            && let Some((hull, carrier)) = raw.split_once('=')
        {
            let hull = hull_named(ctx, hull)?;
            let carrier = carrier_named(ctx, carrier)?;
            super::stow(ctx, hull, Some(carrier))?;
            let (name, into) = (
                crate::port::coop::name_of(ctx.w, hull),
                crate::port::coop::name_of(ctx.w, carrier),
            );
            say(ctx, format!("{name} is stowed in the {into}'s bay."));
            return Ok(());
        }
        let hull = hull_named(ctx, &raw)?;
        if ctx.switches.has("scrap") {
            let scrapped = super::strip::scrap(ctx, hull)?;
            say(
                ctx,
                format!(
                    "#{} is broken up: {} t of structural alloys and {} part(s) go to the \
                     carrier's hold.",
                    hull.0,
                    scrapped.alloys_t,
                    scrapped.parts.len()
                ),
            );
            return Ok(());
        }
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
            super::stow(ctx, hull, None)?;
            let name = crate::port::coop::name_of(ctx.w, hull);
            say(ctx, format!("{name} is stowed back in her carrier's bay."));
            return Ok(());
        }
        show(ctx, hull)
    };
    run(ctx).map_err(CmdError::msg)
}
