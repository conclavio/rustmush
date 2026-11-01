//! World-side surface excursion planning (`ActPlan`, `Reach`, view/home/rig resolution).

use super::*;

// ---- the world plans --------------------------------------------------------------------------

/// **One act, planned against the world** (the brief's 3.3): what the preview prints, what
/// the `go` submits and what the start's offer fixes.
#[derive(Clone, Debug)]
pub(crate) struct ActPlan {
    /// The site entry the hull is settled at.
    pub site: CanonicalId,
    /// How long it takes, IC seconds.
    pub secs: u32,
    /// What it spends, cu.
    pub cost_cu: u32,
    /// What the trip home spends, cu (0 for every SM.3b act; SM.4's drive adds it).
    pub home_cu: u32,
    /// Its parameters (a dump's goods are read later: the command by `read_pool`, the
    /// prepare by the transaction).
    pub params: ActParams,
    /// The rig stands in this hull's bay at plan time (decision 21; doc 20 cut SM.4a, D12): a
    /// launch sets it, a recover from the bay sets it, and for a scan or a sounding it means
    /// **the launch is implied**.
    pub from_bay: bool,
}

/// The common head of every plan: the layer on, the hull's view readable and not held.
pub(super) fn view_of<'w>(w: &'w World, rig: &RigAt) -> Result<(HullView<'w>, String), Refusal> {
    let name = name_of(w, rig.hull);
    let on = w
        .module_state::<TacsimStore>()
        .is_some_and(TacsimStore::enabled);
    if !on {
        return Err(Refusal::Dark { name });
    }
    let Some(view) = hull_view(w, rig.hull) else {
        tracing::warn!(
            hull = rig.hull.0,
            "tacsim: a rig's hull has no programme view"
        );
        return Err(Refusal::Dark { name });
    };
    if view.unit_held {
        tracing::warn!(
            hull = rig.hull.0,
            "tacsim: a rig's hull's unit record is held"
        );
        return Err(Refusal::Dark { name });
    }
    Ok((view, name))
}

/// **The launch's whole world-side gate, in order** (the brief's 3.3; §4.3: "legal only
/// while the hull is settled at a site and the operator is aboard"): the layer on; the hull's
/// view (dark or held → `TV_DARK`); under way → `LAUNCH_UNDER_WAY`; an unreadable sailing
/// plan → `TV_DARK`, logged (finding F11: fail-closed, as `prepare_departure` refuses
/// `VoyageState::Unreadable`); a clearance filed → `LAUNCH_CLEARED`; not settled at a site →
/// `LAUNCH_NOT_SITE`; the actor not aboard → `TV_NOT_ABOARD`; the lease record `Forfeit` →
/// `LAUNCH_FORFEIT`; its economy lease not live → `LAUNCH_ENDED` (SM.3a's operate gate R7;
/// an Active lease past its paid term still launches, decision 11); the rig not in this hull's
/// bay → `LAUNCH_OUT`. `LAUNCH_S`, no cost, no trip home.
///
/// **A scan or a sounding from the bay prepends this** (doc 20 cut SM.4a's lead ruling 2: the
/// implied launch; SM.4b's drive will too): keep the refusal order stable.
///
/// # Errors
///
/// The first [`Refusal`] in that order.
pub(crate) fn launch_plan(w: &World, actor: ObjId, rig: &RigAt) -> Result<ActPlan, Refusal> {
    let (view, name) = view_of(w, rig)?;
    if matches!(berth_of(w, rig.hull), Berth::UnderWay) {
        return Err(Refusal::UnderWay { name });
    }
    if crate::port::voyage::voyage_dark(w, rig.hull) {
        tracing::warn!(
            hull = rig.hull.0,
            "tacsim: a launch found the hull's sailing plan unreadable; it is refused"
        );
        return Err(Refusal::Dark { name });
    }
    if let Some(pending) = crate::port::voyage::pending_of(w, rig.hull) {
        return Err(Refusal::Cleared {
            name,
            port: crate::port::voyage::venue_name(w, &pending.to_venue),
        });
    }
    let berth = site_berth(w, &view)?;
    if !rig.operator_present(w, actor) {
        return Err(Refusal::NotAboard { name });
    }
    if let Err(bar) = lease_gate(w, &rig.record) {
        let (counter, port) = tv::counter_words(w);
        return Err(match bar {
            LeaseBar::Forfeit => Refusal::Forfeit {
                n: rig.slot,
                counter,
                port,
            },
            LeaseBar::Ended => Refusal::Ended {
                n: rig.slot,
                counter,
                port,
            },
        });
    }
    if where_is(w, rig.tv) != (Where::Bay { hull: rig.hull }) {
        return Err(Refusal::Out { n: rig.slot });
    }
    Ok(ActPlan {
        site: berth.entry,
        secs: LAUNCH_S,
        cost_cu: 0,
        home_cu: 0,
        params: ActParams::None {},
        from_bay: true,
    })
}

/// **Why a rig's lease bars it** (doc 20 cut SM.7; the brief's 3.3 item 1, reviewer F11).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LeaseBar {
    /// The lease record is `Forfeit`.
    Forfeit,
    /// Its economy lease is not live.
    Ended,
}

