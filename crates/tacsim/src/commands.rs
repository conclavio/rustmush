//! **The `@tacsim` surface** (B4, B6; §3.1's command list).
//!
//! One primary verb, entered in the pinned rule-1 allowlist in the same commit
//! (08i §4). Three switches: `pack`, `seed`, `pin` — and since E7r1 `time`, and
//! doc 18 cut IC3 `port` (the staff readiness report).
//!
//! # Disclosure
//!
//! *A rules pack is public; a campaign-seed pack is staff's.* A non-staff viewer
//! asking about a seed pack gets [`NO_SUCH_PACK`] — **byte-identical** to the answer
//! for an id that was never published — so `@tacsim/pack show` is not a pack-existence
//! oracle (§3.1's review item, 08c §6.2 D1). Every refusal in this module is **one
//! message** through the ordinary error renderer.
//!
//! **`list` is the switch that needed the care** (independent verify B2, correcting
//! this block's earlier "at every switch … none of them branches on how much the viewer
//! already knows"). `list` renders *rows*, not a refusal, so hiding is not enough: the
//! page marker used to be pushed on records **visited**, ahead of the visibility
//! filter, and it was an oracle twice over. A server loading 33 campaign-seed packs and
//! no rules pack answered a non-staff viewer *"more than 32 packs are loaded"* where an
//! empty server answered *"No packs are loaded."*; and because the visit budget is a
//! fixed [`crate::store::PACK_PAGE`], a viewer who saw the marker could subtract the
//! rendered rows from it and learn exactly how many packs inside the window were
//! hidden. The **work** is still bounded on records visited (D-LBK-5 is right); the
//! **marker** is now bounded on records the viewer may *see*, so a full window of
//! invisible packs is byte-identical to an empty server and a rendered row count is
//! indistinguishable from a smaller server's.
//!
//! # Bounds
//!
//! Every switch that can render more than one row is paged at
//! [`crate::store::PACK_PAGE`], and the page is taken on records **visited**, not
//! matched (D-LBK-5), so a viewer who may see none of them still pays exactly one page
//! of work:
//!
//! | switch | bound |
//! |---|---|
//! | `@tacsim/pack list` | `PACK_PAGE` records visited; the marker on records viewable |
//! | `@tacsim/pack show` | three header rows plus `PACK_PAGE` entries, with a cursor to continue (B3 — it used to render all `MAX_PACK_ENTRIES` of them, 515 messages per non-staff command) |
//! | `@tacsim/pack verify` | **staff-only** and `PACK_PAGE` records visited (B1 — it used to be `require_enabled` alone and unpaged, so any player could make the world's single writer canonically re-encode and SHA-256 up to `MAX_CONFIGURED_PACKS` × `MAX_PACK_BYTES` = 32 MiB, synchronously, at command rate) |
//! | `@tacsim/pin list` | `PACK_PAGE` records visited |
//!
//! `mush-core`'s `Outbox` carries no per-command message cap to fall back on, so an
//! unpaged loop here is the whole bound.

use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{ArgShape, CanonicalId, CmdError, CommandSpec, Ctx, ObjId, World};
use smol_str::SmolStr;

use crate::ephemeris::AU_KM;
use crate::pack::{PackEntry, PackKind};
use crate::record::PinRecord;
use crate::seed::{self, SeedError};
use crate::sky::{Sky, epoch_date};
use crate::store::{MAX_PINS, PACK_PAGE, TacsimStore, del_pin, persist_meta, put_pin};

/// The one existence-hiding refusal. Identical for "never published", "not loaded"
/// and "a seed pack you may not see".
pub const NO_SUCH_PACK: &str = "No such pack.";

/// What an `enabled = false` server answers. The verbs still register (E5c
/// deviation 1): a disabled layer refuses out loud rather than hiding a verb.
pub const NOT_ENABLED: &str = "TacSim is not enabled on this server.";

