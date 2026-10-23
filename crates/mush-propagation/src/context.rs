//! E7 cut 7 B1 — **one physical context, two resolvers** (Q4A, D-E7-14, R4).
//!
//! Detection and delivery share geometry, medium, range, occlusion,
//! interference and time-of-flight, and answer **different questions**:
//!
//! - **Detection** — signature, emissions posture, line of sight, sensor
//!   capability, noise floor. *Was it seen?*
//! - **Delivery** — addressing, routing, relays, jamming, store-and-forward.
//!   *Did it arrive?*
//!
//! R4's two cases are the whole reason they are two: **a jammed unit still
//! takes orders** (delivery degrades, detection does not) and **a courier
//! intercepted but never detected** (delivery succeeds through a relay the
//! detector never saw).
//!
//! Everything here is **read-only over `mush-sim`'s placement graph** and
//! **constant-work by construction**: a miss and a hit walk the same branches
//! and read the same records, so neither timing nor store-read count is an
//! oracle for whether something is out there (the K lens).

use mush_core::{ClockStamp, ObjId, World};
use mush_sim::{DomainId, SpaceAnchor, SpatialState, graph};
use serde::{Deserialize, Serialize};

/// The most placement records one context build may visit (D-LBK-5).
///
/// Applied in [`domain_of_place`] and [`Medium::of_place`] — the two scans a
/// build makes over the domain and space maps. It was declared and unused until
/// the independent verify fold (B15a); it is now the bound on the walk D1 added.
pub const MAX_CONTEXT_RECORDS: usize = 64;

/// The abstract range unit. Cut 7's geometry is the placement graph's, not a
/// metric one: two things in one place are at range 0, two things in different
/// places at range [`ADJACENT_RANGE`], and the resolvers are written so a real
/// metric can replace this without any consumer changing.
pub const ADJACENT_RANGE: u32 = 2;

/// The medium a signal crosses.
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Medium {
    /// Open space: long range, no attenuation, light-speed lag matters.
    #[default]
    Vacuum,
    /// Inside a hull or a structure: short range, heavy occlusion.
    Interior,
    /// A planetary surface or atmosphere.
    Surface,
}

impl Medium {
    /// The medium a base medium id (doc 14 §5.1: `ground`, `air`, `water`,
    /// `space`) maps onto: `space` is a vacuum, everything else a surface;
    /// an interior is a place, never a medium id.
    #[must_use]
    pub fn from_medium_id(id: &str) -> Self {
        match id {
            "space" => Self::Vacuum,
            _ => Self::Surface,
        }
    }

    /// The base medium ids this medium stands for.
    #[must_use]
    pub const fn medium_ids(self) -> &'static [&'static str] {
        match self {
            Self::Vacuum => &["space"],
            Self::Surface => &["ground", "air", "water"],
            Self::Interior => &[],
        }
    }

    /// Attenuation per range unit, in noise-floor points.
    #[must_use]
    pub const fn attenuation(self) -> u32 {
        match self {
            Self::Vacuum => 1,
            Self::Surface => 3,
            Self::Interior => 8,
        }
    }

    /// Seconds of flight per range unit — the same term both resolvers use.
    #[must_use]
    pub const fn seconds_per_range(self) -> i64 {
        match self {
            Self::Vacuum => 4,
            Self::Surface => 1,
            Self::Interior => 0,
        }
    }

    /// Which medium a place is. An interior space (cut 6's `sim.interior`
    /// anchors) is enclosed; a domain host is open; anything else is a surface.
    ///
    /// Bounded by [`MAX_CONTEXT_RECORDS`] records visited (D-LBK-5).
    #[must_use]
    pub fn of_place(state: &SpatialState, place: ObjId) -> Self {
        if state.spaces.contains_key(&place) {
            Self::Interior
        } else if state
            .domains
            .values()
            .take(MAX_CONTEXT_RECORDS)
            .any(|domain| domain.host == place)
        {
            Self::Vacuum
        } else {
            Self::Surface
        }
    }
}

/// Where the two ends are relative to each other.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Geometry {
    /// The observing (or transmitting) platform.
    pub near: ObjId,
    /// The observed (or addressed) platform.
    pub far: ObjId,
    /// Range in abstract units.
    pub range: u32,
    /// Whether a hull, a hold or a closed space stands between them.
    pub occluded: bool,
}

/// What is in the way of a clean read.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Interference {
    /// The ambient floor a signature must clear.
    pub noise_floor: u32,
    /// Deliberate jamming. **Delivery-affecting only**: R4's jammed unit still
    /// takes orders, so this term never enters the detection threshold.
    pub jamming: u32,
}