/// **The lease gate** (SM.3a's operate gate R7, factored by doc 20 cut SM.7 so the launch and the
/// mining run read one rule — the brief's decision R15): the lease record `Forfeit`, then its
/// economy lease not live. **Recover, dump and drive never read it**: the way home is never
/// barred (§8, §19 item 3).
///
/// # Errors
///
/// The [`LeaseBar`], in that order.
pub(crate) fn lease_gate(w: &World, record: &TvLease) -> Result<(), LeaseBar> {
    if record.status == TvLeaseStatus::Forfeit {
        return Err(LeaseBar::Forfeit);
    }
    if !tv::rig_lease_live(w, record) {
        return Err(LeaseBar::Ended);
    }
    Ok(())
}

/// **The recover's world-side gate, in order** (the brief's 3.3; §7.1's placement half): the
/// layer, the view, under way, not at a site, not aboard as the launch; then the rig: in this
/// hull's bay → `Ok` with `from_bay` (the command and the prepare refuse `RECOVER_IN_BAY`
/// unless an excursion is still open, which this recover closes — decision 21); on another
/// domain, another hull's bay or elsewhere → `RECOVER_ELSEWHERE`; on the hull's site domain
/// beside the ramp (Chebyshev ≤ 1) → SM.3b's recover, `RECOVER_S`, no cost. No lease-state gate
/// (§8 / §19 item 3: rights never stop a rig's way home).
///
/// **Doc 20 cut SM.4b — farther off, the rig drives home first** (lead ruling 5; §3.6 of the
/// brief): the site's home field through the non-building peek (`peeks`; cold →
/// `GROUND_COLD`; it reads the keyless field when the site is dark, R3, so a recover never
/// refuses `INSTRUMENTS_DARK` — only a home whose terrain failed this boot reads `RECOVER_DARK`),
/// the descent from the rig's cell frozen by `drive::recover_leg` (never refused for its
/// waypoints), and ONE act of kind `Recover` whose params carry the leg to the beside-ramp cell
/// it ends on: `secs` the leg's then `RECOVER_S`, `cost_cu` the leg's charge, `home_cu` 0 (it IS
/// the way home). `rates` is the rig's (`rig_rates`): without the battery's row the leg costs
/// nothing (a rig is never stranded by its battery, §4.1); without the unit's speed, dark. A
/// cell with no way home at all → `RECOVER_FAR` as before (staff's remedy).
///
/// # Errors
///
/// The first [`Refusal`] in that order.
pub(crate) fn recover_plan(
    w: &World,
    actor: ObjId,
    rig: &RigAt,
    peeks: &mut Peeks,
    rates: Option<DriveRates>,
) -> Result<ActPlan, Refusal> {
    let (view, name) = view_of(w, rig)?;
    if matches!(berth_of(w, rig.hull), Berth::UnderWay) {
        return Err(Refusal::UnderWay { name });
    }
    let berth = site_berth(w, &view)?;
    if !rig.operator_present(w, actor) {
        return Err(Refusal::NotAboard { name });
    }
    let ramp_cell = site_cell(berth.ramp.x_cm, berth.ramp.y_cm);
    let (x_cm, y_cm) = match where_is(w, rig.tv) {
        Where::Bay { hull } if hull == rig.hull => {
            return Ok(ActPlan {
                site: berth.entry,
                secs: RECOVER_S,
                cost_cu: 0,
                home_cu: 0,
                params: ActParams::None {},
                from_bay: true,
            });
        }
        Where::Ground {
            domain, x_cm, y_cm, ..
        } if domain == berth.domain => (x_cm, y_cm),
        Where::Bay { .. } | Where::Ground { .. } | Where::Elsewhere => {
            return Err(Refusal::Elsewhere { n: rig.slot, name });
        }
    };
    let cells = cells_apart(site_cell(x_cm, y_cm), ramp_cell);
    if cells <= 1 {
        return Ok(ActPlan {
            site: berth.entry,
            secs: RECOVER_S,
            cost_cu: 0,
            home_cu: 0,
            params: ActParams::None {},
            from_bay: false,
        });
    }
    let far = || Refusal::Far {
        n: rig.slot,
        m: cells.saturating_mul(10),
        ramp: berth.ramp_label.clone(),
    };
    let field = match crate::surface::peek_home(w, &berth.entry, ramp_cell, peeks) {
        HomePeek::Warm(field) => field,
        HomePeek::Cold => {
            return Err(Refusal::GroundCold {
                site: berth.name.clone(),
                entry: berth.entry.clone(),
            });
        }
        HomePeek::Dark => {
            return Err(Refusal::RecoverDark {
                n: rig.slot,
                ramp: berth.ramp_label.clone(),
            });
        }
    };
    let Some(rates) = rates else {
        return Err(Refusal::Dark { name });
    };
    // Lead ruling 5: a cell with no way home is SM.3b's `RECOVER_FAR` (staff's remedy).
    let leg = drive::recover_leg(&field, (x_cm, y_cm), &rates).ok_or_else(far)?;
    let to = leg
        .cells
        .last()
        .map(|(x, y)| [i64::from(*x), i64::from(*y)])
        .ok_or_else(far)?;
    Ok(ActPlan {
        site: berth.entry,
        secs: leg.secs.saturating_add(RECOVER_S),
        cost_cu: leg.cu,
        home_cu: 0,
        params: ActParams::Drive {
            from_bay: false,
            target: format!("the {}", berth.ramp_label),
            to,
            waypoints: leg.waypoints,
            secs: leg.secs,
            cost: leg.cost,
            metres: leg.metres,
            heading_mdeg: leg.heading_mdeg,
            side: field.side(),
        },
        from_bay: false,
    })
}