/// Register the one TacSim verb.
///
/// # Errors
///
/// [`ModError`] if the name is already taken.
pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "@tacsim".into(),
            shape: ArgShape::EqPair,
            switches: ["pack", "seed", "pin", "time", "port"]
                .iter()
                .copied()
                .map(SmolStr::new)
                .collect(),
            help: "@tacsim/pack list                   every rules pack this server loads.\n\
                   @tacsim/pack show <id>[@<version>] [<from-entry>]\n\
                   \x20                                 one pack: its digest, its dependencies \
                   and a page of its entries, continuing from <from-entry> when the last page \
                   said there were more. A campaign-seed pack is staff-only and answers \"No such \
                   pack.\" to everyone else — the same answer an unknown id gets.\n\
                   @tacsim/pack verify [<id>[@<version>]]\n\
                   \x20                                 (staff) re-verify the loaded bytes' \
                   digests against the ledger row each version was published under. It re-reads \
                   no file: a pack whose declared digest did not match its content was refused at \
                   boot, naming the file.\n\
                   @tacsim/seed <pack>[@<version>] [dry|commit]    (staff) apply a campaign-seed pack through the content latch; a newer version updates it in place. \
                   `dry` writes nothing and lists every planned object; `commit` applies it, and \
                   a second commit is a HIT that writes nothing.\n\
                   @tacsim/pin <subject>=<pack>@<version>\n\
                   \x20                                 (staff) pin one subject to one exact pack \
                   version, so publishing a newer version cannot move it. The pack must declare \
                   the subject.\n\
                   @tacsim/pin <subject>=              (staff) remove that subject's pin.\n\
                   @tacsim/pin list                    (staff) every pin this world carries.
                   @tacsim/time show                   the live world-time generation, its rate,                    the generations left, and each subsystem's declared catch-up policy.
                   @tacsim/time rate=<n>               (staff) open ONE new world-time rate                    generation at <n> IC seconds per real second, and announce each subsystem's                    declared policy for it. It is one attributed act on the kernel's own                    timeline: no second timeline is created and no earlier fact is reinterpreted.
                   @tacsim/port status                 (staff) the port readiness report: every binding the port entries made — rooms, domains, orgs, accounts and capital — with the lane, the sky and the world rate.
                   @tacsim/port clear #<hull>          (staff) stand down a programme hull's filed clearance whose launch is held: the pending passage is cleared, its burn sequence kept, the ticket (if burned) kept as the record; nothing is refunded and the pilot is told."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(tacsim_cmd),
    )?;
    // ---- E7r1: R2 — `@corridor`, a ReadOnly group of its own (10b §15.1 item 7) ----
    //
    // K8.5: the class is declared **before** the `reg.command` it applies to, and the
    // module default is restored immediately after, so a later registration in this
    // crate keeps `LegacyPostImage`. Every switch is a pure read of the materialized
    // route projection: no document, no post, no submission.
    reg.classify_commands(mush_core::CommandCompatibility::ReadOnly);
    reg.command(
        CommandSpec {
            name: "@corridor".into(),
            shape: ArgShape::EqPair,
            switches: ["show", "rules", "domain"]
                .iter()
                .copied()
                .map(SmolStr::new)
                .collect(),
            help: "@corridor/show <route>              one corridor: its endpoints, its legs and                    their unladen times, and who holds it. Pack-authored navigation data is                    public, deliberately: a corridor is a road, not a secret.
                   @corridor/rules <route>             the corridor's authored hazard bands and                    interdiction thresholds, leg by leg.
                   @corridor/domain <route>            which of the corridor's legs an                    interception can promote at, from the pack's own interdiction thresholds and                    domain host bindings. It never names a live domain: which domains are                    standing up is staff's (`@domain`).
                   An unobserved corridor and one that was never authored answer the same line."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(corridor_cmd),
    )?;
    reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
    // ---- Doc 18 cut IC3 (§4.1): `+credit`, a ReadOnly group of its own ----
    //
    // The `@corridor` bracket's pattern: the class is declared before the
    // `reg.command` and the module default restored right after. A pure read of
    // the actor's own account in the economy snapshot: no document, no post, no
    // submission, and no argument, so it reads no other account.
    reg.classify_commands(mush_core::CommandCompatibility::ReadOnly);
    reg.command(
        CommandSpec {
            name: crate::port::credit::CREDIT_VERB.into(),
            shape: ArgShape::None,
            help: crate::port::credit::CREDIT_HELP.into(),
            ..CommandSpec::default()
        },
        Arc::new(crate::port::credit::credit_cmd),
    )?;
    reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
    // ---- Doc 18 cut IC6 (§4.1, §4.2; D-18-23, D-18-34): `+coop` and `+ship` ----
    //
    // Under the module's `LegacyPostImage` class (lead ruling L12: a class is
    // per verb, and each verb carries a writing switch). `+coop` and
    // `+coop/statement` are pure reads; `+coop/enrol` is ONE command and ONE
    // batch through the economy's own API (account, lease, stake, vault) plus
    // tacsim's one enrolment document and the hull's rename. `+ship` and
    // `+ship/hold` are pure reads; `+ship/board` and `+ship/leave` are
    // placement verbs through mush-sim (leave also writes the last-port
    // marker). None submits anything authoritative. (`+ship` moved to its own
    // bracket below in doc 18 cut IC9.)
    reg.command(
        CommandSpec {
            name: crate::port::coop::COOP_VERB.into(),
            shape: ArgShape::Raw,
            switches: ["enrol", "statement"]
                .iter()
                .copied()
                .map(SmolStr::new)
                .collect(),
            help: crate::port::coop::COOP_HELP.into(),
            ..CommandSpec::default()
        },
        Arc::new(crate::port::coop::coop_cmd),
    )?;
    // ---- Doc 18 cut IC9 (§4.2, §4.4; D-18-11, D-18-23, D-18-46): `+ship`, Authoritative ----
    //
    // Lead ruling L11 (the ceiling rule, mush-market's precedent): a CommandSpec is
    // one primary verb and takes the CEILING class of its switches, and
    // `+ship/undock` stages exactly one `tacsim_port_burn` submission. The class is
    // declared before the `reg.command` and the module default restored right
    // after. `+ship/plan` is a pure read; `+ship/undock` writes the hull's one
    // `tacsim:voyage:<hull>` document and submits the burn as the actor. Doc 18 cut
    // IC11: `/orders` is a read, `/order` writes the hull's one `tacsim:orders:<hull>`
    // document (and submits nothing; the port tick's order arm executes it).
    reg.classify_commands(mush_core::CommandCompatibility::Authoritative);
    reg.command(
        CommandSpec {
            name: crate::port::ship::SHIP_VERB.into(),
            shape: ArgShape::Raw,
            switches: [
                "board", "leave", "hold", "plan", "undock", "orders", "order",
            ]
            .iter()
            .copied()
            .map(SmolStr::new)
            .collect(),
            help: crate::port::ship::SHIP_HELP.into(),
            ..CommandSpec::default()
        },
        Arc::new(crate::port::ship::ship_cmd),
    )?;
    reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
    Ok(())
}

fn info(ctx: &mut Ctx<'_>, msg: impl Into<String>) {
    let actor = ctx.actor;
    ctx.out.sys_to_actor(actor, "tacsim", msg.into());
}

fn lines(ctx: &mut Ctx<'_>, rows: Vec<String>) {
    for row in rows {
        info(ctx, row);
    }
}

/// Staff, on `mush-market`'s exact test.
fn is_staff(w: &World, who: ObjId) -> bool {
    mush_core::perm::has_power(w, who, mush_core::Powers::SIM_ADMIN)
        || mush_core::perm::has_role(w, who, mush_core::Role::Royalty)
}

fn require_staff(ctx: &Ctx<'_>) -> Result<(), CmdError> {
    if is_staff(ctx.w, ctx.actor) {
        Ok(())
    } else {
        Err(CmdError::msg("Permission denied."))
    }
}

fn store<'a>(ctx: &'a Ctx<'_>) -> Result<&'a TacsimStore, CmdError> {
    ctx.w
        .module_state::<TacsimStore>()
        .ok_or_else(|| CmdError::msg(NOT_ENABLED))
}

fn require_enabled(ctx: &Ctx<'_>) -> Result<(), CmdError> {
    if store(ctx)?.enabled() {
        Ok(())
    } else {
        Err(CmdError::msg(NOT_ENABLED))
    }
}

/// Parse `<id>` or `<id>@<version>`.
fn parse_ref(text: &str) -> Option<(CanonicalId, Option<u32>)> {
    let text = text.trim();
    match text.rsplit_once('@') {
        Some((id, version)) => {
            let version: u32 = version.trim().parse().ok()?;
            (version > 0)
                .then(|| CanonicalId::parse_canonical(id.trim()).ok())
                .flatten()
                .map(|id| (id, Some(version)))
        }
        None => CanonicalId::parse_canonical(text).ok().map(|id| (id, None)),
    }
}

fn tacsim_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("seed") {
        seed_cmd(ctx)
    } else if ctx.switches.has("pin") {
        pin_cmd(ctx)
    } else if ctx.switches.has("time") {
        time_cmd(ctx)
    } else if ctx.switches.has("port") {
        port_cmd(ctx)
    } else {
        pack_cmd(ctx)
    }
}

