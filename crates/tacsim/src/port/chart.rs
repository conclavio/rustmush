//! **`+chart` — the sky, read from where you stand** (doc 19 §5, cut A.3; D-19-16).
//!
//! Three reads, nothing written, on the one sky seam and the one planner:
//!
//! - **`+chart`**: the lane's bodies at this instant — heliocentric distance,
//!   ecliptic longitude and latitude, the distance from the port you stand at
//!   (or your hull is docked at) and the light lag — then, for each port this
//!   one clears for, the passage **now** at your hull's load and empty, and the
//!   direct chord's passage beside the listed route (the depot's detour, in
//!   hours and tonnes).
//! - **`+chart <port>`**: the **window** — the planner run once per IC day over
//!   the next IC year ([`WINDOW_DAYS`] evaluations, pure): the fastest day, the
//!   cheapest day and the worst, each against now, so a captain reads when a
//!   rotation is worth the fuel and when it is not (the owner's ask).
//! - **`+chart/body <id>`**: one body's row and the osculating elements it moves
//!   on.
//!
//! Without a charter the Dray of the port's programme is planned empty, so a
//! walker on the concourse reads the same sky a captain does. Figures are the
//! planner's and the sky's; none is authored here.

use mush_core::{CanonicalId, CmdError, Ctx, World};

use crate::ephemeris::AU_KM;
use crate::kepler::ElementRow;
use crate::pack::{PackEntry, RulesPack};
use crate::port::burn::{HullSpec, hull_spec_of};
use crate::port::coop::{helm_of, port_name};
use crate::port::def::PortDef;
use crate::port::passage::{
    PassagePlan, PlanError, choose_route, leg_distance_km, leg_seconds, passage_burn,
};
use crate::port::price::{
    emergency_readings, premium_permille, priced, reach_for, reach_ratio_permille,
};
use crate::port::readiness::report_ports;
use crate::port::ship::{Berth, berth_of, hull_view};
use crate::port::voyage::{
    Basis, Destination, PassageOptions, basis, destinations, match_destination, occupants,
    real_span,
};
use crate::sky::{Sky, epoch_date};
use crate::store::TacsimStore;

/// The verb.
pub const CHART_VERB: &str = "+chart";
/// Its help.
pub const CHART_HELP: &str = "+chart                  the sky now, from the port you stand at: every lane body's                    distance, longitude and latitude, how far it is from here and the light lag;                    then each port this one clears for, planned now at your hull's load and empty,                    and what the direct chord would cost beside the listed route.
                   +chart <port>           the window: the passage to that port planned on every                    IC day of the next year — the fastest day, the cheapest and the worst, against                    now. A season that is not worth the fuel is a season to wait out.
                   +chart/body <body>      one body's row and the elements it moves on.
                   Without a charter the port's Dray is planned empty. Nothing is written.
+chart/map              the sky as a flat map: the Sun, every orbit, the bodies by letter,                    the listed routes dotted, you at @ — the TUI's chart pane for a line player.";

/// How many IC days `+chart <port>` scans (one IC year).
pub const WINDOW_DAYS: i64 = 365;
/// The days `+ship/plan`'s window line looks ahead.
pub const PLAN_WINDOW_DAYS: i64 = 30;
/// Seconds in an IC day.
const DAY: i64 = 86_400;

/// Not at a port, aboard nothing: the chart has no vantage.
pub const CHART_NOWHERE: &str =
    "+chart reads the sky from a port: stand in one, or aboard a hull docked at one.";
/// No lane is loaded.
pub const CHART_NO_LANE: &str = "No lane is loaded on this server, so there is no sky to chart.";
/// `+chart <port>` named no destination: `{here}`, `{list}`.
pub const CHART_NO_ROUTE: &str =
    "No passage leaves {here} for that port; {here} clears for {list}.";
/// `+chart/body` named no body: `{list}`.
pub const CHART_NO_BODY: &str = "No lane body is called that; the sky holds {list}.";

/// Where the chart is read from and what it plans with.
pub(crate) struct Vantage<'w> {
    /// The port stood at or docked at.
    pub(crate) def: &'w PortDef,
    /// Its body.
    pub(crate) body: CanonicalId,
    /// The hull planned: the actor's charter, or the port's Dray.
    pub(crate) spec: HullSpec,
    /// Its name for the lines.
    pub(crate) hull: String,
    /// The payload planned, kilograms.
    pub(crate) payload_kg: u64,
    /// Souls aboard.
    pub(crate) occupants: u32,
    /// The lane pack.
    pub(crate) pack: &'w RulesPack,
    /// The sky.
    pub(crate) sky: Sky,
}