/// **The dump's world-side gate, in order** (the brief's 3.3; §4.3): the layer, the view,
/// under way; not at a site → `DUMP_NOT_SITE`; not aboard; the rig neither in this hull's
/// bay nor on its site's domain → `RECOVER_ELSEWHERE`; the site's spoil vault not laid →
/// `DUMP_NO_SPOIL`. No lease gate. `params` names the hopper and the spoil; the goods are
/// read later (the command by `read_pool`, the prepare by its transaction).
///
/// # Errors
///
/// The first [`Refusal`] in that order.
pub(crate) fn dump_plan(w: &World, actor: ObjId, rig: &RigAt) -> Result<ActPlan, Refusal> {
    let (view, name) = view_of(w, rig)?;
    if matches!(berth_of(w, rig.hull), Berth::UnderWay) {
        return Err(Refusal::UnderWay { name });
    }
    let berth = site_berth(w, &view).map_err(|refusal| match refusal {
        Refusal::NotSite { name } => Refusal::DumpNotSite { n: rig.slot, name },
        other => other,
    })?;
    if !rig.operator_present(w, actor) {
        return Err(Refusal::NotAboard { name });
    }
    match where_is(w, rig.tv) {
        Where::Bay { hull } if hull == rig.hull => {}
        Where::Ground { domain, .. } if domain == berth.domain => {}
        Where::Bay { .. } | Where::Ground { .. } | Where::Elsewhere => {
            return Err(Refusal::Elsewhere { n: rig.slot, name });
        }
    }
    let Some(spoil) = crate::port::site::spoil_vault(w, berth.map) else {
        return Err(Refusal::NoSpoil { site: berth.name });
    };
    let Some(hopper) = crate::port::vehicle::hopper_of(w, rig.tv) else {
        tracing::warn!(tv = rig.tv.0, "tacsim: a dump found no hopper vault");
        return Err(Refusal::Dark { name });
    };
    Ok(ActPlan {
        site: berth.entry,
        secs: DUMP_S,
        cost_cu: 0,
        home_cu: 0,
        params: ActParams::Dump {
            hopper: hopper.0,
            spoil: spoil.0,
            goods: BTreeMap::new(),
        },
        from_bay: false,
    })
}

/// **The unload's world-side gate, in order** (doc 20 cut SM.8; §7.1, §8; the brief's 3.5,
/// decisions R5, R6, R9 and R14), cheap first: the layer and the view (dark or held →
/// `TV_DARK`); under way → `UNLOAD_UNDER_WAY`; not on a site's pad → `UNLOAD_NOT_SITE` (an
/// unload is a surface act, the dump's rule — R5); **aboard, for the hand unload only** (`queued`
/// false: `TV_NOT_ABOARD`; a queued unload is the recover's second half, committed aboard by its
/// `go` — R6); the rig in this hull's bay (on the site's ground → `UNLOAD_OUT`; anywhere else →
/// `RECOVER_ELSEWHERE`); the rig's hopper vault and the hull's hold vault (either missing →
/// `TV_DARK`, logged). `UNLOAD_S`, no cost, no trip home; `params` names the two vaults, the
/// goods empty (the command and the offer split the hopper; the prepare re-checks). **No
/// licence, no tenure, no lease, no battery is read** (§8 rights per act; §19 item 3; §4.1).
///
/// # Errors
///
/// The first [`Refusal`] in that order.
pub(crate) fn unload_plan(
    w: &World,
    actor: ObjId,
    rig: &RigAt,
    queued: bool,
) -> Result<ActPlan, Refusal> {
    let (view, name) = view_of(w, rig)?;
    if matches!(berth_of(w, rig.hull), Berth::UnderWay) {
        return Err(Refusal::UnloadUnderWay { name });
    }
    let berth = site_berth(w, &view).map_err(|refusal| match refusal {
        Refusal::NotSite { name } => Refusal::UnloadNotSite { n: rig.slot, name },
        Refusal::UnderWay { name } => Refusal::UnloadUnderWay { name },
        other => other,
    })?;
    if !queued && !rig.operator_present(w, actor) {
        return Err(Refusal::NotAboard { name });
    }
    match where_is(w, rig.tv) {
        Where::Bay { hull } if hull == rig.hull => {}
        Where::Ground { domain, .. } if domain == berth.domain => {
            return Err(Refusal::UnloadOut {
                n: rig.slot,
                site: berth.name,
            });
        }
        Where::Bay { .. } | Where::Ground { .. } | Where::Elsewhere => {
            return Err(Refusal::Elsewhere { n: rig.slot, name });
        }
    }
    let Some(hopper) = crate::port::vehicle::hopper_of(w, rig.tv) else {
        tracing::warn!(tv = rig.tv.0, "tacsim: an unload found no hopper vault");
        return Err(Refusal::Dark { name });
    };
    let Some(hold) = crate::port::ship::hold_vault(w, &view) else {
        tracing::warn!(hull = rig.hull.0, "tacsim: an unload found no hold vault");
        return Err(Refusal::Dark { name });
    };
    Ok(ActPlan {
        site: berth.entry,
        secs: UNLOAD_S,
        cost_cu: 0,
        home_cu: 0,
        params: ActParams::Unload {
            hopper: hopper.0,
            hold: hold.0,
            goods: BTreeMap::new(),
            how: if queued {
                UnloadHow::Queued
            } else {
                UnloadHow::Whole
            },
            excursion: None,
        },
        from_bay: false,
    })
}

