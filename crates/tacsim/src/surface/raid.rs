//! **The raid director** (doc 36 cut GC.11; Q-36-41, Q-36-55).
//!
//! The multi-party field needs somebody hostile when no staff hand is there. The director is a
//! pass on the surface tick (`tacsim.surface`, once a second) that sends the content's raiders
//! (`content/field/raiders.json`, [`crate::field::raiders`]) by each one's schedule
//! ([`crate::field::raiders::RaidSchedule`]): one raider a site, so the schedule is per site.
//! It runs only under `[tacsim.field].raid_director`; off, raiders are staff-run as before.
//!
//! **When a raid comes.** Every fire, for each raider the director schedules, a raid lands when
//! all of these hold:
//! - **presence**: at least `presence_flights` flights out on the site's ground (anyone's units
//!   but a raider's, not counting carriers on their pads or the casualties and wrecks the site
//!   record holds);
//! - **ore value**: at least `ore_kg` of ore aboard the rigs on the site's ground;
//! - **the cooldown**: `cooldown_ic_s` IC seconds since the raider's last landing;
//! - the raider has hulls aboard its carrier (losses are real: a hull that fell stays where it
//!   fell, so a raider that has lost them all sends nobody), and its carrier is no wreck.
//!
//! **What a raid does.**
//! 1. **Seed** the raider when it has never stood (`raiders::seed`: its leader, registered as its
//!    organization's principal, its carrier and hulls). A raider whose carrier is a wreck is
//!    spent and never seeded again: seeding is refused while any hull of it stands, and the
//!    director marks the carrier spent so nothing ever sends it.
//! 2. **Land and launch**: the carrier sets down at its authored cell and at most the schedule's
//!    `threat` hulls stand beside its ramp (`raiders::land_and_launch`); the site hears
//!    [`RAID_LANDED`]. On a site already in a field, the carrier and its hulls **arrive**: the
//!    kernel's join takes each in at the next pulse as a unit of the raider's party, under its
//!    leader's standing Hostile marks, with its carrier's ramp laid as the party's home
//!    (`raiders::join_raid`; doc 36 GC.1b).
//! 3. **Declare** on open ground: at the next fire the leader marks the party of the nearest
//!    flight Hostile through the ordinary door (`crate::field::mark_from_order`), which promotes
//!    the site. Admission then takes the raider in with everyone else.
//! 4. **Withdraw** (Q-36-55) is the field host's, at its paced snapshot check
//!    (`raiders::lift_check`): the group orders its standing hulls home up the ramp once it has
//!    lost `raid_lost_permille` of what it launched, once it has spent `raid_field_s` in the
//!    field, or once no armed party it is hostile to has stood in any of its hulls' reach for
//!    `idle_demote_s` (Q-36-5's interval). Each hull boards at the ramp, and the carrier lifts by
//!    GC.0's lift once the last is aboard or down. `@field/raiders/withdraw` stays the staff
//!    override.
//! 5. **Over.** The raid ends when the carrier has lifted. If its field demoted first (the field's
//!    own quiet or idle bound), the director takes the standing hulls aboard on the open ground
//!    and the carrier lifts there, leaving what fell where it fell. Nobody left to declare on
//!    ends a landing the same way.
//!
//! **What is durable.** The raid's state rides the carrier's own raider attribute
//! ([`crate::field::raiders::RAIDER_ATTR`], owned by `GOD`): `raid` while one is under way (its
//! landing's IC second, and the field it entered with the arena millisecond it entered at),
//! `last_raid_ic` for the cooldown, `spent` once the carrier is a wreck. Only the idle trigger's
//! "last seen in reach" is RAM ([`RaidBook`]): a restart counts it again from the resume.
//!
//! **Staff raids are untouched.** The director sends and withdraws only raids it landed (the
//! `raid` key). A raider staff launched stands as staff left it and is never sent. One staff only
//! seeded (its carrier on the ground, every hull aboard) is sent like any other.

