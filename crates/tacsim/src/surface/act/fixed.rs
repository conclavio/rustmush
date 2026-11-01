//! Offer fixed-parameter payloads and shared provider preparation helpers.

use super::*;

// ---- the offers' fixed parameters ------------------------------------------------------------

/// One act's plan as the start's offer fixes it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FixedPlan {
    pub(super) ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) site: Option<CanonicalId>,
    #[serde(default)]
    pub(super) secs: u32,
    #[serde(default)]
    pub(super) cost_cu: u32,
    #[serde(default)]
    pub(super) home_cu: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) params: Option<ActParams>,
    #[serde(default)]
    pub(super) from_bay: bool,
    /// A dump's hopper's accepted goods (the prepare reads each through its transaction).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) goods: Vec<CanonicalId>,
    /// **Doc 20 cut SM.4a**: a scan's or a sounding's body's site entries (at most
    /// `MAX_CHART_SITES_PER_BODY`), whose chart rows the prepare reads for the body caps.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) body_sites: Vec<CanonicalId>,
    /// **Doc 20 cut SM.4b** (the brief's decision P2): a drive's order's ordinal, which the
    /// prepare binds to the invocation's (`Conflict` otherwise). Doc 20 cut SM.7: a run's too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) ordinal: Option<u32>,
    /// **Doc 20 cut SM.7**: a run's server facts (truth among them — fixed parameters are
    /// server-only, never projected to a player).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) mine: Option<MineFixed>,
    /// **Doc 20 cut SM.8**: an unload's two possible sources, fixed (the offer cannot see the
    /// start's `how`; the prepare picks by it — the brief's decision R15).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) unload: Option<UnloadFixed>,
    /// **Doc 20 cut SM.11**: a plan step's binding — the plan, its `seq`, the step and the word
    /// the interpreter chose, the update its start writes — fixed under the rig's `plan` key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) plan: Option<PlanFixed>,
    /// **Doc 20 cut SM.11**: a filing's compiled plan, fixed under the rig's `plan_file` key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) fill: Option<FillFixed>,
}

/// **A plan step as the start's offer fixes it** (doc 20 cut SM.11; the brief's 3.6): the
/// interpreter's choice over the durable record, which the prepare holds the start's `plan`,
/// `step` and `act` to (`Conflict` "the plan moved on" otherwise). Server-only fixed parameters,
/// no truth: the act's own plan rides `FixedPlan` as the hand's does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PlanFixed {
    pub(super) no: u32,
    pub(super) seq: u32,
    pub(super) step: u16,
    pub(super) word: String,
    pub(super) update: PlanUpdate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) end: Option<PlanEnd>,
}

/// **A filing as the start's offer fixes it** (doc 20 cut SM.11; the brief's 3.6): the order's
/// number and ordinal and its re-compiled list, family and steps — required equal to the preview.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FillFixed {
    pub(super) no: u32,
    pub(super) ordinal: u32,
    pub(super) list: Vec<String>,
    pub(super) family: OreFamily,
    pub(super) steps: Vec<PlanStep>,
    /// Doc 32 cut S.1: the site's side the list compiled under (omitted at 256).
    #[serde(
        default = "crate::surface::excursion::shipped_side",
        skip_serializing_if = "crate::surface::excursion::is_shipped_side"
    )]
    pub(super) side: u16,
}

/// **An unload's server facts, as the start's offer fixes them** (doc 20 cut SM.8; the brief's
/// 3.7, decisions R4, R15 and R16): the vaults, both variants the prepare may run, the hull's
/// hold plan (so the prepare weighs with the one table and compartment the preview did, with no
/// World), the goods the hold's ore compartment accepts (table order) and the hull's other rigs
/// (their hoppers and runs are the reservation the prepare re-reads in its transaction).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UnloadFixed {
    pub(super) hopper: u64,
    pub(super) hold: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) hand: Option<HandFixed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) queued: Option<QueuedFixed>,
    pub(super) plan: crate::port::market::HoldPlan,
    pub(super) goods_of_hold: Vec<CanonicalId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) siblings: Vec<Sibling>,
}

/// **The hand variant**: the operator's `+tv/unload go` order for the rig's next ordinal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HandFixed {
    pub(super) ordinal: u32,
    pub(super) how: UnloadHow,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) qty: Option<u64>,
    pub(super) goods: BTreeMap<CanonicalId, u64>,
}

/// **The queued variant**: the recover's queued unload, split against the hold's room now
/// (possibly moving nothing).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct QueuedFixed {
    pub(super) ordinal: u32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) goods: BTreeMap<CanonicalId, u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) excursion: Option<ExcursionId>,
}

/// One of the hull's other rigs, as the unload's re-check reads it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Sibling {
    pub(super) tv: ObjId,
    pub(super) hopper: u64,
    pub(super) goods: Vec<CanonicalId>,
}

/// **A run's server facts, as the start's offer fixes them** (doc 20 cut SM.7; the brief's 3.4):
/// everything the prepare writes into the run (so the finish needs no geology and no site read)
/// and checks against the transaction. Serialises because fixed parameters must; they are
/// server-only (`InternalOffer` is not serializable to a player), the cursor's precedent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MineFixed {
    pub(super) body: CanonicalId,
    pub(super) stand: [i64; 2],
    pub(super) pocket: [i64; 2],
    pub(super) number: u16,
    pub(super) letter: char,
    pub(super) family: OreFamily,
    pub(super) good: CanonicalId,
    pub(super) deposit: mush_industry::OreDepositId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) source: Option<u64>,
    pub(super) hopper: u64,
    pub(super) cycles: u8,
    pub(super) yields: Vec<u32>,
    pub(super) reserve_t: Vec<u8>,
    pub(super) class_t: u8,
    pub(super) pocket_t: u8,
    pub(super) blind: bool,
    pub(super) secs: u32,
    pub(super) cost_cu: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) lock_rows: Vec<LockRow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) lapses_ic: Option<i64>,
    pub(super) drawn: u32,
    pub(super) outstanding: Vec<(ObjId, Option<(u32, u64)>)>,
    /// Doc 32 cut S.1: the site's side the pocket is named under (omitted at 256).
    #[serde(
        default = "crate::surface::excursion::shipped_side",
        skip_serializing_if = "crate::surface::excursion::is_shipped_side"
    )]
    pub(super) side: u16,
}