/// **The hold's room an unload weighs against** (doc 20 cut SM.8; lead ruling 3, decision R16).
#[derive(Clone, Debug)]
pub(crate) struct UnloadRoom {
    /// The hull's hold plan (`crate::port::market::hold_plan`).
    pub plan: crate::port::market::HoldPlan,
    /// The plan's goods table (`crate::port::market::plan_goods`).
    pub goods: &'static crate::definitions::GoodsTable,
    /// The hold's ore compartment.
    pub compartment: crate::port::capacity::Compartment,
    /// The hold's unreserved room, per measure, with the unloading rig's own hopper left out.
    pub room: Load,
    /// The hull's other rigs (their hoppers and runs are the reservation).
    pub siblings: Vec<ObjId>,
}

/// **The hold's room for `tv`'s unload** (doc 20 cut SM.8; the brief's 3.5, lead ruling 3,
/// decisions R4 and R16) — one function, the command, the start's offer and the recover's preview
/// all call it: the hull's [`crate::port::market::hold_plan`], its goods table
/// ([`crate::port::market::plan_goods`]; a store table loaded and unequal to it is a dark plan,
/// logged, never a preview the prepare then refuses), the hold's ONE ore compartment
/// (`capacity::compartment_for` over the plan's compartments for the ore stowage tag; a hull
/// whose ores route to none or several is dark), and
/// `crate::port::ship::hold_unreserved_with` over that table with **the unloading rig left out**
/// (its hopper is the source; a rig in the bay has no run) — the hull's rigs as the departure's
/// bay gate collects them (`rigs_in_bay` ∪ `live_on_hull`), less `tv`. With one ore compartment
/// the summed room `hold_unreserved_with` computes is that compartment's. `None` when any of it
/// will not read.
pub(crate) fn unload_room(
    w: &World,
    view: &HullView<'_>,
    tv: ObjId,
    read_pool: &dyn Fn(EndpointRef, &CanonicalId) -> Result<u64, crate::port_tick::PoolReadError>,
    read_rig: &dyn Fn(ObjId) -> Result<Option<RigRecord>, String>,
) -> Option<UnloadRoom> {
    let plan = crate::port::market::hold_plan(view)?;
    let goods = crate::port::market::plan_goods(&plan)?;
    if let Some(stored) = w.module_state::<TacsimStore>().and_then(TacsimStore::goods)
        && stored != goods
    {
        tracing::error!(
            hull = view.hull().0,
            "tacsim: the store's goods table differs from the hold plan's; the unload reads dark"
        );
        return None;
    }
    let ore = CanonicalId::new("metal_ore").ok()?;
    let compartment = crate::port::capacity::compartment_for(
        goods,
        &crate::port::market::plan_compartments(&plan),
        &ore,
    )?;
    let mut siblings: Vec<ObjId> = crate::port::vehicle::rigs_in_bay(w, view)
        .iter()
        .map(|rig| rig.tv)
        .collect();
    for record in tv::live_on_hull(w, view.hull()) {
        if !siblings.contains(&record.tv) {
            siblings.push(record.tv);
        }
    }
    siblings.retain(|other| *other != tv);
    siblings.sort_unstable();
    let room =
        crate::port::ship::hold_unreserved_with(goods, w, view, &siblings, read_pool, read_rig)?;
    Some(UnloadRoom {
        plan,
        goods,
        compartment,
        room,
        siblings,
    })
}

/// Where a scan, a sounding or (doc 20 cut SM.4b) a drive is made from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Stand {
    /// The site's port entry.
    pub site: CanonicalId,
    /// The stand cell, site-local.
    pub cell: (i64, i64),
    /// The launch is implied (the rig stands in the bay; it reads from the ramp's cell).
    pub from_bay: bool,
    /// **The exact start pose, centimetres** (doc 20 cut SM.4b): the ramp pose from the bay,
    /// else the rig's own `Direct` pose — a drive's leg starts here.
    pub start_cm: (i64, i64),
    /// The hull's ramp cell (doc 20 cut SM.4b: the home field's key).
    pub ramp_cell: (i64, i64),
    /// The egress's label (`stern ramp`).
    pub ramp_label: String,
    /// The site's name.
    pub site_name: String,
    /// **The site's address scheme** (doc 32 cut S.1): its side's.
    pub scheme: address::Scheme,
}