use std::collections::BTreeMap;

use mush_core::{Ctx, ObjId};

use crate::field::raiders::{self, RaidSchedule, RaiderDef};
use crate::surface::site_record::{LeftCondition, SiteRecord};

/// **The site hears a raider land** (`{name}` the carrier, `{site}` the site): a ship setting
/// down is a physical fact anyone there observes, never whose it is.
pub const RAID_LANDED: &str = "The {name} sets down on {site} and lowers its ramp.";

/// **The director's RAM** (module state): for each director raid's carrier in a field, the arena
/// millisecond at which one of its hulls last held an armed hostile party in reach (Q-36-55's idle
/// trigger). Rebuilt from the field's first check after a restart.
#[derive(Clone, Debug, Default)]
pub struct RaidBook {
    reach: BTreeMap<ObjId, i64>,
}

/// **The raid director's pass** (this module's head): every scheduled raider visited, each
/// failure logged and passed over until the next fire.
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    if !crate::field::config(ctx.w).raid_director || ctx.w.world_timeline().is_frozen() {
        return;
    }
    let Ok(defs) = raiders::raiders() else {
        return;
    };
    for def in &defs {
        let Some(schedule) = def.director else {
            continue;
        };
        if let Err(why) = visit(ctx, def, schedule) {
            tracing::debug!(raider = %def.id, reason = %why, "tacsim: the raid director passed a raider over");
        }
    }
}

/// The carrier of a raider, when it has been seeded.
fn carrier_of(ctx: &Ctx<'_>, def: &RaiderDef) -> Option<ObjId> {
    raiders::hulls_of(ctx.w, def.id.as_str())
        .into_iter()
        .find(|hull| raiders::ramp_cell(ctx.w, *hull).is_some())
}

/// A raider attribute's key on `carrier`.
fn mark_key(ctx: &Ctx<'_>, carrier: ObjId, key: &str) -> Option<serde_json::Value> {
    raiders::mark_of(ctx.w, carrier).and_then(|mark| mark.get(key).cloned())
}

/// Whether the site record holds `unit` as a casualty or a wreck.
fn left_down(record: Option<&SiteRecord>, unit: ObjId) -> Option<LeftCondition> {
    record?
        .units
        .iter()
        .find(|left| left.unit == unit)
        .map(|left| left.condition)
}