/// The body a lane place stands on.
pub(crate) fn body_of(pack: &RulesPack, place: &CanonicalId) -> Option<CanonicalId> {
    match pack.entry(place) {
        Some(PackEntry::Location(def)) => def.body.clone(),
        _ => None,
    }
}

/// Kilometres as AU to three decimals, half up in integer arithmetic.
pub(crate) fn au3(km: u64) -> String {
    let milli = (u128::from(km) * 1_000 + u128::from(AU_KM) / 2) / u128::from(AU_KM);
    format!("{}.{:03}", milli / 1_000, milli % 1_000)
}

/// Seconds as hours at the world rate (`h.h h`), or IC days when the clock is
/// stopped.
pub(crate) fn hours_text(w: &World, secs: i64) -> String {
    let rate = i64::from(w.world_timeline().current().rate);
    if rate == 0 {
        return format!("{} IC days", crate::port::voyage::ic_days(secs));
    }
    real_span(secs.saturating_add(rate - 1) / rate)
}

/// Light seconds as minutes and seconds.
pub(crate) fn lag_text(secs: u64) -> String {
    format!("{}m {:02}s light", secs / 60, secs % 60)
}

/// **The vantage** (see the module doc): the actor's docked charter and its
/// load, else the port whose room the actor stands in (the first lane port when
/// the actor is nowhere the lane knows is refused), planned with the programme's
/// Dray empty.
fn vantage<'w>(ctx: &'w Ctx<'_>) -> Result<Vantage<'w>, CmdError> {
    let w: &World = ctx.w;
    let store = w
        .module_state::<TacsimStore>()
        .ok_or_else(|| CmdError::msg(CHART_NO_LANE))?;
    let (pack, _) = report_ports(store, w);
    let pack = pack.ok_or_else(|| CmdError::msg(CHART_NO_LANE))?;
    let sky = Sky::from_pack(pack, &store.config().time.ephemeris);
    // A docked charter: its port, its load.
    if let Some(charter) = helm_of(w, ctx.actor)
        && let Some(view) = hull_view(w, charter.hull)
        && let Berth::Docked { def, .. } = berth_of(w, charter.hull)
    {
        let souls = view
            .airlock()
            .map_or(0, |airlock| occupants(w, airlock))
            .max(1);
        let basis = basis(ctx, &view, souls).map_err(CmdError::msg)?;
        let body = body_of(pack, &def.location).ok_or_else(|| CmdError::msg(CHART_NOWHERE))?;
        return Ok(Vantage {
            def,
            body,
            spec: basis.spec.clone(),
            hull: crate::port::coop::name_of(w, charter.hull),
            payload_kg: basis.payload,
            occupants: souls,
            pack,
            sky,
        });
    }
    vantage_for_world(w, ctx.actor).map_err(CmdError::msg)
}

/// **The vantage without a helm** (doc 19 cut A.3b, for the `sky.chart` panel,
/// which answers from a `World` and an actor): the port whose domain host or
/// gallery the actor stands in (the port map's rooms), else the lane's first
/// port, planned with the port's programme hull — else the game's Dray — empty.
/// `+chart` from a docked charter's helm plans the charter's load instead
/// ([`vantage`]); the panel never reads a hold.
///
/// # Errors
///
/// No lane loaded, or nowhere the lane knows.
pub(crate) fn vantage_for_world(w: &World, actor: mush_core::ObjId) -> Result<Vantage<'_>, String> {
    let store = w
        .module_state::<TacsimStore>()
        .ok_or_else(|| CHART_NO_LANE.to_owned())?;
    let (pack, ports) = report_ports(store, w);
    let pack = pack.ok_or_else(|| CHART_NO_LANE.to_owned())?;
    let goods = store.goods().ok_or_else(|| CHART_NO_LANE.to_owned())?;
    let sky = Sky::from_pack(pack, &store.config().time.ephemeris);
    let here = w.get(actor).and_then(|o| o.location).map(|room| room.0);
    let standing = crate::port::voyage::lane_ports(w)
        .into_iter()
        .find(|(_, map)| {
            map.is_some_and(|map| {
                here.is_some_and(|room| map.host == Some(room) || map.presentation == Some(room))
            })
        })
        .map(|(def, _)| def);
    let def = match standing {
        Some(def) => def,
        None => ports
            .first()
            .map(|(_, def)| *def)
            .ok_or_else(|| CHART_NOWHERE.to_owned())?,
    };
    let body = body_of(pack, &def.location).ok_or_else(|| CHART_NOWHERE.to_owned())?;
    // The port's programme hull, else the game's Dray (a port without a programme).
    let unit = def
        .programme
        .as_ref()
        .map(|programme| programme.unit.clone())
        .or_else(|| CanonicalId::new(crate::definitions::content::DRAY).ok())
        .ok_or_else(|| CHART_NOWHERE.to_owned())?;
    let (_, spec) =
        hull_spec_of(store.definitions(), goods, &unit).map_err(|_| CHART_NOWHERE.to_owned())?;
    let payload_kg = u64::from(spec.allowance_kg);
    Ok(Vantage {
        def,
        body,
        hull: format!("an empty {unit}"),
        spec,
        payload_kg,
        occupants: 1,
        pack,
        sky,
    })
}