// ---- E7r1: R5(a) — `@tacsim/time`, the rate policy's caller ---------------
//
// 10b §15.1 item 8, re-scoped: `Ctx::change_world_time_rate` existed with **zero
// callers** and `[tacsim.time]` was parsed and inert. This is the caller, and it is
// the whole of what was missing on this side — *journeys, tactical domains and
// propagation are already correct* (cut 6's `reanchor`, cut 8's
// `schedule_scan_clock_is_skippable`, cut 7's re-anchor + `clock_generation`
// Losses), so this pass pins them rather than rebuilding them.
//
// **`[tacsim.time]` still reaches `load_or_seed_world_timeline` never.** Generation
// one comes from `[game].ic_ratio`/`ic_epoch` and persisted truth wins thereafter;
// this verb opens a **later** generation on the kernel's own timeline, through the
// kernel's own call, and constructs no `GameClock` and no `WorldTimelineEpoch` of its
// own. §3.1's pinned `mush-server` test is untouched.

/// The four subsystems `[tacsim.time]` declares a catch-up policy for, in the order
/// they are announced. The words themselves are configuration, never constants here.
pub const CATCH_UP_SUBSYSTEMS: [&str; 4] = ["journeys", "production", "fronts", "domains"];

fn time_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_enabled(ctx)?;
    let lhs = ctx.args.lhs().trim().to_owned();
    let rhs = ctx
        .args
        .rhs()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    match rhs {
        // `@tacsim/time rate=<n>` — the one write.
        Some(rhs) if lhs.eq_ignore_ascii_case("rate") => time_rate(ctx, &rhs),
        _ => time_show(ctx),
    }
}

/// The declared policy words, paired with their subsystem, in announcement order.
fn declared_policies(ctx: &Ctx<'_>) -> Result<Vec<(&'static str, String)>, CmdError> {
    let time = store(ctx)?.config().time.clone();
    Ok(vec![
        ("journeys", time.catch_up_journeys),
        ("production", time.catch_up_production),
        ("fronts", time.catch_up_fronts),
        ("domains", time.catch_up_domains),
    ])
}

fn time_show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let epoch = ctx
        .w
        .world_timeline()
        .epochs()
        .last()
        .copied()
        .ok_or_else(|| CmdError::msg("This world has no time generation."))?;
    let mut rows = vec![format!(
        "World time: generation {}, {} IC seconds per real second (anchored at IC {}).",
        epoch.generation, epoch.rate, epoch.ic_anchor
    )];
    for (subsystem, word) in declared_policies(ctx)? {
        rows.push(format!(
            "  {subsystem}: {}",
            if word.trim().is_empty() {
                "(no policy declared)".to_owned()
            } else {
                word
            }
        ));
    }
    lines(ctx, rows);
    Ok(())
}

fn time_rate(ctx: &mut Ctx<'_>, rhs: &str) -> Result<(), CmdError> {
    if !is_staff(ctx.w, ctx.actor) {
        return Err(CmdError::msg("Only staff change the world's time rate."));
    }
    let rate: u32 = rhs
        .parse()
        .map_err(|_| CmdError::msg("Usage: @tacsim/time rate=<ic seconds per real second>"))?;
    // ---- C9/B10 (independent verify fold; owner decision F9(a)) ----
    //
    // The verb parsed a `u32` and validated only that it differed from the live rate,
    // and neither `Ctx::change_world_time_rate` nor `WorldTimeline::next_rate_epoch`
    // looked at the rate at all. `rate = 0` therefore stopped every `world_ic_now`
    // mechanism for ever — recovery cooldowns, production schedules, journey ETAs,
    // escalation cool-offs — while the compatibility `GameClock`'s two readers both
    // apply `.max(1)`, so the clock and calendar a player reads in the room kept
    // advancing at 1:1 against a stopped world. `u32::MAX` was accepted too.
    //
    // **Refused, naming the band, never clamped** — the pulse period's precedent next
    // door clamps, but a silently clamped *time rate* is a campaign act the staffer
    // did not make. The kernel refuses it as well
    // (`WorldTimelineError::RateOutOfBand`), so no other caller can install one
    // either; this arm exists to say so in a sentence a person can act on.
    if !(mush_core::MIN_WORLD_TIME_RATE..=mush_core::MAX_WORLD_TIME_RATE).contains(&rate) {
        return Err(CmdError::msg(format!(
            "The world time rate must be between {} and {} IC seconds per real second; {rate}              is outside that band and nothing was changed. A rate of 0 is not a pause: it              stops every world-IC mechanism while the clock a player reads keeps running.",
            mush_core::MIN_WORLD_TIME_RATE,
            mush_core::MAX_WORLD_TIME_RATE
        )));
    }
    let live = ctx.w.world_timeline().epochs().last().copied();
    // **One attributed act, and never a generation opened for nothing.** Re-typing
    // the live rate is refused rather than silently opening a second generation that
    // every subsystem would then have to react to.
    if live.is_some_and(|epoch| epoch.rate == rate) {
        return Err(CmdError::msg(format!(
            "The world already runs at {rate} IC seconds per real second."
        )));
    }
    // Read the policies BEFORE the change: `change_world_time_rate` is the last
    // fallible call this handler makes, and nothing after it may fail.
    let policies = declared_policies(ctx)?;
    let epoch = ctx
        .w
        .world_timeline()
        .epochs()
        .last()
        .copied()
        .ok_or_else(|| CmdError::msg("This world has no time generation."))?;
    let _ = epoch;
    let opened = ctx
        .change_world_time_rate(rate)
        .map_err(|error| CmdError::msg(format!("The rate was not changed: {error}")))?;
    // **Each subsystem's declared policy fires exactly once per generation**: one
    // announcement per subsystem, for the one generation this act opened.
    // Z1 (independent verify fold): the literal was wrapped without a `\`
    // continuation, so a player read ten spaces mid-sentence.
    let mut rows = vec![format!(
        "World time generation {} opens at {rate} IC seconds per real second. Nothing \
         already recorded is reinterpreted; every earlier generation stands.",
        opened.generation
    )];
    for (subsystem, word) in policies {
        rows.push(format!(
            "  {subsystem}: {}",
            if word.trim().is_empty() {
                "no policy declared — the subsystem's own default applies".to_owned()
            } else {
                word
            }
        ));
    }
    rows.push(
        "Journeys re-anchor and keep their remaining IC duration; a tactical domain \
         refuses a stale scan and pages per entry; propagation re-anchors and arms its \
         own losses; a work order started on the old generation pauses as \
         `TimelineChanged` at its next transition, and `@wo/restamp` re-anchors its \
         production schedule and its material reservations and resumes it."
            .to_owned(),
    );
    // F9(a): the epoch budget is a LIFETIME one — `MAX_WORLD_TIMELINE_EPOCHS` 4 096
    // generations, never reclaimed, because every old `ClockStamp` keeps its meaning
    // for ever. A staffer toggling the rate can exhaust it, so what is left is now
    // said out loud rather than discovered at 4 096.
    rows.push(format!(
        "  {} of {} world-time generations remain for the life of this world.",
        mush_core::MAX_WORLD_TIMELINE_EPOCHS.saturating_sub(ctx.w.world_timeline().epochs().len()),
        mush_core::MAX_WORLD_TIMELINE_EPOCHS
    ));
    lines(ctx, rows);
    Ok(())
}

