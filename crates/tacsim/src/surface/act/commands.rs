//! Surface excursion commands (`+tv/launch`, `+tv/recover`, `+tv/dump`, `+tv/fill`, `+tv/scan`, `+tv/sound`, `+tv/drive`, `+tv/mine`, `+tv/unload`, `+tv/plan`).

use super::*;

// ---- the command: preview and go ------------------------------------------------------------

/// The typed record a command reads: `ctx.action_state` on the family, absent as fresh
/// ([`RigRecord::fresh_unread`] when the battery cannot be read, the start's rule).
pub(super) fn record_of(
    ctx: &Ctx<'_>,
    handles: &PortHandles,
    tv: ObjId,
    battery: Option<&BatteryDef>,
) -> Result<RigRecord, ()> {
    match ctx.action_state(&handles.families.excursions, &RigKey(tv)) {
        Ok(Some(record)) => Ok(record.clone()),
        Ok(None) => Ok(battery.map_or_else(
            || RigRecord::fresh_unread(tv),
            |battery| RigRecord::fresh(tv, battery),
        )),
        Err(error) => {
            tracing::warn!(tv = tv.0, %error, "tacsim: a rig's excursion record does not read");
            Err(())
        }
    }
}

/// **The goods a rig's dump would move now**: each of the hopper's accepted goods' pool's
/// **available** quantity (`crate::port_tick::read_available`: the balance less every live
/// hold) — the very figure the start's prepare freezes, so the preview names exactly what `go`
/// commits (D-20-9; the SM.3b fixer round's item 9).
pub(super) fn hopper_contents(
    ctx: &Ctx<'_>,
    tv: ObjId,
) -> Result<BTreeMap<CanonicalId, u64>, String> {
    let vault = crate::port::vehicle::hopper_of(ctx.w, tv)
        .ok_or_else(|| format!("rig #{} has no hopper vault", tv.0))?;
    let (accepted, _) = tv::hopper_goods_of(ctx.w, tv)
        .ok_or_else(|| format!("rig #{}'s unit cannot be read", tv.0))?;
    let mut out = BTreeMap::new();
    for good in accepted {
        let qty = crate::port_tick::read_available(ctx, EndpointRef(vault), &good)
            .map_err(|error| format!("rig #{}'s hopper {good}: {error:?}", tv.0))?;
        if qty > 0 {
            out.insert(good, qty);
        }
    }
    Ok(out)
}

/// `{load}`: `"{q} t of {good label}"` per good, joined as a list ([`and_list`]: `a`, `a and b`,
/// `a, b and c`). The SM.8 lead's pre-commit round, item 5: goods v3's labels carry commas
/// ("ice ore, run-of-mine"), so a plain `", "` join read two ores as four words.
#[must_use]
pub(crate) fn load_words(w: &World, goods: &BTreeMap<CanonicalId, u64>) -> String {
    let table = w.module_state::<TacsimStore>().and_then(TacsimStore::goods);
    let words: Vec<String> = goods
        .iter()
        .map(|(good, qty)| {
            let label = table
                .and_then(|table| table.good(good))
                .map_or_else(|| good.to_string(), |def| def.label.clone());
            format!("{qty} t of {label}")
        })
        .collect();
    and_list(&words)
}

/// `a`, `a and b`, `a, b and c` — the goods a load line names (the cockpit's `join_names` shape);
/// empty for none.
pub(super) fn and_list(words: &[String]) -> String {
    match words {
        [] => String::new(),
        [one] => one.clone(),
        [head @ .., last] => format!("{} and {last}", head.join(", ")),
    }
}

/// **The rig's battery row** (finding F5: the one tools table), whether or not the act needs it.
pub(super) fn rig_battery(w: &World, handles: &PortHandles, rig: &RigAt) -> Option<BatteryDef> {
    handles
        .surface_tools
        .as_ref()
        .and_then(|tools| rig_battery_id(w, rig.tv).and_then(|id| battery_of(tools, &id).cloned()))
}

/// **A cold site or home asked for** (doc 20 cut SM.4a; since SM.4b the home field too): the
/// surface tick warms them, never this path (RAM only, the `RigPointer` precedent). Doc 20 cut
/// SM.12b: the dispatcher's filing asks through it too.
pub(crate) fn want_cold(ctx: &mut Ctx<'_>, rig: &RigAt, refusal: &Refusal) {
    if let Refusal::GroundCold { entry, .. } = refusal {
        crate::surface::want_site_ground(ctx.w, entry);
        if let Some((site, ramp)) = crate::surface::hull_home(ctx.w, rig.hull) {
            crate::surface::want_home(ctx.w, &site, ramp);
        }
    }
}

/// **A live plan holds its rig** (doc 20 cut SM.11; the brief's 3.5, D8): `PLAN_LIVE` when the
/// rig's record holds a plan live for the actor — every hand start's command asks it first, before
/// the record's busy refusal, and says nothing else. A record that will not read answers nothing
/// here (the command's own read darkens it).
pub(super) fn plan_holds(ctx: &Ctx<'_>, rig: &RigAt) -> Result<(), CmdError> {
    let Some(handles) = ctx.w.module_state::<PortHandles>() else {
        return Ok(());
    };
    let Ok(Some(record)) = ctx.action_state(&handles.families.excursions, &RigKey(rig.tv)) else {
        return Ok(());
    };
    match record.live_plan_for(ctx.actor) {
        Some(plan) => Err(CmdError::msg(
            Refusal::PlanLive {
                n: rig.slot,
                no: plan.no,
            }
            .line(),
        )),
        None => Ok(()),
    }
}

/// **The record's busy refusal** (`RIG_BUSY`): an act in flight.
pub(super) fn busy_refusal(w: &World, rig: &RigAt, record: &RigRecord) -> Option<Refusal> {
    let act = record.act.as_ref()?;
    let ramp = ramp_label(w, rig.hull);
    Some(Refusal::Busy {
        n: rig.slot,
        doing: doing_words(act.kind, &act.params, &ramp),
        when: tv::due_words(w, act.due_ic),
    })
}

/// **The record's run refusal** (doc 20 cut SM.7; decision R1): a run in the cycle slot holds the
/// rig to its pocket — `MINE_RUNNING`, naming the pocket, the cycle and its due.
pub(crate) fn run_refusal(w: &World, rig: &RigAt, record: &RigRecord) -> Option<Refusal> {
    let run = record.run.as_ref()?;
    Some(Refusal::MineRunning {
        n: rig.slot,
        addr: address::pocket_address(run.pocket[0], run.pocket[1]).unwrap_or_default(),
        k: run.k,
        cycles: run.cycles,
        when: tv::due_words(w, run.due_ic),
    })
}

/// **The launch half's gates** (a launch, or a scan, a sounding or a drive that implies one;
/// lead ruling 2): an excursion still open (`LAUNCH_OPEN`), then the caps (§8; lead ruling 8),
/// counted over the world's rig records.
pub(super) fn launch_half(
    ctx: &Ctx<'_>,
    handles: &PortHandles,
    rig: &RigAt,
    record: &RigRecord,
    site: &CanonicalId,
    dark: &dyn Fn() -> CmdError,
) -> Result<(), CmdError> {
    if record.excursion.is_some() {
        return Err(CmdError::msg(Refusal::Open { n: rig.slot }.line()));
    }
    let w: &World = ctx.w;
    let mut counts = ExcursionCounts::default();
    for tv in world_rigs(w) {
        let found = ctx
            .action_state(&handles.families.excursions, &RigKey(tv))
            .map_err(|_| dark())?;
        if let Some(excursion) = found.and_then(|record| record.excursion.as_ref()) {
            counts.world = counts.world.saturating_add(1);
            if &excursion.site == site {
                counts.site = counts.site.saturating_add(1);
            }
            if excursion.operator == ctx.actor {
                counts.operator = counts.operator.saturating_add(1);
            }
        }
    }
    if let Some(cap) = cap_refusal(&counts) {
        return Err(CmdError::msg(
            Refusal::Cap {
                cap,
                site: site_name(w, site),
            }
            .line(),
        ));
    }
    Ok(())
}

/// **The reserve rule's words** (§4.1; lead ruling 3, enforced again in the start's prepare):
/// `(now %, after the act %, after the trip home %)` from the charge the act spends from (an
/// implied launch: the bay credited no further than the launch half, D22), or the `RESERVE_LOW`
/// line.
pub(super) fn reserve_words(
    record: &RigRecord,
    now_ic: i64,
    launches: bool,
    (cost_cu, home_cu): (u32, u32),
    battery: &BatteryDef,
    n: u8,
) -> Result<(u32, u32, u32), CmdError> {
    let shown = charge_at(&record.charge, now_ic, battery);
    let from = charge_for_act(&record.charge, now_ic, launches, battery);
    match reserve_check(from, reserved_total(record), cost_cu, home_cu, battery) {
        Ok(after) => Ok((
            percent(shown, battery),
            percent(after.after_act, battery),
            percent(after.after_home, battery),
        )),
        Err(short) => {
            let after = u32::try_from(short.after_home.max(0)).unwrap_or(0);
            Err(CmdError::msg(
                Refusal::Reserve {
                    n,
                    now: percent(shown, battery),
                    after: percent(after, battery),
                    floor: percent(short.floor, battery),
                }
                .line(),
            ))
        }
    }
}

/// Whole percent of a battery, rounded up (a leg's need: never under what it costs).
pub(super) fn percent_up(cu: u32, battery: &BatteryDef) -> u32 {
    if battery.capacity_cu == 0 {
        return 0;
    }
    u32::try_from((u64::from(cu).saturating_mul(100)).div_ceil(u64::from(battery.capacity_cu)))
        .unwrap_or(u32::MAX)
}

/// **Whether a rig out on the ground is parked short of its way home** (doc 23 cut S.1, the
/// lead's L6: the durable facts, never the `tacsim:tv` projection): nothing in flight and the
/// rig on the ground, and either its plan ended `Stranded` with no act since (the record), or
/// the idle return's own test fails as its operator — the recover's way home, from where the
/// rig stands, is short of the record's charge (`RECOVER_SHORT`) or there is none
/// (`RECOVER_FAR`), exactly the two refusals the idle return parks a rig on
/// (`crate::surface::tick`). Any other refusal — the operator ashore, the ground cold — reads
/// not parked. A staff home of a parked rig closes its epoch `RecoveredToBase`.
pub(crate) fn parked_short(
    w: &World,
    record: &RigRecord,
    operator: ObjId,
    battery: Option<&BatteryDef>,
) -> bool {
    if record.act.is_some() || record.run.is_some() || record.queued.is_some() {
        return false;
    }
    if !matches!(where_is(w, record.tv), Where::Ground { .. }) {
        return false;
    }
    let stranded = record.plan.as_ref().is_some_and(|plan| {
        matches!(
            plan.state,
            FillState::Ended { why: PlanEnd::Stranded, at_ic }
                if record.last.as_ref().is_none_or(|last| at_ic >= last.at_ic)
        )
    });
    if stranded {
        return true;
    }
    let Some(lease) = crate::port::yard::rig_lease(w, record.tv) else {
        return false;
    };
    let rig = RigAt {
        tv: record.tv,
        slot: lease.slot,
        mark: lease.mark.clone(),
        hull: lease.hull,
        record: lease,
    };
    let rates = rig_rates(w, record.tv, battery);
    let mut peeks = Peeks::default();
    let ramp = ramp_label(w, rig.hull);
    matches!(
        recover_plan(w, operator, &rig, &mut peeks, rates)
            .and_then(|plan| leg_home(record, &plan, battery, &rig, &ramp)),
        Err(Refusal::LegShort { .. } | Refusal::Far { .. })
    )
}

/// **A recover's leg home, affordable or `RECOVER_SHORT`** (doc 20 cut SM.4b; lead ruling 5,
/// decision P6): `leg_affordable` on the record's own value — never the 5 % floor, never the
/// battery's row; the battery reads only the words (`?` without it). **SM.11: its idle
/// return-and-park must treat this refusal as "parked where it stands".**
pub(crate) fn leg_home(
    record: &RigRecord,
    plan: &ActPlan,
    battery: Option<&BatteryDef>,
    rig: &RigAt,
    ramp: &str,
) -> Result<(), Refusal> {
    if leg_affordable(&record.charge, reserved_total(record), plan.cost_cu) {
        return Ok(());
    }
    let words = |cu: u32, up: bool| {
        battery.map_or_else(
            || "?".to_owned(),
            |battery| {
                if up {
                    percent_up(cu, battery).to_string()
                } else {
                    percent(cu, battery).to_string()
                }
            },
        )
    };
    Err(Refusal::LegShort {
        n: rig.slot,
        now: words(record.charge.value_cu, false),
        need: words(plan.cost_cu, true),
        ramp: ramp.to_owned(),
    })
}