/// Plan the listed routes to `dest` at `depart_ic` for the vantage's hull at
/// `payload_kg`.
pub(crate) fn plan_at(
    v: &Vantage<'_>,
    dest: &Destination,
    payload_kg: u64,
    depart_ic: i64,
) -> Result<PassagePlan, PlanError> {
    let options = PassageOptions::default();
    choose_route(
        &v.sky,
        v.pack,
        &dest.routes,
        &v.spec.drive,
        v.spec.dry_kg,
        payload_kg,
        v.occupants,
        depart_ic,
        options.policy,
        None,
    )
}

/// The direct chord's passage from the vantage's body to `to` at `depart_ic`:
/// one leg, the law's seconds and burn.
pub(crate) fn direct_at(
    v: &Vantage<'_>,
    to: &CanonicalId,
    payload_kg: u64,
    depart_ic: i64,
) -> Option<(u64, u32, u64)> {
    let dist = leg_distance_km(&v.sky, &v.body, depart_ic, to, depart_ic)?;
    let secs = leg_seconds(&v.spec.drive, v.spec.dry_kg, payload_kg, dist).ok()?;
    let leg = crate::port::families::LegPlan {
        to: to.clone(),
        body: to.clone(),
        dist_km: dist,
        secs,
        t0: depart_ic,
        t1: depart_ic.checked_add(i64::from(secs))?,
    };
    let burn = passage_burn(&v.spec.drive, v.spec.dry_kg, payload_kg, &[leg]).ok()?;
    Some((dist, secs, burn))
}

/// `+chart`: the sky table and the passages now.
fn chart_sky(ctx: &mut Ctx<'_>) -> Result<Vec<String>, CmdError> {
    let v = vantage(ctx)?;
    let now = ctx.w.world_ic_now().at;
    Ok(sky_lines(ctx.w, &v, now))
}