/// One raider's visit.
fn visit(ctx: &mut Ctx<'_>, def: &RaiderDef, schedule: RaidSchedule) -> Result<(), String> {
    let record = crate::field::close::site_record(ctx, &def.site);
    let now = ctx.w.world_ic_now().at;
    let carrier = carrier_of(ctx, def);
    if let Some(carrier) = carrier {
        if mark_key(ctx, carrier, "spent").is_some() {
            return Ok(());
        }
        // A raider group whose carrier is wrecked does not respawn (Q-36-41).
        if left_down(record.as_ref(), carrier) == Some(LeftCondition::Wreck) {
            raiders::set_mark(ctx, carrier, "spent", Some(serde_json::Value::Bool(true)))?;
            tracing::info!(raider = %def.id, carrier = carrier.0, "tacsim: a raider's carrier is a wreck; the director never sends it again");
            return Ok(());
        }
        if let Some(raid) = mark_key(ctx, carrier, "raid") {
            return under_way(ctx, def, schedule, carrier, &raid, record.as_ref());
        }
        // A raider staff launched is staff's: some hull of its carrier stands on the ground. A
        // carrier on the ground with every hull aboard (seeded, never launched) may be sent.
        if raiders::hulls_of(ctx.w, def.id.as_str())
            .into_iter()
            .filter(|hull| raiders::carrier_of(ctx.w, *hull) == Some(carrier))
            .any(|hull| mush_sim::pose_of(ctx.w, hull).is_some())
        {
            return Ok(());
        }
        let last = mark_key(ctx, carrier, "last_raid_ic").and_then(|at| at.as_i64());
        if last.is_some_and(|last| now.saturating_sub(last) < schedule.cooldown_ic_s) {
            return Ok(());
        }
    }
    let Some(domain) = crate::field::parties::site_domain(ctx.w, &def.site) else {
        return Err("not a lane site".to_owned());
    };
    if presence(ctx, domain, record.as_ref()) < usize::from(schedule.presence_flights)
        || ore_kg(ctx, domain) < schedule.ore_kg
    {
        return Ok(());
    }
    // On a site in a field, only a running one takes an arrival; a promotion still being
    // admitted or closed waits.
    let in_field = record
        .as_ref()
        .is_some_and(|record| record.promoted.is_some());
    let hosted = crate::field::live_on(ctx.w, &def.site).is_some();
    if in_field && !hosted {
        return Ok(());
    }
    if carrier.is_none() {
        let line = raiders::seed(ctx, def)?;
        tracing::info!(raider = %def.id, line = %line, "tacsim: the raid director seeded a raider");
    }
    let record =
        record.unwrap_or_else(|| crate::surface::site_record::fresh_record(ctx.w, &def.site));
    let (carrier, launched) =
        raiders::land_and_launch(ctx, def, usize::from(schedule.threat), &record)?;
    raiders::set_mark(
        ctx,
        carrier,
        "raid",
        Some(serde_json::json!({ "landed_ic": now, "launched": launched.len() })),
    )?;
    raiders::set_mark(ctx, carrier, "last_raid_ic", Some(now.into()))?;
    let line = RAID_LANDED
        .replace("{name}", &crate::port::coop::name_of(ctx.w, carrier))
        .replace("{site}", &crate::surface::act::site_name(ctx.w, &def.site));
    crate::field::tell_site(ctx, &def.site, &line);
    tracing::info!(raider = %def.id, hulls = launched.len(), in_field, "tacsim: the raid director landed a raid");
    if in_field {
        raiders::join_raid(ctx, &def.site, carrier, &launched)?;
    }
    Ok(())
}

/// **Flights out on the site's ground** (presence): every unit on it but a raider's, a carrier on
/// its pad, and a casualty or wreck the site record holds.
fn presence(ctx: &Ctx<'_>, domain: mush_sim::DomainId, record: Option<&SiteRecord>) -> usize {
    crate::field::parties::on_ground(ctx.w, domain)
        .into_iter()
        .filter(|unit| raiders::raider_of(ctx.w, *unit).is_none())
        .filter(|unit| !crate::field::carrier::is_carrier(ctx.w, *unit))
        .filter(|unit| left_down(record, *unit).is_none())
        .count()
}

/// **The ore aboard the rigs on the site's ground**, kilograms (ore value).
fn ore_kg(ctx: &Ctx<'_>, domain: mush_sim::DomainId) -> u64 {
    crate::surface::act::world_rigs(ctx.w)
        .into_iter()
        .filter(|rig| mush_sim::pose_of(ctx.w, *rig).is_some_and(|(on, _)| on == domain))
        .filter_map(|rig| crate::port::tv::hopper_load_of(ctx, rig).ok())
        .map(|(kg, _)| kg)
        .fold(0_u64, u64::saturating_add)
}