impl MineFixed {
    pub(super) fn of(plan: &MinePlan) -> MineFixed {
        MineFixed {
            body: plan.body.clone(),
            stand: plan.stand,
            pocket: plan.pocket,
            number: plan.number,
            letter: plan.letter,
            family: plan.family,
            good: plan.good.clone(),
            deposit: plan.deposit,
            source: plan.source,
            hopper: plan.hopper,
            cycles: plan.cycles,
            yields: plan.yields.clone(),
            reserve_t: plan.reserve_t.clone(),
            class_t: plan.class_t,
            pocket_t: plan.pocket_t,
            blind: plan.blind,
            secs: plan.secs,
            cost_cu: plan.cost_cu,
            lock_rows: plan.lock_rows.clone(),
            lapses_ic: plan.lapses_ic,
            drawn: plan.drawn,
            outstanding: plan.outstanding.clone(),
            side: plan.side,
        }
    }
}

impl FixedPlan {
    pub(super) fn refused(word: &str) -> FixedPlan {
        FixedPlan {
            ok: false,
            code: Some(word.to_owned()),
            ..FixedPlan::default()
        }
    }

    pub(super) fn of(plan: Result<ActPlan, Refusal>) -> FixedPlan {
        match plan {
            Ok(plan) => FixedPlan {
                ok: true,
                code: None,
                site: Some(plan.site),
                secs: plan.secs,
                cost_cu: plan.cost_cu,
                home_cu: plan.home_cu,
                params: Some(plan.params),
                from_bay: plan.from_bay,
                goods: Vec::new(),
                body_sites: Vec::new(),
                ordinal: None,
                mine: None,
                unload: None,
                plan: None,
                fill: None,
            },
            Err(refusal) => FixedPlan::refused(refusal.word()),
        }
    }
}

/// **The start's fixed plan of a drive** (doc 20 cut SM.4b; the brief's decision P2): the rig's
/// [`DriveOrder`] — none for this rig and actor → `unplanned` — re-checked against the world:
/// the same stand gate a scan runs (`stand_of`: from the bay the launch's whole order), and the
/// rig's stand must be where the order's leg begins on the order's site (the ramp's cell from the
/// bay), else `moved`. Then the order's plan with its ordinal. The offer reads no composite for a
/// drive.
pub(super) fn drive_fixed(w: &World, actor: ObjId, rig: &RigAt) -> FixedPlan {
    let Some(order) = order_of(w, rig.tv).filter(|order| order.actor == actor) else {
        return FixedPlan::refused(Refusal::Unplanned.word());
    };
    let stand = match stand_of(w, actor, rig) {
        Ok(stand) => stand,
        Err(refusal) => return FixedPlan::refused(refusal.word()),
    };
    let start = order
        .params
        .leg()
        .and_then(|(waypoints, _)| waypoints.first())
        .map(|first| site_cell(first.x_cm, first.y_cm));
    if stand.site != order.site || stand.from_bay != order.from_bay || start != Some(stand.cell) {
        return FixedPlan::refused(Refusal::Moved.word());
    }
    FixedPlan {
        ok: true,
        code: None,
        site: Some(order.site),
        secs: order.secs,
        cost_cu: order.cost_cu,
        home_cu: order.home_cu,
        params: Some(order.params),
        from_bay: order.from_bay,
        goods: Vec::new(),
        body_sites: Vec::new(),
        ordinal: Some(order.ordinal),
        mine: None,
        unload: None,
        plan: None,
        fill: None,
    }
}

/// **The start's fixed plan of a run** (doc 20 cut SM.7; the brief's 3.4, decision R10): the
/// rig's [`MineOrder`] — none for this rig and actor → `unplanned`; nor one whose ordinal is not
/// the rig's next (`next_ordinal`, read from its record by the offer: the order's start landed, or
/// another act overtook it), dropped quietly **without a re-plan** (the SM.7 lead's pre-commit
/// round, item 5; the tick's `prune_mine_orders` then clears it) — then [`mine_plan`] re-run
/// with the order's choice over the engine's own snapshot (the offer's peeks) and **required equal
/// to the order's plan**, else `mine_changed`: `go` commits exactly what was previewed or nothing.
/// The run's charge and its trip home ride the plan's figures; its server facts ride
/// [`MineFixed`]; a blind core names the body's site entries for the prepare's body caps.
#[allow(clippy::too_many_arguments)]
pub(super) fn mine_fixed(
    engine: &mush_core::Engine,
    actor: ObjId,
    rig: &RigAt,
    drill: Option<&DrillDef>,
    has_battery: bool,
    rates: Option<&DriveRates>,
    peeks: &mut Peeks,
    next_ordinal: Option<u32>,
) -> FixedPlan {
    let w: &World = &engine.world;
    let Some(order) = mine_order_of(w, rig.tv)
        .filter(|order| order.actor == actor)
        .filter(|order| next_ordinal.is_none_or(|next| next == order.ordinal))
    else {
        return FixedPlan::refused(Refusal::Unplanned.word());
    };
    let (Some(drill), true) = (drill, has_battery) else {
        return FixedPlan::refused("tv_dark");
    };
    let reads = crate::port::site_map::EngineReads(engine);
    let plan = match mine_plan(w, &reads, actor, rig, &order.choice, drill, rates, peeks) {
        Ok(plan) => plan,
        Err(refusal) => return FixedPlan::refused(refusal.word()),
    };
    if plan != order.plan {
        return FixedPlan::refused(Refusal::MineChanged.word());
    }
    let (Some(cost_cu), Some(secs)) = (plan.run_cost_cu(), plan.run_secs()) else {
        return FixedPlan::refused("tv_dark");
    };
    FixedPlan {
        ok: true,
        code: None,
        site: Some(plan.site.clone()),
        secs,
        cost_cu,
        home_cu: plan.home_cu,
        params: None,
        from_bay: false,
        goods: Vec::new(),
        body_sites: if plan.blind {
            chart::body_sites(w, &plan.body)
        } else {
            Vec::new()
        },
        ordinal: Some(order.ordinal),
        mine: Some(MineFixed::of(&plan)),
        unload: None,
        plan: None,
        fill: None,
    }
}

