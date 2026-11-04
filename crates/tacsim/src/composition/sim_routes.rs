//! E7c7 (cut 6 fold, B6 — tracked to this cut): the `RouteCatalog`
//! implementation, hosted at the composition root so **`mush-sim` gains no
//! `tacsim` edge**.
//!
//! Cut 6's verify found `@journey/plan|depart <route id>` to be a label only:
//! `build_route` fabricated a fresh `RouteDef` at the `manual` drive class with
//! zero declared propellant and no pinned distance, so `RouteDef::propellant()`,
//! `alignment_band()` and `route_duration` had no caller at all. The verify's
//! own fix ("resolve the id against cut 1's pack, which needs no edge") **cannot
//! be built**: the authored pack store is `crate::TacsimStore` and
//! `tacsim` depends on `mush-sim`, so the lookup would be a cycle. It is
//! therefore a composition seam in the `with_accounts` / `with_recipes` shape —
//! the exact same answer cut 4 gave for recipes.
//!
//! Read-only and bounded: one `BTreeMap` lookup against the store's own
//! materialized index, and the `RouteDef` handed back is the store's, which
//! already re-ran every constructor invariant when it was materialized.

use crate::TacsimStore;
use mush_core::{CanonicalId, Ctx, ObjId, World};
use mush_sim::{CarrierHooks, CarrierJourney, RouteCatalog, RouteDef};

/// The one route catalog, handed to `SimulationModule::with_route_catalog` at
/// composition. Holds nothing: it reads the per-world `TacsimStore`.
#[derive(Clone, Copy, Debug, Default)]
pub struct TacsimRouteCatalog;

impl RouteCatalog for TacsimRouteCatalog {
    fn resolve(&self, world: &World, id: &CanonicalId) -> Option<RouteDef> {
        // `TacsimStore::route` answers for the pack version that governs the id
        // (`TacsimStore::resolve`'s rule, the same one every other subject goes
        // through). A route whose places are not all placed was skipped loudly
        // at reindex time and is simply absent here — which is "fall back to the
        // manual route", never "hand out a half-resolved route".
        world
            .module_state::<TacsimStore>()?
            .route_in_world(world, id)
    }
}

// ---- Doc 18 cut IC9: the carrier hooks --------------------------------------------

/// **Whether tacsim's flavoured aboard lines REPLACE mush-sim's generic ones** (doc
/// 18 cut IC9, lead ruling L8): `false` keeps IC4's recorded surface — "IC9's
/// flavoured lines are additional, not replacements" — until the owner rules; the
/// owner's "replace" is this one token.
pub const REPLACE_GENERIC_ABOARD_LINES: bool = false;

/// **The trade run's carrier hooks** (doc 18 cut IC9; lead rulings L8, L32), handed
/// to `SimulationModule::with_carrier_hooks` beside [`TacsimRouteCatalog`] on every
/// roster. A no-op for anything that is not a programme hull (and on a roster
/// without the layer, where no hull resolves).
///
/// - **The manual arm** (`refuse_manual_departure`, reviewer F3 — IC3's F9, IC4
///   L18, closed on the ROUTE whatever the carrier): a route whose source pack is
///   the lane is refused for every carrier and every actor, staff included
///   ([`crate::port::voyage::LANE_UNPLANNED`]); a programme hull is refused on any
///   other manual route ([`crate::port::voyage::MANUAL_DEPARTURE`]); a
///   non-programme carrier on a non-lane route still departs for staff.
/// - **The narration** (`boundary`, `arrived`): the flavoured line to every
///   walkable room of the hull, and — for an absent tenant — the same line queued
///   with its UTC prefix (lead ruling L24). World-only: no typed-state read.
#[derive(Clone, Copy, Debug, Default)]
pub struct TacsimCarrierHooks;

impl CarrierHooks for TacsimCarrierHooks {
    fn refuse_manual_departure(
        &self,
        w: &World,
        carrier: ObjId,
        route: &RouteDef,
    ) -> Option<String> {
        let lane_route = w
            .module_state::<TacsimStore>()
            .and_then(|store| store.route_source(w, route))
            .is_some_and(|(pack, _)| {
                pack.manifest().id().as_str() == crate::definitions::content::LANE_PACK
            });
        if lane_route {
            return Some(
                crate::port::voyage::LANE_UNPLANNED.replace("{route}", route.id().as_str()),
            );
        }
        crate::port::ship::hull_view(w, carrier).map(|_| {
            crate::port::voyage::MANUAL_DEPARTURE
                .replace("{name}", &crate::port::coop::name_of(w, carrier))
        })
    }

    /// Doc 42 cut CR.0 (Q-42-2): `@station` and `+focus` refuse a captive and an ejectee
    /// waiting on their pickup with the crew record's held line.
    fn restrains_operator(&self, w: &World, actor: ObjId) -> Option<String> {
        crate::field::crew::refusal(w, actor, crate::field::crew::Gate::Operate)
    }