/// One physical context, built read-only off the placement graph.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PropagationContext {
    /// The live domain both ends sit in, when they sit in one.
    pub domain: Option<mush_sim::DomainId>,
    pub geometry: Geometry,
    pub medium: Medium,
    pub interference: Interference,
    pub at: ClockStamp,
}

impl PropagationContext {
    /// Build the context between two objects, or `None` when the placement
    /// graph does not connect them at all.
    ///
    /// `None` means **neither observable nor addressable**, and it is returned
    /// for three cases, all of which cost the same work as a hit:
    ///
    /// 1. Either end has no projected place (an unplaced thing, or one whose
    ///    only projection is a projection room — see [`place_of`]).
    /// 2. The two places are in **different domains** (E7 cut 7 independent
    ///    verify fold, D1(a)). Before the fold, `build` read no link, exit,
    ///    domain boundary or metric, so "a different place" meant *every other
    ///    place in the world*: with one `DEFAULT_SIGNATURE` against a Vacuum
    ///    threshold of 3, every placed object outside an interior space detected
    ///    every other one, in any domain, and `detection_pass` armed a real
    ///    `Observe` on each. A pair the graph cannot connect is simply not a
    ///    context.
    /// 3. Either end's domain cannot be resolved at all (an unanchored interior
    ///    space, a place in no domain): fail closed.
    ///
    /// [`Geometry::range`] stays the **sole** consumer-visible distance term, so
    /// a real metric can still replace it without a consumer changing.
    #[must_use]
    pub fn build(
        state: &SpatialState,
        world: &World,
        near: ObjId,
        far: ObjId,
        at: ClockStamp,
    ) -> Option<Self> {
        let near_place = place_of(state, world, near)?;
        let far_place = place_of(state, world, far)?;
        let same = near_place == far_place;
        let domain = domain_of_place(state, world, near_place);
        // Doc 14 §5.2 (S5): two ends on one tactical window have a metric
        // between them whatever rooms the graph puts them in.
        let metric = mush_sim::metric_between(world, near, far);
        if !same && metric.is_none() {
            // Domain adjacency, bounded by MAX_CONTEXT_RECORDS inside the walk.
            // Two places are adjacent when they are in one domain — which is
            // also what makes a domain host and its presentation room adjacent,
            // since both resolve to the same `DomainId`.
            let far_domain = domain_of_place(state, world, far_place);
            match (domain, far_domain) {
                (Some(near_domain), Some(far_domain)) if near_domain == far_domain => {}
                _ => return None,
            }
        }
        let medium = Medium::of_place(state, near_place);
        // Occlusion: a closed space stands between the two ends. **Both
        // directions** (E7 cut 7 independent verify fold, K9): before the fold
        // only the far end was read, so a sensor sealed inside a hull looking
        // out was not occluded and only the fixed signature constant kept the
        // miss — a structural gate, not an arithmetic one.
        let occluded = !same
            && (matches!(Medium::of_place(state, far_place), Medium::Interior)
                || matches!(medium, Medium::Interior));
        let range = if same { 0 } else { ADJACENT_RANGE };
        // Doc 14 §5.2 (S5): when both ends stand on one tactical window, the
        // metric replaces the graph's 0 / ADJACENT_RANGE and the window's line
        // of sight replaces the hull test — the swap this type reserved. The
        // atlas answers through `mush-sim`'s geometry slot; no atlas edge here.
        let (range, occluded) = match metric {
            Some(metric) => (metric.range_cells, metric.occluded),
            None => (range, occluded),
        };
        Some(Self {
            domain,
            geometry: Geometry {
                near,
                far,
                range,
                occluded,
            },
            medium,
            interference: Interference {
                noise_floor: medium.attenuation(),
                jamming: 0,
            },
            at,
        })
    }

    /// With a declared jamming level (staff, or a later electronic-warfare
    /// result). Saturating, so no arithmetic here can overflow or stall.
    #[must_use]
    pub fn jammed(mut self, jamming: u32) -> Self {
        self.interference.jamming = self.interference.jamming.saturating_add(jamming);
        self
    }

    /// Time of flight in IC seconds — the one term both resolvers share.
    #[must_use]
    pub fn time_of_flight(&self) -> i64 {
        i64::from(self.geometry.range).saturating_mul(self.medium.seconds_per_range())
    }
}