/// **The start's fixed plan of an unload** (doc 20 cut SM.8; the brief's 3.7, decisions R4, R6,
/// R15): both possible sources, since the legal-action query carries no parameters and the
/// prepare picks by the start's `how`:
///
/// - **the hand variant** — the actor's [`UnloadOrder`] for this rig **planned for the rig's next
///   ordinal** (an order left from an earlier, landed unload is stale and ignored: RAM orders are
///   never removed on landing), [`unload_plan`] re-run (`queued` false);
/// - **the queued variant** — the record's [`Queued`] for this actor, [`unload_plan`] re-run
///   (`queued` true: no aboard gate) and the hopper's available goods split against the hold's
///   room now ([`unload_room`], `crate::surface::unload::whole`) — possibly moving nothing — and,
///   doc 20 cut SM.11 (the lead's pre-commit round, item 1), **the hopper's marker ring able to
///   record the split's moves at the offer's real second** ([`ring_admits`]): while it
///   cannot, the variant is refused `unload_markers`, nothing lands and the queue stands. The
///   surface tick reads the same rule first ([`queued_ring_full`], the reviewer's R1): while the
///   ring is full it submits nothing, tells `WHY_MARKERS_QUEUED` once and re-reads the ring on the
///   queue's back-off (every 60 fires), so the unload, and a plan waiting on it, goes on by itself
///   once the ring frees; this refusal is the backstop for a ring that filled in between.
///
/// `ok` when either variant planned; otherwise refused with the hand variant's word, else the
/// queued's (`unload_markers` for a full ring), else `unplanned`. The plan names the site,
/// `UNLOAD_S`, and the server facts ([`UnloadFixed`]); its `params` are `None` — the prepare builds
/// the act's parameters from the chosen variant.
pub(super) fn unload_fixed(engine: &mush_core::Engine, actor: ObjId, rig: &RigAt) -> FixedPlan {
    let w: &World = &engine.world;
    let reads = crate::port::site_map::EngineReads(engine);
    let Ok(record) = reads.rig(rig.tv) else {
        return FixedPlan::refused("tv_dark");
    };
    let Some(next) = record
        .as_ref()
        .map_or(Some(1), |record| record.ordinal.checked_add(1))
    else {
        return FixedPlan::refused("tv_dark");
    };
    let order =
        unload_order_of(w, rig.tv).filter(|order| order.actor == actor && order.ordinal == next);
    let queued = record
        .as_ref()
        .and_then(|record| record.queued.clone())
        .filter(|queued| queued.operator == actor && queued.kind == ActKind::Unload);
    if order.is_none() && queued.is_none() {
        return FixedPlan::refused(Refusal::Unplanned.word());
    }
    let hand_plan = order.as_ref().map(|_| unload_plan(w, actor, rig, false));
    let queued_plan = queued.as_ref().map(|_| unload_plan(w, actor, rig, true));
    let base = match (&hand_plan, &queued_plan) {
        (Some(Ok(plan)), _) | (_, Some(Ok(plan))) => plan.clone(),
        (Some(Err(refusal)), _) | (None, Some(Err(refusal))) => {
            return FixedPlan::refused(refusal.word());
        }
        (None, None) => return FixedPlan::refused(Refusal::Unplanned.word()),
    };
    let ActParams::Unload { hopper, hold, .. } = &base.params else {
        return FixedPlan::refused("tv_dark");
    };
    let (hopper, hold) = (*hopper, *hold);
    let Some(view) = hull_view(w, rig.hull) else {
        return FixedPlan::refused("tv_dark");
    };
    let read_pool = |endpoint: EndpointRef, good: &CanonicalId| reads.pool(endpoint, good);
    let read_rig = |tv: ObjId| reads.rig(tv);
    let Some(room) = unload_room(w, &view, rig.tv, &read_pool, &read_rig) else {
        return FixedPlan::refused("tv_dark");
    };
    let mut siblings = Vec::with_capacity(room.siblings.len());
    for other in &room.siblings {
        let (Some(vault), Some((goods, _))) = (
            crate::port::vehicle::hopper_of(w, *other),
            tv::hopper_goods_of(w, *other),
        ) else {
            return FixedPlan::refused("tv_dark");
        };
        siblings.push(Sibling {
            tv: *other,
            hopper: vault.0,
            goods,
        });
    }
    let goods_of_hold: Vec<CanonicalId> = room
        .goods
        .goods
        .iter()
        .filter(|good| room.compartment.accepts.contains(&good.stowage))
        .map(|good| good.id.clone())
        .collect();
    let hand = match (order, &hand_plan) {
        (Some(order), Some(Ok(_))) => Some(HandFixed {
            ordinal: next,
            how: order.how,
            qty: order.qty,
            goods: order.goods,
        }),
        _ => None,
    };
    let mut markers_full = false;
    let queued = match (queued, &queued_plan) {
        (Some(queued), Some(Ok(_))) => {
            // Doc 20 cut SM.11 (the lead's pre-commit round, item 1): a ring that cannot record
            // the moves refuses the queued start before it lands (its finish would only record
            // `MarkersFull` again); a split that moves nothing writes no marker. The surface tick
            // reads the same two functions first (`queued_ring_full`, the reviewer's R1) and does
            // not submit into a full ring, so this refusal is the backstop for a ring that filled
            // between the tick's read and this offer.
            match queued_split(w, &reads, rig.tv, &room) {
                Some(split) if ring_admits(&reads, hopper, split.moves.len(), w.now()) => {
                    Some(QueuedFixed {
                        ordinal: next,
                        goods: split.moves,
                        excursion: queued.excursion,
                    })
                }
                Some(_) => {
                    markers_full = true;
                    None
                }
                None => None,
            }
        }
        _ => None,
    };
    if hand.is_none() && queued.is_none() {
        let word = match (&hand_plan, &queued_plan) {
            (Some(Err(refusal)), _) | (None, Some(Err(refusal))) => refusal.word(),
            _ if markers_full => Refusal::UnloadMarkers { n: rig.slot }.word(),
            _ => "tv_dark",
        };
        return FixedPlan::refused(word);
    }
    FixedPlan {
        ok: true,
        code: None,
        site: Some(base.site),
        secs: UNLOAD_S,
        cost_cu: 0,
        home_cu: 0,
        params: None,
        from_bay: false,
        goods: Vec::new(),
        body_sites: Vec::new(),
        ordinal: Some(next),
        mine: None,
        unload: Some(UnloadFixed {
            hopper,
            hold,
            hand,
            queued,
            plan: room.plan,
            goods_of_hold,
            siblings,
        }),
        plan: None,
        fill: None,
    }
}