/// **`+tv/<n> <act> [go]`** (the brief's 3.6; Q-20-8, D-20-9's `go` rule): the act's planner,
/// then the typed record (unreadable → `TV_DARK`), mirroring every refusal of the prepare it
/// can read in the prepare's order (finding F10: `RIG_BUSY`, `LAUNCH_OPEN`,
/// `RECOVER_IN_BAY`, the battery, the launch's caps, the reserve, the dump's hopper). Bare,
/// the preview line — nothing written, nothing queued. With `go`, ONE
/// `tacsim_surface_act` submission as the actor and the filed line; the handler writes
/// nothing else (the projection is the tick's).
///
/// **Doc 20 cut SM.4a**: a scan or a sounding (`quiet` for `sound quiet`) plans through
/// `scan_plan`/`sound_plan` — the site's composite peeked once for the call, a cold one
/// answering `GROUND_COLD` and asking for its warm (RAM only, the `RigPointer` precedent) —
/// then, when it implies the launch, the launch's open-excursion refusal and its caps; then the
/// operator's chart: "You learn nothing new here." (the same-cell rule, D4) and the body caps
/// (`CHART_FULL`); then the reserve from `charge_for_act` (the bay credited no further than the
/// launch half, D22). No licence and no tenure is read (lead ruling 9).
///
/// **Doc 20 cut SM.4b**: a scan's and a sounding's reserve carries the real trip home from the
/// stand (`home_here`); a recover from farther than beside the ramp drives home first — one act
/// whose leg is checked by `leg_affordable` (`RECOVER_SHORT`), never by the 5 % floor, and
/// previewed `RECOVER_DRIVE_PREVIEW`. A drive is `drive_cmd`'s.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line.
#[allow(clippy::too_many_lines)] // one ordered refusal list, then the preview or the go
pub(crate) fn act_cmd(
    ctx: &mut Ctx<'_>,
    rig: &RigAt,
    kind: ActKind,
    quiet: bool,
    go: bool,
) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let handles = ctx
        .w
        .module_state::<PortHandles>()
        .cloned()
        .ok_or_else(|| CmdError::msg(tv::TV_UNWIRED))?;
    let Some(action) = handles.surface_act.clone() else {
        return Err(CmdError::msg(tv::TV_UNWIRED));
    };
    // Doc 20 cut SM.11 (D8): a live plan holds its rig.
    plan_holds(ctx, rig)?;
    let name = name_of(ctx.w, rig.hull);
    let dark = || CmdError::msg(tv::TV_DARK.replace("{name}", &name));
    // 1. The world plan (the start's offer runs the same function). A reading act needs the
    // rig's scanner; the site's composite and its home field are peeked once for the whole call.
    let scanner = handles.surface_tools.as_ref().and_then(|tools| {
        rig_scanner_id(ctx.w, rig.tv).and_then(|id| scanner_of(tools, &id).copied())
    });
    let battery = rig_battery(ctx.w, &handles, rig);
    let rates = rig_rates(ctx.w, rig.tv, battery.as_ref());
    let mut peeks = Peeks::default();
    let planned = match kind {
        ActKind::Launch => launch_plan(ctx.w, actor, rig),
        ActKind::Recover => recover_plan(ctx.w, actor, rig, &mut peeks, rates),
        ActKind::Dump => dump_plan(ctx.w, actor, rig),
        ActKind::Scan | ActKind::Sound => {
            let Some(scanner) = scanner.as_ref() else {
                tracing::warn!(tv = rig.tv.0, "tacsim: a rig carries no readable scanner");
                return Err(dark());
            };
            if kind == ActKind::Scan {
                scan_plan(ctx.w, actor, rig, scanner, &mut peeks, rates.as_ref())
            } else {
                sound_plan(
                    ctx.w,
                    actor,
                    rig,
                    scanner,
                    quiet,
                    &mut peeks,
                    rates.as_ref(),
                )
            }
        }
        // A drive names its target: `drive_cmd`; a run is `mine_cmd`'s.
        ActKind::Drive | ActKind::Mine | ActKind::Unload => return Err(dark()),
    };
    let plan = match planned {
        Ok(plan) => plan,
        Err(refusal) => {
            // A cold site or home is asked for: the surface tick warms it, never this path.
            want_cold(ctx, rig, &refusal);
            return Err(CmdError::msg(refusal.line()));
        }
    };
    let w: &World = ctx.w;
    // 2. The battery, read only where the act needs it — a spend or a launch
    // (`needs_battery`); a recover or a dump never does, so a rig whose battery row stops
    // resolving still comes home (§4.1; the fixer round's item 3; since doc 20 cut SM.4b a
    // recover with a leg too).
    if battery.is_none() && needs_battery(kind, plan.cost_cu, plan.home_cu) {
        return Err(dark());
    }
    // 3. The typed record, and every refusal of the prepare it can read, in its order.
    let record = record_of(ctx, &handles, rig.tv, battery.as_ref()).map_err(|()| dark())?;
    let ramp = ramp_label(w, rig.hull);
    if let Some(busy) = busy_refusal(w, rig, &record) {
        return Err(CmdError::msg(busy.line()));
    }
    // Doc 20 cut SM.7 (decision R1): while a run drills, only a scan or a sounding may start.
    if !matches!(kind, ActKind::Scan | ActKind::Sound)
        && let Some(running) = run_refusal(w, rig, &record)
    {
        return Err(CmdError::msg(running.line()));
    }
    // The launch's half: a launch, or a scan or a sounding that implies one (lead ruling 2).
    let launches = kind == ActKind::Launch || plan.params.launches();
    if kind == ActKind::Recover && plan.from_bay && record.excursion.is_none() {
        return Err(CmdError::msg(Refusal::InBay { n: rig.slot }.line()));
    }
    if launches {
        launch_half(ctx, &handles, rig, &record, &plan.site, &dark)?;
    }
    // 4. The chart (doc 20 cut SM.4a): the same-cell rule, then the body caps.
    if let Some(reach) = Reach::of(&plan.params) {
        let read = |site: &CanonicalId| -> Result<Option<ChartRecord>, CmdError> {
            ctx.action_state(
                &handles.families.charts,
                &ChartKey {
                    principal: actor,
                    site: site.clone(),
                },
            )
            .map(|found| found.cloned())
            .map_err(|error| {
                tracing::warn!(tv = rig.tv.0, %error, "tacsim: a chart row does not read");
                dark()
            })
        };
        let row = read(&plan.site)?;
        if reach.refused(row.as_ref()) {
            return Err(CmdError::msg(Refusal::NothingNew.line()));
        }
        let body = match &plan.params {
            ActParams::Scan { body, .. } | ActParams::Sound { body, .. } => body.clone(),
            ActParams::None {}
            | ActParams::Dump { .. }
            | ActParams::Drive { .. }
            | ActParams::Unload { .. } => {
                return Err(dark());
            }
        };
        let mut rows = Vec::new();
        for site in chart::body_sites(w, &body) {
            if let Some(found) = read(&site)? {
                rows.push(found);
            }
        }
        let (fields, blocks) = reach.units();
        if !body_admits(&rows, &plan.site, &fields, &blocks) {
            return Err(CmdError::msg(
                Refusal::ChartFull {
                    body: crate::port::site::body_name(&body),
                }
                .line(),
            ));
        }
    }
    let now_ic = w.world_ic_now().at;
    // `(now %, after %)`, `None` with no battery (a zero spend: the rule passes it unread). An
    // implied launch spends from the charge the bay holds when the launch half ends (D22). Doc
    // 20 cut SM.4b: a recover's leg home is checked by `leg_affordable`, never the floor.
    let homeward = kind == ActKind::Recover && plan.params.leg().is_some();
    let charge = if homeward {
        leg_home(&record, &plan, battery.as_ref(), rig, &ramp)
            .map_err(|refusal| CmdError::msg(refusal.line()))?;
        battery.as_ref().map(|battery| {
            let shown = charge_at(&record.charge, now_ic, battery);
            (
                percent(shown, battery),
                percent(shown.saturating_sub(plan.cost_cu), battery),
            )
        })
    } else {
        match &battery {
            Some(battery) => {
                let (now, _, after) = reserve_words(
                    &record,
                    now_ic,
                    plan.params.launches(),
                    (plan.cost_cu, plan.home_cu),
                    battery,
                    rig.slot,
                )?;
                Some((now, after))
            }
            None => None,
        }
    };
    let load = if kind == ActKind::Dump {
        let goods = hopper_contents(ctx, rig.tv).map_err(|error| {
            tracing::warn!(tv = rig.tv.0, %error, "tacsim: a dump could not read the hopper");
            dark()
        })?;
        if goods.is_empty() {
            return Err(CmdError::msg(Refusal::Empty { n: rig.slot }.line()));
        }
        load_words(w, &goods)
    } else {
        String::new()
    };
    let n = rig.slot.to_string();
    let site = site_name(w, &plan.site);
    let real = match crate::port::voyage::real_until(w, now_ic.saturating_add(i64::from(plan.secs)))
    {
        Some(real) => format!(", about {}", tv::span_words(real)),
        None => String::new(),
    };
    if !go {
        let line = match kind {
            ActKind::Launch => {
                // `needs_battery` holds for every launch, so `charge` is `Some` here.
                let Some((now, after)) = charge else {
                    return Err(dark());
                };
                tv::LAUNCH_PREVIEW
                    .replace("{n}", &n)
                    .replace("{mark}", &rig.mark)
                    .replace("{ramp}", &ramp)
                    .replace("{site}", &site)
                    .replace("{secs}", &plan.secs.to_string())
                    .replace("{real}", &real)
                    .replace("{now}", &now.to_string())
                    .replace("{after}", &after.to_string())
            }
            ActKind::Recover => {
                let hopper = match tv::hopper_load_of(ctx, rig.tv) {
                    Ok((kg, cap)) => tv::hopper_text(kg, cap),
                    Err(error) => {
                        tracing::warn!(tv = rig.tv.0, %error, "tacsim: a recover could not read the hopper");
                        return Err(dark());
                    }
                };
                let (preview, unread) = match &plan.params {
                    ActParams::Drive { .. } => {
                        (tv::RECOVER_DRIVE_PREVIEW, tv::RECOVER_DRIVE_PREVIEW_UNREAD)
                    }
                    _ => (tv::RECOVER_PREVIEW, tv::RECOVER_PREVIEW_UNREAD),
                };
                let (metres, drive_s) = match &plan.params {
                    ActParams::Drive { metres, secs, .. } => (*metres, *secs),
                    _ => (0, 0),
                };
                match charge {
                    Some((now, after)) => preview
                        .replace("{now}", &now.to_string())
                        .replace("{after}", &after.to_string()),
                    None => unread.to_owned(),
                }
                .replace("{n}", &n)
                .replace("{mark}", &rig.mark)
                .replace("{ramp}", &ramp)
                .replace("{name}", &name)
                .replace("{secs}", &plan.secs.to_string())
                .replace("{m}", &metres.to_string())
                .replace("{drive}", &drive_s.to_string())
                .replace("{climb}", &RECOVER_S.to_string())
                .replace("{real}", &real)
                .replace("{hopper}", &hopper)
                // Doc 20 cut SM.8 (lead ruling 1): the unload the recover's `Done` will queue.
                .replace("{then}", &recover_then(ctx, rig))
            }
            ActKind::Dump => tv::DUMP_PREVIEW
                .replace("{n}", &n)
                .replace("{mark}", &rig.mark)
                .replace("{load}", &load)
                .replace("{site}", &site),
            ActKind::Scan | ActKind::Sound => {
                // A scan or a sounding spends, so `charge` is `Some` here.
                let Some((now, after)) = charge else {
                    return Err(dark());
                };
                let Some(scanner) = scanner.as_ref() else {
                    return Err(dark());
                };
                reading_preview(
                    w,
                    &plan,
                    scanner,
                    &mut peeks,
                    PreviewWords {
                        n: &n,
                        mark: &rig.mark,
                        ramp: &ramp,
                        site: &site,
                        real: &real,
                        now,
                        after,
                    },
                )
                .ok_or_else(dark)?
            }
            ActKind::Drive | ActKind::Mine | ActKind::Unload => return Err(dark()),
        };
        crate::port::coop::say(ctx, line);
        return Ok(());
    }
    // `go`: ONE submission as the actor, keyed `tacsim.surface/{tv}/{ordinal}/act`.
    let Some(ordinal) = record.ordinal.checked_add(1) else {
        tracing::error!(tv = rig.tv.0, "tacsim: a rig's act ordinal is exhausted");
        return Err(dark());
    };
    let Some(key) = act_key(rig.tv, ordinal) else {
        tracing::error!(tv = rig.tv.0, "tacsim: a rig act's key did not form");
        return Err(dark());
    };
    let due = now_ic.saturating_add(i64::from(plan.secs));
    let when = tv::due_words(ctx.w, due);
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({
            "tv": rig.tv.0.to_string(),
            "act": wire_word(kind, quiet),
            "ordinal": ordinal,
        }),
        key,
    );
    let line = match kind {
        ActKind::Launch => tv::LAUNCH_FILED
            .replace("{n}", &n)
            .replace("{ramp}", &ramp)
            .replace("{when}", &when),
        ActKind::Recover if homeward => tv::RECOVER_DRIVE_FILED
            .replace("{n}", &n)
            .replace("{ramp}", &ramp)
            .replace("{when}", &when),
        ActKind::Recover => tv::RECOVER_FILED
            .replace("{n}", &n)
            .replace("{ramp}", &ramp)
            .replace("{when}", &when),
        ActKind::Dump => tv::DUMP_FILED.replace("{n}", &n),
        ActKind::Scan | ActKind::Sound => {
            let template = match (kind, plan.from_bay) {
                (ActKind::Scan, false) => tv::SCAN_FILED,
                (ActKind::Scan, true) => tv::SCAN_FILED_LAUNCH,
                (_, false) => tv::SOUND_FILED,
                (_, true) => tv::SOUND_FILED_LAUNCH,
            };
            template
                .replace("{n}", &n)
                .replace("{ramp}", &ramp)
                .replace("{when}", &when)
        }
        ActKind::Drive | ActKind::Mine | ActKind::Unload => return Err(dark()),
    };
    crate::port::coop::say(ctx, line);
    Ok(())
}

// ---- doc 20 cut SM.8: `+tv/unload` and the recover's unload clause -----------------------------

/// **An unload's split against its room** (doc 20 cut SM.8; the brief's 3.4): the whole unload
/// (`qty` `None`) or a part one, through [`crate::surface::unload`]; `Err(())` when the capacity
/// function refuses the question (a good the plan's table does not weigh — never expected).
pub(super) fn unload_split(
    room: &UnloadRoom,
    hopper: &BTreeMap<CanonicalId, u64>,
    qty: Option<u64>,
) -> Result<Result<crate::surface::unload::Split, Short>, ()> {
    match qty {
        None => crate::surface::unload::whole(room.goods, &room.compartment, hopper, &room.room)
            .map(Ok)
            .map_err(|_| ()),
        Some(qty) => {
            crate::surface::unload::part(room.goods, &room.compartment, hopper, &room.room, qty)
                .map_err(|_| ())
        }
    }
}

/// **The hold's unreserved mass after `moves`**, whole tonnes floored (the previews' `{room}`).
pub(super) fn room_after_t(room: &UnloadRoom, moves: &BTreeMap<CanonicalId, u64>) -> Option<u64> {
    let mut after = room.room.clone();
    for (good, qty) in moves {
        crate::port::capacity::take_room(room.goods, &room.compartment, &mut after, good, *qty)
            .ok()?;
    }
    let mass = CanonicalId::new(crate::definitions::logistics::MASS_KG).ok()?;
    Some(after.get(&mass).copied().unwrap_or(0) / 1_000)
}

/// **A recover's `{then}`** (doc 20 §1 beat 9; cut SM.8 lead rulings 1 and 3): empty for an empty
/// hopper; else the unload the recover's `Done` will queue, split against the hold's room now —
/// `RECOVER_THEN` (all of it, and the room after), `RECOVER_THEN_PART` (what fits and what
/// stays) or `RECOVER_THEN_NONE` (none of it: the dump named). A hopper or a room that will not
/// read leaves it empty, logged: a recover is never refused for it (the way home is never barred).
pub(super) fn recover_then(ctx: &Ctx<'_>, rig: &RigAt) -> String {
    let w: &World = ctx.w;
    let hopper = match hopper_contents(ctx, rig.tv) {
        Ok(hopper) => hopper,
        Err(error) => {
            tracing::warn!(tv = rig.tv.0, %error, "tacsim: a recover's preview could not read the hopper");
            return String::new();
        }
    };
    if hopper.is_empty() {
        return String::new();
    }
    let Some(view) = hull_view(w, rig.hull) else {
        return String::new();
    };
    let read_pool = |endpoint: EndpointRef, good: &CanonicalId| {
        crate::port_tick::read_pool(ctx, endpoint, good)
    };
    let read_rig = |tv: ObjId| SurfaceReads::rig(ctx, tv);
    let Some(room) = unload_room(w, &view, rig.tv, &read_pool, &read_rig) else {
        tracing::warn!(
            tv = rig.tv.0,
            "tacsim: a recover's preview could not read the hold's room"
        );
        return String::new();
    };
    let Ok(Ok(split)) = unload_split(&room, &hopper, None) else {
        return String::new();
    };
    if split.moves.is_empty() {
        tv::RECOVER_THEN_NONE
            .replace("{stays}", &load_words(w, &split.stays))
            .replace("{n}", &rig.slot.to_string())
    } else if split.stays.is_empty() {
        tv::RECOVER_THEN
            .replace("{load}", &load_words(w, &split.moves))
            .replace(
                "{room}",
                &room_after_t(&room, &split.moves).unwrap_or(0).to_string(),
            )
    } else {
        tv::RECOVER_THEN_PART
            .replace("{load}", &load_words(w, &split.moves))
            .replace("{stays}", &load_words(w, &split.stays))
    }
}

/// **`+tv/<n> unload [<t>] [go]`** (doc 20 §1 beat 9, §7.1, §10; cut SM.8, the brief's 3.5; lead
/// rulings 2 and 3, decisions R4, R10, R18): [`unload_plan`] (`queued` false: the hand unload
/// reads aboard); the typed record (unreadable → `TV_DARK`); an act in flight → `RIG_BUSY`; a
/// run → `MINE_RUNNING`; the hopper's available goods (unreadable → `TV_DARK`; empty →
/// `DUMP_EMPTY`); the hold's room ([`unload_room`]; `None` → `TV_DARK`, logged); then the split:
/// bare, every good up to the room (none fits → `UNLOAD_NO_ROOM`); `<t>`, that much of the first
/// good, past the fit `UNLOAD_NO_FIT` naming it — a fit of 0 is `UNLOAD_NO_ROOM`, so the suggested
/// command always parses. Bare, the preview (`UNLOAD_PREVIEW`), nothing written; with `go`, the
/// order written ([`ActOrders`]: `ActOrder::Unload`), then ONE `tacsim_surface_act` with `act:
/// unload` and `how` (`whole`, or `part` with `qty`) keyed `tacsim.surface/{tv}/{ordinal}/act`,
/// and `UNLOAD_FILED`. No licence, tenure, lease or battery is read.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line.
#[allow(clippy::too_many_lines)] // one ordered refusal list, then the preview or the go
pub(crate) fn unload_cmd(
    ctx: &mut Ctx<'_>,
    rig: &RigAt,
    qty: Option<u64>,
    go: bool,
) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let handles = ctx
        .w
        .module_state::<PortHandles>()
        .cloned()
        .ok_or_else(|| CmdError::msg(tv::TV_UNWIRED))?;
    let Some(action) = handles.surface_act.clone() else {
        return Err(CmdError::msg(tv::TV_UNWIRED));
    };
    // Doc 20 cut SM.11 (D8): a live plan holds its rig.
    plan_holds(ctx, rig)?;
    let name = name_of(ctx.w, rig.hull);
    let dark = || CmdError::msg(tv::TV_DARK.replace("{name}", &name));
    // 1. The world plan (the start's offer runs the same function).
    let plan =
        unload_plan(ctx.w, actor, rig, false).map_err(|refusal| CmdError::msg(refusal.line()))?;
    // 2. The record: busy, then a run.
    let battery = rig_battery(ctx.w, &handles, rig);
    let record = record_of(ctx, &handles, rig.tv, battery.as_ref()).map_err(|()| dark())?;
    if let Some(busy) = busy_refusal(ctx.w, rig, &record) {
        return Err(CmdError::msg(busy.line()));
    }
    if let Some(running) = run_refusal(ctx.w, rig, &record) {
        return Err(CmdError::msg(running.line()));
    }
    // 3. The hopper, then the hold's room.
    let hopper = hopper_contents(ctx, rig.tv).map_err(|error| {
        tracing::warn!(tv = rig.tv.0, %error, "tacsim: an unload could not read the hopper");
        dark()
    })?;
    if hopper.is_empty() {
        return Err(CmdError::msg(Refusal::Empty { n: rig.slot }.line()));
    }
    let w: &World = ctx.w;
    let view = hull_view(w, rig.hull).ok_or_else(dark)?;
    let read_pool = |endpoint: EndpointRef, good: &CanonicalId| {
        crate::port_tick::read_pool(ctx, endpoint, good)
    };
    let read_rig = |tv: ObjId| SurfaceReads::rig(&*ctx, tv);
    let Some(room) = unload_room(w, &view, rig.tv, &read_pool, &read_rig) else {
        tracing::warn!(
            tv = rig.tv.0,
            "tacsim: an unload could not read the hold's room"
        );
        return Err(dark());
    };
    // 4. The split.
    let no_room = || {
        CmdError::msg(
            Refusal::UnloadNoRoom {
                n: rig.slot,
                name: name.clone(),
                load: load_words(w, &hopper),
            }
            .line(),
        )
    };
    let split = match unload_split(&room, &hopper, qty).map_err(|()| dark())? {
        Ok(split) if split.moves.is_empty() => return Err(no_room()),
        Ok(split) => split,
        Err(Short { fit: 0, .. }) => return Err(no_room()),
        Err(Short { fit, bound }) => {
            let first = hopper.keys().next().cloned().ok_or_else(dark)?;
            // SM.7's `{ore}` (`ore_words`: `metal ore`), the mining lines' own word.
            let ore = tv::ore_words(first.as_str());
            // The lead's pre-commit round, item 4: the line names the term that bound the fit
            // — the hopper's own stock, or the hold's room.
            let refusal = match bound {
                ShortBound::Stock => Refusal::UnloadNoStock {
                    n: rig.slot,
                    fit,
                    ore,
                },
                ShortBound::Room => Refusal::UnloadNoFit {
                    n: rig.slot,
                    q: qty.unwrap_or(0),
                    fit,
                    ore,
                },
            };
            return Err(CmdError::msg(refusal.line()));
        }
    };
    let n = rig.slot.to_string();
    let now_ic = w.world_ic_now().at;
    if !go {
        let (kg, cap_kg) = tv::hopper_load_of(ctx, rig.tv).map_err(|error| {
            tracing::warn!(tv = rig.tv.0, %error, "tacsim: an unload could not weigh the hopper");
            dark()
        })?;
        let mut moved_kg: u64 = 0;
        for (good, qty) in &split.moves {
            let per = room
                .goods
                .good(good)
                .and_then(crate::definitions::GoodDef::mass_kg_per_unit)
                .ok_or_else(dark)?;
            moved_kg = moved_kg.saturating_add(qty.saturating_mul(per));
        }
        let real =
            match crate::port::voyage::real_until(w, now_ic.saturating_add(i64::from(plan.secs))) {
                Some(real) => format!(", about {}", tv::span_words(real)),
                None => String::new(),
            };
        let stays = if qty.is_none() && !split.stays.is_empty() {
            tv::UNLOAD_STAYS.replace("{stays}", &load_words(w, &split.stays))
        } else {
            String::new()
        };
        let line = tv::UNLOAD_PREVIEW
            .replace("{n}", &n)
            .replace("{mark}", &rig.mark)
            .replace("{load}", &load_words(w, &split.moves))
            .replace("{name}", &name)
            .replace("{secs}", &plan.secs.to_string())
            .replace("{real}", &real)
            .replace("{hop0}", &(kg / 1_000).to_string())
            .replace("{hop1}", &(kg.saturating_sub(moved_kg) / 1_000).to_string())
            .replace("{cap}", &(cap_kg / 1_000).to_string())
            .replace(
                "{room}",
                &room_after_t(&room, &split.moves)
                    .ok_or_else(dark)?
                    .to_string(),
            )
            .replace("{stays}", &stays)
            .replace("{arg}", &qty.map_or_else(String::new, |q| format!(" {q}")));
        crate::port::coop::say(ctx, line);
        return Ok(());
    }
    // `go`: the order, then ONE submission as the actor (the drive's `to` precedent: `how` and
    // `qty` bind the start to the order it was planned from — decision R15).
    let Some(ordinal) = record.ordinal.checked_add(1) else {
        tracing::error!(tv = rig.tv.0, "tacsim: a rig's act ordinal is exhausted");
        return Err(dark());
    };
    let Some(key) = act_key(rig.tv, ordinal) else {
        tracing::error!(tv = rig.tv.0, "tacsim: a rig act's key did not form");
        return Err(dark());
    };
    let how = if qty.is_some() {
        UnloadHow::Part
    } else {
        UnloadHow::Whole
    };
    let when = tv::due_words(w, now_ic.saturating_add(i64::from(plan.secs)));
    put_order(
        ctx,
        rig.tv,
        ActOrder::Unload(UnloadOrder {
            actor,
            ordinal,
            goods: split.moves,
            how,
            qty,
        }),
    );
    let parameters = match qty {
        Some(qty) => json!({
            "tv": rig.tv.0.to_string(),
            "act": wire_word(ActKind::Unload, false),
            "ordinal": ordinal,
            "how": how.word(),
            "qty": qty,
        }),
        None => json!({
            "tv": rig.tv.0.to_string(),
            "act": wire_word(ActKind::Unload, false),
            "ordinal": ordinal,
            "how": how.word(),
        }),
    };
    ctx.submit_authoritative(action, PerceivedRef::SelfActor, Vec::new(), parameters, key);
    crate::port::coop::say(
        ctx,
        tv::UNLOAD_FILED.replace("{n}", &n).replace("{when}", &when),
    );
    Ok(())
}