/// **Where a scan or a sounding stands, in the plan's order** (doc 20 cut SM.4a; the brief's
/// 3.4, D12, D13): the layer and the view (`view_of`); under way → `LAUNCH_UNDER_WAY`; then
/// the rig: in this hull's bay → **`launch_plan`, its whole order unchanged**, and the stand
/// is the ramp's cell (`from_bay`); otherwise the hull's site berth (not a site →
/// `LAUNCH_NOT_SITE`), the actor aboard (else `TV_NOT_ABOARD`), and the rig `Direct` on the
/// hull's site domain inside the site (else `RECOVER_ELSEWHERE`) — **no lease-state gate**, as
/// a recover's and a dump's (decision 11). **Doc 20 cut SM.4b**: a drive stands the same way
/// (the implied launch's whole gate from the bay), so the command, the start's offer and the
/// drive's order all read this one rule.
pub(crate) fn stand_of(w: &World, actor: ObjId, rig: &RigAt) -> Result<Stand, Refusal> {
    let (view, name) = view_of(w, rig)?;
    if matches!(berth_of(w, rig.hull), Berth::UnderWay) {
        return Err(Refusal::UnderWay { name });
    }
    if where_is(w, rig.tv) == (Where::Bay { hull: rig.hull }) {
        let launch = launch_plan(w, actor, rig)?;
        let berth = site_berth(w, &view)?;
        let ramp_cell = site_cell(berth.ramp.x_cm, berth.ramp.y_cm);
        let scheme = chart::site_scheme(w, &launch.site);
        return Ok(Stand {
            scheme,
            site: launch.site,
            cell: ramp_cell,
            from_bay: true,
            start_cm: (berth.ramp.x_cm, berth.ramp.y_cm),
            ramp_cell,
            ramp_label: berth.ramp_label.clone(),
            site_name: berth.name.clone(),
        });
    }
    let berth = site_berth(w, &view)?;
    if !rig.operator_present(w, actor) {
        return Err(Refusal::NotAboard { name });
    }
    let scheme = chart::site_scheme(w, &berth.entry);
    match where_is(w, rig.tv) {
        Where::Ground {
            domain, x_cm, y_cm, ..
        } if domain == berth.domain && scheme.cell_on_site(site_cell(x_cm, y_cm)) => Ok(Stand {
            scheme,
            site: berth.entry.clone(),
            cell: site_cell(x_cm, y_cm),
            from_bay: false,
            start_cm: (x_cm, y_cm),
            ramp_cell: site_cell(berth.ramp.x_cm, berth.ramp.y_cm),
            ramp_label: berth.ramp_label.clone(),
            site_name: berth.name.clone(),
        }),
        Where::Bay { .. } | Where::Ground { .. } | Where::Elsewhere => {
            Err(Refusal::Elsewhere { n: rig.slot, name })
        }
    }
}

/// **The charge home from a stand a reading act occupies** (doc 20 cut SM.4b; lead ruling 4,
/// §3.8 of the brief): 0 from the bay (the stand is the ramp's cell); else the home field's figure
/// through the non-building peek (cold → `GROUND_COLD`, dark → `INSTRUMENTS_DARK`) and
/// `drive::home_cu_here` — the conservative bound where the stand has no way home, since there is
/// nothing left to prevent there (R2).
pub(super) fn home_here(
    w: &World,
    stand: &Stand,
    rates: Option<&DriveRates>,
    name: &str,
    peeks: &mut Peeks,
) -> Result<u32, Refusal> {
    // From the bay, or from beside the ramp (Chebyshev ≤ 1: the recover's own rule), the way
    // home is the climb alone — no field is read, so a rig at the ramp scans and sounds with
    // the home field still cold.
    if stand.from_bay || cells_apart(stand.cell, stand.ramp_cell) <= 1 {
        return Ok(0);
    }
    let field = match crate::surface::peek_home(w, &stand.site, stand.ramp_cell, peeks) {
        HomePeek::Warm(field) => field,
        HomePeek::Cold => {
            return Err(Refusal::GroundCold {
                site: stand.site_name.clone(),
                entry: stand.site.clone(),
            });
        }
        HomePeek::Dark => return Err(Refusal::InstrumentsDark),
    };
    let rates = rates.ok_or_else(|| Refusal::Dark {
        name: name.to_owned(),
    })?;
    Ok(drive::home_cu_here(
        &field,
        stand.cell,
        rates,
        mush_atlas::physics::LocomotionTable::shipped(),
    ))
}

/// **A rig's driving figures** (doc 20 cut SM.4b; lead ruling 3): the rig unit's tracked
/// `max_speed_cms` (its live unit record's, else the programme pool's unit) and the battery's
/// `drive_cu_per_nominal_cell` — **0 without the battery's row**, so a recover's leg home never
/// needs it (§4.1; a drive needs the battery anyway). `None` when the unit has no tracked mode or
/// a zero speed (`TV_DARK`).
#[must_use]
pub(crate) fn rig_rates(w: &World, tv: ObjId, battery: Option<&BatteryDef>) -> Option<DriveRates> {
    let store = w.module_state::<TacsimStore>()?;
    let unit = crate::port::hull::live_unit_id(w, tv).or_else(|| tv::pool_unit(w))?;
    let speed = store
        .definitions()
        .specs
        .get(&unit)?
        .mobility
        .tracked
        .as_ref()?
        .max_speed_cms;
    if speed == 0 {
        return None;
    }
    Some(DriveRates {
        max_speed_cms: speed,
        cu_per_nominal_cell: battery.map_or(0, |battery| battery.drive_cu_per_nominal_cell),
    })
}