/// **The queued unload's split** (doc 20 cut SM.8's whole split; cut SM.11, the reviewer's R1):
/// the hopper's available goods (`SurfaceReads::available`) split against `room` —
/// `crate::surface::unload::whole` — exactly as the queued start's offer moves them. `None` when
/// the hopper or the split will not read. One function for the offer, the surface tick and the
/// plan's `Markers` wait, so the three never disagree on how many markers the start would write.
pub(crate) fn queued_split(
    w: &World,
    reads: &dyn SurfaceReads,
    tv: ObjId,
    room: &UnloadRoom,
) -> Option<crate::surface::unload::Split> {
    let pool_unit = tv::pool_unit(w);
    let available = |endpoint: EndpointRef, good: &CanonicalId| reads.available(endpoint, good);
    tv::hopper_contents_with(w, tv, pool_unit.as_ref(), &available)
        .ok()
        .and_then(|contents| unload_split(room, &contents, None).ok())
        .and_then(Result::ok)
}

/// **Whether the hopper's marker ring can record `moves` more markers at `now_wall`** (doc 20 cut
/// SM.11, the lead's pre-commit round, item 1): the ring read through `reads`
/// ([`SurfaceReads::marker_ring`]) and `mush_inventory::MarkerRing::admits` at the real second
/// (`World::now`, the `wall` the finish's offer stamps). Nothing to move needs no marker; an
/// absent ring is empty; a ring or a roster that will not read admits (the finish's own
/// `MarkerRingFull` refusal stays the authority, so a dark read never wedges a queue).
pub(crate) fn ring_admits(
    reads: &dyn SurfaceReads,
    hopper: u64,
    moves: usize,
    now_wall: i64,
) -> bool {
    if moves == 0 {
        return true;
    }
    match reads.marker_ring(EndpointRef(VaultId(hopper))) {
        Ok(Some(ring)) => ring.admits(moves, now_wall),
        Ok(None) | Err(_) => true,
    }
}

/// **Whether a queued unload waits on its hopper's full marker ring** (doc 20 cut SM.11, the
/// reviewer's R1): `queued`'s rig's split ([`queued_split`] against [`unload_room`] for the queue's
/// hull) moves goods the ring cannot record at `now_wall` ([`ring_admits`]) — the very test the
/// queued start's offer refuses `unload_markers` by. The surface tick reads it before it submits
/// the queued start (and holds, telling `WHY_MARKERS_QUEUED` once, rather than submit a start
/// known to fail); the plan's interpreter reads it for its `Markers` wait. `false` whenever any
/// read is dark (the offer and the finish stay the authority).
pub(crate) fn queued_ring_full(
    w: &World,
    reads: &dyn SurfaceReads,
    tv: ObjId,
    queued: &Queued,
    now_wall: i64,
) -> bool {
    if queued.kind != ActKind::Unload {
        return false;
    }
    let Some(view) = hull_view(w, queued.hull) else {
        return false;
    };
    let read_pool = |endpoint: EndpointRef, good: &CanonicalId| reads.pool(endpoint, good);
    let read_rig = |rig: ObjId| reads.rig(rig);
    let Some(room) = unload_room(w, &view, tv, &read_pool, &read_rig) else {
        return false;
    };
    let (Some(split), Some(hopper)) = (
        queued_split(w, reads, tv, &room),
        crate::port::vehicle::hopper_of(w, tv),
    ) else {
        return false;
    };
    !ring_admits(reads, hopper.0, split.moves.len(), now_wall)
}