/// Where a thing is, for propagation's purposes.
///
/// The placement graph first — that is the tactical topology, and a placed
/// platform's projection is where it really is. An object the graph says nothing
/// about (a person standing in an ordinary room) falls back to its `Obj.location`:
/// propagation addresses and observes the whole world, not only the units cut 5
/// and cut 6 placed. `None` means it is nowhere at all, and a thing that is
/// nowhere is neither observable nor addressable.
///
/// **A projection room is not a place** (E7 cut 7 independent verify fold,
/// D2(a)). Cut 6 projects a quarantined root into `SpatialState::recovery_room`
/// and an under-way carrier root into `SpatialState::in_transit_room`, and both
/// default to the same nowhere-room — so every wreck and every carrier in
/// passage on every unrelated journey sat at range 0 with all the others and
/// mutually detected. Neither room is somewhere a thing can be seen from or
/// addressed at; the accepted consequence is that an envelope to a ship under
/// way **waits** (store-and-forward) rather than being delivered.
#[must_use]
pub fn place_of(state: &SpatialState, world: &World, obj: ObjId) -> Option<ObjId> {
    let place = if world
        .get(obj)
        .is_some_and(|o| o.kind == mush_core::ObjKind::Room)
    {
        obj
    } else {
        match graph::project(state, world, obj) {
            Some(place) => place,
            None => world.get(obj).and_then(|object| object.location)?,
        }
    };
    (place != state.recovery_room && place != state.in_transit_room).then_some(place)
}

/// Which live domain a **place** belongs to, or `None` when it belongs to none.
///
/// A domain host and its presentation room both answer that domain's id, so
/// [`PropagationContext::build`]'s "same domain" test covers the presentation
/// relation without a second clause. An interior space answers through its
/// anchor: a fixed interior is in the domain it was anchored to, and a moving
/// one (a hull riding a carrier) is in whichever domain its carrier is in, so a
/// hull and the ships around it are adjacent exactly while the carrier is.
///
/// Bounded by [`MAX_CONTEXT_RECORDS`] records visited, in both the domain scan
/// and the anchor walk (D-LBK-5).
#[must_use]
pub fn domain_of_place(state: &SpatialState, world: &World, place: ObjId) -> Option<DomainId> {
    let mut current = place;
    for _ in 0..MAX_CONTEXT_RECORDS {
        if let Some((id, _)) = state
            .domains
            .iter()
            .take(MAX_CONTEXT_RECORDS)
            .find(|(_, domain)| domain.host == current || domain.presentation_location == current)
        {
            return Some(*id);
        }
        let space = state.spaces.get(&current)?;
        match &space.anchor {
            SpaceAnchor::FixedDomain { domain, .. } => return Some(*domain),
            SpaceAnchor::MovingObject { host } => {
                let next = place_of(state, world, *host)?;
                if next == current {
                    return None;
                }
                current = next;
            }
            SpaceAnchor::Unanchored => return None,
        }
    }
    None
}

/// What detection decided. Both arms carry the same fields, so a caller cannot
/// tell a hit from a miss by the SHAPE of what came back either.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum DetectionOutcome {
    /// The subject cleared the floor.
    Detected { margin: u32, lag: i64 },
    /// It did not. **The same work, the same reads, the same allocations.**
    Missed { margin: u32, lag: i64 },
}

impl DetectionOutcome {
    /// Whether this is a hit.
    #[must_use]
    pub const fn is_detected(&self) -> bool {
        matches!(self, Self::Detected { .. })
    }

    /// The lag either way.
    #[must_use]
    pub const fn lag(&self) -> i64 {
        match self {
            Self::Detected { lag, .. } | Self::Missed { lag, .. } => *lag,
        }
    }
}

/// Detection: signature against the floor, with occlusion absolute.
///
/// **Constant work.** Both branches compute `threshold`, `margin` and `lag`
/// from the same terms and return the same-shaped value; there is no early
/// return, no extra lookup on a hit and no allocation on either side. A `False`
/// contact resolved through this path costs exactly what a real one costs.
#[must_use]
pub fn resolve_detection(context: &PropagationContext, signature: u32) -> DetectionOutcome {
    let threshold = context.interference.noise_floor.saturating_add(
        context
            .geometry
            .range
            .saturating_mul(context.medium.attenuation()),
    );
    let lag = context.time_of_flight();
    // Jamming deliberately does NOT enter here (R4: a jammed unit is still
    // seen, and still takes orders).
    let clear = !context.geometry.occluded && signature >= threshold;
    let margin = signature.saturating_sub(threshold);
    if clear {
        DetectionOutcome::Detected { margin, lag }
    } else {
        DetectionOutcome::Missed { margin, lag }
    }
}

