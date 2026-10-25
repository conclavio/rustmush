//! Public berth and departure boards (doc 18a C01/C14/P01/P05).
//!
//! Read-only projections of placement, legal title, a runnable head departure,
//! filed clearance and carrier journey. Never render the private order log,
//! cargo, passengers, prices or hold reasons. Room roles come from the applied
//! lane; the current room's atlas key is a single indexed lookup. No migration
//! or additional authority over berths is introduced.

use mush_core::{ClockRef, ClockStamp, EventHook, Flags, HookReadContext, ObjId, World};
use mush_economy::EconomyStore;
use mush_org::{OrgState, PrincipalId};
use mush_proto::RichText;

use crate::definitions::content::LANE_PACK;
use crate::port::coop::{helm_holder, helm_of};
use crate::port::orders::{OrderState, OrdersState, PlanState, What, depart_due, orders_state};
use crate::port::ship::{Berth, berth_of};
use crate::port::voyage::{pending_of, real_until, utc_day_word, utc_hhmm, venue_name};
use crate::store::TacsimStore;

fn name(w: &World, obj: ObjId) -> String {
    w.get(obj)
        .filter(|o| !o.flags.intersects(Flags::DARK | Flags::GOING))
        .map_or_else(|| "unavailable".to_owned(), |o| o.name.to_string())
}

fn title(w: &World, hull: ObjId) -> String {
    let Some(asset) = w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().asset(hull))
    else {
        return "unavailable".to_owned();
    };
    match &asset.title_owner {
        PrincipalId::Object(obj) => w
            .get(*obj)
            .filter(|o| !o.flags.intersects(Flags::DARK | Flags::GOING))
            .map_or_else(|| "private registry".to_owned(), |o| o.name.to_string()),
        PrincipalId::Organization(id) => w
            .module_state::<OrgState>()
            .and_then(|s| s.org(*id))
            .filter(|org| !org.hidden)
            .map_or_else(|| "private registry".to_owned(), |org| org.name.to_string()),
        PrincipalId::System(_) => "registry".to_owned(),
    }
}

fn hour(w: &World, at: i64) -> String {
    format!("{} UTC {}", utc_hhmm(at), utc_day_word(w.now(), at))
}

fn departure(w: &World, stamp: &ClockStamp) -> String {
    let ClockRef::WorldIc { generation } = stamp.clock else {
        return "unavailable".to_owned();
    };
    w.world_timeline()
        .epochs()
        .iter()
        .find(|epoch| epoch.generation == generation && epoch.rate > 0)
        .and_then(|epoch| {
            stamp.at.checked_sub(epoch.ic_anchor).and_then(|delta| {
                epoch
                    .real_anchor
                    .checked_add(delta.div_euclid(i64::from(epoch.rate)))
            })
        })
        .map_or_else(|| "unavailable".to_owned(), |at| hour(w, at))
}

fn arrival(w: &World, at: Option<i64>) -> String {
    at.and_then(|at| real_until(w, at)).map_or_else(
        || "unavailable".to_owned(),
        |remaining| hour(w, w.now().saturating_add(remaining)),
    )
}

/// Only the active head departure is public, with the same authority and due
/// calculation as the executor. A later departure behind private commerce is
/// not a clearance; paused/held/obsolete plans advertise no departure.
fn scheduled(w: &World, hull: ObjId) -> Option<(String, i64)> {
    let OrdersState::Present(plan) = orders_state(w, hull) else {
        return None;
    };
    if plan.state != PlanState::Running
        || plan.hold.is_some()
        || !helm_of(w, plan.author).is_some_and(|helm| helm.hull == hull)
    {
        return None;
    }
    let order = plan.order(plan.cursor?)?;
    if order.state != OrderState::Pending {
        return None;
    }
    let What::Depart { to, .. } = &order.what else {
        return None;
    };
    Some((venue_name(w, to), depart_due(w, hull, order)?))
}

/// The public local-room projection; no store writes or planning work.
#[derive(Debug)]
pub(crate) struct TrafficLook;