/// **A plan step's act as the start fixes it** (doc 20 cut SM.11; the brief's 3.6): the plan
/// variant built exactly as the hand path builds that act's — a sounding's or a recover's plan
/// (the body's sites for a reading's caps), a drive's leg with the record's next ordinal (the
/// drive's order shape), a run's [`MineFixed`] — plus its [`PlanFixed`] binding.
pub(super) fn plan_fixed(
    w: &World,
    plan: &FillPlan,
    next: u32,
    moved: crate::surface::plan::PlanMove,
    has_battery: bool,
) -> FixedPlan {
    use crate::surface::plan::{PlanActInner, PlanMove};
    match moved {
        PlanMove::Start {
            word,
            step,
            update,
            act,
        } => {
            let binding = PlanFixed {
                no: plan.no,
                seq: plan.seq,
                step,
                word: word.to_owned(),
                update,
                end: None,
            };
            let mut fixed = match act.0 {
                PlanActInner::Act(planned) => {
                    if word == "sound" {
                        reading_fixed(w, Some(Ok(planned)), has_battery)
                    } else {
                        FixedPlan::of(Ok(planned))
                    }
                }
                PlanActInner::Drive(planned) => {
                    if !has_battery {
                        return FixedPlan::refused("tv_dark");
                    }
                    let mut fixed = FixedPlan::of(Ok(planned));
                    fixed.ordinal = Some(next);
                    fixed
                }
                PlanActInner::Mine(planned) => {
                    let (Some(cost_cu), Some(secs), true) =
                        (planned.run_cost_cu(), planned.run_secs(), has_battery)
                    else {
                        return FixedPlan::refused("tv_dark");
                    };
                    FixedPlan {
                        ok: true,
                        code: None,
                        site: Some(planned.site.clone()),
                        secs,
                        cost_cu,
                        home_cu: planned.home_cu,
                        params: None,
                        from_bay: false,
                        goods: Vec::new(),
                        body_sites: if planned.blind {
                            chart::body_sites(w, &planned.body)
                        } else {
                            Vec::new()
                        },
                        ordinal: Some(next),
                        mine: Some(MineFixed::of(&planned)),
                        unload: None,
                        plan: None,
                        fill: None,
                    }
                }
            };
            if fixed.ok {
                fixed.plan = Some(binding);
            }
            fixed
        }
        PlanMove::End { why, update } => FixedPlan {
            ok: true,
            plan: Some(PlanFixed {
                no: plan.no,
                seq: plan.seq,
                step: plan.cursor,
                word: "plan_end".to_owned(),
                update,
                end: Some(why),
            }),
            ..FixedPlan::default()
        },
        PlanMove::Wait(_) => FixedPlan::refused("plan_waits"),
    }
}

/// One rig as the start's offer fixes it: its plans keyed by the start's `act` word
/// ([`ACT_WIRE_WORDS`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FixedRig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) crew: Option<Box<crate::detachment::sortie::SurfaceCrew>>,
    pub(super) slot: u8,
    pub(super) hull: ObjId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) battery_id: Option<CanonicalId>,
    pub(super) plans: BTreeMap<String, FixedPlan>,
}

/// The world as the finish's offer reads it at landing (decision 5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "where", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Facts {
    /// In a programme hull's bay.
    Bay { hull: ObjId },
    /// On a domain: the site whose domain it is (if any) and the cells from that site's
    /// ramp (through the hull of the rig's live record), and — doc 20 cut SM.4a — the rig's
    /// site cell on a site's domain (a scan's or a sounding's frozen stand is compared with it).
    Ground {
        domain: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        site: Option<CanonicalId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cells: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cell: Option<[i64; 2]>,
    },
    /// Anywhere else.
    Elsewhere,
}

/// One rig as the finish's offer fixes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DoneRig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) crew: Option<Box<crate::detachment::sortie::SurfaceCrew>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) battery_id: Option<CanonicalId>,
    pub(super) facts: Facts,
    /// **The live spoil vault of the site the rig's hull is settled on** (`spoil_vault`: the
    /// spoil Thing live, the vault the sites' system principal's and Active), when there is
    /// one: a dump whose frozen spoil is not this one records `SpoilGone` (the fixer round's
    /// item 4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) spoil: Option<u64>,
    /// **The act ordinals of this rig whose finish was handed to the engine and did not land**
    /// (the tick's counted attempts, `crate::surface::tick::finish_retried`: ascending, at most
    /// `MAX_RETRIED_ORDINALS`). A dump's transfer that refuses mid-bundle cannot be refused
    /// inside its own bundle — `ActionTxn` keeps no savepoint and refuses a second mutation of
    /// one key, so the debit it staged would commit — so its next finish records `SpoilGone`
    /// and moves nothing (the fixer round's item 4: a failed dump transfer records a refusal,
    /// it never wedges the act). Keyed by the act (review finding F1): only the finishing act's
    /// own ordinal counts, never a stale attempt of an earlier act of the rig. **Doc 20 cut SM.7**:
    /// `(ordinal, cycle)` pairs — cycle 0 an act's, `k` a run's cycle — so a cycle whose transfer
    /// refused mid-bundle records `Unworked` on its next finish.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) failed: Vec<(u32, u8)>,
    /// **The rights of the run's next cycle, read at landing** (doc 20 cut SM.7; §8 "rights are per
    /// act"; the brief's 3.7): `Some` for a rig whose record holds a run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) rights: Option<RightsVerdict>,
    /// **The rig's hopper at landing** (doc 20 cut SM.8): its vault and the goods it accepts — a
    /// `Done` recover reads them to queue the unload; the unload reads them to stay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) hopper: Option<HopperFacts>,
    /// **The hold of the rig's lease record's hull at landing** (doc 20 cut SM.8): its vault and
    /// its plan — the unload's finish re-checks the hold against it with no World.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) hold: Option<HoldFacts>,
    /// **What the launch's `Done` opens the epoch with** (doc 23 cut S.1, L3/L8): the ROM seal's
    /// facts, the unit class and the site's open epochs — `Some` for a rig whose act in flight
    /// leaves the bay (`crate::surface::black_box::epoch_facts`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) epoch: Option<crate::surface::black_box::EpochFacts>,
}