/// **The sky table and the passages now** (`+chart`'s lines; doc 19 cut A.3b
/// factored them out for the `sky.chart` panel, which carries them too).
pub(crate) fn sky_lines(w: &World, v: &Vantage<'_>, now: i64) -> Vec<String> {
    let motion = if v.sky.ephemeris().rate_permille() == 0 {
        "constant".to_owned()
    } else {
        format!("moving, rate {}", v.sky.ephemeris().rate_permille())
    };
    let mut lines = vec![format!(
        "The sky on {} ({}; {}), from {} on {}:",
        epoch_date(now),
        v.sky.model(),
        motion,
        port_name(v.def),
        v.body
    )];
    let mut bodies: Vec<(CanonicalId, f64, f64, f64)> = v
        .sky
        .ephemeris()
        .bodies()
        .filter_map(|(id, _)| {
            let (r, lon, lat) = v.sky.heliocentric(id, now)?;
            Some((id.clone(), r, lon, lat))
        })
        .collect();
    bodies.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    for (id, r, lon, lat) in &bodies {
        let from_here = if id == &v.body {
            "here".to_owned()
        } else {
            match (
                v.sky.distance_km(&v.body, now, id, now),
                v.sky.light_lag_s(&v.body, now, id, now),
            ) {
                (Some(km), Some(lag)) => format!("{} AU from here, {}", au3(km), lag_text(lag)),
                _ => "unplaced".to_owned(),
            }
        };
        // Doc 19 cut A.7: a moon says whose it is.
        let moon = v
            .sky
            .parent_of(id)
            .map_or_else(String::new, |parent| format!("  moon of {parent}"));
        #[allow(clippy::cast_precision_loss)]
        lines.push(format!(
            "  {id:<9} r {:>6.3} AU  lon {:>6.1}  lat {:>+5.1}  {from_here}{moon}",
            r / AU_KM as f64,
            lon,
            lat
        ));
    }
    // The passages now, listed route and direct chord, this load and empty.
    let dests = destinations(w, v.def);
    let empty = u64::from(v.spec.allowance_kg) * u64::from(v.occupants.max(1));
    for dest in &dests {
        let Some(to) = v
            .pack
            .entries()
            .iter()
            .find_map(|(_, entry)| match entry {
                PackEntry::Port(port) if port.venue == dest.venue => {
                    body_of(v.pack, &port.location)
                }
                _ => None,
            })
            .or_else(|| {
                body_of(
                    v.pack,
                    &CanonicalId::new(format!("{}_yards", dest.venue)).ok()?,
                )
            })
        else {
            continue;
        };
        let loaded = plan_at(v, dest, v.payload_kg, now);
        let bare = plan_at(v, dest, empty, now);
        let direct = direct_at(v, &to, v.payload_kg, now);
        let mut line = format!("  {}: ", dest.port);
        match loaded {
            Ok(plan) => line.push_str(&format!(
                "{} at this load ({} t aboard the {}: burn {} t, {} crew-days)",
                hours_text(w, plan.total_s),
                v.payload_kg / 1_000,
                v.hull,
                plan.burn_t,
                plan.crew_days
            )),
            Err(error) => line.push_str(&format!("not plannable at this load ({error})")),
        }
        if let Ok(plan) = bare {
            line.push_str(&format!(
                "; empty {} and {} t",
                hours_text(w, plan.total_s),
                plan.burn_t
            ));
        }
        if let Some((km, secs, burn)) = direct {
            line.push_str(&format!(
                "; the direct chord {} AU would be {} and {} t",
                au3(km),
                hours_text(w, i64::from(secs)),
                burn
            ));
        }
        line.push('.');
        lines.push(line);
    }
    lines
}

/// One day of the window scan.
struct Day {
    offset: i64,
    total_s: i64,
    burn_t: u64,
}

/// `+chart <port>`: the window over the next IC year.
fn chart_window(ctx: &mut Ctx<'_>, word: &str) -> Result<Vec<String>, CmdError> {
    let v = vantage(ctx)?;
    let w: &World = ctx.w;
    let now = w.world_ic_now().at;
    let dests = destinations(w, v.def);
    let dest = match_destination(&dests, word).ok_or_else(|| {
        let list: Vec<String> = dests.iter().map(|d| d.port.clone()).collect();
        CmdError::msg(
            CHART_NO_ROUTE
                .replace("{here}", &port_name(v.def))
                .replace("{list}", &list.join(", ")),
        )
    })?;
    window_lines_for(w, &v, dest, now).map_err(CmdError::msg)
}