/// **The drive's world plan** (doc 20 cut SM.4b; lead rulings 2 and 4, R1–R4; §3.5 of the brief),
/// called only after the command's cheap gates: the site's ground resolved (dark →
/// `INSTRUMENTS_DARK`); the composite through the non-building peek (cold → `GROUND_COLD`, dark →
/// `INSTRUMENTS_DARK`); the home field at the hull's ramp cell (likewise); the target's cell (off
/// the site → `DRIVE_OFF_SITE`); **ONE A*** from the stand's exact start pose
/// (`drive::drive_leg`: the rig's cell → `DRIVE_HERE`, no way → `NO_PATH`, past 32 waypoints even
/// at 80 m → `TOO_WINDING`); then the charge home from the end stand (`drive::home_cu_to`: no way
/// home at all → `NO_WAY_HOME`, R2). The plan: `LAUNCH_S` more from the bay, the leg's seconds,
/// its charge, the real trip home, the leg in the params.
///
/// # Errors
///
/// The first [`Refusal`] in that order.
pub(crate) fn drive_plan(
    w: &World,
    rig: &RigAt,
    stand: &Stand,
    target: &Target,
    rates: &DriveRates,
    peeks: &mut Peeks,
) -> Result<(ActPlan, Leg), Refusal> {
    chart::site_context(w, &stand.site).map_err(|_| Refusal::InstrumentsDark)?;
    let cold = || Refusal::GroundCold {
        site: stand.site_name.clone(),
        entry: stand.site.clone(),
    };
    let ground = match chart::peek_site(w, &stand.site, peeks) {
        SitePeek::Warm(ground) => ground,
        SitePeek::Cold => return Err(cold()),
        SitePeek::Dark => return Err(Refusal::InstrumentsDark),
    };
    let field = match crate::surface::peek_home(w, &stand.site, stand.ramp_cell, peeks) {
        HomePeek::Warm(field) => field,
        HomePeek::Cold => return Err(cold()),
        HomePeek::Dark => return Err(Refusal::InstrumentsDark),
    };
    let surface = chart::ground_surface(&ground).ok_or(Refusal::InstrumentsDark)?;
    let to = drive::target_cell(target, stand.cell, surface.cells.width()).ok_or_else(|| {
        Refusal::OffSite {
            site: stand.site_name.clone(),
        }
    })?;
    let words = drive::target_words_in(target, stand.scheme);
    let table = mush_atlas::physics::LocomotionTable::shipped();
    let leg =
        drive::drive_leg(surface, table, stand.start_cm, to, rates).map_err(|why| match why {
            LegRefused::Here => Refusal::Here { n: rig.slot },
            LegRefused::NoPath | LegRefused::Overflow => Refusal::NoPath {
                n: rig.slot,
                target: words.clone(),
            },
            LegRefused::Winding { needed } => Refusal::Winding {
                n: rig.slot,
                target: words.clone(),
                needed,
            },
        })?;
    let home_cu = drive::home_cu_to(&field, to, rates).ok_or_else(|| Refusal::NoWayHome {
        n: rig.slot,
        target: words.clone(),
        ramp: stand.ramp_label.clone(),
    })?;
    let launch = if stand.from_bay { LAUNCH_S } else { 0 };
    let plan = ActPlan {
        site: stand.site.clone(),
        secs: launch.saturating_add(leg.secs),
        cost_cu: leg.cu,
        home_cu,
        params: ActParams::Drive {
            from_bay: stand.from_bay,
            target: words,
            to: [to.0, to.1],
            waypoints: leg.waypoints.clone(),
            secs: leg.secs,
            cost: leg.cost,
            metres: leg.metres,
            heading_mdeg: leg.heading_mdeg,
            side: surface.cells.width(),
        },
        from_bay: stand.from_bay,
    };
    Ok((plan, leg))
}

/// **The scan's world-side gate, in order** (doc 20 cut SM.4a; §5's R1, the brief's 3.4): the
/// stand (`stand_of`, the implied launch's whole gate first); the site's ground resolved
/// (`chart::site_context`, else `INSTRUMENTS_DARK`); the composite through the **non-building
/// peek**, shared by every rig of the call (`peeks`; cold → `GROUND_COLD`, dark →
/// `INSTRUMENTS_DARK`); the footing at the stand (none → `NO_FOOTING`); the fields the mast
/// sees (`chart::seen_fields`). `LAUNCH_S` more when it launches; the scanner's `scan_s` and
/// `scan_cu`; **since doc 20 cut SM.4b the real trip home** from the stand (`home_here`: 0 from
/// the bay, else the home field's figure — cold → `GROUND_COLD`, dark → `INSTRUMENTS_DARK` — and
/// without the rig's rates `TV_DARK`).
///
/// # Errors
///
/// The first [`Refusal`] in that order.
pub(crate) fn scan_plan(
    w: &World,
    actor: ObjId,
    rig: &RigAt,
    scanner: &ScannerDef,
    peeks: &mut Peeks,
    rates: Option<&DriveRates>,
) -> Result<ActPlan, Refusal> {
    let stand = stand_of(w, actor, rig)?;
    let site = chart::site_context(w, &stand.site).map_err(|_| Refusal::InstrumentsDark)?;
    let ground = match chart::peek_site(w, &stand.site, peeks) {
        SitePeek::Warm(ground) => ground,
        SitePeek::Cold => {
            return Err(Refusal::GroundCold {
                site: site.name.clone(),
                entry: stand.site.clone(),
            });
        }
        SitePeek::Dark => return Err(Refusal::InstrumentsDark),
    };
    let surface = chart::ground_surface(&ground).ok_or(Refusal::InstrumentsDark)?;
    let footing = chart::footing_at(surface, stand.cell).map_err(|terrain| Refusal::NoFooting {
        n: rig.slot,
        terrain: terrain.label().replace('_', " "),
    })?;
    let (fields, hidden) = chart::seen_fields(surface, stand.cell, scanner);
    if fields.is_empty() {
        return Err(Refusal::InstrumentsDark);
    }
    // Doc 20 cut SM.4b (lead ruling 4): the real trip home from the stand.
    let home_cu = home_here(w, &stand, rates, &name_of(w, rig.hull), peeks)?;
    let launch = if stand.from_bay { LAUNCH_S } else { 0 };
    Ok(ActPlan {
        site: stand.site,
        secs: launch.saturating_add(scanner.scan_s),
        cost_cu: scanner.scan_cu,
        home_cu,
        params: ActParams::Scan {
            from_bay: stand.from_bay,
            body: site.spec.body.clone(),
            cell: [stand.cell.0, stand.cell.1],
            footing,
            fields,
            hidden,
            side: stand.scheme.side(),
        },
        from_bay: stand.from_bay,
    })
}