// ---- doc 20 cut SM.4b: the drive's order and `+tv/drive` --------------------------------------

/// **One planned drive, waiting for its start** (doc 20 cut SM.4b; the brief's decision P2): the
/// command plans the drive, writes this order for the rig and submits; the start's offer fixes
/// it (after re-running the same stand gate) and the prepare binds it to the invocation's
/// ordinal. A player-typed target cannot reach the offer (the legal-action query carries no
/// parameters) and a player-supplied leg cannot be trusted (the prepare has no composite), so the
/// plan the prepare trusts is server-fixed this way. RAM only: a restart loses an unlanded order,
/// and its start then refuses "not planned; preview it again".
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DriveOrder {
    /// Who planned it.
    pub actor: ObjId,
    /// The ordinal its start carries.
    pub ordinal: u32,
    /// The site entry.
    pub site: CanonicalId,
    /// The act's IC seconds (`LAUNCH_S` more from the bay).
    pub secs: u32,
    /// The leg's charge.
    pub cost_cu: u32,
    /// The charge home from the end stand.
    pub home_cu: u32,
    /// The frozen leg ([`ActParams::Drive`]).
    pub params: ActParams,
    /// The launch is implied.
    pub from_bay: bool,
}

/// **One planned mining run, waiting for its start** (doc 20 cut SM.7; the brief's decision R10,
/// SM.4b's P2 precedent): the command plans the run and writes this order; the start's offer
/// re-plans with the same choice and requires the plan equal (`mine_changed` otherwise), so `go`
/// commits exactly the preview or nothing. RAM only: a restart loses an unlanded order ("not
/// planned; preview it again").
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MineOrder {
    /// Who planned it.
    pub actor: ObjId,
    /// The ordinal its start carries.
    pub ordinal: u32,
    /// The site entry.
    pub site: CanonicalId,
    /// What was asked.
    pub choice: crate::surface::mine::MineChoice,
    /// The plan previewed.
    pub plan: crate::surface::mine::MinePlan,
}

/// **One planned hand unload, waiting for its start** (doc 20 cut SM.8; the brief's decisions R4
/// and R15, SM.7's `MineOrder` precedent): the command splits the hopper against the hold's room
/// and writes this order; the start's offer fixes it — only for the ordinal it was planned for
/// (RAM orders are never removed on landing) — and the prepare holds the start's `how` and `qty`
/// to it, then re-checks the hopper and the room in the transaction. RAM only: a restart loses an
/// unlanded order ("that act was not planned; preview it again").
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UnloadOrder {
    /// Who planned it.
    pub actor: ObjId,
    /// The ordinal its start carries.
    pub ordinal: u32,
    /// What moves (the preview's split).
    pub goods: BTreeMap<CanonicalId, u64>,
    /// `Whole` or `Part`.
    pub how: UnloadHow,
    /// The tonnes named (`Part` only).
    pub qty: Option<u64>,
}

/// **One planned filing, waiting for its start** (doc 20 cut SM.11; the brief's 3.5 and D10, the
/// `MineOrder` precedent): the command compiles the list and writes this order; the start's offer
/// — only for the record's ordinal and next plan number (the reviewer's F11) — re-compiles over the
/// engine and requires it equal (`plan_changed` otherwise), so `go` files exactly the preview or
/// nothing. RAM only: a restart loses an unlanded filing ("that act was not planned"). The plan's
/// own steps never read a RAM order (lead ruling 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PlanOrder {
    /// Who filed it.
    pub actor: ObjId,
    /// The record's ordinal when it was planned (a filing writes no act, so no ordinal of its own).
    pub ordinal: u32,
    /// The plan's number: the record's `plans + 1`.
    pub no: u32,
    /// The list as typed.
    pub list: String,
    /// `family=` as typed.
    pub family: Option<OreFamily>,
    /// The compiled plan previewed.
    pub compiled: crate::surface::plan::Compiled,
}

/// **One rig's order** (doc 20 cut SM.7): a drive's or a mining run's — and since doc 20 cut SM.8
/// a hand unload's, since SM.11 a plan's filing; the newer of a rig's orders replaces the older
/// (one slot).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ActOrder {
    /// `+tv/drive … go`.
    Drive(DriveOrder),
    /// `+tv/mine … go` (boxed: the plan is large beside a drive's order).
    Mine(Box<MineOrder>),
    /// `+tv/unload … go` (doc 20 cut SM.8).
    Unload(UnloadOrder),
    /// `+tv/plan … go` (doc 20 cut SM.11; boxed beside the others).
    Plan(Box<PlanOrder>),
}

/// **The act orders** (doc 20 cut SM.4b's `DriveOrders`, generalised by cut SM.7 to every act
/// whose plan the legal-action query cannot carry — the brief's decision R10; the `RigPointer`
/// precedent): tacsim's own RAM module slot, registered by `crate::surface::tick::register`, keyed
/// by the rig, bounded at `crate::port::tv::MAX_POINTERS` (the smallest key dropped past it). A
/// convenience for the offer, never a fact: it decides nothing the prepare does not re-check.
#[derive(Clone, Debug, Default)]
pub(crate) struct ActOrders(pub(super) BTreeMap<ObjId, ActOrder>);

/// Write `order` for `tv`.
pub(super) fn put_order(ctx: &mut Ctx<'_>, tv: ObjId, order: ActOrder) {
    if let Some(orders) = ctx.w.module_state_mut::<ActOrders>() {
        orders.0.insert(tv, order);
        while orders.0.len() > tv::MAX_POINTERS {
            if orders.0.pop_first().is_none() {
                break;
            }
        }
    }
}

/// **A drive's end cell as one word** (the review's correctness/F5): `y × 256 + x` for the
/// `to` of an [`ActParams::Drive`], `0..=65,535` — the start's `to` parameter, which the prepare
/// holds to the order the offer fixed. `None` for any other params or a cell off the site.
pub(super) fn order_to_word(params: &ActParams) -> Option<u32> {
    let ActParams::Drive { to, .. } = params else {
        return None;
    };
    let side = i64::from(crate::surface::ground::SITE_CELLS);
    let ([x, y], range) = (*to, 0..side);
    if !range.contains(&x) || !range.contains(&y) {
        return None;
    }
    u32::try_from(y.checked_mul(side)?.checked_add(x)?).ok()
}

/// The drive order planned for `tv`, if its slot holds one.
pub(super) fn order_of(w: &World, tv: ObjId) -> Option<DriveOrder> {
    match w
        .module_state::<ActOrders>()
        .and_then(|orders| orders.0.get(&tv))
    {
        Some(ActOrder::Drive(order)) => Some(order.clone()),
        Some(ActOrder::Mine(_) | ActOrder::Unload(_) | ActOrder::Plan(_)) | None => None,
    }
}

/// The hand unload order planned for `tv`, if its slot holds one (doc 20 cut SM.8).
pub(super) fn unload_order_of(w: &World, tv: ObjId) -> Option<UnloadOrder> {
    match w
        .module_state::<ActOrders>()
        .and_then(|orders| orders.0.get(&tv))
    {
        Some(ActOrder::Unload(order)) => Some(order.clone()),
        Some(ActOrder::Drive(_) | ActOrder::Mine(_) | ActOrder::Plan(_)) | None => None,
    }
}

/// **Landed mining orders leave the slot** (doc 20 cut SM.7; the SM.7 lead's pre-commit round,
/// item 5): every mining order whose ordinal is not its rig's next (`record.ordinal + 1`; 1 for a
/// rig with no record in `records`) — its start landed, or another act overtook it — is dropped,
/// so no later offer re-plans it. The surface tick calls it with a **complete** scan of the
/// family (a capped scan prunes nothing). A drive's order is kept as SM.4b left it: its stale
/// start answers "that rig's orders moved on" at the prepare (`tests/driving.rs` D9). **Doc 20
/// cut SM.8**: a hand unload's order is pruned the same way. The number of orders dropped.
pub(crate) fn prune_mine_orders(w: &mut World, records: &[RigRecord]) -> usize {
    let next: BTreeMap<ObjId, u32> = records
        .iter()
        .map(|record| (record.tv, record.ordinal.saturating_add(1)))
        .collect();
    // Doc 20 cut SM.11 (the reviewer's F11): a filing leaves once it landed (`plans ≥ no`) or the
    // rig's orders moved on (`ordinal` changed); a rig with no record keeps a first filing at 0.
    let filed: BTreeMap<ObjId, (u32, u32)> = records
        .iter()
        .map(|record| (record.tv, (record.ordinal, record.plans)))
        .collect();
    let Some(orders) = w.module_state_mut::<ActOrders>() else {
        return 0;
    };
    let before = orders.0.len();
    orders.0.retain(|tv, order| match order {
        ActOrder::Drive(_) => true,
        ActOrder::Mine(mine) => next.get(tv).copied().unwrap_or(1) == mine.ordinal,
        ActOrder::Unload(unload) => next.get(tv).copied().unwrap_or(1) == unload.ordinal,
        ActOrder::Plan(plan) => {
            let (ordinal, plans) = filed.get(tv).copied().unwrap_or((0, 0));
            plans < plan.no && ordinal == plan.ordinal
        }
    });
    before.saturating_sub(orders.0.len())
}

/// **Test hook** (doc 20 cut SM.11; the brief's P5): the plan number of the filing `tv`'s RAM
/// slot holds, if any — so a test can prove a landed filing leaves the slot. Reads only.
#[doc(hidden)]
#[must_use]
pub fn plan_order_for_tests(w: &World, tv: ObjId) -> Option<u32> {
    plan_order_of(w, tv).map(|order| order.no)
}

/// **Test hook** (doc 20 cut SM.11; the brief's P5 and P8): how many times the start's offer
/// compiled a filing, the interpreter ran and it planned a drive leg, this boot — so a test can
/// prove a stale filing compiles nothing and a capped plan plans no leg. The players' plans only:
/// a yard rig's are never counted (doc 20 cut SM.12b). Reads only.
#[doc(hidden)]
#[must_use]
pub fn plan_counts_for_tests(w: &World) -> (u64, u64, u64) {
    use std::sync::atomic::Ordering;
    w.module_state::<crate::surface::plan::PlanCounters>()
        .map_or((0, 0, 0), |counters| {
            (
                counters.0.compiles.load(Ordering::Relaxed),
                counters.0.decides.load(Ordering::Relaxed),
                counters.0.legs.load(Ordering::Relaxed),
            )
        })
}

/// The filing planned for `tv`, if its slot holds one (doc 20 cut SM.11).
pub(super) fn plan_order_of(w: &World, tv: ObjId) -> Option<PlanOrder> {
    match w
        .module_state::<ActOrders>()
        .and_then(|orders| orders.0.get(&tv))
    {
        Some(ActOrder::Plan(order)) => Some(PlanOrder::clone(order)),
        Some(ActOrder::Drive(_) | ActOrder::Mine(_) | ActOrder::Unload(_)) | None => None,
    }
}

/// **Test hook** (doc 20 cut SM.7; the SM.7 lead's pre-commit round, item 5): the ordinal of the
/// mining order `tv`'s RAM slot holds, if any — so a test can prove a landed order leaves the
/// slot. Reads only; nothing in the shipped lane calls it.
#[doc(hidden)]
#[must_use]
pub fn mine_order_for_tests(w: &World, tv: ObjId) -> Option<u32> {
    mine_order_of(w, tv).map(|order| order.ordinal)
}

/// **Test hook** (doc 20 cut SM.8; the fix round's tests/T4): the ordinal of the hand unload
/// order `tv`'s RAM slot holds, if any — so a test can pin the offer's ordinal filter on a stale
/// order still in the slot, and the prune that drops it. Reads only; nothing in the shipped lane
/// calls it.
#[doc(hidden)]
#[must_use]
pub fn unload_order_for_tests(w: &World, tv: ObjId) -> Option<u32> {
    unload_order_of(w, tv).map(|order| order.ordinal)
}

/// The mining order planned for `tv`, if its slot holds one.
pub(super) fn mine_order_of(w: &World, tv: ObjId) -> Option<MineOrder> {
    match w
        .module_state::<ActOrders>()
        .and_then(|orders| orders.0.get(&tv))
    {
        Some(ActOrder::Mine(order)) => Some(MineOrder::clone(order)),
        Some(ActOrder::Drive(_) | ActOrder::Unload(_) | ActOrder::Plan(_)) | None => None,
    }
}