/// **One destination's window** (`+chart <port>`'s lines; doc 19 cut A.3b
/// factored them out for the `sky.chart` panel): now, then the fastest, the
/// cheapest and the worst day of the next [`WINDOW_DAYS`] against now, or
/// "every day is today" under a constant sky.
///
/// # Errors
///
/// The plan's refusal, in words.
pub(crate) fn window_lines_for(
    w: &World,
    v: &Vantage<'_>,
    dest: &Destination,
    now: i64,
) -> Result<Vec<String>, String> {
    let today = plan_at(v, dest, v.payload_kg, now).map_err(|e| e.to_string())?;
    let mut lines = vec![format!(
        "The window for {} from {}, the {} at {} t, scanned a day at a time over the next {} IC days:",
        dest.port,
        port_name(v.def),
        v.hull,
        v.payload_kg / 1_000,
        WINDOW_DAYS
    )];
    lines.push(format!(
        "  now ({}): {}, burn {} t, {} crew-days.",
        epoch_date(now),
        hours_text(w, today.total_s),
        today.burn_t,
        today.crew_days
    ));
    lines.extend(demand_lines(w, &dest.venue));
    if v.sky.ephemeris().rate_permille() == 0 {
        lines.push("  The sky is constant on this server: every day is today.".to_owned());
        return Ok(lines);
    }
    let mut days: Vec<Day> = Vec::with_capacity(usize::try_from(WINDOW_DAYS).unwrap_or(0) + 1);
    for offset in 0..=WINDOW_DAYS {
        let depart = now.saturating_add(offset.saturating_mul(DAY));
        if let Ok(plan) = plan_at(v, dest, v.payload_kg, depart) {
            days.push(Day {
                offset,
                total_s: plan.total_s,
                burn_t: plan.burn_t,
            });
        }
    }
    let Some(fastest) = days.iter().min_by_key(|d| (d.total_s, d.offset)) else {
        return Ok(lines);
    };
    let cheapest = days
        .iter()
        .min_by_key(|d| (d.burn_t, d.total_s, d.offset))
        .unwrap_or(fastest);
    let worst = days
        .iter()
        .max_by_key(|d| (d.total_s, d.burn_t))
        .unwrap_or(fastest);
    let describe = |label: &str, day: &Day| -> String {
        let date = epoch_date(now.saturating_add(day.offset.saturating_mul(DAY)));
        let dh = day.total_s - today.total_s;
        let sign = if dh < 0 { "-" } else { "+" };
        #[allow(clippy::cast_possible_wrap)]
        let dt = day.burn_t as i64 - today.burn_t as i64;
        let tsign = if dt < 0 { "-" } else { "+" };
        format!(
            "  {label}: {date} (in {} IC days): {}, burn {} t ({sign}{} and {tsign}{} t against now).",
            day.offset,
            hours_text(w, day.total_s),
            day.burn_t,
            hours_text(w, dh.abs()),
            dt.abs()
        )
    };
    lines.push(describe("fastest", fastest));
    if cheapest.offset != fastest.offset {
        lines.push(describe("cheapest", cheapest));
    }
    lines.push(describe("worst", worst));
    Ok(lines)
}

/// **The destination's demand side under its season** (doc 19 §6, cut A.4b): for
/// the port at `venue`, each standing bid's list price and what it posts at — the
/// premium against the median reach — then the emergency line: which life-support
/// goods are due at the cap, or that none are and where the reach stands.
fn demand_lines(w: &World, venue: &CanonicalId) -> Vec<String> {
    let Some(store) = w.module_state::<TacsimStore>() else {
        return Vec::new();
    };
    let (_, ports) = report_ports(store, w);
    let Some((_, def)) = ports.iter().find(|(_, def)| &def.venue == venue) else {
        return Vec::new();
    };
    let name = port_name(def);
    let mut lines = Vec::new();
    let Some(premium) = def.premium else {
        lines.push(format!(
            "  {name} posts its list prices whatever the season."
        ));
        return lines;
    };
    let org_name = |org: &CanonicalId| -> String {
        def.orgs
            .iter()
            .find(|o| &o.id == org)
            .map_or_else(|| org.to_string(), |o| o.name.clone())
    };
    for vendor in &def.vendors {
        for bid in &vendor.bids {
            let line = match reach_for(w, def, &bid.good) {
                Some(reach) => {
                    let p = premium_permille(premium.k_permille, premium.cap_permille, reach);
                    format!(
                        "  {} bids {}: list {}, posts {} (+{p} permille; reach {} now against a \
                         median of {}).",
                        org_name(&vendor.org),
                        bid.good,
                        bid.price_major,
                        priced(bid.price_major, p),
                        hours_text(w, reach.now_s),
                        hours_text(w, reach.median_s)
                    )
                }
                None => format!(
                    "  {} bids {}: list {}, posts {} (no port supplies it).",
                    org_name(&vendor.org),
                    bid.good,
                    bid.price_major,
                    bid.price_major
                ),
            };
            lines.push(line);
        }
    }
    if let Some(emergency) = &def.emergency {
        let readings = emergency_readings(w, def);
        let ratio = readings
            .iter()
            .filter_map(|r| r.reach)
            .filter_map(reach_ratio_permille)
            .max();
        let where_it_stands = ratio.map_or_else(
            || "no port supplies them".to_owned(),
            |ratio| format!("reach is {ratio} permille of the median"),
        );
        let due: Vec<String> = readings
            .iter()
            .filter(|r| r.crossed)
            .map(|r| match r.price_major {
                Some(price) => format!("{} at {price}", r.good),
                None => r.good.to_string(),
            })
            .collect();
        if due.is_empty() {
            lines.push(format!(
                "  emergency bids (at {} permille of the median): none due; {where_it_stands}.",
                emergency.ratio_permille
            ));
        } else {
            lines.push(format!(
                "  emergency bids (at {} permille of the median; {where_it_stands}): {} — {} each, \
                 at the cap (+{} permille), posted by {}.",
                emergency.ratio_permille,
                due.join(", "),
                emergency.intake,
                premium.cap_permille,
                org_name(&emergency.org)
            ));
        }
    }
    lines
}