// ---- Doc 18 cut IC3 — `@tacsim/port status`, the readiness report ----------

/// The one usage line of `@tacsim/port`.
pub const PORT_USAGE: &str = "Usage: @tacsim/port status | @tacsim/port clear #<hull>";

/// `@tacsim/port [status]` (doc 18 §4.7; staff): the port readiness report
/// ([`crate::port::readiness()`]), a pure read of the port maps, the economy, org
/// and sim state and the live rate. Writes nothing.
///
/// `@tacsim/port clear #<hull>` (doc 18 cut IC9, lead ruling L29; staff): the
/// terminal for a held launch — an argument to the existing switch, no new
/// switch — through `crate::port::voyage::clear_cmd` (one `tacsim:voyage:<hull>`
/// whole-document write and a notice).
fn port_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_enabled(ctx)?;
    require_staff(ctx)?;
    let lhs = ctx.args.lhs().trim().to_owned();
    let mut words = lhs.split_whitespace();
    if words
        .next()
        .is_some_and(|word| word.eq_ignore_ascii_case("clear"))
    {
        let hull = match (words.next(), words.next()) {
            (Some(target), None) => target
                .strip_prefix('#')
                .and_then(crate::port::families::strict_decimal_u32)
                .map(ObjId),
            _ => None,
        };
        let Some(hull) = hull else {
            return Err(CmdError::msg(PORT_USAGE));
        };
        let reply = crate::port::voyage::clear_cmd(ctx, hull)?;
        info(ctx, reply);
        return Ok(());
    }
    if !(lhs.is_empty() || lhs.eq_ignore_ascii_case("status")) {
        return Err(CmdError::msg(PORT_USAGE));
    }
    let rows = crate::port::readiness_in(ctx).render();
    lines(ctx, rows);
    Ok(())
}

// ---- E7r1: R2 — @corridor (ReadOnly) ---------------------------------------

/// The one existence-hiding refusal every `@corridor` switch answers with.
///
/// **Byte-identical for an unobserved corridor and one that was never authored**
/// (§4 rule 4): a route materialized from a seed pack this world has not applied,
/// a route id no pack declares, and a malformed argument all render this and
/// nothing else — so the verb is not a corridor-existence oracle.
pub const NO_SUCH_CORRIDOR: &str = "No such corridor.";

/// The behaviour a place carries when it is authored as a simulation domain's host.
///
/// `@corridor/domain` reads **this binding** and never `SpatialState::domain_of_host`
/// (independent verify fold, C5/B6; owner decision F8(b)): the binding is authored
/// data the pack and `@tacsim/seed` already publish, while the live domain record is
/// staff-only state that `mush_sim::commands::cmd_domain` guards with
/// `require_admin` on every one of its eleven switches.
pub const DOMAIN_HOST_BEHAVIOR: &str = "sim.domain";

fn corridor_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_enabled(ctx)?;
    if ctx.switches.has("rules") {
        corridor_rules(ctx)
    } else if ctx.switches.has("domain") {
        corridor_domain(ctx)
    } else {
        corridor_show(ctx)
    }
}

/// Resolve the argument to a materialized route, or the one refusal.
fn corridor_of(ctx: &Ctx<'_>, raw: &str) -> Result<mush_sim::RouteDef, CmdError> {
    let (id, version) = parse_ref(raw).ok_or_else(|| CmdError::msg(NO_SUCH_CORRIDOR))?;
    let store = store(ctx)?;
    let found = match version {
        // A version was named: it must be that pack's own route, not whichever pack
        // `resolve` would have chosen.
        Some(version) => store
            .routes()
            .find(|(_, v, def)| *v == version && def.id() == &id)
            .map(|(_, _, def)| def),
        None => store.route(&id),
    };
    found
        .cloned()
        .or_else(|| store.route_in_world(ctx.w, &id))
        .ok_or_else(|| CmdError::msg(NO_SUCH_CORRIDOR))
}

fn corridor_show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.raw().trim().to_string();
    let route = corridor_of(ctx, &raw)?;
    let (from, to) = route.endpoints();
    let controller = route
        .controller()
        .map_or_else(|| "unclaimed".to_owned(), |c| c.as_str().to_owned());
    let mut rows = vec![format!(
        "Corridor {}: #{} -> #{}, {} legs, drive class {}, propellant {}. Held by {controller}.",
        route.id(),
        from.0,
        to.0,
        route.legs().len(),
        route.drive_class(),
        route.propellant()
    )];
    // Doc 18 cut IC2 (§4.8, D-18-43): the pack's own sky through the one seam,
    // and the symbolic route behind the materialized one, so each leg can name its
    // two bodies and the distance between them at the epoch.
    let (sky, entry) = {
        let store = store(ctx)?;
        match store.route_source(ctx.w, &route) {
            Some((pack, entry)) => (
                Some(Sky::from_pack(pack, &store.config().time.ephemeris)),
                Some((pack.clone(), entry.clone())),
            ),
            None => (None, None),
        }
    };
    // S7 (T17): the sky's pin, when the corridor crosses between bodies — the
    // direct chord at the epoch (D-18-45), dated on the game calendar at the
    // origin body's own epoch, the instant the pin was taken at.
    if let (Some(km), Some((a, b))) = (route.distance_km(), route.bodies()) {
        let epoch = sky
            .as_ref()
            .and_then(|sky| sky.epoch_of(a))
            .unwrap_or(store(ctx)?.config().time.ephemeris.epoch);
        rows.push(format!(
            "  direct chord at the epoch: {km} km ({} AU) between {a} and {b}; constant sky at \
             epoch {} (T17).",
            au_text(km),
            epoch_date(epoch)
        ));
    }
    // The body each symbolic place stands on, from the route's own pack.
    let body_of = |symbol: &CanonicalId| -> Option<CanonicalId> {
        let (pack, _) = entry.as_ref()?;
        match pack.entry(symbol) {
            Some(PackEntry::Location(place)) => place.body.clone(),
            _ => None,
        }
    };
    // Bounded by `mush_sim::MAX_LEGS`, which the wire re-checks on load, so the walk
    // is bounded on records the definition could possibly hold.
    let mut from_place: Option<CanonicalId> = entry.as_ref().map(|(_, e)| e.from.clone());
    for (index, leg) in route.legs().iter().enumerate() {
        let to_place: Option<CanonicalId> = entry
            .as_ref()
            .and_then(|(_, e)| e.legs.get(index))
            .map(|leg| leg.to.clone());
        let n = index.saturating_add(1);
        let (to, secs) = (leg.to().0, leg.base_s());
        // Doc 18 cut IC2: a leg between two bodies says how far apart they stand
        // at the origin body's epoch; any other leg reads as it always did.
        let between = match (
            sky.as_ref(),
            from_place.as_ref().and_then(&body_of),
            to_place.as_ref().and_then(&body_of),
        ) {
            (Some(sky), Some(a), Some(b)) if a != b => sky
                .epoch_of(&a)
                .and_then(|t| sky.distance_km(&a, t, &b, t))
                .map(|km| (a, b, km)),
            _ => None,
        };
        rows.push(match between {
            Some((a, b, km)) => format!(
                "  leg {n}: -> #{to} ({secs}s unladen); {a} to {b}, {} AU at the epoch.",
                au_text(km)
            ),
            None => format!("  leg {n}: -> #{to} ({secs}s unladen)"),
        });
        from_place = to_place;
    }
    lines(ctx, rows);
    Ok(())
}