/// **`+tv/<n> drive <target> [go]`** (doc 20 §1 beat 7, §8, §10; cut SM.4b, the brief's 3.5;
/// lead rulings 1–4 and 9, R4): **the cheap gates first** — the target parsed (`DRIVE_USAGE`);
/// the stand (`stand_of`: from the bay the implied launch's whole gate; else on the site's
/// ground); the battery and the rig's rates (`TV_DARK`); the record (busy → `RIG_BUSY`; the launch
/// half's open excursion and the caps) — **then** the plan (`drive_plan`: the peeks, a cold site or
/// home wanted and refused `GROUND_COLD`; the target; ONE A*; no way home); then the reserve with
/// the real trip home (`RESERVE_LOW`); then the preview (`DRIVE_PREVIEW`) or, with `go`, the order
/// written ([`ActOrders`]) and ONE `tacsim_surface_act` submission with `act: drive` keyed
/// `tacsim.surface/{tv}/{ordinal}/act`. No licence and no tenure is read (lead ruling 9): a rig
/// may drive onto any block.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line.
#[allow(clippy::too_many_lines)] // one ordered refusal list, then the preview or the go
pub(crate) fn drive_cmd(
    ctx: &mut Ctx<'_>,
    rig: &RigAt,
    words: &str,
    go: bool,
) -> Result<(), CmdError> {
    let n = rig.slot.to_string();
    let split: Vec<&str> = words.split_whitespace().collect();
    let Some(target) = drive::parse_target(&split) else {
        return Err(CmdError::msg(tv::DRIVE_USAGE.replace("{n}", &n)));
    };
    // Doc 20 cut SM.11 (D8): a live plan holds its rig.
    plan_holds(ctx, rig)?;
    let actor = ctx.actor;
    let handles = ctx
        .w
        .module_state::<PortHandles>()
        .cloned()
        .ok_or_else(|| CmdError::msg(tv::TV_UNWIRED))?;
    let Some(action) = handles.surface_act.clone() else {
        return Err(CmdError::msg(tv::TV_UNWIRED));
    };
    let name = name_of(ctx.w, rig.hull);
    let dark = || CmdError::msg(tv::TV_DARK.replace("{name}", &name));
    // 1. The stand: from the bay the implied launch's whole gate, else on the site's ground.
    let stand = stand_of(ctx.w, actor, rig).map_err(|refusal| CmdError::msg(refusal.line()))?;
    // 2. The battery and the rates: a drive spends.
    let Some(battery) = rig_battery(ctx.w, &handles, rig) else {
        return Err(dark());
    };
    let Some(rates) = rig_rates(ctx.w, rig.tv, Some(&battery)) else {
        return Err(dark());
    };
    // 3. The record: busy, the launch half's open excursion and its caps.
    let record = record_of(ctx, &handles, rig.tv, Some(&battery)).map_err(|()| dark())?;
    if let Some(busy) = busy_refusal(ctx.w, rig, &record) {
        return Err(CmdError::msg(busy.line()));
    }
    // Doc 20 cut SM.7 (§6 "the next drive's preview" is legal during a run; decision R1): a run
    // holds the rig to its pocket, so the preview prints and `go` refuses.
    if go && let Some(running) = run_refusal(ctx.w, rig, &record) {
        return Err(CmdError::msg(running.line()));
    }
    if stand.from_bay {
        launch_half(ctx, &handles, rig, &record, &stand.site, &dark)?;
    }
    // 4. The plan: the peeks, the target, ONE A*, the way home (R4: never before the gates).
    let mut peeks = Peeks::default();
    let (plan, leg) = match drive_plan(ctx.w, rig, &stand, &target, &rates, &mut peeks) {
        Ok(planned) => planned,
        Err(refusal) => {
            want_cold(ctx, rig, &refusal);
            return Err(CmdError::msg(refusal.line()));
        }
    };
    let w: &World = ctx.w;
    // 5. The reserve, with the real trip home.
    let now_ic = w.world_ic_now().at;
    let (now, after, home) = reserve_words(
        &record,
        now_ic,
        stand.from_bay,
        (plan.cost_cu, plan.home_cu),
        &battery,
        rig.slot,
    )?;
    let target_text = drive::target_words(&target);
    let ramp = stand.ramp_label.clone();
    if !go {
        let launch = if stand.from_bay {
            tv::IMPLIED_LAUNCH
                .replace("{ramp}", &ramp)
                .replace("{site}", &stand.site_name)
        } else {
            String::new()
        };
        let time = if stand.from_bay {
            tv::TWO_LEGS
                .replace("{down}", &LAUNCH_S.to_string())
                .replace("{secs}", &leg.secs.to_string())
        } else {
            format!("{} IC s", leg.secs)
        };
        let real =
            match crate::port::voyage::real_until(w, now_ic.saturating_add(i64::from(plan.secs))) {
                Some(real) => format!(", about {}", tv::span_words(real)),
                None => String::new(),
            };
        let line = tv::DRIVE_PREVIEW
            .replace("{n}", &n)
            .replace("{mark}", &rig.mark)
            .replace("{launch}", &launch)
            .replace("{m}", &leg.metres.to_string())
            .replace("{target}", &target_text)
            .replace("{time}", &time)
            .replace("{real}", &real)
            .replace("{now}", &now.to_string())
            .replace("{after}", &after.to_string())
            .replace("{home}", &home.to_string())
            .replace("{arg}", &split.join(" "));
        crate::port::coop::say(ctx, line);
        return Ok(());
    }
    // `go`: the order, then ONE submission as the actor.
    let Some(ordinal) = record.ordinal.checked_add(1) else {
        tracing::error!(tv = rig.tv.0, "tacsim: a rig's act ordinal is exhausted");
        return Err(dark());
    };
    let Some(key) = act_key(rig.tv, ordinal) else {
        tracing::error!(tv = rig.tv.0, "tacsim: a rig act's key did not form");
        return Err(dark());
    };
    let due = now_ic.saturating_add(i64::from(plan.secs));
    let when = tv::due_words(w, due);
    put_order(
        ctx,
        rig.tv,
        ActOrder::Drive(DriveOrder {
            actor,
            ordinal,
            site: plan.site.clone(),
            secs: plan.secs,
            cost_cu: plan.cost_cu,
            home_cu: plan.home_cu,
            params: plan.params.clone(),
            from_bay: plan.from_bay,
        }),
    );
    // The review's correctness/F5: the start names the end cell this command planned, and the
    // prepare holds it to the order the offer fixed — so a second `go` before the first lands
    // cannot start the first's start on the second's target (the first then refuses "that rig's
    // orders moved on", and the told line never names a drive the world does not run).
    let Some(to_word) = order_to_word(&plan.params) else {
        tracing::error!(tv = rig.tv.0, "tacsim: a drive's end cell did not form");
        return Err(dark());
    };
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({
            "tv": rig.tv.0.to_string(),
            "act": wire_word(ActKind::Drive, false),
            "ordinal": ordinal,
            "to": to_word,
        }),
        key,
    );
    let line = if stand.from_bay {
        tv::DRIVE_FILED_LAUNCH
    } else {
        tv::DRIVE_FILED
    }
    .replace("{n}", &n)
    .replace("{ramp}", &ramp)
    .replace("{target}", &target_text)
    .replace("{when}", &when);
    crate::port::coop::say(ctx, line);
    Ok(())
}

/// The words every reading preview shares.
pub(super) struct PreviewWords<'a> {
    pub(super) n: &'a str,
    pub(super) mark: &'a str,
    pub(super) ramp: &'a str,
    pub(super) site: &'a str,
    pub(super) real: &'a str,
    pub(super) now: u32,
    pub(super) after: u32,
}

/// **A scan's or a sounding's preview** (doc 20 cut SM.4a; the brief's 3.8): `SCAN_PREVIEW` —
/// the fields the mast sees around the stand's field, the footing and what better footing
/// would buy (`chart::better_footing` within the scan's reach), the hidden count — or
/// `SOUND_PREVIEW` — the pockets in reach around the stand's pocket and who hears it; both name
/// the implied launch's two legs. `None` when a name does not form (never expected).
pub(super) fn reading_preview(
    w: &World,
    plan: &ActPlan,
    scanner: &ScannerDef,
    peeks: &mut Peeks,
    words: PreviewWords<'_>,
) -> Option<String> {
    let launch = if plan.from_bay {
        tv::IMPLIED_LAUNCH
            .replace("{ramp}", words.ramp)
            .replace("{site}", words.site)
    } else {
        String::new()
    };
    let own_secs = if plan.from_bay {
        plan.secs.saturating_sub(LAUNCH_S)
    } else {
        plan.secs
    };
    let time = if plan.from_bay {
        tv::TWO_LEGS
            .replace("{down}", &LAUNCH_S.to_string())
            .replace("{secs}", &own_secs.to_string())
    } else {
        format!("{own_secs} IC s")
    };
    let head = |template: &str| {
        template
            .replace("{n}", words.n)
            .replace("{mark}", words.mark)
            .replace("{launch}", &launch)
            .replace("{time}", &time)
            .replace("{real}", words.real)
            .replace("{now}", &words.now.to_string())
            .replace("{after}", &words.after.to_string())
    };
    match &plan.params {
        ActParams::Scan {
            cell,
            footing,
            fields,
            hidden,
            ..
        } => {
            let stand = (cell[0], cell[1]);
            let field =
                address::field_of_cell(stand).and_then(|(fx, fy)| address::field_name(fx, fy))?;
            let radius = i64::from(scanner.scan_radius_cells);
            let better = match chart::peek_site(w, &plan.site, peeks) {
                SitePeek::Warm(ground) => chart::ground_surface(&ground)
                    .and_then(|surface| chart::better_footing(surface, stand, *footing, radius)),
                SitePeek::Cold | SitePeek::Dark => None,
            };
            let reach_m = radius.saturating_mul(10).to_string();
            let footing_words = match (footing, better) {
                (Footing::Rock, _) => tv::FOOTING_ROCK.to_owned(),
                (Footing::Regolith, Some((_, m, dir))) => tv::FOOTING_REGOLITH
                    .replace("{m}", &m.to_string())
                    .replace("{dir}", dir.abbrev()),
                (Footing::Regolith, None) => tv::FOOTING_REGOLITH_NONE.replace("{r}", &reach_m),
                (Footing::Ice, Some((sharper, m, dir))) => tv::FOOTING_ICE
                    .replace("{better}", sharper.name())
                    .replace("{m}", &m.to_string())
                    .replace("{dir}", dir.abbrev()),
                (Footing::Ice, None) => tv::FOOTING_ICE_NONE.replace("{r}", &reach_m),
            };
            let hidden_words = if hidden.is_empty() {
                String::new()
            } else {
                tv::HIDDEN_FIELDS.replace("{k}", &hidden.len().to_string())
            };
            Some(
                head(tv::SCAN_PREVIEW)
                    .replace("{count}", &fields.len().to_string())
                    .replace("{field}", &field)
                    .replace("{footing}", &footing_words)
                    .replace("{hidden}", &hidden_words),
            )
        }
        ActParams::Sound {
            cell,
            radius,
            quiet,
            ..
        } => {
            let stand = (cell[0], cell[1]);
            let (px, py) = address::pocket_of_cell(stand)?;
            let addr = address::pocket_address(px, py)?;
            let count = pockets_in_reach(stand, i64::from(*radius)).len();
            let heard = if *quiet {
                tv::HEARD_BY_NOBODY.to_owned()
            } else {
                tv::HEARD_ACROSS.replace("{site}", words.site)
            };
            Some(
                head(tv::SOUND_PREVIEW)
                    .replace("{count}", &count.to_string())
                    .replace("{addr}", &addr)
                    .replace("{heard}", &heard)
                    .replace("{quiet}", if *quiet { " quiet" } else { "" }),
            )
        }
        ActParams::None {}
        | ActParams::Dump { .. }
        | ActParams::Drive { .. }
        | ActParams::Unload { .. } => None,
    }
}

// ---- doc 20 cut SM.7: mining ---------------------------------------------------------------------

/// **Which drill a rig carries** (doc 20 cut SM.7; `rig_scanner_id`'s rule): the lessor pool's
/// tools whose row in the one tools table is a drill.
#[must_use]
pub(crate) fn rig_drill_id(w: &World, rig_tv: ObjId) -> Option<CanonicalId> {
    crate::port::vehicle::rig_row(w, rig_tv)?;
    let tools = w.module_state::<PortHandles>()?.surface_tools.clone()?;
    let port = crate::port::coop::programme_port(w)?;
    let pool = port.def.vehicles.as_deref()?;
    pool.tools
        .iter()
        .find(|id| tools.get(*id).and_then(ToolDef::drill).is_some())
        .cloned()
}

/// **A drill's figures** by its id, from a tools table (`ToolDef::drill`).
#[must_use]
pub(crate) fn drill_of<'t>(
    tools: &'t BTreeMap<CanonicalId, ToolDef>,
    id: &CanonicalId,
) -> Option<&'t DrillDef> {
    tools.get(id).and_then(ToolDef::drill)
}

/// **A family's word on the verb** (`+tv/mine ice`): the first word `geology::family_of_word`
/// reads for it.
#[must_use]
pub(crate) fn family_word(family: OreFamily) -> &'static str {
    match family {
        OreFamily::Metals => "metal",
        OreFamily::Volatiles => "ice",
        OreFamily::CarbonFeedstock => "carbon",
        OreFamily::StrategicElements => "strategic",
    }
}

/// **The block's authored tenure, as mining reads it** (the brief's 3.3 item 5, reviewer F5):
/// `port::site::block_tenure`'s rule exactly — `(Commons, _)` commons; `(Held, Some(_))` held;
/// `(Open, _)` and `(Held, None)` open (a holder-less held block reads open, as the status line
/// and `tenure_parts` count it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Authored {
    /// Commons: no claim is read.
    Commons,
    /// Open ground.
    Open,
    /// An org's (until SM.10 seeds its claims).
    Held,
}

/// The authored word of `letter` on `site`.
#[must_use]
pub(crate) fn authored_of(site: &crate::port::def::SiteDef, letter: char) -> Authored {
    use crate::port::def::Tenure;
    let block = site.block(letter);
    match (block.tenure, block.holder) {
        (Tenure::Commons, _) => Authored::Commons,
        (Tenure::Held, Some(_)) => Authored::Held,
        (Tenure::Open, _) | (Tenure::Held, None) => Authored::Open,
    }
}

/// **Why tenure bars a block** (doc 20 cut SM.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TenureBar {
    /// `MINE_CLAIMED`.
    Claimed,
    /// The claims or the issuer cannot be read: fail closed. Which read failed (the SM.7 lead's
    /// pre-commit round, item 3: a run's landing pages it and ends `RunEnd::Dark`).
    Dark(DarkCause),
}

/// **A live claim** (the brief's decision R20, reviewer F5): not lapsed, and a term not before
/// `now`; an uncomparable term (another clock generation, K4a) counts as **live** — a claim we
/// cannot date bars the block (fail closed). A claim past its term that nothing has lapsed is not
/// live (SM.10 lapses such claims; before it a staff-filed claim can outlive its term).
#[must_use]
pub(crate) fn claim_live(claim: &mush_industry::ClaimRecord, now: &mush_core::ClockStamp) -> bool {
    !claim.is_lapsed() && !matches!(claim.term().checked_cmp(now), Ok(std::cmp::Ordering::Less))
}

/// **The tenure verdict, pure** (doc 20 §6 "tenure allows it"; the brief's 3.3 item 5, decision
/// R5): commons → Ok with no claim read; then the **live** claims of the site's issuer over the
/// block's opened deposits (`claims`, filtered by the caller to the issuer): none live → an
/// authored held block refuses `Claimed` (R5 — since doc 20 cut SM.10 the fallback for a held
/// block the site clerk has not seeded yet: a seeded one carries the org's live claims, F15),
/// open ground is Ok; some live → Ok iff `authorizes` one of them — **whatever its
/// contest state** (a suspended or seized claim authorizes nobody, so it bars its holder too:
/// industry's rule), else `Claimed`. Block-level: it answers the same whatever family a run names.
///
/// # Errors
///
/// [`TenureBar::Claimed`].
pub(crate) fn tenure_verdict(
    authored: Authored,
    claims: &[mush_industry::ClaimRecord],
    now: &mush_core::ClockStamp,
    authorizes: &dyn Fn(&mush_industry::ClaimRecord) -> bool,
) -> Result<(), TenureBar> {
    if authored == Authored::Commons {
        return Ok(());
    }
    let live: Vec<&mush_industry::ClaimRecord> = claims
        .iter()
        .filter(|claim| claim_live(claim, now))
        .collect();
    if live.is_empty() {
        return if authored == Authored::Held {
            Err(TenureBar::Claimed)
        } else {
            Ok(())
        };
    }
    if live.iter().any(|claim| authorizes(claim)) {
        Ok(())
    } else {
        Err(TenureBar::Claimed)
    }
}