/// `+chart/body <id>`: one body and its elements.
fn chart_body(ctx: &mut Ctx<'_>, word: &str) -> Result<Vec<String>, CmdError> {
    let v = vantage(ctx)?;
    let w: &World = ctx.w;
    let now = w.world_ic_now().at;
    let wanted = word.trim().to_lowercase();
    let id = v
        .sky
        .ephemeris()
        .bodies()
        .map(|(id, _)| id.clone())
        .find(|id| id.as_str().eq_ignore_ascii_case(&wanted))
        .ok_or_else(|| {
            let list: Vec<String> = v
                .sky
                .ephemeris()
                .bodies()
                .map(|(id, _)| id.to_string())
                .collect();
            CmdError::msg(CHART_NO_BODY.replace("{list}", &list.join(", ")))
        })?;
    let (r, lon, lat) = v
        .sky
        .heliocentric(&id, now)
        .ok_or_else(|| CmdError::msg(CHART_NO_BODY.replace("{list}", "nothing")))?;
    #[allow(clippy::cast_precision_loss)]
    let mut lines = vec![format!(
        "{id} on {}: r {:.4} AU, ecliptic longitude {:.2}, latitude {:+.2}.",
        epoch_date(now),
        r / AU_KM as f64,
        lon,
        lat
    )];
    if id != v.body
        && let (Some(km), Some(lag)) = (
            v.sky.distance_km(&v.body, now, &id, now),
            v.sky.light_lag_s(&v.body, now, &id, now),
        )
    {
        lines.push(format!(
            "  From {} on {}: {} AU, {}.",
            port_name(v.def),
            v.body,
            au3(km),
            lag_text(lag)
        ));
    }
    match &v.sky {
        Sky::Kepler(kepler) => match kepler.table(&id) {
            Some(rows) => {
                let row: &ElementRow = rows
                    .iter()
                    .rev()
                    .find(|row| row.epoch_ic <= now)
                    .or_else(|| rows.first())
                    .ok_or_else(|| CmdError::msg(CHART_NO_BODY.replace("{list}", "nothing")))?;
                let [_, a, e, i, node, peri, ma, n] = row.authored();
                lines.push(format!(
                    "  Elements at {} ({} rows, {}..{}): a {a:.4} AU, e {e:.4}, i {i:.2}, node {node:.2}, perihelion {peri:.2}, mean anomaly {ma:.2}, period {:.1} days.",
                    epoch_date(row.epoch_ic),
                    rows.len(),
                    rows.first().map_or_else(String::new, |r| epoch_date(r.epoch_ic)),
                    rows.last().map_or_else(String::new, |r| epoch_date(r.epoch_ic)),
                    360.0 / n
                ));
            }
            None => lines.push("  A circular body: an orbit, a period, a phase.".to_owned()),
        },
        Sky::Circular(_) => {
            lines.push("  A circular body: an orbit, a period, a phase.".to_owned());
        }
    }
    Ok(lines)
}

/// **The verb.**
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line.
pub(crate) fn chart_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let enabled = ctx
        .w
        .module_state::<TacsimStore>()
        .is_some_and(TacsimStore::enabled);
    if !enabled {
        return Err(CmdError::msg(crate::commands::NOT_ENABLED));
    }
    let raw = ctx.args.raw().trim().to_owned();
    let lines = if ctx.switches.has("body") {
        chart_body(ctx, &raw)?
    } else if ctx.switches.has("map") {
        // Doc 19 cut A.3b: the flat render, the TUI's chart pane for a line player.
        crate::port::chart_panel::flat_lines(ctx.w, ctx.actor).map_err(CmdError::msg)?
    } else if raw.is_empty() {
        chart_sky(ctx)?
    } else {
        chart_window(ctx, &raw)?
    };
    let actor = ctx.actor;
    for line in lines {
        ctx.out.sys_to_actor(actor, "tacsim", line);
    }
    Ok(())
}