/// Kilometres as astronomical units to three decimals, rounded half up in
/// integer arithmetic (doc 18 cut IC2's `@corridor/show` lines).
fn au_text(km: u64) -> String {
    let milli = (u128::from(km) * 1_000 + u128::from(AU_KM) / 2) / u128::from(AU_KM);
    format!("{}.{:03}", milli / 1_000, milli % 1_000)
}

fn corridor_rules(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.raw().trim().to_string();
    let route = corridor_of(ctx, &raw)?;
    let mut rows = vec![format!(
        "Corridor {}: {}.",
        route.id(),
        if route.is_interdicted() {
            "interdicted"
        } else {
            "no leg is interdicted"
        }
    )];
    for (index, leg) in route.legs().iter().enumerate() {
        rows.push(format!(
            "  leg {}: hazard {}, interdiction {}{}",
            index.saturating_add(1),
            leg.hazard(),
            leg.interdiction(),
            if leg.is_interdicted() {
                " — crossing this boundary raises an interception"
            } else {
                ""
            }
        ));
    }
    lines(ctx, rows);
    Ok(())
}

fn corridor_domain(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.raw().trim().to_string();
    let route = corridor_of(ctx, &raw)?;
    // ---- C5/B6 (independent verify fold; owner decision F8(b)) ----
    //
    // This used to read `SpatialState::domain_of_host(leg.to())` and print
    // *"leg N: #P presents into domain #D"* — the **live** domain's own id, to any
    // player, from a verb behind nothing but `require_enabled` and a `ReadOnly`
    // persistence class. `mush_sim::commands::cmd_domain` opens with `require_admin`
    // on all eleven of its switches, so the existence, id and status of a simulation
    // domain is staff-only everywhere else in the tree, and after R2 a live domain at
    // a corridor endpoint is exactly where an interception has just promoted. The
    // pass's own disclosure test composed no `mush-sim` module, so every leg answered
    // *"presents into no domain"* and the case was never exercised; the same test
    // pinned `!out.contains("domain #0")` as a proxy for "never a sentinel", but
    // `DomainId(0)` is a legitimate **first** domain.
    //
    // What the verb now renders is the **pack-derived** answer to the question its
    // own help text asks — *"the places an interception can actually promote"* —
    // built from the leg's authored `is_interdicted()` and the presence of a domain
    // **host binding** on the endpoint. No live `DomainId` crosses to any reader, and
    // the binding is the same authored fact `@tacsim/seed` and the pack already
    // publish.
    let host = mush_core::BehaviorId::new(DOMAIN_HOST_BEHAVIOR);
    let mut rows = Vec::new();
    for (index, leg) in route.legs().iter().enumerate() {
        let bound = host
            .as_ref()
            .is_ok_and(|id| ctx.w.has_behavior(leg.to(), id));
        let n = index.saturating_add(1);
        let place = leg.to().0;
        rows.push(match (bound, leg.is_interdicted()) {
            (true, true) => {
                format!("  leg {n}: #{place} is a domain host; an interception here can promote")
            }
            (true, false) => format!(
                "  leg {n}: #{place} is a domain host, but this leg is not interdicted; nothing \
                 promotes here"
            ),
            (false, _) => {
                format!("  leg {n}: #{place} hosts no domain; an interception here raises nothing")
            }
        });
    }
    rows.insert(
        0,
        format!("Corridor {}: {} legs.", route.id(), route.legs().len()),
    );
    lines(ctx, rows);
    Ok(())
}

// ---- @tacsim/pack ----------------------------------------------------------

fn pack_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_enabled(ctx)?;
    let arg = ctx.args.raw().trim().to_string();
    let mut words = arg.split_whitespace();
    match words.next().unwrap_or("list") {
        "list" => pack_list(ctx),
        "show" => {
            let named = words.next().unwrap_or_default().to_string();
            let cursor = words.next().unwrap_or_default().to_string();
            pack_show(ctx, &named, &cursor)
        }
        "verify" => {
            let named = words.next().unwrap_or_default().to_string();
            pack_verify(ctx, &named)
        }
        _ => Err(CmdError::msg(
            "@tacsim/pack list | show <id>[@<version>] [<from-entry>] | verify [<id>[@<version>]]",
        )),
    }
}

/// Whether this viewer may see this pack at all.
///
/// A rules pack is the rules and is public; a campaign-seed pack names what a world
/// was built out of and is staff's. Nothing else discriminates.
fn may_view(kind: PackKind, staff: bool) -> bool {
    matches!(kind, PackKind::RulesDefinition) || staff
}

fn pack_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let staff = is_staff(ctx.w, ctx.actor);
    let store = store(ctx)?;
    let mut rows: Vec<String> = Vec::new();
    // Two different budgets, on purpose (B2). The WORK is bounded on records VISITED,
    // not matched: the loop stops after PACK_PAGE candidates whether or not this viewer
    // may see them (D-LBK-5). The MARKER is bounded on records this viewer may SEE, so
    // it never reports the existence of packs the viewer may not know about — a window
    // full of hidden packs is byte-identical to an empty server.
    let mut viewable = 0_usize;
    for (visited, loaded) in store.packs().enumerate() {
        if visited >= PACK_PAGE {
            break;
        }
        let manifest = loaded.pack.manifest();
        if !may_view(manifest.kind(), staff) {
            continue;
        }
        viewable += 1;
        rows.push(format!(
            "{}@{} [{}] {} entries, digest {}",
            manifest.id(),
            manifest.version(),
            crate::pack::kind_word(manifest.kind()),
            loaded.pack.entries().len(),
            &manifest.digest().to_hex()[..12]
        ));
    }
    // The marker fires on a full page of VIEWABLE rows and on nothing else, so it
    // reports only what this viewer has already been shown.
    if viewable >= PACK_PAGE {
        rows.push(format!(
            "(more than {PACK_PAGE} packs are loaded; name one with @tacsim/pack show)"
        ));
    }
    if rows.is_empty() {
        rows.push("No packs are loaded.".to_string());
    }
    lines(ctx, rows);
    Ok(())
}