/// **The tenure of a block, for `actor`** (doc 20 §6; the brief's 3.3 item 5 and decision R5):
/// the block's authored word read directly ([`authored_of`]; never `port::site::block_tenure`'s
/// display string); commons answers at once; else the claims over every opened deposit of the
/// block (the four families' receipt-derived ids that `reads.deposit` finds) whose issuer is the
/// site's issuer principal (`port::site::org_principal`), judged by [`tenure_verdict`] with
/// `mush_industry::extraction_authorized` over that claim's own deposit (`remaining` its initial
/// reserve: the cursor, not the pool, sets the tonnes, §3.4) and industry's licence instrument.
/// **Doc 20 cut SM.10**: then the block's public record (`tacsim.claims`): a live claim held by
/// neither the actor nor someone the actor `acts_for` refuses `Claimed` — the one answer for a
/// block with no opened deposit. **Read only**: no industry family is written (SM.6's D10
/// tripwire).
///
/// # Errors
///
/// [`TenureBar`]: claimed, or the issuer, a deposit or a claim cannot be read.
pub(crate) fn mine_tenure(
    w: &World,
    reads: &dyn SurfaceReads,
    actor: ObjId,
    (entry, site): (&CanonicalId, &crate::port::def::SiteDef),
    letter: char,
    now: &mush_core::ClockStamp,
) -> Result<(), TenureBar> {
    let authored = authored_of(site, letter);
    if authored == Authored::Commons {
        return Ok(());
    }
    let Some(issuer) = crate::port::site::org_principal(w, &site.issuer) else {
        tracing::error!(
            site = %entry,
            issuer = %site.issuer,
            "tacsim: a mining site's issuer does not resolve; its blocks refuse mining (fail closed)"
        );
        return Err(TenureBar::Dark(DarkCause::Issuer));
    };
    let instrument = w
        .module_state::<mush_industry::IndustryStore>()
        .and_then(|store| store.licence().cloned());
    let mut claims: Vec<mush_industry::ClaimRecord> = Vec::new();
    let mut bodies: BTreeMap<mush_industry::ClaimId, mush_industry::OreDeposit> = BTreeMap::new();
    for family in OreFamily::ALL {
        let Ok(id) = mush_industry::derive_system_deposit_id(&crate::surface::opening::receipt(
            entry, letter, family,
        )) else {
            continue;
        };
        let Some(body) = reads
            .deposit(id)
            .map_err(|_| TenureBar::Dark(DarkCause::Deposit))?
        else {
            continue;
        };
        for claim in reads
            .claims(id)
            .map_err(|_| TenureBar::Dark(DarkCause::Claims))?
        {
            if claim.issuer() == &issuer {
                bodies.insert(claim.id(), body.clone());
                claims.push(claim);
            }
        }
    }
    // Doc 20 cut SM.12b (D3): the operator principal — `Organization(house)` for a yard's
    // dispatcher, whose seeded claims and licence are the house's; `Object(actor)` for a player.
    let operator = crate::port::yard::operator_principal(w, actor);
    tenure_verdict(authored, &claims, now, &|claim| {
        bodies.get(&claim.id()).is_some_and(|body| {
            mush_industry::extraction_authorized(
                w,
                actor,
                &operator,
                body,
                body.initial_reserve(),
                std::slice::from_ref(claim),
                instrument.as_ref(),
                now,
            )
            .is_ok()
        })
    })?;
    // Doc 20 cut SM.10 (the brief's 3.8 item 21, reviewer F4; a stated, narrow touch of SM.7's
    // gate): the block's public record too. On an ore-bearing block the two always agree (both
    // are written in one bundle); on a block with no opened deposit the record alone answers, so
    // that block refuses a stranger exactly as an ore block does. A live claim held by neither
    // the actor nor someone the actor acts for bars it; a past-term one is open ground (R20).
    let record = reads
        .block_tenure(&crate::surface::claims::BlockKey {
            site: entry.clone(),
            letter,
        })
        .map_err(|_| TenureBar::Dark(DarkCause::Claims))?;
    match record.and_then(|record| record.claim) {
        Some(claim)
            if claim.live(now.at)
                && claim.holder != operator
                && !mush_economy::acts_for(w, actor, &claim.holder) =>
        {
            Err(TenureBar::Claimed)
        }
        _ => Ok(()),
    }
}

/// The live lease's bar as a mining refusal.
pub(super) fn mine_lease_refusal(w: &World, rig: &RigAt, bar: LeaseBar) -> Refusal {
    let (counter, port) = tv::counter_words(w);
    match bar {
        LeaseBar::Forfeit => Refusal::MineForfeit {
            n: rig.slot,
            counter,
            port,
        },
        LeaseBar::Ended => Refusal::MineEnded {
            n: rig.slot,
            counter,
            port,
        },
    }
}

/// **The licence, as mining reads it** (lead ruling 5; reviewer F21): in force or in its grace →
/// its lapse; no licence → `MINE_NO_LICENCE`; every other standing → `+licence`'s own line
/// (`crate::port::licence::standing_line`).
pub(super) fn mine_licence(w: &World, actor: ObjId, now_ic: i64) -> Result<i64, Refusal> {
    use crate::port::licence::LicenceStanding;
    match crate::port::licence::licence_standing(w, actor, now_ic) {
        LicenceStanding::InForce { lapses_ic, .. } | LicenceStanding::Grace { lapses_ic, .. } => {
            Ok(lapses_ic)
        }
        LicenceStanding::Unlicensed => Err(Refusal::Licence {
            line: tv::MINE_NO_LICENCE.to_owned(),
        }),
        other => Err(Refusal::Licence {
            line: crate::port::licence::standing_line(w, actor, &other)
                .unwrap_or_else(|| tv::MINE_NO_LICENCE.to_owned()),
        }),
    }
}

/// **The licence's verdict for the next cycle** (the brief's 3.7): `Ok(lapses_ic)` in force or
/// in its grace, else the standing. A `Lapsed { at_ic }` read at a late landing is still judged
/// against the next cycle's start, never against now (`RightsVerdict::stops_next`, R9).
pub(crate) fn licence_verdict(w: &World, actor: ObjId, now_ic: i64) -> Result<i64, LicenceEnd> {
    use crate::port::licence::LicenceStanding;
    match crate::port::licence::licence_standing(w, actor, now_ic) {
        LicenceStanding::InForce { lapses_ic, .. } | LicenceStanding::Grace { lapses_ic, .. } => {
            Ok(lapses_ic)
        }
        LicenceStanding::Lapsed { at_ic } => Err(LicenceEnd::Lapsed { at_ic }),
        LicenceStanding::Unlicensed => Err(LicenceEnd::Unlicensed),
        LicenceStanding::Suspended => Err(LicenceEnd::Suspended),
        LicenceStanding::Other => Err(LicenceEnd::Other),
        LicenceStanding::Unreadable => Err(LicenceEnd::Unreadable),
    }
}

/// **A stored licence end as `+licence`'s line** (the tell's; reviewer F1: formatted at the tell,
/// never stored): `MINE_NO_LICENCE` for no licence, else `standing_line`'s.
#[must_use]
pub(crate) fn licence_end_line(w: &World, operator: ObjId, why: LicenceEnd) -> String {
    use crate::port::licence::LicenceStanding;
    let standing = match why {
        LicenceEnd::Lapsed { at_ic } => LicenceStanding::Lapsed { at_ic },
        LicenceEnd::Unlicensed => return tv::MINE_NO_LICENCE.to_owned(),
        LicenceEnd::Suspended => LicenceStanding::Suspended,
        LicenceEnd::Other => LicenceStanding::Other,
        LicenceEnd::Unreadable => LicenceStanding::Unreadable,
    };
    crate::port::licence::standing_line(w, operator, &standing)
        .unwrap_or_else(|| tv::MINE_NO_LICENCE.to_owned())
}

/// **The site the actor stands aboard** (doc 20 cut SM.10, D11: a contest is filed from aboard
/// the hull settled on the site): the rig's hull's view (`view_of`), its site berth
/// (`site_berth`: under way, not a site, dark) and the actor aboard it (`TV_NOT_ABOARD`); the
/// site's entry and name.
///
/// # Errors
///
/// The [`Refusal`] the launch's own gates answer.
pub(crate) fn aboard_site(
    w: &World,
    actor: ObjId,
    rig: &RigAt,
) -> Result<(CanonicalId, String), Refusal> {
    let (view, name) = view_of(w, rig)?;
    let berth = site_berth(w, &view)?;
    if !crate::port::voyage::aboard(w, rig.hull, actor) {
        return Err(Refusal::NotAboard { name });
    }
    Ok((berth.entry.clone(), berth.name.clone()))
}

/// The lane site definition of `entry` (cloned), when the latched lane authors one.
pub(crate) fn site_def_of(w: &World, entry: &CanonicalId) -> Option<crate::port::def::SiteDef> {
    let store = w.module_state::<TacsimStore>()?;
    let (_, ports) = crate::port::readiness::report_ports(store, w);
    ports
        .into_iter()
        .find(|(id, _)| id == entry)
        .and_then(|(_, def)| def.site.as_deref().cloned())
}

/// **One measure-wise room less `units` of a good** (saturating), for the preview's hold room
/// after the run.
pub(super) fn room_less(room: &Load, per_unit: &[(CanonicalId, u64)], units: u64) -> Load {
    room.iter()
        .map(|(measure, left)| {
            let per = per_unit
                .iter()
                .find(|(m, _)| m == measure)
                .map_or(0, |(_, per)| *per);
            (
                measure.clone(),
                left.saturating_sub(units.saturating_mul(per)),
            )
        })
        .collect()
}

/// **The R1-richest family of the operator's own row of a field** (the brief's decision R6): each
/// family's reading ranked by its upper word then its lower (`Exact(w)` is `(w, w)`, a band
/// `(high, low)`), ties to `OreFamily::ALL`'s order; `None` when the row does not cover the field,
/// reads ice, or reads barren in every family.
pub(super) fn richest_read(
    site: &chart::SiteContext,
    row: Option<&ChartRecord>,
    field: (i64, i64),
) -> Option<OreFamily> {
    use crate::surface::reading::FieldReading;
    let footing = chart::row_reads_field(row, field)?;
    let ground = chart::field_ground_at(site, field).ok()?;
    let readings = crate::surface::reading::read_r1(&site.mac, &site.body, &ground, footing);
    let mut best: Option<((u8, u8), OreFamily)> = None;
    for (family, reading) in OreFamily::ALL.into_iter().zip(readings) {
        let rank = match reading {
            FieldReading::Exact(word) => (word.code(), word.code()),
            FieldReading::Band(low, high) => (high.code(), low.code()),
            FieldReading::NoReturn => continue,
        };
        if best.is_none_or(|(kept, _)| rank > kept) {
            best = Some((rank, family));
        }
    }
    best.filter(|((high, _), _)| *high > 0)
        .map(|(_, family)| family)
}