/// A rig's hopper, as the finish's offer fixes it (doc 20 cut SM.8).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HopperFacts {
    pub(super) vault: u64,
    pub(super) goods: Vec<CanonicalId>,
}

/// A hull's hold, as the finish's offer fixes it (doc 20 cut SM.8).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HoldFacts {
    pub(super) vault: u64,
    pub(super) plan: crate::port::market::HoldPlan,
}

/// The live spoil vault of the site `hull` is settled on (`crate::port::site::spoil_vault`).
pub(super) fn spoil_of(w: &World, hull: ObjId) -> Option<u64> {
    let view = hull_view(w, hull)?;
    let berth = site_berth(w, &view).ok()?;
    crate::port::site::spoil_vault(w, berth.map).map(|vault| vault.0)
}

/// The facts of `tv` at landing, its ramp read through `hull` (its live record's).
pub(super) fn facts_of(w: &World, tv: ObjId, hull: ObjId) -> Facts {
    match where_is(w, tv) {
        Where::Bay { hull } => Facts::Bay { hull },
        Where::Ground {
            domain, x_cm, y_cm, ..
        } => {
            let site = site_of_domain(w, domain);
            // Doc 32 cut S.1: the ramp off the site's own pad.
            let pad = site.as_ref().map_or(
                (
                    crate::surface::excursion::PAD_CENTRE_CM,
                    crate::surface::excursion::PAD_CENTRE_CM,
                ),
                |site| crate::surface::excursion::site_pad_cm(w, site),
            );
            let cells = hull_view(w, hull)
                .and_then(|view| {
                    view.bay()
                        .and_then(|(bay, _)| crate::surface::excursion::ramp_pose_at(bay, pad).ok())
                })
                .map(|ramp| cells_apart(site_cell(x_cm, y_cm), site_cell(ramp.x_cm, ramp.y_cm)));
            let cell = site.as_ref().map(|_| {
                let (x, y) = site_cell(x_cm, y_cm);
                [x, y]
            });
            Facts::Ground {
                domain: domain.0,
                site,
                cells,
                cell,
            }
        }
        Where::Elsewhere => Facts::Elsewhere,
    }
}

// ---- the providers' shared plumbing -------------------------------------------------------

/// The offer's guard set and presentation (the burn's shape).
pub(crate) fn offer_draft(
    action: &DefinitionRef,
    engine: &mush_core::Engine,
    fixed: Value,
    parameters: Vec<ProjectedParameter>,
    presentation: (&str, &str, &str),
) -> Result<ProviderOfferDraft, ActionError> {
    offer_draft_with_revisions(
        action,
        engine,
        fixed,
        parameters,
        presentation,
        RevisionSet::default(),
    )
}

/// The same provider presentation with exact owner-supplied world observations.
pub(crate) fn offer_draft_with_revisions(
    action: &DefinitionRef,
    engine: &mush_core::Engine,
    fixed: Value,
    parameters: Vec<ProjectedParameter>,
    presentation: (&str, &str, &str),
    revisions: RevisionSet,
) -> Result<ProviderOfferDraft, ActionError> {
    let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
    let definitions = engine
        .registry
        .definitions()
        .ok_or_else(|| integrity("the surface act's definition registry is unavailable"))?;
    let mut guards = engine
        .registry
        .server_guard_set_builder(revisions)
        .map_err(|_| integrity("the surface act's guard registry is unavailable"))?;
    guards
        .add_definition(action.clone(), definitions)
        .map_err(|_| integrity("the surface act's definition guard is unavailable"))?;
    Ok(ProviderOfferDraft {
        fixed_parameters: fixed,
        parameters: ProjectedParameterSchema::new(parameters)
            .map_err(|_| integrity("the surface act's parameter schema is invalid"))?,
        targets: ProjectedTargetSchema::None,
        required_channels: Vec::new(),
        expires: None,
        presentation: mush_core::ActionPresentation::new(
            presentation.0,
            presentation.1,
            presentation.2,
        )
        .map_err(|_| integrity("the surface act's presentation is invalid"))?,
        guards: guards
            .finish()
            .map_err(|_| integrity("the surface act's offer guards are invalid"))?,
    })
}

/// One projected parameter.
pub(crate) fn parameter(
    id: &str,
    required: bool,
    kind: ProjectedParameterKind,
) -> Result<ProjectedParameter, ActionError> {
    Ok(ProjectedParameter {
        id: CanonicalId::new(id).map_err(|_| {
            port_error(
                ActionErrorCode::Integrity,
                "the surface act's parameter id is invalid",
            )
        })?,
        required,
        kind,
    })
}

/// Whether the query is this provider's self-scoped one.
pub(crate) fn self_query(
    action: &DefinitionRef,
    ours: &DefinitionRef,
    query: &mush_core::LegalActionQuery,
) -> bool {
    let SubjectKey::Object { id: subject } = query.truth_subject() else {
        return false;
    };
    action == ours
        && query.actor() == *subject
        && query.projected_subject() == &PerceivedRef::SelfActor
}