/// **A raid under way**: in its field, the host's check withdraws it; on open ground it declares,
/// or it is over and goes aboard.
fn under_way(
    ctx: &mut Ctx<'_>,
    def: &RaiderDef,
    schedule: RaidSchedule,
    carrier: ObjId,
    raid: &serde_json::Value,
    record: Option<&SiteRecord>,
) -> Result<(), String> {
    if let Some(record) = record.filter(|record| record.promoted.is_some()) {
        let Some(live) = crate::field::live_on(ctx.w, &def.site) else {
            return Ok(()); // admission, or the close, is still to come
        };
        // In the running field: note the field it entered (the snapshots' check reads it).
        if live.units.iter().any(|row| row.unit == carrier) {
            entered(ctx, carrier, live.field.ordinal, live.at_ms)?;
            return rejoin_refused(ctx, def, schedule, carrier, &live, record);
        }
        if mush_sim::pose_of(ctx.w, carrier).is_some() {
            return Ok(()); // its join is still to be answered
        }
        // Out of this field and off the ground: it lifted from this field, and the raid is over;
        // or the kernel refused its join before it ever entered (a think in flight at that
        // pulse), and it lands and joins again.
        let entered_here =
            field_entered(ctx, carrier).is_some_and(|(ordinal, _)| ordinal == live.field.ordinal);
        if entered_here {
            return over(ctx, carrier);
        }
        // The kernel refused the carrier and took its hulls: it lands alone and joins again, so
        // its group has a ramp to go home up and a carrier to lift with.
        if live
            .units
            .iter()
            .any(|row| raiders::carrier_of(ctx.w, row.unit) == Some(carrier))
        {
            raiders::land_carrier(ctx, def, carrier)?;
            tracing::info!(raider = %def.id, "tacsim: a refused raider carrier lands and joins again");
            return raiders::join_raid(ctx, &def.site, carrier, &[]);
        }
        let (carrier, launched) =
            raiders::land_and_launch(ctx, def, usize::from(schedule.threat), record)?;
        tracing::info!(raider = %def.id, hulls = launched.len(), "tacsim: a refused raid lands and joins again");
        return raiders::join_raid(ctx, &def.site, carrier, &launched);
    }
    // The carrier lifted: the raid is over.
    if mush_sim::pose_of(ctx.w, carrier).is_none() {
        return over(ctx, carrier);
    }
    // The field it fought in demoted with it on the ground, or there is nobody left to declare
    // on: its standing hulls go aboard and it lifts there.
    let fought = raid.get("field").is_some();
    let target = if fought {
        None
    } else {
        target(ctx, def, record)
    };
    let Some((from, at)) = target else {
        recall(ctx, def, carrier, record)?;
        return over(ctx, carrier);
    };
    let Some(leader) = raiders::leader_of(ctx.w, carrier) else {
        return Err("the raider's leader is gone".to_owned());
    };
    if ctx.authoritative_pending(leader) {
        return Ok(());
    }
    crate::field::mark_from_order(ctx, leader, &def.site, (from, at))
        .map_err(|error| error.to_string())?;
    tracing::info!(raider = %def.id, at = at.0, "tacsim: a director raid declared on open ground");
    Ok(())
}

/// **Hulls the kernel refused at the join land and join again** (a director raid in its field):
/// a refused hull went back aboard, so while the raid has not turned for home (no hull of it
/// withdrawing or out at the ramp), the hulls aboard stand beside the ramp again, up to its
/// threat less those in the field, and are sent as joins. The kernel refuses a join while a
/// think is in flight, so this is the common case, not a rare one.
fn rejoin_refused(
    ctx: &mut Ctx<'_>,
    def: &RaiderDef,
    schedule: RaidSchedule,
    carrier: ObjId,
    live: &crate::field::FieldLive,
    record: &SiteRecord,
) -> Result<(), String> {
    let hulls: Vec<ObjId> = raiders::hulls_of(ctx.w, def.id.as_str())
        .into_iter()
        .filter(|hull| *hull != carrier && raiders::carrier_of(ctx.w, *hull) == Some(carrier))
        .collect();
    let turned = hulls
        .iter()
        .any(|hull| raiders::withdrawing(ctx.w, *hull) || record.exited(*hull));
    if turned {
        return Ok(());
    }
    let standing = hulls
        .iter()
        .filter(|hull| live.units.iter().any(|row| row.unit == **hull))
        .count();
    let cap = usize::from(schedule.threat).saturating_sub(standing);
    let aboard = hulls
        .iter()
        .any(|hull| mush_sim::pose_of(ctx.w, *hull).is_none());
    if cap == 0 || !aboard {
        return Ok(());
    }
    let (carrier, launched) = raiders::land_and_launch(ctx, def, cap, record)?;
    tracing::info!(raider = %def.id, hulls = launched.len(), "tacsim: refused raider hulls land and join again");
    raiders::join_raid(ctx, &def.site, carrier, &launched)
}