/// **The mining plan, against the world** (doc 20 §6; the SM.7 brief's 3.3; lead rulings 2–5,
/// decisions R4–R6, R8, R11, R15, R16), called by `+tv/mine` (`reads` the command's `Ctx`) and by
/// the start's offer (`reads` the engine's snapshot) — one function, so the offer's re-plan equals
/// the preview's unless the world moved. **The gates, cheap first**:
///
/// 1. **the stand** — the layer and the view; under way; a rig in the bay → `MINE_IN_BAY` (no
///    implied launch: the drill works only a pocket it stands on); the ground half of `stand_of`
///    (`RECOVER_ELSEWHERE`); **then the lease** ([`lease_gate`]: `MINE_FORFEIT`, `MINE_ENDED`);
/// 2. **the pocket** under the stand: its address, block letter and number;
/// 3. **the site's ground** — resolved (`INSTRUMENTS_DARK`), the composite and the home field
///    peeked (`GROUND_COLD`, `INSTRUMENTS_DARK`), the footing (`MINE_NO_FOOTING`);
/// 4. **the licence** (`MINE_NO_LICENCE`, else `+licence`'s line);
/// 5. **the tenure** ([`mine_tenure`]: `MINE_CLAIMED`);
/// 6. **the record**: an act in flight → `RIG_BUSY`; a run → `MINE_RUNNING`;
/// 7. **the lock**: another rig's live lock on any family's row of the pocket, or another rig's
///    run on this pocket (decision R4: a barren pocket has no row) → `MINE_LOCKED`;
/// 8. **the reading and the family** — the operator's OWN row: read → the classes of their R2
///    (the named family barren → `MINE_NO_ORE`; every family barren → `MINE_ALL_BARREN`; the
///    default the family with most tonnes left, ties to the richer class); unread → a blind core:
///    the family named, else the operator's R1-richest, else metals (decision R6); a count past one
///    → `MINE_BLIND_COUNT`; **8b** the public pit (`site_map::pit_at`) shapes the blind preview by
///    one clause and never refuses (decision R16);
/// 9. **the truth, server-side** (never printed for a blind core): the class's yield, the pocket's
///    tonnes, the cursor's drawn, the lock rows (≤ [`MAX_LOCK_ROWS`]), the block vault;
/// 10. **the yields and the fit** — read: `pocket_yields` (empty → `MINE_WORKED_OUT`) and
///     `mine::fit` against the hopper's and the hold's unreserved room in every measure (a named
///     count that fits; past the fit `MINE_NO_FIT` naming the count that does; nothing fits → the
///     bound's line); blind: one cycle needing the blind room in both (`MINE_BLIND_ROOM`, the only
///     room line);
/// 11. **the chart's body caps** (blind only: the core adds the block to the row) → `CHART_FULL`.
///
/// The reserve is the caller's (the command's words, the prepare's rule), over the plan's run
/// cost and its trip home.
///
/// # Errors
///
/// The first [`Refusal`] in that order.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)] // one ordered gate list
pub(crate) fn mine_plan(
    w: &World,
    reads: &dyn SurfaceReads,
    actor: ObjId,
    rig: &RigAt,
    choice: &MineChoice,
    drill: &DrillDef,
    rates: Option<&DriveRates>,
    peeks: &mut Peeks,
) -> Result<MinePlan, Refusal> {
    let n = rig.slot;
    // 1. The stand, then the lease.
    let (view, name) = view_of(w, rig)?;
    if matches!(berth_of(w, rig.hull), Berth::UnderWay) {
        return Err(Refusal::UnderWay { name });
    }
    if where_is(w, rig.tv) == (Where::Bay { hull: rig.hull }) {
        return Err(Refusal::MineInBay { n });
    }
    let stand = stand_of(w, actor, rig)?;
    let berth = site_berth(w, &view)?;
    if let Err(bar) = lease_gate(w, &rig.record) {
        return Err(mine_lease_refusal(w, rig, bar));
    }
    let dark = || Refusal::Dark { name: name.clone() };
    // 2. The pocket.
    let (px, py) = address::pocket_of_cell(stand.cell).ok_or_else(dark)?;
    let addr = address::pocket_address(px, py).ok_or_else(dark)?;
    let letter = address::block_letter(px, py).ok_or_else(dark)?;
    let number = address::pocket_number(px, py).ok_or_else(dark)?;
    // 3. The site's ground: resolved, peeked, the way home, the footing.
    let site = chart::site_context(w, &stand.site).map_err(|_| Refusal::InstrumentsDark)?;
    let ground = match chart::peek_site(w, &stand.site, peeks) {
        SitePeek::Warm(ground) => ground,
        SitePeek::Cold => {
            return Err(Refusal::GroundCold {
                site: stand.site_name.clone(),
                entry: stand.site.clone(),
            });
        }
        SitePeek::Dark => return Err(Refusal::InstrumentsDark),
    };
    let home_cu = home_here(w, &stand, rates, &name, peeks)?;
    let surface = chart::ground_surface(&ground).ok_or(Refusal::InstrumentsDark)?;
    let footing =
        chart::footing_at(surface, stand.cell).map_err(|terrain| Refusal::MineNoFooting {
            n,
            terrain: terrain.label().replace('_', " "),
        })?;
    // 4. The licence.
    let now = w.world_ic_now();
    let lapses_ic = mine_licence(w, actor, now.at)?;
    // 5. The tenure, block-level.
    let site_def = berth.def.site.as_ref().ok_or_else(dark)?;
    mine_tenure(w, reads, actor, (&stand.site, site_def), letter, &now).map_err(
        |bar| match bar {
            TenureBar::Claimed => Refusal::Claimed,
            TenureBar::Dark(_) => dark(),
        },
    )?;
    // 6. The record: busy, or drilling.
    let record = reads.rig(rig.tv).map_err(|error| {
        tracing::warn!(tv = rig.tv.0, %error, "tacsim: a rig's excursion record does not read");
        dark()
    })?;
    if let Some(record) = &record {
        if let Some(busy) = busy_refusal(w, rig, record) {
            return Err(busy);
        }
        if let Some(running) = run_refusal(w, rig, record) {
            return Err(running);
        }
    }
    // 7. The lock: the cursor rows (one home a family), then every other rig's run.
    let mut deposits: Vec<(OreFamily, mush_industry::OreDepositId)> = Vec::new();
    let mut cursors: BTreeMap<u8, PocketCursor> = BTreeMap::new();
    for family in OreFamily::ALL {
        let receipt = crate::surface::opening::receipt(&stand.site, letter, family);
        let id = mush_industry::derive_system_deposit_id(&receipt).map_err(|_| dark())?;
        deposits.push((family, id));
        if let Some(cursor) = reads.cursor(id).map_err(|_| dark())? {
            if let Some(lock) = cursor.row(number).and_then(|row| row.lock.as_ref())
                && lock.tv != rig.tv
            {
                let holder = reads.rig(lock.tv).map_err(|_| dark())?;
                let row = LockRow {
                    deposit: id,
                    pocket: number,
                };
                if lock_live(lock, holder.as_ref(), &stand.site, &row) {
                    return Err(Refusal::Locked);
                }
            }
            cursors.insert(family.code(), cursor);
        }
    }
    for other in world_rigs(w).into_iter().filter(|other| *other != rig.tv) {
        let found = reads.rig(other).map_err(|_| dark())?;
        if found
            .and_then(|record| record.run)
            .is_some_and(|run| run.site == stand.site && run.pocket == [px, py])
        {
            return Err(Refusal::Locked);
        }
    }
    let drawn_of = |family: OreFamily| {
        cursors
            .get(&family.code())
            .and_then(|cursor| cursor.row(number))
            .map_or(0, |row| row.drawn_t)
    };
    // 8. The reading and the family — the operator's own row, never the ground.
    let row = reads.chart(actor, &stand.site).map_err(|_| dark())?;
    let read = chart::row_reads_pocket(row.as_ref(), (px, py));
    let truth = chart::pocket_at(&site, (px, py)).map_err(|_| Refusal::InstrumentsDark)?;
    let fam = choice
        .word
        .as_ref()
        .map_or_else(String::new, |word| format!(" {word}"));
    let ore_of = |family: OreFamily| tv::ore_words(crate::surface::opening::ore_good(family));
    let classes = crate::surface::reading::read_r2(&truth);
    let family = if read {
        if let Some(named) = choice.family {
            if by_family(classes, named) == Word::Barren {
                return Err(Refusal::NoOre {
                    addr,
                    ore: ore_of(named),
                });
            }
            named
        } else {
            let mut best: Option<((u32, u8), OreFamily)> = None;
            for family in OreFamily::ALL {
                let class = by_family(classes, family);
                if class == Word::Barren {
                    continue;
                }
                let left =
                    u32::from(by_family(truth.tonnes, family)).saturating_sub(drawn_of(family));
                let rank = (left, class.code());
                if best.is_none_or(|(kept, _)| rank > kept) {
                    best = Some((rank, family));
                }
            }
            let Some((_, family)) = best else {
                return Err(Refusal::AllBarren { addr });
            };
            family
        }
    } else {
        if choice.cycles.is_some_and(|m| m > 1) {
            return Err(Refusal::BlindCount { n, fam });
        }
        choice
            .family
            .or_else(|| {
                address::field_of_pocket((px, py))
                    .and_then(|field| richest_read(&site, row.as_ref(), field))
            })
            .unwrap_or(OreFamily::Metals)
    };
    let blind = !read;
    let also: Vec<(OreFamily, Word)> = if blind {
        Vec::new()
    } else {
        OreFamily::ALL
            .into_iter()
            .filter(|other| *other != family)
            .map(|other| (other, by_family(classes, other)))
            .filter(|(_, class)| *class != Word::Barren)
            .collect()
    };
    // 8b. The public pit (blind only): one clause, never a refusal.
    let pit = if blind {
        crate::port::site_map::pit_at(reads, &stand.site, &ground, (px, py))
    } else {
        None
    };
    // 9. The truth, server-side.
    let class = by_family(classes, family);
    let pocket_t = by_family(truth.tonnes, family);
    let class_t = u8::try_from(class_yield(drill, class)).map_err(|_| dark())?;
    let drawn = drawn_of(family);
    let deposit = deposits
        .iter()
        .find(|(f, _)| *f == family)
        .map(|(_, id)| *id)
        .ok_or_else(dark)?;
    let mut lock_rows: Vec<LockRow> = Vec::new();
    for (other, id) in &deposits {
        if by_family(truth.tonnes, *other) > 0 && reads.deposit(*id).map_err(|_| dark())?.is_some()
        {
            lock_rows.push(LockRow {
                deposit: *id,
                pocket: number,
            });
        }
    }
    if lock_rows.len() > MAX_LOCK_ROWS {
        tracing::error!(
            site = %stand.site,
            %addr,
            rows = lock_rows.len(),
            "tacsim: a pocket carries more ore families than the geology allows (SM.1's two); \
             mining it is refused"
        );
        return Err(dark());
    }
    let source = if class_t > 0 && pocket_t > 0 {
        let Some(vault) = crate::port::site::block_vault(w, berth.map, letter) else {
            tracing::error!(
                site = %stand.site,
                block = %letter,
                "tacsim: an ore-bearing block's vault is gone; mining it is refused (staff repair \
                 the site map)"
            );
            return Err(dark());
        };
        Some(vault.0)
    } else {
        None
    };
    // 10. The yields and the fit.
    let good = CanonicalId::new(crate::surface::opening::ore_good(family)).map_err(|_| dark())?;
    let goods = w
        .module_state::<TacsimStore>()
        .and_then(TacsimStore::goods)
        .ok_or_else(dark)?;
    let good_def = goods.good(&good).ok_or_else(dark)?;
    let pool_unit = tv::pool_unit(w);
    let hopper = crate::port::vehicle::hopper_of(w, rig.tv).ok_or_else(dark)?;
    let read_pool = |endpoint: EndpointRef, good: &CanonicalId| reads.pool(endpoint, good);
    let contents = tv::hopper_contents_with(w, rig.tv, pool_unit.as_ref(), &read_pool).map_err(
        |error| {
            tracing::warn!(tv = rig.tv.0, %error, "tacsim: a mining plan could not read the hopper");
            dark()
        },
    )?;
    let hopper_room = {
        let comps = tv::hopper_compartments(w, rig.tv, pool_unit.as_ref()).ok_or_else(dark)?;
        let mut merged = crate::port::capacity::Compartment {
            id: CanonicalId::new("hopper").map_err(|_| dark())?,
            accepts: std::collections::BTreeSet::new(),
            limits: BTreeMap::new(),
        };
        for comp in comps {
            merged.accepts.extend(comp.accepts);
            for (measure, limit) in comp.limits {
                let slot = merged.limits.entry(measure).or_insert(0);
                *slot = slot.saturating_add(limit);
            }
        }
        let load = crate::port::capacity::load(goods, &merged, &contents).map_err(|_| dark())?;
        merged
            .limits
            .iter()
            .map(|(measure, limit)| {
                (
                    measure.clone(),
                    limit.saturating_sub(load.get(measure).copied().unwrap_or(0)),
                )
            })
            .collect::<Load>()
    };
    let mass = CanonicalId::new(crate::definitions::logistics::MASS_KG).map_err(|_| dark())?;
    let hopper_cap_kg = tv::hopper_compartments(w, rig.tv, pool_unit.as_ref())
        .map(|comps| {
            comps
                .iter()
                .filter_map(|comp| comp.limits.get(&mass).copied())
                .sum::<u64>()
        })
        .unwrap_or(0);
    let hopper_before_kg =
        hopper_cap_kg.saturating_sub(hopper_room.get(&mass).copied().unwrap_or(0));
    let hull_rigs: Vec<ObjId> = tv::live_on_hull(w, rig.hull)
        .iter()
        .map(|lease| lease.tv)
        .collect();
    let hold_room =
        crate::port::ship::hold_unreserved(w, &view, &hull_rigs, &read_pool, &|tv| reads.rig(tv))
            .ok_or_else(dark)?;
    let mut per_unit: Vec<(CanonicalId, u64)> = Vec::new();
    for measure in hopper_room.keys().chain(hold_room.keys()) {
        if per_unit.iter().any(|(m, _)| m == measure) {
            continue;
        }
        per_unit.push((
            measure.clone(),
            good_def.per_unit(measure).ok_or_else(dark)?,
        ));
    }
    let (yields, reserve_t, cycles): (Vec<u32>, Vec<u32>, u8) = if blind {
        let room = blind_room_t(drill);
        if fit(&[room], &per_unit, &hopper_room, &hold_room).0 == 0 {
            return Err(Refusal::BlindRoom { room });
        }
        (Vec::new(), vec![room], 1)
    } else {
        let all = pocket_yields(
            u32::from(class_t),
            u32::from(pocket_t).saturating_sub(drawn),
        );
        let ore = ore_of(family);
        if all.is_empty() {
            return Err(Refusal::WorkedOut { addr, ore });
        }
        let (n_fit, bound) = fit(&all, &per_unit, &hopper_room, &hold_room);
        let zero = |bound: Bound| match bound {
            Bound::Hopper => Refusal::HopperFull {
                n,
                hopper: tv::hopper_text(hopper_before_kg, hopper_cap_kg),
            },
            Bound::Hold => Refusal::HoldFull { name: name.clone() },
            Bound::WorkedOut | Bound::Cap => Refusal::WorkedOut {
                addr: addr.clone(),
                ore: ore.clone(),
            },
        };
        let cycles = match choice.cycles {
            Some(m) if m <= n_fit => m,
            Some(_) | None if n_fit == 0 => return Err(zero(bound)),
            Some(m) => {
                return Err(Refusal::NoFit {
                    n,
                    m,
                    fit: n_fit,
                    bound,
                    fam,
                });
            }
            None => n_fit,
        };
        let taken: Vec<u32> = all.into_iter().take(usize::from(cycles)).collect();
        (taken.clone(), taken, cycles)
    };
    let reserve_t: Vec<u8> = reserve_t
        .into_iter()
        .map(u8::try_from)
        .collect::<Result<_, _>>()
        .map_err(|_| dark())?;
    let run_t: u64 = reserve_t.iter().map(|t| u64::from(*t)).sum();
    let hold_after = room_less(&hold_room, &per_unit, run_t);
    let mass_per = good_def.per_unit(&mass).unwrap_or(0);
    let shown_t: u64 = yields.iter().map(|t| u64::from(*t)).sum();
    let hopper_after_kg = hopper_before_kg.saturating_add(shown_t.saturating_mul(mass_per));
    let mut outstanding: Vec<(ObjId, Option<(u32, u64)>)> = Vec::new();
    for tv in &hull_rigs {
        let found = reads.rig(*tv).map_err(|_| dark())?;
        outstanding.push((
            *tv,
            found
                .and_then(|record| record.run)
                .map(|run| (run.ordinal, run.outstanding_t())),
        ));
    }
    // 11. The chart's body caps (blind: the core adds the block to the row).
    if blind {
        let mut rows = Vec::new();
        for other in chart::body_sites(w, &site.spec.body) {
            if let Some(found) = reads.chart(actor, &other).map_err(|_| dark())? {
                rows.push(found);
            }
        }
        if !body_admits(&rows, &stand.site, &[], &[letter.to_string()]) {
            return Err(Refusal::ChartFull {
                body: crate::port::site::body_name(&site.spec.body),
            });
        }
    }
    let secs = cycle_secs(drill, footing).ok_or_else(dark)?;
    Ok(MinePlan {
        site: stand.site.clone(),
        body: site.spec.body.clone(),
        stand: [stand.cell.0, stand.cell.1],
        pocket: [px, py],
        number,
        letter,
        addr,
        family,
        good,
        deposit,
        source,
        hopper: hopper.0,
        cycles,
        yields,
        reserve_t,
        class_t,
        pocket_t,
        drawn,
        class: if blind { Word::Barren } else { class },
        also,
        blind,
        pit,
        secs,
        cost_cu: drill.energy_cu,
        home_cu,
        lock_rows,
        lapses_ic: Some(lapses_ic),
        hold_after,
        hopper_before_kg,
        hopper_after_kg,
        hopper_cap_kg,
        outstanding,
    })
}

/// **`+tv/<n> mine [family] [cycles] [go]`** (alias `drill`; doc 20 §1 beat 8, §6, §10; cut SM.7,
/// the brief's 3.4; D-20-9's `go` rule): the drill and the battery (a run spends: `TV_DARK`
/// without either), the rig's rates; [`mine_plan`] (a cold site or home asked for and refused
/// `GROUND_COLD`); the reserve over the run's cost and the trip home (`RESERVE_LOW`); then the
/// preview — `MINE_PREVIEW` from the operator's own reading, or `MINE_PREVIEW_BLIND` — or, with
/// `go`, the order written ([`ActOrders`]) and ONE `tacsim_surface_act` submission with `act:
/// mine` keyed `tacsim.surface/{tv}/{ordinal}/act`, and `MINE_FILED` (`CORE_FILED` blind).
/// `words` are the family and count as typed (the preview's `{arg}`).
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line.
#[allow(clippy::too_many_lines)] // the plan, the reserve, then the preview or the go
pub(crate) fn mine_cmd(
    ctx: &mut Ctx<'_>,
    rig: &RigAt,
    choice: &MineChoice,
    words: &str,
    go: bool,
) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let handles = ctx
        .w
        .module_state::<PortHandles>()
        .cloned()
        .ok_or_else(|| CmdError::msg(tv::TV_UNWIRED))?;
    let Some(action) = handles.surface_act.clone() else {
        return Err(CmdError::msg(tv::TV_UNWIRED));
    };
    // Doc 20 cut SM.11 (D8): a live plan holds its rig.
    plan_holds(ctx, rig)?;
    let name = name_of(ctx.w, rig.hull);
    let dark = || CmdError::msg(tv::TV_DARK.replace("{name}", &name));
    let tools = handles.surface_tools.clone().ok_or_else(dark)?;
    let Some(drill) = rig_drill_id(ctx.w, rig.tv).and_then(|id| drill_of(&tools, &id).copied())
    else {
        tracing::warn!(tv = rig.tv.0, "tacsim: a rig carries no readable drill");
        return Err(dark());
    };
    // A run spends (`needs_battery(Mine, …)`): no battery row, no run.
    let Some(battery) = rig_battery(ctx.w, &handles, rig) else {
        return Err(dark());
    };
    let rates = rig_rates(ctx.w, rig.tv, Some(&battery));
    let mut peeks = Peeks::default();
    let planned = mine_plan(
        ctx.w,
        &*ctx,
        actor,
        rig,
        choice,
        &drill,
        rates.as_ref(),
        &mut peeks,
    );
    let plan = match planned {
        Ok(plan) => plan,
        Err(refusal) => {
            want_cold(ctx, rig, &refusal);
            return Err(CmdError::msg(refusal.line()));
        }
    };
    let w: &World = ctx.w;
    let record = record_of(ctx, &handles, rig.tv, Some(&battery)).map_err(|()| dark())?;
    let now_ic = w.world_ic_now().at;
    let run_cost = plan.run_cost_cu().ok_or_else(dark)?;
    let run_secs = plan.run_secs().ok_or_else(dark)?;
    let (now, after, home) = reserve_words(
        &record,
        now_ic,
        false,
        (run_cost, plan.home_cu),
        &battery,
        rig.slot,
    )?;
    let n = rig.slot.to_string();
    let ore = tv::ore_words(plan.good.as_str());
    let arg = if words.is_empty() {
        String::new()
    } else {
        format!(" {words}")
    };
    if !go {
        let real =
            match crate::port::voyage::real_until(w, now_ic.saturating_add(i64::from(run_secs))) {
                Some(real) => format!(", about {}", tv::span_words(real)),
                None => String::new(),
            };
        let line = if plan.blind {
            let pit = match plan.pit {
                Some('v') => tv::PIT_WORKED,
                Some(_) => tv::PIT_WORKED_OUT,
                None => "",
            };
            tv::MINE_PREVIEW_BLIND
                .replace("{n}", &n)
                .replace("{mark}", &rig.mark)
                .replace("{addr}", &plan.addr)
                .replace("{ore}", &ore)
                .replace("{secs}", &run_secs.to_string())
                .replace("{real}", &real)
                .replace(
                    "{room}",
                    &plan.reserve_t.first().copied().unwrap_or(0).to_string(),
                )
                .replace("{now}", &now.to_string())
                .replace("{after}", &after.to_string())
                .replace("{home}", &home.to_string())
                .replace("{pit}", pit)
                .replace("{arg}", &arg)
        } else {
            let cycles = if plan.cycles == 1 {
                tv::ONE_CYCLE.to_owned()
            } else {
                tv::SOME_CYCLES.replace("{c}", &plan.cycles.to_string())
            };
            let yields: Vec<String> = plan.yields.iter().map(u32::to_string).collect();
            let total: u32 = plan.yields.iter().sum();
            let mass = CanonicalId::new(crate::definitions::logistics::MASS_KG).ok();
            let room_kg = mass
                .and_then(|mass| plan.hold_after.get(&mass).copied())
                .unwrap_or(0);
            let also: String = plan
                .also
                .iter()
                .map(|(family, class)| {
                    tv::MINE_ALSO
                        .replace("{class}", class.name())
                        .replace(
                            "{ore}",
                            &tv::ore_words(crate::surface::opening::ore_good(*family)),
                        )
                        .replace("{n}", &n)
                        .replace("{word}", family_word(*family))
                })
                .collect();
            tv::MINE_PREVIEW
                .replace("{n}", &n)
                .replace("{mark}", &rig.mark)
                .replace("{addr}", &plan.addr)
                .replace("{class}", plan.class.name())
                .replace("{ore}", &ore)
                .replace("{cycles}", &cycles)
                .replace("{yields}", &yields.join("/"))
                .replace("{total}", &total.to_string())
                .replace("{secs}", &run_secs.to_string())
                .replace("{real}", &real)
                .replace("{hop0}", &(plan.hopper_before_kg / 1_000).to_string())
                .replace("{hop1}", &(plan.hopper_after_kg / 1_000).to_string())
                .replace("{cap}", &(plan.hopper_cap_kg / 1_000).to_string())
                .replace("{room}", &(room_kg / 1_000).to_string())
                .replace("{now}", &now.to_string())
                .replace("{after}", &after.to_string())
                .replace("{home}", &home.to_string())
                .replace("{also}", &also)
                .replace("{arg}", &arg)
        };
        crate::port::coop::say(ctx, line);
        return Ok(());
    }
    // `go`: the order, then ONE submission as the actor.
    let Some(ordinal) = record.ordinal.checked_add(1) else {
        tracing::error!(tv = rig.tv.0, "tacsim: a rig's act ordinal is exhausted");
        return Err(dark());
    };
    let Some(key) = act_key(rig.tv, ordinal) else {
        tracing::error!(tv = rig.tv.0, "tacsim: a rig act's key did not form");
        return Err(dark());
    };
    let when = tv::due_words(w, now_ic.saturating_add(i64::from(plan.secs)));
    let line = if plan.blind {
        tv::CORE_FILED
    } else {
        tv::MINE_FILED
    }
    .replace("{n}", &n)
    .replace("{addr}", &plan.addr)
    .replace("{when}", &when);
    put_order(
        ctx,
        rig.tv,
        ActOrder::Mine(Box::new(MineOrder {
            actor,
            ordinal,
            site: plan.site.clone(),
            choice: choice.clone(),
            plan,
        })),
    );
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({
            "tv": rig.tv.0.to_string(),
            "act": wire_word(ActKind::Mine, false),
            "ordinal": ordinal,
        }),
        key,
    );
    crate::port::coop::say(ctx, line);
    Ok(())
}