pub(super) fn invalid(message: &'static str) -> ActionError {
    port_error(ActionErrorCode::InvalidRequest, message)
}

/// The invocation's parameters: self-scoped, no targets, an object.
pub(super) fn parameters_of(
    invocation: &ActionInvocation,
) -> Result<&Map<String, Value>, ActionError> {
    if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
        return Err(invalid("a surface act is self-scoped and takes no targets"));
    }
    invocation
        .parameters()
        .as_object()
        .ok_or_else(|| invalid("a surface act names a rig and an act"))
}

/// The `tv` parameter: strict decimal text.
pub(super) fn tv_of(parameters: &Map<String, Value>) -> Result<ObjId, ActionError> {
    parameters
        .get("tv")
        .and_then(Value::as_str)
        .and_then(RigKey::parse)
        .map(|key| key.0)
        .ok_or_else(|| invalid("a surface act names one rig"))
}

/// The `ordinal` parameter: `1..=u32::MAX`.
pub(super) fn ordinal_of(parameters: &Map<String, Value>) -> Result<u32, ActionError> {
    parameters
        .get("ordinal")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n >= 1)
        .ok_or_else(|| invalid("a surface act names its ordinal"))
}

/// The `act` parameter through [`start_word`] (D23; doc 20 cut SM.7): an act's kind and the
/// sounding's quiet flag, or `mine_stop`.
pub(super) fn act_param(parameters: &Map<String, Value>) -> Result<StartWord, ActionError> {
    parameters
        .get("act")
        .and_then(Value::as_str)
        .and_then(start_word)
        .ok_or_else(|| {
            invalid(
                "a surface act names launch, recover, dump, scan, sound, sound_quiet, drive, mine, \
                 mine_stop, unload, plan_file, plan_clear, plan_strike or plan_end",
            )
        })
}

/// **The finish's `cycle` parameter** (doc 20 cut SM.7): absent for an act's finish, else a run's
/// cycle `1..=`[`crate::surface::excursion::MAX_RUN_CYCLES`].
pub(super) fn cycle_param(parameters: &Map<String, Value>) -> Result<Option<u8>, ActionError> {
    match parameters.get("cycle") {
        None => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|k| u8::try_from(k).ok())
            .filter(|k| (1..=crate::surface::excursion::MAX_RUN_CYCLES).contains(k))
            .map(Some)
            .ok_or_else(|| invalid("a run's finish names its cycle, 1 to 8")),
    }
}

/// **A drive start's `to` parameter** (the review's correctness/F5): the planned end cell's
/// word ([`order_to_word`]), `None` when absent (every other act's start, and a forged drive
/// start, which the prepare then refuses).
pub(super) fn to_word_of(parameters: &Map<String, Value>) -> Option<u32> {
    parameters
        .get("to")
        .and_then(Value::as_u64)
        .and_then(|word| u32::try_from(word).ok())
}

/// **An unload start's `how` parameter** (doc 20 cut SM.8; decision R15): `None` when absent or
/// not one of the three words.
pub(super) fn how_of(parameters: &Map<String, Value>) -> Option<UnloadHow> {
    parameters
        .get("how")
        .and_then(Value::as_str)
        .and_then(UnloadHow::from_word)
}

/// **A part unload's `qty` parameter** (doc 20 cut SM.8): whole tonnes `1..=99`, `None` when
/// absent or out of range.
pub(super) fn qty_of(parameters: &Map<String, Value>) -> Option<u64> {
    parameters
        .get("qty")
        .and_then(Value::as_u64)
        .filter(|qty| (1..=tv::MAX_TYPED_TONNES).contains(qty))
}

/// The fixed `now`.
pub(super) fn now_of(parameters: &Map<String, Value>) -> Result<i64, ActionError> {
    parameters
        .get("now")
        .and_then(Value::as_i64)
        .ok_or_else(|| invalid("a surface act's offer fixes the instant"))
}

/// **The start's `by` word** (doc 20 cut SM.11; the brief's 3.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ByWord {
    /// A fill plan's step or end.
    Plan,
    /// The idle return.
    Idle,
}

/// **The start's `by`, `plan` and `step` parameters**, read and held to the parameter rule.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ByParams {
    /// `by`.
    pub by: Option<ByWord>,
    /// `plan`: the plan's number.
    pub plan: Option<u32>,
    /// `step`: the step.
    pub step: Option<u16>,
}