/// One pack, and **one page** of its entries (independent verify B3).
///
/// The entry loop used to be unpaged, so `@tacsim/pack show <public rules pack>` from
/// any player emitted three header rows plus one message per entry — 515 output events
/// at [`crate::MAX_PACK_ENTRIES`], on a tree where every other listing surface pages and
/// `mush-core`'s `Outbox` has no per-command message cap to fall back on. `cursor` is an
/// **argument**, not a switch: it names the entry id to resume from, which the trailing
/// row prints for you.
fn pack_show(ctx: &mut Ctx<'_>, arg: &str, cursor: &str) -> Result<(), CmdError> {
    let staff = is_staff(ctx.w, ctx.actor);
    let Some((id, version)) = parse_ref(arg) else {
        return Err(CmdError::msg(NO_SUCH_PACK));
    };
    let cursor = cursor.trim();
    let cursor = if cursor.is_empty() {
        None
    } else {
        match CanonicalId::parse_canonical(cursor) {
            Ok(cursor) => Some(cursor),
            // An unreadable cursor is an unreadable reference, and answers the way
            // every other unreadable reference here does.
            Err(_) => return Err(CmdError::msg(NO_SUCH_PACK)),
        }
    };
    let store = store(ctx)?;
    let version = match version.or_else(|| store.newest_loaded(&id)) {
        Some(version) => version,
        None => return Err(CmdError::msg(NO_SUCH_PACK)),
    };
    let Some(loaded) = store.pack(&id, version) else {
        return Err(CmdError::msg(NO_SUCH_PACK));
    };
    let manifest = loaded.pack.manifest();
    if !may_view(manifest.kind(), staff) {
        // Byte-identical to an id that was never published.
        return Err(CmdError::msg(NO_SUCH_PACK));
    }
    let mut rows = vec![
        format!(
            "{}@{} [{}]",
            manifest.id(),
            manifest.version(),
            crate::pack::kind_word(manifest.kind())
        ),
        format!("digest {}", manifest.digest().to_hex()),
    ];
    if manifest.requires().is_empty() {
        rows.push("requires nothing".to_string());
    } else {
        rows.push(format!(
            "requires {}",
            manifest
                .requires()
                .iter()
                .map(|r| format!("{}@{}", r.id, r.version))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let mut rendered = 0_usize;
    let mut remaining = 0_usize;
    let mut resume: Option<&CanonicalId> = None;
    for (entry_id, entry) in loaded.pack.entries() {
        if cursor.as_ref().is_some_and(|from| entry_id < from) {
            continue;
        }
        if rendered >= PACK_PAGE {
            remaining += 1;
            if resume.is_none() {
                resume = Some(entry_id);
            }
            continue;
        }
        rendered += 1;
        let detail = match entry {
            PackEntry::Commodity(def) => {
                let used = if def.in_use { " (in use)" } else { "" };
                format!("{}{used}", def.label)
            }
            PackEntry::Recipe(def) => format!(
                "{} — {} in, {} out",
                def.label,
                def.inputs.len(),
                def.outputs.len()
            ),
            PackEntry::Location(def) => format!("{} [{}]", def.label, def.role),
            // E7 cut 3: the authored **bands** and never the authored reserve.
            // `@tacsim/pack show` is a staff-adjacent catalogue read, but the
            // reserve is exactly the number cut 3's whole fog budget was spent
            // closing, and a pack listing must not be the way around it.
            PackEntry::Deposit(def) => format!(
                "{} at {} — {} (grade band {}, reserve band {})",
                def.label, def.place, def.commodity, def.grade_class, def.reserve_class
            ),
            PackEntry::Route(route) => format!(
                "{} → {}, {} legs, drive {}",
                route.from,
                route.to,
                route.legs.len(),
                route.drive_class
            ),
            // E7 cut 4: the authored capability set and slot cap — catalogue
            // policy, disclosing nothing a viewer may not know.
            PackEntry::Facility(def) => format!(
                "{} at {} — {} slots, {} capabilities",
                def.label,
                def.at,
                def.slots,
                def.capabilities.len()
            ),
            // E7 cut 9: the authored constraint set. `exact` is a DERIVED
            // predicate (`minimum == maximum`), rendered as such rather than
            // stored as a mode.
            PackEntry::Formation(def) => format!(
                "{} — {} echelon {}, {}",
                def.label,
                def.branch,
                def.echelon,
                if def.minimum == def.maximum {
                    format!("exactly {}", def.minimum)
                } else {
                    format!(
                        "{}-{} (prefers {})",
                        def.minimum, def.maximum, def.preferred
                    )
                }
            ),
            // S7 (T17): the sky, as authored — an orbit, a period, a phase.
            PackEntry::Body(def) => format!(
                "body — {:.3} AU, {:.2} days, {:.1}° at the epoch",
                def.orbit_uau as f64 / 1_000_000.0,
                def.period_mdays as f64 / 1_000.0,
                def.phase_mdeg as f64 / 1_000.0
            ),
            // 16b cut C.2: doc 16 §4's six. Each renders the shape a reviewer
            // checks a version bump against, and nothing a listing should not
            // carry — these are rules definitions, so there is no fog to keep.
            // 16b cut X.2: the speed shown is the unit's first declared mode's —
            // its tracks if it has any, its legs otherwise — which is the mode
            // `tracked_rules` and the arena both take.
            PackEntry::Unit(def) => format!(
                "{} — {} sections, {} components, {} cm/s",
                def.label,
                def.sections.len(),
                def.components.len(),
                crate::definitions::tracked_rules(def).max_speed_cms
            ),
            PackEntry::Weapon(def) => format!(
                "{} — {}, {} cm at {} ms, {}",
                def.label,
                def.delivery.as_str(),
                def.max_range_cm(),
                def.cycle_ms,
                def.ammunition
            ),
            PackEntry::Ammunition(def) => format!(
                "{} — {} damage, {} per load",
                def.label, def.damage, def.count_per_load
            ),
            PackEntry::Sensor(def) => format!(
                "{} — {} cells, classifies after {} ms",
                def.label, def.max_range_cells, def.classify_after_ms
            ),
            // 17b cut M3.1: a core is its class and its governed bit; the
            // figures are the class table's.
            PackEntry::Core(def) => format!(
                "{} — class {}, {}",
                def.label,
                def.class.as_str(),
                if def.governed {
                    "governed"
                } else {
                    "ungoverned"
                }
            ),
            PackEntry::Resolution(def) => format!(
                "{} — {} curve points, clamped {}‰ to {}‰",
                def.label,
                def.hit_curve.len(),
                def.p_min,
                def.p_max
            ),
            PackEntry::Arena(def) => format!(
                "{} — {}×{} at {} cm, {} units, {} waypoints",
                def.label,
                def.cols,
                def.rows,
                def.cell_cm,
                def.tanks.len(),
                def.waypoints.nodes.len()
            ),
            // Doc 18 cut IC3: a port is its venue, its place and its institutions.
            PackEntry::Port(def) => format!(
                "port {} at {}, {} orgs",
                def.venue,
                def.location,
                def.orgs.len()
            ),
        };
        rows.push(format!("  {entry_id} ({}) {detail}", entry.label()));
    }
    if let Some(resume) = resume {
        rows.push(format!(
            "({remaining} more entries; @tacsim/pack show {}@{} {resume})",
            manifest.id(),
            manifest.version()
        ));
    }
    lines(ctx, rows);
    Ok(())
}

/// Re-hash the loaded packs and compare them with **the ledger**.
///
/// Two independent findings meet here. **B1:** this loop was gated by `require_enabled`
/// alone and had no page bound at all, so one invocation from any player canonically
/// re-encoded and SHA-256'd every loaded pack — up to `MAX_CONFIGURED_PACKS` ×
/// `MAX_PACK_BYTES` = 32 MiB — synchronously on the world's single writer task, at
/// command rate, emitting one message per pack. It is now **staff-only** (the owner's
/// decision of 2026-09-05) and paged on records **visited**, exactly as `pack_list` is.
/// **P1:** it compared `loaded.pack.computed_digest()` with `manifest.digest()`, which
/// is the equality `load_pack` already proved at boot over the same in-memory value
/// that nothing mutates — a tautology that could only ever print `ok`, paying 32 MiB to
/// prove nothing. The comparison that can actually say something is against the
/// **`tacsim:pack:` ledger row**: the digest this world published that version under.
/// `boot_reconcile` refuses a boot where those disagree, so a `MISMATCH` here is the
/// same fact seen from the other side rather than a second policy.
fn pack_verify(ctx: &mut Ctx<'_>, arg: &str) -> Result<(), CmdError> {
    require_staff(ctx)?;
    let staff = is_staff(ctx.w, ctx.actor);
    let wanted = (!arg.trim().is_empty()).then(|| parse_ref(arg)).flatten();
    if !arg.trim().is_empty() && wanted.is_none() {
        return Err(CmdError::msg(NO_SUCH_PACK));
    }
    let store = store(ctx)?;
    let mut rows: Vec<String> = Vec::new();
    let mut more = false;
    // Re-hash the LOADED bytes and compare them with the digest this world PUBLISHED
    // that version under. It re-reads no file: a file whose declared digest did not
    // match its content was refused at boot, naming the file. Without a ledger row
    // there is nothing but the declaration to compare with, and the answer says so
    // rather than pretending to more.
    let verdict = |loaded: &crate::LoadedPack| -> String {
        let manifest = loaded.pack.manifest();
        let published = store.published_digest(manifest.id(), manifest.version());
        let outcome = match loaded.pack.computed_digest() {
            Ok(computed) => match published {
                Some(row) if row == computed => "ok".to_string(),
                Some(row) => format!(
                    "MISMATCH (published as {}, content hashes to {})",
                    row.to_hex(),
                    computed.to_hex()
                ),
                None if computed == manifest.digest() => "ok (not in the ledger)".to_string(),
                None => format!(
                    "MISMATCH (declared {}, content hashes to {})",
                    manifest.digest().to_hex(),
                    computed.to_hex()
                ),
            },
            Err(error) => format!("UNVERIFIABLE ({error})"),
        };
        format!("{}@{} {outcome}", manifest.id(), manifest.version())
    };
    // A named pack is a **lookup**, not a scan: it is exactly one re-hash, and it stays
    // reachable however many packs a server loads.
    if let Some((id, wanted_version)) = wanted {
        let Some(version) = wanted_version.or_else(|| store.newest_loaded(&id)) else {
            return Err(CmdError::msg(NO_SUCH_PACK));
        };
        let Some(loaded) = store.pack(&id, version) else {
            return Err(CmdError::msg(NO_SUCH_PACK));
        };
        if !may_view(loaded.pack.manifest().kind(), staff) {
            return Err(CmdError::msg(NO_SUCH_PACK));
        }
        let row = verdict(loaded);
        lines(ctx, vec![row]);
        return Ok(());
    }
    let mut matched = false;
    for (visited, loaded) in store.packs().enumerate() {
        if visited >= PACK_PAGE {
            more = true;
            break;
        }
        if !may_view(loaded.pack.manifest().kind(), staff) {
            continue;
        }
        matched = true;
        rows.push(verdict(loaded));
    }
    if !matched {
        return Err(CmdError::msg(NO_SUCH_PACK));
    }
    if more {
        rows.push(format!(
            "(more than {PACK_PAGE} packs are loaded; name one with @tacsim/pack verify <id>)"
        ));
    }
    lines(ctx, rows);
    Ok(())
}

// ---- @tacsim/seed ----------------------------------------------------------

fn refuse_seed(error: &SeedError) -> CmdError {
    match error {
        SeedError::NoSuchPack => CmdError::msg(NO_SUCH_PACK),
        other => CmdError::msg(other.to_string()),
    }
}

fn seed_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_staff(ctx)?;
    require_enabled(ctx)?;
    let arg = ctx.args.raw().trim().to_string();
    let mut words = arg.split_whitespace();
    let named = words.next().unwrap_or_default().to_string();
    let mode = words.next().unwrap_or("dry").to_lowercase();
    let default_corridor = store(ctx)?.config().corridor.clone();
    let target = if named.is_empty() {
        default_corridor
    } else {
        named
    };
    // S9.1: `<pack>[@<version>]` — a named version, else the newest loaded.
    let (name, wanted) = match target.trim().rsplit_once('@') {
        Some((name, version)) => match version.parse::<u32>() {
            Ok(v) => (name.to_string(), Some(v)),
            Err(_) => return Err(CmdError::msg(NO_SUCH_PACK)),
        },
        None => (target.trim().to_string(), None),
    };
    let Ok(pack) = CanonicalId::parse_canonical(&name) else {
        return Err(CmdError::msg(NO_SUCH_PACK));
    };
    let plan = match seed::plan(store(ctx)?, ctx.w, &pack, wanted) {
        Ok(plan) => plan,
        // **The HIT.** This exact `(pack, version)` is already applied, so the honest
        // answer is "done, and nothing was written" — not a refusal. A *different*
        // version is a refusal naming the applied one, and falls through below.
        Err(SeedError::AlreadyApplied { pack, version }) => {
            info(
                ctx,
                format!(
                    "{pack}@{version} has already been applied to this world; nothing was written."
                ),
            );
            return Ok(());
        }
        Err(other) => return Err(refuse_seed(&other)),
    };
    match mode.as_str() {
        "dry" => {
            let mut rows = vec![match plan.update_from {
                Some(from) => format!(
                    "{}@{} would update the corridor applied at @{from}: {} place(s), rooms kept. Nothing has been written.",
                    plan.pack,
                    plan.version,
                    plan.objects.len()
                ),
                None => format!(
                    "{}@{} would create {} object(s). Nothing has been written.",
                    plan.pack,
                    plan.version,
                    plan.objects.len()
                ),
            }];
            for object in &plan.objects {
                rows.push(format!(
                    "  {} — {} [{}]",
                    object.id, object.label, object.role
                ));
            }
            lines(ctx, rows);
            Ok(())
        }
        "commit" => {
            let owner = ctx.actor;
            let outcome = seed::apply(ctx, &plan, owner).map_err(|e| refuse_seed(&e))?;
            // The module's one irreversible command, so its confirmation waits on the
            // durable ack the way every other irreversible staff command on this tree
            // does (independent verify S7; `Ctx::gate_on_durable_ack`). Not strict: the
            // objects and the latch are lost together (the one-batch argument), so
            // re-running after a dropped write is correct and the staffer is better off
            // being told which command may not have survived.
            ctx.gate_on_durable_ack();
            let line = if outcome.through_content {
                format!(
                    "{}@{} applied through the content latch: {} created, {} updated, {} retired{}. \
                     A second commit is a HIT.",
                    outcome.pack,
                    outcome.version,
                    outcome.created,
                    outcome.updated,
                    outcome.retired,
                    outcome
                        .anomaly
                        .as_ref()
                        .map(|a| format!(" (stopped part way: {a})"))
                        .unwrap_or_default()
                )
            } else {
                format!(
                    "{}@{} applied: {} object(s) created, and the latch was written in the same \
                     batch. A second commit is a HIT.",
                    outcome.pack, outcome.version, outcome.created
                )
            };
            info(ctx, line);
            Ok(())
        }
        _ => Err(CmdError::msg(
            "@tacsim/seed <pack>[@<version>] [dry|commit]",
        )),
    }
}

// ---- @tacsim/pin -----------------------------------------------------------

/// The usage line every malformed `@tacsim/pin` answers with.
const PIN_USAGE: &str = "@tacsim/pin <subject>=<pack>@<version> | <subject>= | list";

fn pin_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_staff(ctx)?;
    require_enabled(ctx)?;
    let subject = ctx.args.lhs().trim().to_string();
    let target = ctx.args.rhs().map(|rhs| rhs.trim().to_string());
    // `@tacsim/pin list` — the family was uncapped, undeletable and invisible from the
    // game until B4 (`grep ModuleWrite::Del` over the crate found none), so it now has
    // a way to be read and a way to be drained, both paged like every other listing.
    if target.is_none() && subject.eq_ignore_ascii_case("list") {
        return pin_list(ctx);
    }
    let Some(target) = target else {
        return Err(CmdError::msg(PIN_USAGE));
    };
    if subject.is_empty() {
        return Err(CmdError::msg(PIN_USAGE));
    }
    let Ok(subject) = CanonicalId::parse_canonical(&subject) else {
        return Err(CmdError::msg(PIN_USAGE));
    };
    // `@tacsim/pin <subject>=` — the unpin path, the crate's one `ModuleWrite::Del`.
    if target.is_empty() {
        return pin_clear(ctx, &subject);
    }
    let Some((pack, Some(version))) = parse_ref(&target) else {
        return Err(CmdError::msg(
            "A pin names one exact version: @tacsim/pin <subject>=<pack>@<version>",
        ));
    };
    // A pin to a version this server does not load would be a pin to nothing, which
    // is worse than no pin: resolution would refuse for ever with no way to tell
    // whether the operator meant it. Refuse at the command instead.
    //
    // And the pinned pack must **declare the subject** (B4/P4). `resolve` gives a pin
    // absolute precedence, ahead of the `candidates` scan that would otherwise have
    // answered `Unknown`, so a typo'd or stale subject name became a durable pin
    // answering `Ok` for a definition no pack declares — contradicting the store's own
    // "a pin wins and is exact" and handing cut 2 onward a resolvable id with nothing
    // behind it. The refusal reuses the existing existence-hiding message, so no new
    // oracle appears.
    let store = store(ctx)?;
    let Some(loaded) = store.pack(&pack, version) else {
        return Err(CmdError::msg(NO_SUCH_PACK));
    };
    if loaded.pack.entry(&subject).is_none() {
        return Err(CmdError::msg(NO_SUCH_PACK));
    }
    // Bounded like every other document family in the programme (10b §7). Re-pinning a
    // subject that already has a pin replaces it and is never past the cap.
    if store.pin(&subject).is_none() && store.pins().count() >= MAX_PINS {
        return Err(CmdError::msg(format!(
            "This world already carries {MAX_PINS} pins, which is the maximum; remove one with \
             @tacsim/pin <subject>= first."
        )));
    }
    let pinned_at = ctx.world_ic_now();
    let record = match PinRecord::new(subject.clone(), pack.clone(), version, pinned_at) {
        Ok(record) => record,
        Err(error) => return Err(CmdError::msg(error.to_string())),
    };
    put_pin(ctx, record);
    persist_meta(ctx);
    info(
        ctx,
        format!(
            "{subject} is pinned to {pack}@{version}; publishing a newer version will not move it."
        ),
    );
    Ok(())
}

/// Every pin this world carries, paged on records **visited**.
fn pin_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let store = store(ctx)?;
    let mut rows: Vec<String> = Vec::new();
    let mut more = false;
    for (visited, pin) in store.pins().enumerate() {
        if visited >= PACK_PAGE {
            more = true;
            break;
        }
        rows.push(format!(
            "{} -> {}@{}",
            pin.subject(),
            pin.pack(),
            pin.version()
        ));
    }
    if rows.is_empty() {
        rows.push("No subjects are pinned.".to_string());
    } else if more {
        rows.push(format!(
            "(more than {PACK_PAGE} subjects are pinned; the cap is {MAX_PINS})"
        ));
    }
    lines(ctx, rows);
    Ok(())
}

/// Remove one subject's pin.
///
/// Answering "there was no such pin" out loud is right here and is not an oracle: this
/// switch is staff-only, and a pin is this world's own bookkeeping rather than a fact
/// about a pack.
fn pin_clear(ctx: &mut Ctx<'_>, subject: &CanonicalId) -> Result<(), CmdError> {
    if store(ctx)?.pin(subject).is_none() {
        return Err(CmdError::msg(format!("{subject} is not pinned.")));
    }
    del_pin(ctx, subject);
    persist_meta(ctx);
    info(
        ctx,
        format!("{subject} is no longer pinned; it resolves by the ordinary rule again."),
    );
    Ok(())
}