/// **The sounding's world-side gate, in order** (doc 20 cut SM.4a; §5's R2): the stand
/// (`stand_of`), the site's ground resolved (`chart::site_context`, else `INSTRUMENTS_DARK`:
/// an act that could render nothing never spends charge, D16). It needs no composite. The
/// reach is the scanner's `sound_radius_cells`, or `quiet_radius_cells` when quiet, at the
/// same `sound_s` and `sound_cu`; `LAUNCH_S` more when it launches; **since doc 20 cut SM.4b
/// the real trip home** from the stand (`home_here`, as the scan's).
///
/// # Errors
///
/// The first [`Refusal`] in that order.
pub(crate) fn sound_plan(
    w: &World,
    actor: ObjId,
    rig: &RigAt,
    scanner: &ScannerDef,
    quiet: bool,
    peeks: &mut Peeks,
    rates: Option<&DriveRates>,
) -> Result<ActPlan, Refusal> {
    let stand = stand_of(w, actor, rig)?;
    let site = chart::site_context(w, &stand.site).map_err(|_| Refusal::InstrumentsDark)?;
    let home_cu = home_here(w, &stand, rates, &name_of(w, rig.hull), peeks)?;
    let launch = if stand.from_bay { LAUNCH_S } else { 0 };
    let radius = if quiet {
        scanner.quiet_radius_cells
    } else {
        scanner.sound_radius_cells
    };
    Ok(ActPlan {
        site: stand.site,
        secs: launch.saturating_add(scanner.sound_s),
        cost_cu: scanner.sound_cu,
        home_cu,
        params: ActParams::Sound {
            from_bay: stand.from_bay,
            body: site.spec.body.clone(),
            cell: [stand.cell.0, stand.cell.1],
            radius,
            quiet,
            side: stand.scheme.side(),
        },
        from_bay: stand.from_bay,
    })
}

/// **Which scanner a rig carries** (doc 20 cut SM.4a; `rig_battery_id`'s rule): the lessor
/// pool's tools whose row in the one tools table is a scanner.
#[must_use]
pub(crate) fn rig_scanner_id(w: &World, rig_tv: ObjId) -> Option<CanonicalId> {
    crate::port::vehicle::rig_row(w, rig_tv)?;
    let tools = w.module_state::<PortHandles>()?.surface_tools.clone()?;
    crate::port::vehicle::rig_tools(w, rig_tv)
        .into_iter()
        .find(|id| tools.get(id).and_then(ToolDef::scanner).is_some())
}

/// **A scanner's figures** by its id, from a tools table (`ToolDef::scanner`).
#[must_use]
pub(crate) fn scanner_of<'t>(
    tools: &'t BTreeMap<CanonicalId, ToolDef>,
    id: &CanonicalId,
) -> Option<&'t ScannerDef> {
    tools.get(id).and_then(ToolDef::scanner)
}

/// **The plan of a reading act's stand and reach**, for the chart rules: the stand cell, the
/// fields a scan reads with its footing, or the pockets a sounding reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Reach {
    /// A scan.
    Scan {
        stand: [i64; 2],
        fields: Vec<u8>,
        footing: Footing,
    },
    /// A sounding.
    Sound {
        stand: [i64; 2],
        pockets: Vec<(i64, i64)>,
        /// The site's scheme (doc 32 cut S.1), which names the pockets' blocks.
        scheme: address::Scheme,
    },
}

impl Reach {
    /// A scan's or a sounding's reach from its frozen parameters; `None` for every other act.
    pub(super) fn of(params: &ActParams) -> Option<Reach> {
        match params {
            ActParams::Scan {
                cell,
                fields,
                footing,
                ..
            } => Some(Reach::Scan {
                stand: *cell,
                fields: fields.clone(),
                footing: *footing,
            }),
            ActParams::Sound { cell, radius, .. } => Some(Reach::Sound {
                stand: *cell,
                pockets: params
                    .scheme()
                    .pockets_in_reach((cell[0], cell[1]), i64::from(*radius)),
                scheme: params.scheme(),
            }),
            ActParams::None {}
            | ActParams::Dump { .. }
            | ActParams::Drive { .. }
            | ActParams::Unload { .. } => None,
        }
    }