/// **The parameter rule** (doc 20 cut SM.11; the brief's 3.6): `by: plan` requires `plan` and
/// `step` and is legal only on `drive`, `scan`, `sound`, `mine`, `recover` and `plan_end`; `by:
/// idle` only on `recover`, carrying neither; `plan_file` carries none of the three; `plan_clear`
/// and `plan_strike` carry `plan` only; any other combination is `InvalidRequest` "a plan's start
/// names its plan and step". A malformed value of any of the three is refused the same way.
pub(super) fn by_params(
    word: StartWord,
    parameters: &Map<String, Value>,
) -> Result<ByParams, ActionError> {
    let refuse = || invalid("a plan's start names its plan and step");
    let by = match parameters.get("by") {
        None => None,
        Some(value) => match value.as_str() {
            Some("plan") => Some(ByWord::Plan),
            Some("idle") => Some(ByWord::Idle),
            _ => return Err(refuse()),
        },
    };
    let plan = match parameters.get("plan") {
        None => None,
        Some(value) => Some(
            value
                .as_u64()
                .and_then(|no| u32::try_from(no).ok())
                .filter(|no| *no >= 1)
                .ok_or_else(refuse)?,
        ),
    };
    let step = match parameters.get("step") {
        None => None,
        Some(value) => Some(
            value
                .as_u64()
                .and_then(|step| u16::try_from(step).ok())
                .filter(|step| usize::from(*step) <= MAX_PLAN_STEPS)
                .ok_or_else(refuse)?,
        ),
    };
    let ok = match (word, by) {
        (StartWord::PlanFile, None) => plan.is_none() && step.is_none(),
        (StartWord::PlanClear | StartWord::PlanStrike, None) => plan.is_some() && step.is_none(),
        (StartWord::PlanEnd, Some(ByWord::Plan)) => plan.is_some() && step.is_some(),
        (
            StartWord::Act {
                kind:
                    ActKind::Drive | ActKind::Scan | ActKind::Sound | ActKind::Mine | ActKind::Recover,
                ..
            },
            Some(ByWord::Plan),
        ) => plan.is_some() && step.is_some(),
        (
            StartWord::Act {
                kind: ActKind::Recover,
                ..
            },
            Some(ByWord::Idle),
        ) => plan.is_none() && step.is_none(),
        (StartWord::Act { .. } | StartWord::MineStop, None) => plan.is_none() && step.is_none(),
        (
            StartWord::PlanFile
            | StartWord::PlanClear
            | StartWord::PlanStrike
            | StartWord::PlanEnd
            | StartWord::MineStop
            | StartWord::Act { .. },
            Some(_),
        )
        | (StartWord::PlanEnd, None) => false,
    };
    if ok {
        Ok(ByParams { by, plan, step })
    } else {
        Err(refuse())
    }
}

/// **Who set a start going, and its plan binding** (doc 20 cut SM.11): what the write stamps on
/// the act or the run, and the plan's update it applies in the same bundle.
#[derive(Clone, Debug, Default)]
pub(super) struct Stamp {
    pub(super) by: Option<ActBy>,
    pub(super) plan: Option<PlanFixed>,
}

/// `Conflict` "the plan moved on" (the brief's D16: the world moved between the decision and the
/// landing; the next fire decides again).
pub(super) fn moved_on() -> ActionError {
    port_error(ActionErrorCode::Conflict, "the plan moved on")
}

/// Push `note` onto `plan`'s ring under a fresh `seq`, the oldest dropped past
/// [`MAX_PLAN_NOTES`].
pub(super) fn push_note(
    plan: &mut FillPlan,
    note: PlanNoteKind,
    at_ic: i64,
) -> Result<(), ActionError> {
    plan.seq = plan
        .seq
        .checked_add(1)
        .ok_or_else(|| port_error(ActionErrorCode::Integrity, "a plan's seq overflows"))?;
    plan.notes.push(PlanNote {
        seq: plan.seq,
        at_ic,
        note,
    });
    while plan.notes.len() > MAX_PLAN_NOTES {
        plan.notes.remove(0);
    }
    Ok(())
}

/// **A plan step's update, applied** (doc 20 cut SM.11; the brief's 3.6): the cursor, the passed
/// pockets, a parking, the notes (each under its own `seq`, in order) and one more `seq` — on the
/// record the start writes. For a `plan_end` binding the plan then ends with its reason and one
/// `Ended` note.
pub(super) fn apply_update(
    record: &mut RigRecord,
    binding: &PlanFixed,
    now: i64,
) -> Result<(), ActionError> {
    let integrity = || port_error(ActionErrorCode::Integrity, "a plan's update does not apply");
    let plan = record.plan.as_mut().ok_or_else(integrity)?;
    let update = &binding.update;
    if usize::from(update.cursor) > plan.steps.len() {
        return Err(integrity());
    }
    plan.cursor = update.cursor;
    plan.passed.clone_from(&update.passed);
    if let Some(why) = update.parking {
        plan.state = FillState::Parking { why };
    }
    for note in &update.notes {
        push_note(plan, *note, now)?;
    }
    if let Some(why) = binding.end {
        plan.passed.clear();
        plan.state = FillState::Ended { why, at_ic: now };
        push_note(plan, PlanNoteKind::Ended { why }, now)?;
    }
    plan.seq = plan.seq.checked_add(1).ok_or_else(integrity)?;
    plan.check().map_err(|_| integrity())
}

/// The family's record, cloned; `None` when absent.
pub(crate) fn read_record(
    txn: &mut ActionTxn<'_>,
    family: &StateFamilyHandle<RigKey, RigRecord>,
    tv: ObjId,
) -> Result<Option<RigRecord>, ActionError> {
    let found = txn
        .get(family, &RigKey(tv))
        .map_err(|_| {
            port_error(
                ActionErrorCode::Integrity,
                "the rig records are unavailable",
            )
        })?
        .cloned();
    if let Some(record) = &found
        && record.tv != tv
    {
        // The families rule: readers re-check the key.
        return Err(port_error(
            ActionErrorCode::Integrity,
            "a rig record disagrees with its own key",
        ));
    }
    Ok(found)
}

/// Write `record` under its own key (insert or replace).
pub(crate) fn write_record(
    txn: &mut ActionTxn<'_>,
    family: &StateFamilyHandle<RigKey, RigRecord>,
    record: RigRecord,
    existed: bool,
) -> Result<(), ActionError> {
    record.check().map_err(|_| {
        port_error(
            ActionErrorCode::Integrity,
            "the rig record is out of bounds",
        )
    })?;
    let key = record.key();
    let staged = if existed {
        txn.replace(family, &key, record)
    } else {
        txn.insert(family, &key, record)
    };
    staged.map_err(|_| port_error(ActionErrorCode::Integrity, "the rig record did not stage"))
}