/// **`+tv/<n> mine stop`** (doc 20 §6: "lets the running cycle complete and yield and drops the
/// cycles after it at no cost"; cut SM.7, the brief's 3.4 and decision R7): committed at once, with
/// no `go` — a control on a committed run that only drops future cycles. The record's run: none →
/// `MINE_NOT_RUNNING`; already stopping → `MINE_STOPPING_ALREADY` (nothing queued); else ONE
/// `tacsim_surface_act` with `act: mine_stop` and the run's ordinal, keyed
/// `tacsim.surface/{tv}/{ordinal}/stop`, and `MINE_STOPPING`. Never a refund of the running cycle.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line.
pub(crate) fn mine_stop_cmd(ctx: &mut Ctx<'_>, rig: &RigAt) -> Result<(), CmdError> {
    let handles = ctx
        .w
        .module_state::<PortHandles>()
        .cloned()
        .ok_or_else(|| CmdError::msg(tv::TV_UNWIRED))?;
    let Some(action) = handles.surface_act.clone() else {
        return Err(CmdError::msg(tv::TV_UNWIRED));
    };
    // Doc 20 cut SM.11 (D8): a live plan holds its rig — its stop too.
    plan_holds(ctx, rig)?;
    let name = name_of(ctx.w, rig.hull);
    let dark = || CmdError::msg(tv::TV_DARK.replace("{name}", &name));
    let battery = rig_battery(ctx.w, &handles, rig);
    let record = record_of(ctx, &handles, rig.tv, battery.as_ref()).map_err(|()| dark())?;
    let n = rig.slot.to_string();
    let Some(run) = record.run else {
        return Err(CmdError::msg(tv::MINE_NOT_RUNNING.replace("{n}", &n)));
    };
    if run.stop {
        return Err(CmdError::msg(tv::MINE_STOPPING_ALREADY.replace("{n}", &n)));
    }
    let Some(key) = stop_key(rig.tv, run.ordinal) else {
        tracing::error!(tv = rig.tv.0, "tacsim: a run's stop key did not form");
        return Err(dark());
    };
    let line = tv::MINE_STOPPING
        .replace("{n}", &n)
        .replace(
            "{addr}",
            &address::pocket_address(run.pocket[0], run.pocket[1]).unwrap_or_default(),
        )
        .replace("{when}", &tv::due_words(ctx.w, run.due_ic));
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({
            "tv": rig.tv.0.to_string(),
            "act": "mine_stop",
            "ordinal": run.ordinal,
        }),
        key,
    );
    crate::port::coop::say(ctx, line);
    Ok(())
}

// ---- doc 20 cut SM.11: the fill plan's verbs ---------------------------------------------------

/// **What a plan's preview and its filing read** (doc 20 cut SM.11; the brief's 3.5): the words
/// the lines name and the figures beside the compile.
#[derive(Clone, Debug)]
pub(crate) struct PlanFacts {
    /// The rig's record (fresh when absent).
    pub record: RigRecord,
    /// The hull's name.
    pub name: String,
    /// The site's entry.
    pub site: CanonicalId,
    /// The site's name.
    pub site_name: String,
    /// The hold's unreserved room in whole tonnes of the compiled plan's ore, in every measure
    /// (doc 20 cut SM.11r; 0 when the compile refused, which no text reads).
    pub room_t: u64,
    /// The slots of the hull's other rigs working a plan live for their lessee.
    pub shared: Vec<u8>,
}

/// **Gather a plan's compile over the world** (doc 20 cut SM.11; the brief's 3.5 — one function,
/// the command and the start's offer both call it, `mine_plan`'s precedent), cheap first: the
/// layer and the view; under way; the hull on a site's pad (`LAUNCH_NOT_SITE`); the actor aboard
/// (`TV_NOT_ABOARD`: filing is a hand act); the record (unreadable → `TV_DARK`); a plan live for
/// the actor (`plan_filed`); the site's ground and home warm (`GROUND_COLD`, `INSTRUMENTS_DARK`);
/// the tools rows and the rig's rates (`TV_DARK`); the hold's room (`TV_DARK`); the hopper; the
/// site's ground resolved; the author's chart row; the rig's cell (the ramp's from the bay) — then
/// [`crate::surface::plan::compile`] over the author's licence, lease, tenure and readings. `Err` is
/// the world's refusal; `Ok` carries the compile's own answer.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)] // one ordered gather
pub(crate) fn plan_ask(
    w: &World,
    reads: &dyn SurfaceReads,
    actor: ObjId,
    rig: &RigAt,
    list: &str,
    family: Option<OreFamily>,
    tools: &BTreeMap<CanonicalId, ToolDef>,
    peeks: &mut Peeks,
    now_ic: i64,
) -> Result<
    (
        Result<crate::surface::plan::Compiled, crate::surface::plan::PlanRefusal>,
        PlanFacts,
    ),
    Refusal,
> {
    let (view, name) = view_of(w, rig)?;
    if matches!(berth_of(w, rig.hull), Berth::UnderWay) {
        return Err(Refusal::UnderWay { name });
    }
    let berth = site_berth(w, &view)?;
    if !crate::port::voyage::aboard(w, rig.hull, actor) {
        return Err(Refusal::NotAboard { name });
    }
    let dark = || Refusal::Dark { name: name.clone() };
    let record = reads
        .rig(rig.tv)
        .map_err(|error| {
            tracing::warn!(tv = rig.tv.0, %error, "tacsim: a rig's excursion record does not read");
            dark()
        })?
        .unwrap_or_else(|| RigRecord::fresh_unread(rig.tv));
    if let Some(plan) = record.live_plan_for(actor) {
        return Err(Refusal::PlanFiled {
            n: rig.slot,
            no: plan.no,
        });
    }
    let cold = || Refusal::GroundCold {
        site: berth.name.clone(),
        entry: berth.entry.clone(),
    };
    let ground = match chart::peek_site(w, &berth.entry, peeks) {
        SitePeek::Warm(ground) => ground,
        SitePeek::Cold => return Err(cold()),
        SitePeek::Dark => return Err(Refusal::InstrumentsDark),
    };
    let ramp_cell = site_cell(berth.ramp.x_cm, berth.ramp.y_cm);
    let field = match crate::surface::peek_home(w, &berth.entry, ramp_cell, peeks) {
        HomePeek::Warm(field) => field,
        HomePeek::Cold => return Err(cold()),
        HomePeek::Dark => return Err(Refusal::InstrumentsDark),
    };
    let battery = rig_battery_id(w, rig.tv)
        .and_then(|id| battery_of(tools, &id).copied())
        .ok_or_else(dark)?;
    let scanner = rig_scanner_id(w, rig.tv)
        .and_then(|id| scanner_of(tools, &id).copied())
        .ok_or_else(dark)?;
    let drill = rig_drill_id(w, rig.tv)
        .and_then(|id| drill_of(tools, &id).copied())
        .ok_or_else(dark)?;
    let rates = rig_rates(w, rig.tv, Some(&battery)).ok_or_else(dark)?;
    let hull_rigs: Vec<ObjId> = tv::live_on_hull(w, rig.hull)
        .iter()
        .map(|lease| lease.tv)
        .collect();
    let read_pool = |endpoint: EndpointRef, good: &CanonicalId| reads.pool(endpoint, good);
    let read_rig = |tv: ObjId| reads.rig(tv);
    // Doc 20 cut SM.11r (doc 20a §10, SM.11r-2): the hold's room in every measure, read once and
    // turned into tonnes of each family's ore by the interpreter's own rule, so the compile and the
    // preview never read the mass entry alone (`hold_unreserved_kg`) while the packed volume binds.
    let goods = w
        .module_state::<crate::TacsimStore>()
        .and_then(crate::TacsimStore::goods)
        .ok_or_else(dark)?;
    let room =
        crate::port::ship::hold_unreserved_with(goods, w, &view, &hull_rigs, &read_pool, &read_rig)
            .ok_or_else(dark)?;
    let rooms: BTreeMap<OreFamily, u64> = OreFamily::ALL
        .iter()
        .map(|family| {
            crate::surface::plan::room_in_tonnes_of(goods, &room, *family).map(|t| (*family, t))
        })
        .collect::<Option<_>>()
        .ok_or_else(dark)?;
    let room_of = |family: OreFamily| rooms.get(&family).copied().unwrap_or(0);
    let pool_unit = tv::pool_unit(w);
    let (_, hopper_cap_kg) =
        tv::hopper_load_with(w, rig.tv, pool_unit.as_ref(), &read_pool).map_err(|_| dark())?;
    let site = chart::site_context(w, &berth.entry).map_err(|_| Refusal::InstrumentsDark)?;
    let row = reads.chart(actor, &berth.entry).map_err(|_| dark())?;
    let start_cell = match where_is(w, rig.tv) {
        Where::Bay { hull } if hull == rig.hull => ramp_cell,
        Where::Ground {
            domain, x_cm, y_cm, ..
        } if domain == berth.domain => site_cell(x_cm, y_cm),
        Where::Bay { .. } | Where::Ground { .. } | Where::Elsewhere => {
            return Err(Refusal::Elsewhere { n: rig.slot, name });
        }
    };
    let site_def = berth.def.site.as_ref().ok_or_else(dark)?;
    let now = w.world_ic_now();
    let tenure =
        |letter: char| mine_tenure(w, reads, actor, (&berth.entry, site_def), letter, &now);
    let cache = std::cell::RefCell::new(BTreeMap::new());
    let classes = |pocket: (i64, i64)| {
        crate::surface::plan::read_classes(&site, row.as_ref(), pocket, &mut cache.borrow_mut())
    };
    let fields = |field: (i64, i64)| crate::surface::plan::read_field(&site, row.as_ref(), field);
    let home = |stand: (i64, i64)| drive::home_cu_to(&field, stand, &rates);
    // Doc 20 cut SM.12c-lane follow-up: the walk's own `pit_out`, so the compile refuses a list
    // with nothing left to mine (`PlanRefusal::NothingLeft`) on the public worked-out pits.
    let pit_out = |pocket: (i64, i64), family: OreFamily| {
        crate::port::site_map::pit_exhausted(reads, &berth.entry, &ground, pocket, family)
    };
    let compiled = crate::surface::plan::compile(&crate::surface::plan::CompileInput {
        row: row.as_ref(),
        list,
        family,
        licence: licence_verdict(w, actor, now_ic),
        lease: lease_gate(w, &rig.record),
        tenure: &tenure,
        classes: &classes,
        field_readings: &fields,
        start_cell,
        home: &home,
        battery: &battery,
        scanner: &scanner,
        drill: &drill,
        hold_room_t: &room_of,
        hopper_t: hopper_cap_kg / 1_000,
        pit_out: &pit_out,
    });
    let mut shared: Vec<u8> = Vec::new();
    for lease in tv::live_on_hull(w, rig.hull) {
        if lease.tv == rig.tv {
            continue;
        }
        if reads
            .rig(lease.tv)
            .ok()
            .flatten()
            .is_some_and(|other| other.live_plan_for(lease.pilot).is_some())
        {
            shared.push(lease.slot);
        }
    }
    shared.sort_unstable();
    // The preview prints it only over a compiled plan: the frozen family's room.
    let room_t = compiled
        .as_ref()
        .map_or(0, |compiled| room_of(compiled.family));
    Ok((
        compiled,
        PlanFacts {
            record,
            name,
            site: berth.entry.clone(),
            site_name: berth.name.clone(),
            room_t,
            shared,
        },
    ))
}

/// **A compile's refusal as the player hears it** (the brief's 3.5 texts): the two §5 lines byte
/// for byte behind the address (`PLAN_REFUSED_AT`), the licence through `+licence`'s own line, the
/// lease through the mining lines.
pub(super) fn plan_refusal_line(
    w: &World,
    actor: ObjId,
    rig: &RigAt,
    name: &str,
    refusal: &crate::surface::plan::PlanRefusal,
) -> String {
    use crate::surface::plan::{MAX_PLAN_ADDRESSES, MAX_PLAN_STEPS, PlanRefusal};
    let n = rig.slot.to_string();
    let at = |addr: &str, line: &str| {
        tv::PLAN_REFUSED_AT
            .replace("{addr}", addr)
            .replace("{line}", line)
    };
    match refusal {
        PlanRefusal::Usage => tv::PLAN_USAGE.to_owned(),
        PlanRefusal::TooMany => tv::PLAN_TOO_MANY.replace("{max}", &MAX_PLAN_ADDRESSES.to_string()),
        PlanRefusal::Twice { addr } => tv::PLAN_TWICE.replace("{addr}", addr),
        PlanRefusal::NoReading { addr } => at(addr, tv::NO_READING),
        PlanRefusal::Barren { addr, family } => tv::PLAN_BARREN
            .replace("{addr}", addr)
            .replace("{family}", family.label()),
        PlanRefusal::Licence { why } => licence_end_line(w, actor, *why),
        PlanRefusal::Lease { bar } => mine_lease_refusal(w, rig, *bar).line(),
        PlanRefusal::Claimed { addr } => at(addr, tv::MINE_CLAIMED),
        PlanRefusal::Dark { addr } => at(addr, tv::PLAN_DARK_LINE),
        PlanRefusal::TooFar { addr } => at(addr, &tv::PLAN_TOO_FAR_LINE.replace("{n}", &n)),
        PlanRefusal::TooLong { hours } => tv::PLAN_TOO_LONG.replace("{hours}", &hours.to_string()),
        PlanRefusal::TooManySteps => {
            tv::PLAN_TOO_MANY_STEPS.replace("{max}", &MAX_PLAN_STEPS.to_string())
        }
        PlanRefusal::HoldFull => tv::PLAN_HOLD_FULL.replace("{name}", name),
        PlanRefusal::NothingLeft { family } => {
            tv::PLAN_NOTHING_LEFT.replace("{family}", family.label())
        }
    }
}

/// `{hours}` for an estimate: whole IC hours, rounded up.
pub(super) fn hours_up(secs: u64) -> u64 {
    secs.div_ceil(3_600)
}

/// **`+tv/<n> plan <list> [family=<good>] [go]`** (doc 20 §1 beat 11, §8 "Leaving it running", §10;
/// cut SM.11, the brief's 3.5; lead rulings 1 and 2, decision D10): [`plan_ask`] (the same gather
/// the start's offer runs), then the compile's answer — a refusal's line, or bare the preview
/// (`PLAN_PREVIEW_HEAD`, one `PLAN_PREVIEW_STEP` a step, `PLAN_PREVIEW_TAIL`), nothing written;
/// with `go`, the filing written as an `ActOrders` entry (`ActOrder::Plan`) and ONE
/// `tacsim_surface_act` with `act: plan_file` keyed `tacsim.surface/{tv}/plan/{no}/file`, and
/// `PLAN_FILED`.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line.
#[allow(clippy::too_many_lines)] // the gather, then the preview or the go
pub(crate) fn plan_cmd(
    ctx: &mut Ctx<'_>,
    rig: &RigAt,
    list: &str,
    family: Option<OreFamily>,
    family_word: Option<&str>,
    go: bool,
) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let handles = ctx
        .w
        .module_state::<PortHandles>()
        .cloned()
        .ok_or_else(|| CmdError::msg(tv::TV_UNWIRED))?;
    if handles.surface_act.is_none() {
        return Err(CmdError::msg(tv::TV_UNWIRED));
    }
    let name = name_of(ctx.w, rig.hull);
    let dark = || CmdError::msg(tv::TV_DARK.replace("{name}", &name));
    let tools = handles.surface_tools.clone().ok_or_else(dark)?;
    let now_ic = ctx.w.world_ic_now().at;
    let mut peeks = Peeks::default();
    let asked = plan_ask(
        ctx.w, &*ctx, actor, rig, list, family, &tools, &mut peeks, now_ic,
    );
    let (compiled, facts) = match asked {
        Ok(asked) => asked,
        Err(refusal) => {
            want_cold(ctx, rig, &refusal);
            return Err(CmdError::msg(refusal.line()));
        }
    };
    let w: &World = ctx.w;
    let compiled = compiled
        .map_err(|refusal| CmdError::msg(plan_refusal_line(w, actor, rig, &name, &refusal)))?;
    let n = rig.slot.to_string();
    let ore = tv::ore_words(crate::surface::opening::ore_good(compiled.family));
    let typed_list = {
        let mut words = compiled.list.join(",");
        if let Some(word) = family_word {
            words.push_str(" family=");
            words.push_str(word);
        }
        words
    };
    if !go {
        let real = match crate::port::voyage::real_until(
            w,
            now_ic.saturating_add(crate::surface::plan::PLAN_BOUND_IC_SECS),
        ) {
            Some(real) => format!(", about {}", tv::span_words(real)),
            None => String::new(),
        };
        let mut lines = vec![
            tv::PLAN_PREVIEW_HEAD
                .replace("{n}", &n)
                .replace("{mark}", &rig.mark)
                .replace("{site}", &facts.site_name)
                .replace("{ore}", &ore)
                .replace("{real}", &real),
        ];
        for (k, step) in compiled.steps.iter().enumerate() {
            let secs = compiled.step_secs.get(k).copied().unwrap_or(0);
            let hours = if secs > 0 {
                tv::PLAN_STEP_HOURS.replace("{h}", &hours_up(secs).to_string())
            } else {
                String::new()
            };
            let far = compiled
                .far
                .iter()
                .find(|(at, _)| usize::from(*at) == k)
                .map_or_else(String::new, |(_, count)| {
                    tv::PLAN_PREVIEW_FAR.replace("{k}", &count.to_string())
                });
            lines.push(
                tv::PLAN_PREVIEW_STEP
                    .replace("{k}", &k.saturating_add(1).to_string())
                    .replace("{words}", &crate::surface::plan::step_words(step, true))
                    .replace("{hours}", &hours)
                    .replace("{far}", &far),
            );
        }
        let shared: String = facts
            .shared
            .iter()
            .map(|m| tv::PLAN_SHARED.replace("{m}", &m.to_string()))
            .collect();
        lines.push(
            tv::PLAN_PREVIEW_TAIL
                .replace("{name}", &facts.name)
                .replace("{room}", &facts.room_t.to_string())
                .replace("{shared}", &shared)
                .replace("{hours}", &hours_up(compiled.est_secs).to_string())
                .replace("{n}", &n)
                .replace("{list}", &typed_list),
        );
        for line in lines {
            crate::port::coop::say(ctx, line);
        }
        return Ok(());
    }
    // `go`: the order, then ONE submission as the actor (`file_compiled`, doc 20 cut SM.12b).
    let (no, when) = file_compiled(ctx, rig, list, family, compiled, &facts)?;
    crate::port::coop::say(
        ctx,
        tv::PLAN_FILED
            .replace("{n}", &n)
            .replace("{no}", &no.to_string())
            .replace("{when}", &when),
    );
    Ok(())
}