impl EventHook for TrafficLook {
    fn look_rendered(
        &self,
        ctx: &HookReadContext<'_>,
        _looker: ObjId,
        room: ObjId,
    ) -> Vec<RichText> {
        let w = ctx.world();
        let Some(store) = w.module_state::<TacsimStore>().filter(|s| s.enabled()) else {
            return Vec::new();
        };
        let room_key = w
            .module_state::<mush_atlas::AtlasState>()
            .and_then(|atlas| atlas.records.roomgeo.get(&room))
            .map(|geo| &geo.key);
        let (_, ports) = crate::port::readiness::report_ports(store, w);
        let Some((map, office)) = store
            .port_maps()
            .filter(|map| map.pack.as_str() == LANE_PACK)
            .find_map(|map| {
                let (_, def) = ports.iter().find(|(entry, _)| *entry == map.entry)?;
                // Mining pads are not station berth galleries.
                if def.site.is_some() {
                    return None;
                }
                if map.presentation == Some(room.0) {
                    return Some((map, false));
                }
                let key = room_key?;
                def.rooms
                    .iter()
                    .any(|(role, reference)| {
                        matches!(role.as_str(), "dockmaster" | "harbourmaster")
                            && crate::port::def::room_key(&def.area, reference)
                                .is_ok_and(|k| k == *key)
                    })
                    .then_some((map, true))
            })
        else {
            return Vec::new();
        };
        let mut lines = vec![if office {
            "Departures board: HULL / REGISTRY / BERTH / FOR / CLEARANCE / ARRIVES".to_owned()
        } else {
            "Berth board: HULL / REGISTRY / BERTH / TITLE / AGENT / STATUS".to_owned()
        }];
        for row in store
            .port_maps()
            .filter(|m| m.pack.as_str() == LANE_PACK)
            .flat_map(|m| &m.hulls)
        {
            let hull = ObjId(row.hull);
            if !w
                .get(hull)
                .is_some_and(|o| !o.flags.intersects(Flags::GOING | Flags::DARK))
            {
                continue;
            }
            let docked = matches!(berth_of(w, hull), Berth::Docked { map: berth, .. } if berth.domain == map.domain);
            let journey = w
                .module_state::<mush_sim::JourneyBook>()
                .and_then(|book| book.get(hull))
                .filter(|j| {
                    j.state().is_under_way() && map.host == Some(j.route().endpoints().0.0)
                });
            if !(docked || office && journey.is_some()) {
                continue;
            }
            let berth = if docked {
                name(w, room_for_gallery(map, room))
            } else {
                "under way".to_owned()
            };
            let identity = format!("{} / {} / {berth}", name(w, hull), row.mark);
            let pending = docked.then(|| pending_of(w, hull)).flatten();
            let next = docked.then(|| scheduled(w, hull)).flatten();
            if office {
                let (to, clearance, arrives) = if let Some(j) = journey {
                    let to = ports
                        .iter()
                        .find_map(|(entry, def)| {
                            store
                                .port_maps()
                                .find(|m| {
                                    m.entry == *entry
                                        && m.pack == map.pack
                                        && m.host == Some(j.route().endpoints().1.0)
                                })
                                .map(|_| crate::port::coop::port_name(def))
                        })
                        .unwrap_or_else(|| "unavailable".to_owned());
                    (
                        to,
                        format!("UNDER WAY {}", departure(w, j.departed())),
                        arrival(w, j.arrival_at()),
                    )
                } else if let Some(p) = pending {
                    (
                        venue_name(w, &p.to_venue),
                        format!("CLEARED {}", hour(w, p.filed_at)),
                        arrival(w, p.plan.depart_ic.checked_add(p.plan.total_s)),
                    )
                } else if let Some((to, due)) = next {
                    let status = if due < w.now() { "OVERDUE" } else { "FILED" };
                    (
                        to,
                        format!("{status} {}", hour(w, due)),
                        "awaiting clearance".to_owned(),
                    )
                } else {
                    ("—".to_owned(), "IDLE".to_owned(), "—".to_owned())
                };
                lines.push(format!("  {identity} / {to} / {clearance} / {arrives}"));
            } else {
                let status = if pending.is_some() {
                    "CLEARED".to_owned()
                } else if let Some((_, due)) = next {
                    format!("FILED {}", hour(w, due))
                } else {
                    "IDLE".to_owned()
                };
                let agent = helm_holder(w, hull).map_or_else(|| "—".to_owned(), |obj| name(w, obj));
                lines.push(format!(
                    "  {identity} / {} / {agent} / {status}",
                    title(w, hull)
                ));
            }
        }
        if lines.len() == 1 {
            lines.push("  No hulls posted.".to_owned());
        }
        lines.into_iter().map(RichText::plain).collect()
    }
}

fn room_for_gallery(map: &crate::port::PortMap, fallback: ObjId) -> ObjId {
    map.presentation.map_or(fallback, ObjId)
}