/// **`+ship/plan`'s window line** (doc 19 §5): the best departure in the next
/// [`PLAN_WINDOW_DAYS`] IC days against this one, when the sky moves; `None`
/// under a constant sky (every day is today).
#[must_use]
pub(crate) fn plan_window_line(
    w: &World,
    basis: &Basis,
    routes: &[CanonicalId],
    today: &PassagePlan,
) -> Option<String> {
    let store = w.module_state::<TacsimStore>()?;
    if store.config().time.ephemeris.rate == 0 {
        return None;
    }
    let (offset, plan) = best_day(PLAN_WINDOW_DAYS, |offset| {
        let mut later = basis.clone();
        later.depart_ic = basis.depart_ic.saturating_add(offset.saturating_mul(DAY));
        crate::port::voyage::plan_on(w, &later, routes).ok()
    })?;
    let dh = today.total_s - plan.total_s;
    #[allow(clippy::cast_possible_wrap)]
    let dt = today.burn_t as i64 - plan.burn_t as i64;
    Some(if dh <= 0 && dt <= 0 {
        format!("Window: today is the best departure in the next {PLAN_WINDOW_DAYS} IC days.")
    } else {
        format!(
            "Window: the best departure in the next {PLAN_WINDOW_DAYS} IC days is {} (in {offset} IC days): {} and {} t, {} and {} t less than today.",
            epoch_date(plan.depart_ic),
            hours_text(w, plan.total_s),
            plan.burn_t,
            hours_text(w, dh.max(0)),
            dt.max(0)
        )
    })
}

/// **The fastest plan over days `1..=days`** (the window's core): `plan(offset)`
/// for each day; the least `total_s` wins, the earlier day on a tie; `None`
/// when no day plans.
pub(crate) fn best_day(
    days: i64,
    mut plan: impl FnMut(i64) -> Option<PassagePlan>,
) -> Option<(i64, PassagePlan)> {
    let mut best: Option<(i64, PassagePlan)> = None;
    for offset in 1..=days {
        if let Some(candidate) = plan(offset)
            && best
                .as_ref()
                .is_none_or(|(_, b)| candidate.total_s < b.total_s)
        {
            best = Some((offset, candidate));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::port::families::LegPlan;

    fn plan(total_s: i64, burn_t: u64) -> PassagePlan {
        PassagePlan {
            route: CanonicalId::new("r").expect("id"),
            law: CanonicalId::new("flip").expect("id"),
            depart_ic: 0,
            sky_rev: mush_core::DefinitionDigest::for_canonical_value("t", &serde_json::json!(1))
                .expect("digest"),
            legs: vec![LegPlan {
                to: CanonicalId::new("x").expect("id"),
                body: CanonicalId::new("x").expect("id"),
                dist_km: 1,
                secs: u32::try_from(total_s).expect("fits"),
                t0: 0,
                t1: total_s,
            }],
            total_s,
            burn_t,
            crew_days: 1,
            throttle_permille: 1_000,
        }
    }

    #[test]
    fn a3_the_best_day_is_the_least_total_earliest_on_a_tie() {
        let totals = [0_i64, 900, 800, 700, 700, 950];
        let (offset, best) = best_day(5, |k| {
            totals
                .get(usize::try_from(k).expect("a day"))
                .map(|t| plan(*t, 10))
        })
        .expect("a day plans");
        assert_eq!((offset, best.total_s), (3, 700));
        assert!(best_day(3, |_| None).is_none());
        // A day that does not plan is skipped, not fatal.
        let (offset, _) = best_day(3, |k| (k == 2).then(|| plan(5, 1))).expect("one day plans");
        assert_eq!(offset, 2);
    }

    #[test]
    fn a3_the_formatters_read_as_the_lines_do() {
        assert_eq!(au3(AU_KM), "1.000");
        assert_eq!(au3(AU_KM / 2), "0.500");
        assert_eq!(lag_text(125), "2m 05s light");
        assert_eq!(lag_text(59), "0m 59s light");
    }
}