/// **File a compiled plan** (doc 20 cut SM.12b, factored out of [`plan_cmd`]'s `go` half so the
/// yard's dispatcher files through the very path `+tv/plan … go` uses; 12b-T6): the action from
/// `PortHandles.surface_act` (`TV_UNWIRED`), the plan number `facts.record.plans + 1` (overflow →
/// `TV_DARK`), its filing key (`TV_DARK`), the `when` of its bound, the RAM `ActOrder::Plan`
/// (which `fill_fixed`'s re-compile equality then requires), and ONE `submit_authoritative` of
/// `{ tv, act: "plan_file", ordinal }` as `ctx.actor`. It says nothing: the caller prints
/// `PLAN_FILED` from its answer (`(no, when)`).
///
/// # Errors
///
/// `TV_UNWIRED` or `TV_DARK`, as a [`CmdError`].
pub(crate) fn file_compiled(
    ctx: &mut Ctx<'_>,
    rig: &RigAt,
    list: &str,
    family: Option<OreFamily>,
    compiled: crate::surface::plan::Compiled,
    facts: &PlanFacts,
) -> Result<(u32, String), CmdError> {
    let actor = ctx.actor;
    let Some(action) = ctx
        .w
        .module_state::<PortHandles>()
        .and_then(|handles| handles.surface_act.clone())
    else {
        return Err(CmdError::msg(tv::TV_UNWIRED));
    };
    let name = name_of(ctx.w, rig.hull);
    let dark = || CmdError::msg(tv::TV_DARK.replace("{name}", &name));
    let now_ic = ctx.w.world_ic_now().at;
    let ordinal = facts.record.ordinal;
    let Some(no) = facts.record.plans.checked_add(1) else {
        return Err(dark());
    };
    let Some(key) = plan_file_key(rig.tv, no) else {
        tracing::error!(tv = rig.tv.0, "tacsim: a plan's filing key did not form");
        return Err(dark());
    };
    let when = tv::due_words(
        ctx.w,
        now_ic.saturating_add(crate::surface::plan::PLAN_BOUND_IC_SECS),
    );
    put_order(
        ctx,
        rig.tv,
        ActOrder::Plan(Box::new(PlanOrder {
            actor,
            ordinal,
            no,
            list: list.to_owned(),
            family,
            compiled,
        })),
    );
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({
            "tv": rig.tv.0.to_string(),
            "act": "plan_file",
            "ordinal": ordinal.max(1),
        }),
        key,
    );
    Ok((no, when))
}

/// The words a plan's lines name (`{n}`, `{name}`, `{site}`, `{ramp}`, `{body}`).
pub(crate) struct PlanWords {
    /// The slot.
    pub n: String,
    /// The hull's name.
    pub name: String,
    /// The site's name.
    pub site: String,
    /// The egress's label.
    pub ramp: String,
    /// The site's body's name.
    pub body: String,
}

impl PlanWords {
    /// The words of `plan` on `w`.
    pub(crate) fn of(w: &World, plan: &FillPlan) -> PlanWords {
        let body = chart::lane_sites(w)
            .into_iter()
            .find(|site| site.entry == plan.site)
            .map_or_else(String::new, |site| crate::port::site::body_name(&site.body));
        PlanWords {
            n: plan.slot.to_string(),
            name: name_of(w, plan.hull),
            site: site_name(w, &plan.site),
            ramp: ramp_label(w, plan.hull),
            body,
        }
    }
}

/// **Why a plan parked or ended, in a sentence** (the brief's 3.5 reason words): each ends in a
/// full stop; the licence is `+licence`'s own line.
#[must_use]
pub(crate) fn end_why(w: &World, author: ObjId, why: PlanEnd, words: &PlanWords) -> String {
    match why {
        PlanEnd::WorkedOut => tv::END_WORKED_OUT.to_owned(),
        PlanEnd::HoldFull => tv::END_HOLD_FULL.replace("{name}", &words.name),
        PlanEnd::Bound => tv::END_BOUND.to_owned(),
        PlanEnd::Licence { why } => licence_end_line(w, author, why),
        PlanEnd::Claimed => tv::MINE_CLAIMED.to_owned(),
        PlanEnd::Lease { forfeit: true } => tv::END_LEASE_FORFEIT.to_owned(),
        PlanEnd::Lease { forfeit: false } => tv::END_LEASE_ENDED.to_owned(),
        PlanEnd::Dark => tv::END_DARK.to_owned(),
        PlanEnd::ChartFull => tv::END_CHART_FULL.replace("{body}", &words.body),
        PlanEnd::Cleared => tv::END_CLEARED.to_owned(),
        PlanEnd::Stranded => tv::END_STRANDED.replace("{ramp}", &words.ramp),
    }
}

/// **Why a plan waits, in words** (the brief's 3.5): `Busy` is never told.
#[must_use]
pub(crate) fn wait_why(
    w: &World,
    wait: crate::surface::plan::PlanWait,
    words: &PlanWords,
) -> String {
    use crate::surface::plan::PlanWait;
    match wait {
        PlanWait::Helm => tv::WAIT_HELM.replace("{name}", &words.name),
        PlanWait::Ashore => tv::WAIT_ASHORE.replace("{name}", &words.name),
        PlanWait::Cap => tv::WAIT_CAP.to_owned(),
        PlanWait::Charging { until_ic } => {
            tv::WAIT_CHARGING.replace("{when}", &tv::due_words(w, until_ic))
        }
        PlanWait::Cold => tv::WAIT_COLD.to_owned(),
        PlanWait::Markers => tv::WAIT_MARKERS.to_owned(),
        PlanWait::Dark | PlanWait::Busy => tv::WAIT_DARK.to_owned(),
    }
}

/// **Why a plan passed something over, in words** (the brief's 3.5).
#[must_use]
pub(crate) fn pass_why(why: crate::surface::plan::PassWhy, family: OreFamily) -> String {
    use crate::surface::plan::PassWhy;
    match why {
        PassWhy::WorkedOut => tv::PASS_WORKED_OUT.to_owned(),
        PassWhy::Barren => tv::PASS_BARREN.replace("{family}", family.label()),
        PassWhy::Locked => tv::PASS_LOCKED.to_owned(),
        PassWhy::NoWay => tv::PASS_NO_WAY.to_owned(),
        PassWhy::TooFar => tv::PASS_TOO_FAR.to_owned(),
        PassWhy::NothingNew => tv::PASS_NOTHING_NEW.to_owned(),
    }
}

/// **The excursion caps' census for `operator` at `site`** (doc 20 cut SM.11; the brief's 3.4
/// "the caller gathers"): the world's rig records' open excursions, read through `read_rig` — the
/// launch's own count.
pub(crate) fn excursion_counts(
    w: &World,
    read_rig: &dyn Fn(ObjId) -> Result<Option<RigRecord>, String>,
    operator: ObjId,
    site: &CanonicalId,
) -> ExcursionCounts {
    let mut counts = ExcursionCounts::default();
    for tv in world_rigs(w) {
        if let Ok(Some(record)) = read_rig(tv)
            && let Some(excursion) = record.excursion.as_ref()
        {
            counts.world = counts.world.saturating_add(1);
            if &excursion.site == site {
                counts.site = counts.site.saturating_add(1);
            }
            if excursion.operator == operator {
                counts.operator = counts.operator.saturating_add(1);
            }
        }
    }
    counts
}

/// **What a plan's next move does, in words** (`PLAN_SHOW_NOW`): the act's doing words, the wait's
/// reason, or the end's.
pub(super) fn plan_now_words(
    w: &World,
    rig: &RigAt,
    plan: &FillPlan,
    words: &PlanWords,
    moved: Option<crate::surface::plan::PlanMove>,
) -> String {
    use crate::surface::plan::{PlanActInner, PlanMove};
    match moved {
        Some(PlanMove::Start { word, act, .. }) => match act.0 {
            PlanActInner::Act(planned) | PlanActInner::Drive(planned) => {
                let kind = ActKind::from_word(word).unwrap_or(ActKind::Recover);
                doing_words(kind, &planned.params, &ramp_label(w, rig.hull))
            }
            PlanActInner::Mine(planned) => drilling_words(&planned.addr),
        },
        Some(PlanMove::End { why, .. }) => {
            format!("it ends: {}", end_why(w, plan.author, why, words))
                .trim_end_matches('.')
                .to_owned()
        }
        Some(PlanMove::Wait(wait)) => match wait {
            crate::surface::plan::PlanWait::Busy => tv::WAIT_BUSY.to_owned(),
            other => wait_why(w, other, words),
        },
        None => tv::WAIT_DARK.to_owned(),
    }
}

/// **`+tv/<n> plan show`** (doc 20 cut SM.11; the brief's 3.5): a pure read of the record — no plan
/// → `PLAN_NONE`; else `PLAN_SHOW_HEAD`, one `PLAN_SHOW_STEP` a step (done, now, next, pending,
/// passed over), the tally, and for a live plan `PLAN_SHOW_NOW` from the interpreter over the
/// command's own reads; an ended plan `PLAN_SHOW_ENDED`. Never a class, a tonnage of the ground or
/// a reserve.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line.
pub(crate) fn plan_show_cmd(ctx: &mut Ctx<'_>, rig: &RigAt) -> Result<(), CmdError> {
    let handles = ctx
        .w
        .module_state::<PortHandles>()
        .cloned()
        .ok_or_else(|| CmdError::msg(tv::TV_UNWIRED))?;
    let name = name_of(ctx.w, rig.hull);
    let dark = || CmdError::msg(tv::TV_DARK.replace("{name}", &name));
    let battery = rig_battery(ctx.w, &handles, rig);
    let record = record_of(ctx, &handles, rig.tv, battery.as_ref()).map_err(|()| dark())?;
    let n = rig.slot.to_string();
    // Only the asker's own plan (the fixer round's conformance/F1): a previous lessee's plan left
    // on the record reads as none — its pockets, family and tally are theirs.
    let Some(plan) = record.plan.as_ref().filter(|plan| plan.author == ctx.actor) else {
        return Err(CmdError::msg(tv::PLAN_NONE.replace("{n}", &n)));
    };
    let w: &World = ctx.w;
    let words = PlanWords::of(w, plan);
    let ore = tv::ore_words(crate::surface::opening::ore_good(plan.family));
    let state = match plan.state {
        FillState::Live => tv::STATE_LIVE,
        FillState::Parking { .. } => tv::STATE_PARKING,
        FillState::Ended { .. } => tv::STATE_ENDED,
    };
    let mut lines = vec![
        tv::PLAN_SHOW_HEAD
            .replace("{n}", &n)
            .replace("{no}", &plan.no.to_string())
            .replace("{site}", &words.site)
            .replace("{ore}", &ore)
            .replace("{state}", state)
            .replace(
                "{filed}",
                &format!(
                    "IC {}",
                    mush_core::CivilDate::from_ic_seconds(plan.filed_ic)
                ),
            )
            .replace("{deadline}", &tv::due_words(w, plan.deadline_ic)),
    ];
    let passed: std::collections::BTreeSet<(i64, i64, bool)> = plan
        .notes
        .iter()
        .filter_map(|note| match note.note {
            PlanNoteKind::Passed { at, field, .. } => Some((at[0], at[1], field)),
            PlanNoteKind::Parking { .. } | PlanNoteKind::Ended { .. } => None,
        })
        .collect();
    let cursor = usize::from(plan.cursor);
    for (k, step) in plan.steps.iter().enumerate() {
        let skipped = match step {
            PlanStep::Mine { pocket } => passed.contains(&(pocket[0], pocket[1], false)),
            PlanStep::Sound { field } => address::field_of_index(*field)
                .is_some_and(|(fx, fy)| passed.contains(&(fx, fy, true))),
            PlanStep::Block { .. } => false,
        };
        let mark = if k < cursor && skipped {
            tv::MARK_PASSED
        } else if k < cursor {
            tv::MARK_DONE
        } else if k == cursor && matches!(plan.state, FillState::Live) {
            tv::MARK_NOW
        } else if k == cursor.saturating_add(1) && matches!(plan.state, FillState::Live) {
            // A parking plan works no further step (the fixer round's correctness/F4).
            tv::MARK_NEXT
        } else {
            tv::MARK_PENDING
        };
        lines.push(
            tv::PLAN_SHOW_STEP
                .replace("{k}", &k.saturating_add(1).to_string())
                .replace("{words}", &crate::surface::plan::step_words(step, false))
                .replace("{mark}", mark),
        );
    }
    lines.push(
        tv::PLAN_SHOW_TALLY
            .replace("{t}", &plan.tally.tonnes.to_string())
            .replace("{ore}", &ore)
            .replace(
                "{c}",
                &tv::count_words(u64::from(plan.tally.cycles), "cycle", "cycles"),
            )
            .replace(
                "{trips}",
                &tv::count_words(u64::from(plan.tally.trips), "trip", "trips"),
            ),
    );
    match plan.state {
        FillState::Ended { why, at_ic } => lines.push(
            tv::PLAN_SHOW_ENDED
                .replace(
                    "{at}",
                    &format!("IC {}", mush_core::CivilDate::from_ic_seconds(at_ic)),
                )
                .replace(
                    "{why}",
                    end_why(w, plan.author, why, &words).trim_end_matches('.'),
                ),
        ),
        FillState::Live | FillState::Parking { .. } => {
            let tools = handles.surface_tools.clone().unwrap_or_default();
            let read_rig = |tv: ObjId| SurfaceReads::rig(&*ctx, tv);
            let counts = excursion_counts(w, &read_rig, plan.author, &plan.site);
            let mut peeks = Peeks::default();
            let moved = crate::surface::plan::decide(
                w,
                &*ctx,
                rig,
                &record,
                &counts,
                &tools,
                &mut peeks,
                w.world_ic_now().at,
            );
            lines.push(
                tv::PLAN_SHOW_NOW.replace("{doing}", &plan_now_words(w, rig, plan, &words, moved)),
            );
        }
    }
    for line in lines {
        crate::port::coop::say(ctx, line);
    }
    Ok(())
}

/// **`+tv/<n> plan clear`** (doc 20 cut SM.11; the brief's 3.5, D6 and D17): committed with no
/// `go` (`+tv/mine stop`'s R7 precedent). No plan → `PLAN_NONE`, nothing submitted; a live plan →
/// ONE `tacsim_surface_act` with `act: plan_clear` keyed `tacsim.surface/{tv}/plan/{no}/clear/
/// {seq}`, `ordinal` [`PLAN_CLEAR_ORDINAL`], and `PLAN_CLEARED` (the step in flight finishes by
/// its own rules); an ended plan → ONE with `act: plan_strike` keyed `tacsim.surface/{tv}/plan/
/// {no}/strike/{ordinal}` and `PLAN_DROPPED`. Two writes on two keys, so a second clear is never a silent replay of the first.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line.
pub(crate) fn plan_clear_cmd(ctx: &mut Ctx<'_>, rig: &RigAt) -> Result<(), CmdError> {
    let handles = ctx
        .w
        .module_state::<PortHandles>()
        .cloned()
        .ok_or_else(|| CmdError::msg(tv::TV_UNWIRED))?;
    let Some(action) = handles.surface_act.clone() else {
        return Err(CmdError::msg(tv::TV_UNWIRED));
    };
    let name = name_of(ctx.w, rig.hull);
    let dark = || CmdError::msg(tv::TV_DARK.replace("{name}", &name));
    let battery = rig_battery(ctx.w, &handles, rig);
    let record = record_of(ctx, &handles, rig.tv, battery.as_ref()).map_err(|()| dark())?;
    let n = rig.slot.to_string();
    // Only the asker's own plan (the fixer round's conformance/F1): a previous lessee's plan is
    // inert, and the asker's own next filing replaces it.
    let Some(plan) = record.plan.as_ref().filter(|plan| plan.author == ctx.actor) else {
        return Err(CmdError::msg(tv::PLAN_NONE.replace("{n}", &n)));
    };
    let no = plan.no;
    // A clear carries `PLAN_CLEAR_ORDINAL` (the lead's pre-commit round, item 2: its key names the
    // plan's `seq`, never the record's ordinal, so neither do its parameters); a strike, keyed by
    // the ordinal, carries it.
    let (word, key, line, ordinal) = if plan.live() {
        (
            "plan_clear",
            plan_clear_key(rig.tv, no, plan.seq),
            tv::PLAN_CLEARED,
            PLAN_CLEAR_ORDINAL,
        )
    } else {
        (
            "plan_strike",
            plan_strike_key(rig.tv, no, record.ordinal),
            tv::PLAN_DROPPED,
            record.ordinal.max(1),
        )
    };
    let Some(key) = key else {
        tracing::error!(tv = rig.tv.0, "tacsim: a plan's clear key did not form");
        return Err(dark());
    };
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({
            "tv": rig.tv.0.to_string(),
            "act": word,
            "ordinal": ordinal,
            "plan": no,
        }),
        key,
    );
    crate::port::coop::say(
        ctx,
        line.replace("{n}", &n).replace("{no}", &no.to_string()),
    );
    Ok(())
}