    /// **"You learn nothing new here."** (D4) against the operator's row for the site.
    pub(super) fn refused(&self, row: Option<&ChartRecord>) -> bool {
        match self {
            Reach::Scan {
                stand,
                fields,
                footing,
            } => repeat_refused(row, false, *stand, scan_adds(row, fields, *footing)),
            Reach::Sound { stand, pockets, .. } => {
                repeat_refused(row, true, *stand, sound_adds(row, pockets))
            }
        }
    }

    /// The units the act names, for the body caps: field indexes, block letters.
    pub(super) fn units(&self) -> (Vec<u8>, Vec<String>) {
        match self {
            Reach::Scan { fields, .. } => (fields.clone(), Vec::new()),
            Reach::Sound {
                pockets, scheme, ..
            } => {
                let blocks: std::collections::BTreeSet<String> = pockets
                    .iter()
                    .filter_map(|(px, py)| scheme.block_letter(*px, *py))
                    .map(|letter| letter.to_string())
                    .collect();
                (Vec::new(), blocks.into_iter().collect())
            }
        }
    }
}

/// **Which battery a rig carries** (finding F5): the lessor pool's tools
/// (`VehiclesDef.tools`) whose row in the one tools table (`PortHandles.surface_tools`) is a
/// battery — the world names **which**; the numbers always come from the server-held table.
#[must_use]
pub(crate) fn rig_battery_id(w: &World, rig_tv: ObjId) -> Option<CanonicalId> {
    crate::port::vehicle::rig_row(w, rig_tv)?;
    let tools = w.module_state::<PortHandles>()?.surface_tools.clone()?;
    crate::port::vehicle::rig_tools(w, rig_tv)
        .into_iter()
        .find(|id| tools.get(id).and_then(ToolDef::battery).is_some())
}

/// **A battery's figures** by its id, from a tools table (`ToolDef::battery`).
#[must_use]
pub(crate) fn battery_of<'t>(
    tools: &'t BTreeMap<CanonicalId, ToolDef>,
    id: &CanonicalId,
) -> Option<&'t BatteryDef> {
    tools.get(id).and_then(ToolDef::battery)
}

/// **Every rig of every lessor's pool** (the caps' census, lead ruling 8): each port map's
/// rig rows, bounded by `MAX_VEHICLE_POOL` a lessor. **Doc 20 cut SM.12b**: then every site map's
/// yard rigs (bounded by `MAX_BAY_ITEMS` a yard), so the caps and the site map's other rigs count
/// them.
#[must_use]
pub(crate) fn world_rigs(w: &World) -> Vec<ObjId> {
    w.module_state::<TacsimStore>()
        .map(|store| {
            let mut rigs: Vec<ObjId> = store
                .port_maps()
                .flat_map(crate::port::vehicle::lessor_rigs)
                .map(|rig| ObjId(rig.tv))
                .collect();
            rigs.extend(store.port_maps().flat_map(|map| {
                crate::port::vehicle::rig_rows(map, crate::port::vehicle::RigRows::Yard)
                    .iter()
                    .map(|rig| ObjId(rig.tv))
            }));
            rigs
        })
        .unwrap_or_default()
}

/// Whole percent of a battery: `⌊cu × 100 / capacity⌋`.
#[must_use]
pub(crate) fn percent(cu: u32, battery: &BatteryDef) -> u32 {
    if battery.capacity_cu == 0 {
        return 0;
    }
    u32::try_from(u64::from(cu).saturating_mul(100) / u64::from(battery.capacity_cu))
        .unwrap_or(u32::MAX)
}

/// **What an act in flight is doing** (`RIG_BUSY`'s and `ACT_LINE`'s `{doing}`). Doc 20 cut
/// SM.4b: a drive is `driving to {target}`, a recover with a leg `driving to the {ramp}`.
#[must_use]
pub(crate) fn doing_words(kind: ActKind, params: &ActParams, ramp: &str) -> String {
    match (kind, params) {
        (ActKind::Drive, ActParams::Drive { target, .. }) => format!("driving to {target}"),
        (ActKind::Recover, ActParams::Drive { .. }) => format!("driving to the {ramp}"),
        (ActKind::Drive, _) => "driving".to_owned(),
        (ActKind::Launch, _) => format!("going down the {ramp}"),
        (ActKind::Recover, _) => format!("climbing the {ramp}"),
        (ActKind::Dump, _) => "dumping its hopper".to_owned(),
        (ActKind::Scan, _) => "scanning".to_owned(),
        (ActKind::Sound, _) => "sounding".to_owned(),
        // Doc 20 cut SM.7: a run never sits in the act slot; the word is the status's.
        (ActKind::Mine, _) => "drilling".to_owned(),
        // Doc 20 cut SM.8.
        (ActKind::Unload, _) => "unloading its hopper into the hold".to_owned(),
    }
}

/// **`{doing}` for a run in flight** (doc 20 cut SM.7; the SM.7 lead's pre-commit round, item 2):
/// `drilling C117` — [`doing_words`]' word for the cycle slot, which holds a run rather than an
/// act, with the pocket it works. The departure's `TV_BUSY` and the lease return's `RIG_BUSY`
/// read it, each followed by `crate::port::tv::RUN_STOP_REMEDY`.
#[must_use]
pub(crate) fn drilling_words(pocket: &str) -> String {
    format!("drilling {pocket}")
}