/// What delivery decided.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeliveryOutcome {
    /// Reachable in `hops` relays, arriving `lag` IC seconds later.
    Reachable { hops: u8, lag: i64 },
    /// Jammed: the envelope waits (store-and-forward), it is **not** lost, and
    /// the recipient keeps its last delivered order and fallback doctrine.
    Jammed { hops: u8, lag: i64 },
}

impl DeliveryOutcome {
    /// Whether the hop may be taken now.
    #[must_use]
    pub const fn is_reachable(&self) -> bool {
        matches!(self, Self::Reachable { .. })
    }

    /// The hop count either way.
    #[must_use]
    pub const fn hops(&self) -> u8 {
        match self {
            Self::Reachable { hops, .. } | Self::Jammed { hops, .. } => *hops,
        }
    }
}

/// The jamming level at which addressing fails. Above it the envelope waits.
pub const JAMMING_BLOCKS_AT: u32 = 4;

/// Delivery: addressing and routing, with jamming the one thing that stops it.
///
/// **Occlusion does not block delivery** — a courier crosses a hull that a
/// sensor cannot see through, which is R4's second case — and detection is not
/// consulted at all, which is R4's first.
#[must_use]
pub fn resolve_delivery(context: &PropagationContext) -> DeliveryOutcome {
    let hops = u8::try_from(context.geometry.range.max(1)).unwrap_or(u8::MAX);
    let lag = context.time_of_flight();
    if context.interference.jamming >= JAMMING_BLOCKS_AT {
        DeliveryOutcome::Jammed { hops, lag }
    } else {
        DeliveryOutcome::Reachable { hops, lag }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::ClockRef;

    fn context(range: u32, occluded: bool, medium: Medium) -> PropagationContext {
        PropagationContext {
            domain: None,
            geometry: Geometry {
                near: ObjId(1),
                far: ObjId(2),
                range,
                occluded,
            },
            medium,
            interference: Interference {
                noise_floor: medium.attenuation(),
                jamming: 0,
            },
            at: ClockStamp {
                clock: ClockRef::Utc,
                at: 0,
            },
        }
    }

    #[test]
    fn the_media_map_onto_the_base_vocabulary_and_back() {
        assert_eq!(Medium::from_medium_id("space"), Medium::Vacuum);
        assert_eq!(Medium::from_medium_id("ground"), Medium::Surface);
        assert_eq!(Medium::from_medium_id("air"), Medium::Surface);
        assert_eq!(Medium::from_medium_id("water"), Medium::Surface);
        for m in [Medium::Vacuum, Medium::Surface] {
            for id in m.medium_ids() {
                assert_eq!(Medium::from_medium_id(id), m);
            }
        }
        assert!(Medium::Interior.medium_ids().is_empty());
    }

    #[test]
    fn a_jammed_unit_is_still_detected_and_an_occluded_courier_still_arrives() {
        // R4 case 1: jamming stops delivery, never detection.
        let jammed = context(0, false, Medium::Vacuum).jammed(JAMMING_BLOCKS_AT);
        assert!(resolve_detection(&jammed, 50).is_detected());
        assert!(!resolve_delivery(&jammed).is_reachable());
        // R4 case 2: occlusion stops detection, never delivery.
        let occluded = context(ADJACENT_RANGE, true, Medium::Interior);
        assert!(!resolve_detection(&occluded, u32::MAX).is_detected());
        assert!(resolve_delivery(&occluded).is_reachable());
    }

    #[test]
    fn a_miss_and_a_hit_are_the_same_shape_and_carry_the_same_lag() {
        let open = context(ADJACENT_RANGE, false, Medium::Vacuum);
        let hit = resolve_detection(&open, 1_000);
        let miss = resolve_detection(&open, 0);
        assert!(hit.is_detected() && !miss.is_detected());
        assert_eq!(hit.lag(), miss.lag(), "timing is not an oracle");
        assert_eq!(
            serde_json::to_value(hit)
                .unwrap()
                .as_object()
                .unwrap()
                .len(),
            serde_json::to_value(miss)
                .unwrap()
                .as_object()
                .unwrap()
                .len(),
            "the two outcomes serialize to the same field set"
        );
    }

    #[test]
    fn every_term_saturates_rather_than_overflowing() {
        let far = PropagationContext {
            geometry: Geometry {
                near: ObjId(1),
                far: ObjId(2),
                range: u32::MAX,
                occluded: false,
            },
            ..context(0, false, Medium::Vacuum)
        };
        // No panic, no wrap: an arithmetic edge pages, it never stalls (E5a S2).
        let _ = far.time_of_flight();
        let _ = resolve_detection(&far, u32::MAX);
        assert_eq!(resolve_delivery(&far).hops(), u8::MAX);
    }
}