/// **Note the field a director raid entered** (once a field ordinal): the arena millisecond it
/// was first seen there, the twenty-minute trigger's start.
pub(crate) fn entered(
    ctx: &mut Ctx<'_>,
    carrier: ObjId,
    ordinal: u32,
    at_ms: i64,
) -> Result<(), String> {
    let Some(mut raid) = mark_key(ctx, carrier, "raid") else {
        return Ok(());
    };
    if raid
        .get("field")
        .and_then(|field| field.get(0))
        .and_then(serde_json::Value::as_u64)
        == Some(u64::from(ordinal))
    {
        return Ok(());
    }
    if let Some(map) = raid.as_object_mut() {
        map.insert("field".to_owned(), serde_json::json!([ordinal, at_ms]));
    }
    raiders::set_mark(ctx, carrier, "raid", Some(raid))
}

/// The field a director raid entered, as its ordinal and the arena millisecond.
fn field_entered(ctx: &Ctx<'_>, carrier: ObjId) -> Option<(u32, i64)> {
    let raid = mark_key(ctx, carrier, "raid")?;
    let field = raid.get("field")?.as_array()?;
    match field.as_slice() {
        [ordinal, at] => Some((u32::try_from(ordinal.as_u64()?).ok()?, at.as_i64()?)),
        _ => None,
    }
}

/// **The nearest flight to declare on**, seen from the nearest of the raider's hulls on the
/// ground: `(from, at)`, ties to the lower object numbers.
fn target(ctx: &Ctx<'_>, def: &RaiderDef, record: Option<&SiteRecord>) -> Option<(ObjId, ObjId)> {
    let domain = crate::field::parties::site_domain(ctx.w, &def.site)?;
    let cell = |unit: ObjId| {
        mush_sim::pose_of(ctx.w, unit)
            .filter(|(on, _)| *on == domain)
            .map(|(_, pose)| crate::surface::excursion::site_cell(pose.x_cm, pose.y_cm))
    };
    let ground = crate::field::parties::on_ground(ctx.w, domain);
    let own: Vec<(ObjId, (i64, i64))> = ground
        .iter()
        .copied()
        .filter(|unit| raiders::raider_of(ctx.w, *unit).as_deref() == Some(def.id.as_str()))
        .filter(|unit| raiders::carrier_of(ctx.w, *unit).is_some())
        .filter_map(|unit| cell(unit).map(|at| (unit, at)))
        .collect();
    ground
        .iter()
        .copied()
        .filter(|unit| raiders::raider_of(ctx.w, *unit).is_none())
        .filter(|unit| left_down(record, *unit).is_none())
        .filter_map(|other| {
            let at = cell(other)?;
            own.iter()
                .map(|(from, here)| {
                    (
                        crate::surface::excursion::cells_apart(*here, at),
                        *from,
                        other,
                    )
                })
                .min()
        })
        .min()
        .map(|(_, from, at)| (from, at))
}

/// **Take a director raid's standing hulls aboard on open ground, and lift its carrier**: every
/// hull of the carrier on the site's ground but the casualties and wrecks the site record holds,
/// which stay where they fell.
fn recall(
    ctx: &mut Ctx<'_>,
    def: &RaiderDef,
    carrier: ObjId,
    record: Option<&SiteRecord>,
) -> Result<(), String> {
    for hull in raiders::hulls_of(ctx.w, def.id.as_str()) {
        if raiders::carrier_of(ctx.w, hull) != Some(carrier)
            || mush_sim::pose_of(ctx.w, hull).is_none()
            || left_down(record, hull).is_some()
            || mush_sim::deployed::is_deployed(ctx.w, hull)
        {
            continue;
        }
        raiders::set_mark(ctx, hull, "withdraw", None)?;
        mush_sim::unplace(ctx, hull, "tacsim: aboard its raider carrier")?;
    }
    raiders::lift(ctx, carrier, &def.site)
}