    fn boundary(&self, ctx: &mut Ctx<'_>, journey: &CarrierJourney, reached: u32) -> bool {
        let hull = journey.root();
        if crate::port::ship::hull_view(ctx.w, hull).is_none() {
            return false;
        }
        if let Some(leg_idx) = usize::try_from(reached.saturating_sub(1)).ok()
            && let Some(leg) = journey.route().legs().get(leg_idx)
            && leg.is_interdicted()
        {
            let now_ic = ctx.w.world_ic_now().at;
            let on_time_boundary = journey
                .leg_due_at(reached as usize)
                .is_none_or(|next_due| now_ic < next_due)
                && journey
                    .leg_due_at(leg_idx)
                    .is_some_and(|due_ic| now_ic.saturating_sub(due_ic) < 3_600);
            let active_bridge_crew =
                crate::port::ship::hull_view(ctx.w, hull).is_some_and(|view| {
                    let crew_room = mush_core::CanonicalId::new("crew")
                        .ok()
                        .and_then(|id| view.room(&id));
                    view.row.rooms.values().any(|&room_u32| {
                        let room = mush_core::ObjId(room_u32);
                        Some(room) != crew_room
                            && ctx.w.contents_of(room).iter().any(|&obj| {
                                ctx.w
                                    .get(obj)
                                    .is_some_and(|o| o.kind == mush_core::ObjKind::Player)
                                    && ctx.w.is_connected(obj)
                            })
                    })
                });
            if on_time_boundary && active_bridge_crew {
                let dom_opt = ctx
                    .w
                    .module_state::<mush_sim::SpatialState>()
                    .and_then(|s| s.domain_of_host(leg.to()).map(|d| d.id))
                    .or_else(|| mush_sim::open_passive_domain(ctx, leg.to(), leg.to()).ok());
                if let Some(dom) = dom_opt
                    && crate::space::picket::is_kirkwood_picket_host(ctx.w, leg.to())
                    && let Some(book) = ctx.w.module_state_mut::<crate::space::SpaceBubbleBook>()
                {
                    book.kirkwood_picket_domains.insert(dom);
                }
            }
        }
        let Some(line) = crate::port::voyage::boundary_line(ctx.w, journey, reached) else {
            return false;
        };
        crate::port::voyage::narrate_hull(ctx, hull, &line);
        crate::port::passenger::voyage_event(ctx, hull, &line);
        let queued = format!("{}{line}", crate::port::voyage::utc_prefix(ctx.w));
        crate::port::voyage::tell_absent_tenant(ctx, hull, &queued);
        REPLACE_GENERIC_ABOARD_LINES
    }

    fn arrived(&self, ctx: &mut Ctx<'_>, journey: &CarrierJourney) -> bool {
        let hull = journey.root();
        if crate::port::ship::hull_view(ctx.w, hull).is_none() {
            return false;
        }
        // Doc 42 cut CR.2 (Q-42-6, Draft 0.3 42-3): persons first: every captive aboard is
        // handed to the port's authority before any hull hook of this arrival runs.
        crate::field::crew::hand_over(ctx, hull);
        let (docks, notice) = crate::port::voyage::arrival_lines(ctx.w, journey);
        crate::port::voyage::narrate_hull(ctx, hull, &docks);
        crate::port::passenger::voyage_event(ctx, hull, &docks);
        let notice = notice.replace("{hhmm}", &crate::port::voyage::utc_hhmm(ctx.w.now()));
        crate::port::voyage::tell_absent_tenant(ctx, hull, &notice);
        // Doc 20 cut SM.4a (lead ruling 6): a hull settled on a site's pad asks for the site's
        // ground, so the surface tick warms it before the first scan (RAM only; a no-op with the
        // geology layer dark). Doc 20 cut SM.4b (lead ruling 4, R3): and its way home from
        // every cell — the home field for the hull's ramp cell, wanted whatever the layer's
        // state (`want_home` wants the site's ground too).
        // Doc 41 cut RS.5 (Q-41-8, Q-41-14): the shed's hand-back at the destination, and a
        // written-off rig aboard handed back at the lessor's port. Doc 41 cut RS.8 (§4.7): a
        // hull her retriever docks under her own power is delivered by the docking itself.
        if let crate::port::ship::Berth::Docked { map, def } =
            crate::port::ship::berth_of(ctx.w, hull)
            && def.site.is_none()
        {
            let entry = map.entry.clone();
            let (venue, port) = (def.venue.clone(), crate::port::coop::port_name(def));
            crate::surface::retrieval::on_docked(ctx, hull, &entry);
            if let Some(helm) = crate::port::coop::helm_holder(ctx.w, hull) {
                let _ = crate::surface::retrieval::deliver(ctx, hull, &entry, helm);
            }
            // Doc 41 cut RS.7 (Q-41-25): port security fines her master's proven acts.
            crate::surface::security::on_docked(ctx, hull, &venue, &port);
        }
        if let crate::port::ship::Berth::Docked { map, def } =
            crate::port::ship::berth_of(ctx.w, hull)
            && def.site.is_some()
        {
            let entry = map.entry.clone();
            match crate::surface::hull_home(ctx.w, hull) {
                Some((site, ramp)) => crate::surface::want_home(ctx.w, &site, ramp),
                None => crate::surface::want_site_ground(ctx.w, &entry),
            }
        }
        REPLACE_GENERIC_ABOARD_LINES
    }
}