/// **A director raid is over**: its state cleared (the cooldown runs from its landing).
fn over(ctx: &mut Ctx<'_>, carrier: ObjId) -> Result<(), String> {
    if let Some(book) = ctx.w.module_state_mut::<RaidBook>() {
        book.reach.remove(&carrier);
    }
    tracing::info!(carrier = carrier.0, "tacsim: a director raid is over");
    raiders::set_mark(ctx, carrier, "raid", None)
}

/// **What the snapshots' check read for one director raid** in its field (Q-36-55).
pub(crate) struct RaidReading {
    /// The field's ordinal.
    pub ordinal: u32,
    /// The snapshot's arena millisecond.
    pub at_ms: i64,
    /// Hulls launched into this field (in it now, or out at the ramp).
    pub launched: usize,
    /// Of them, the casualties and wrecks.
    pub lost: usize,
    /// Whether one of its hulls holds an armed hostile party in reach.
    pub in_reach: bool,
}

/// **Whether a director raid turns for home now** (Q-36-55), and why: it has lost
/// `raid_lost_permille` of what it launched, spent `raid_field_s` in the field, or held no armed
/// hostile party in reach for `idle_demote_s`. `None` for a raid the director did not land (staff
/// raids are staff's), or one that stays.
pub(crate) fn withdrawal_due(
    ctx: &mut Ctx<'_>,
    carrier: ObjId,
    reading: &RaidReading,
) -> Option<&'static str> {
    mark_key(ctx, carrier, "raid")?;
    let cfg = crate::field::config(ctx.w);
    let _ = entered(ctx, carrier, reading.ordinal, reading.at_ms);
    let from_ms = field_entered(ctx, carrier)
        .filter(|(ordinal, _)| *ordinal == reading.ordinal)
        .map_or(reading.at_ms, |(_, at)| at);
    let book = ctx.w.module_state_mut::<RaidBook>()?;
    if reading.in_reach {
        book.reach.insert(carrier, reading.at_ms);
    }
    // Before the first reach this field, the field's entry; a resume that rewound re-reads it.
    let last = book
        .reach
        .get(&carrier)
        .copied()
        .filter(|at| *at <= reading.at_ms && *at >= from_ms)
        .unwrap_or(from_ms);
    if reading.launched > 0
        && reading.lost.saturating_mul(1_000)
            >= reading
                .launched
                .saturating_mul(cfg.raid_lost_permille as usize)
    {
        return Some("lost");
    }
    if reading.at_ms.saturating_sub(from_ms) >= i64::from(cfg.raid_field_s) * 1_000 {
        return Some("field time");
    }
    if reading.at_ms.saturating_sub(last) >= i64::from(cfg.idle_demote_s) * 1_000 {
        return Some("idle");
    }
    None
}

/// The director's carrier for `site`'s raider in a field, when it is a director raid (the
/// gate reads it).
#[must_use]
pub fn director_raid(w: &mush_core::World, carrier: ObjId) -> bool {
    raiders::mark_of(w, carrier).is_some_and(|mark| mark.get("raid").is_some())
}

/// The IC second a raider's carrier last landed by the director, when it has.
#[must_use]
pub fn last_raid_ic(w: &mush_core::World, carrier: ObjId) -> Option<i64> {
    raiders::mark_of(w, carrier)?.get("last_raid_ic")?.as_i64()
}

/// Whether the director marked a raider's carrier spent (a wreck; never sent again).
#[must_use]
pub fn spent(w: &mush_core::World, carrier: ObjId) -> bool {
    raiders::mark_of(w, carrier).is_some_and(|mark| mark.get("spent").is_some())
}
