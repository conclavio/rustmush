//! **The presence picture** (doc 32 cut CF.1; Q-CF-15, Q-CF-48's campaign layer).
//!
//! What a unit on the unpromoted surface knows of the other flights on its site: for each, a
//! **bearing** (the site's integer [`Octant`]) and a **range band** at cell resolution and, once
//! within [`SIGHT_CELLS`], its **hull class**. No identity, no party, no damage; it is not a sensor
//! contact and carries no `ContactId`. It is what a Hostile mark or a fire order will name in CF.2.
//!
//! **Published by the surface tick** ([`publish`], every `tacsim.surface` fire, RAM only, rebuilt
//! by the first fire after a restart): one objective [`PresenceBook`] of every rig on the ground,
//! by site, at the cell its frozen leg puts it at the fire's instant (the map's own placement
//! rule) — what CF.2's surface fire order and the surface host's pushed frame (Q-CF-50) will read.
//! `+tv/map` renders the same rule at the frame's own instant, so its bands never lag its grid.
//! **Read per viewer** ([`seen_from`]): each other flight is seen from the viewer's nearest rig on
//! the ground — or from the pad, the carrier's own view, when none is out — so two viewers read
//! the same flights differently and neither reads anyone's identity from them.
//!
//! The bands and the sight are provisional and per cell (doc 32 Q-CF-51): tuned at 256 a side and
//! re-measured at 512.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{CanonicalId, ObjId, World};
use serde_json::{Value, json};
use smol_str::SmolStr;

use crate::surface::address::Octant;
use crate::surface::excursion::{RigRecord, cells_apart};

/// **The short visual range** (doc 32 Q-CF-15), cells: within it a flight's hull class is seen
/// (200 m).
pub const SIGHT_CELLS: u64 = 20;
/// The near band's outer edge, cells (500 m).
pub const NEAR_CELLS: u64 = 50;
/// The far band's outer edge, cells (1 km); beyond it a flight is distant.
pub const FAR_CELLS: u64 = 100;

/// **A range band** (doc 32 Q-CF-15), nearest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Band {
    /// Within [`SIGHT_CELLS`]: the hull class is seen.
    Close,
    /// Out to [`NEAR_CELLS`].
    Near,
    /// Out to [`FAR_CELLS`].
    Far,
    /// Beyond it.
    Distant,
}

impl Band {
    /// The band of a Chebyshev distance in cells.
    #[must_use]
    pub fn of(cells: u64) -> Band {
        if cells <= SIGHT_CELLS {
            Band::Close
        } else if cells <= NEAR_CELLS {
            Band::Near
        } else if cells <= FAR_CELLS {
            Band::Far
        } else {
            Band::Distant
        }
    }

    /// The frame's word.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Band::Close => "close",
            Band::Near => "near",
            Band::Far => "far",
            Band::Distant => "distant",
        }
    }

    /// The words a line prints.
    #[must_use]
    pub fn words(self) -> &'static str {
        match self {
            Band::Close => "within 200 m",
            Band::Near => "200-500 m",
            Band::Far => "500 m-1 km",
            Band::Distant => "beyond 1 km",
        }
    }
}

/// **One flight on a site's ground**, as the tick published it. Never shown whole to a viewer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flight {
    /// The rig.
    pub tv: ObjId,
    /// The cell it stands in at the fire's instant.
    pub cell: (i64, i64),
    /// Its hull class: the unit's type, in words (`tv prospector`).
    pub class: SmolStr,
}

/// **The surface tick's book**: every flight on the ground, by site, at `at_ic`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PresenceBook {
    /// The fire's IC instant.
    pub at_ic: i64,
    /// Site entry → its flights, by rig.
    pub sites: BTreeMap<CanonicalId, Vec<Flight>>,
}

/// **One other flight as one viewer sees it** ([`seen_from`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    /// The bearing from the observer; `None` on the observer's own cell.
    pub dir: Option<Octant>,
    /// The range band.
    pub band: Band,
    /// The hull class, within sight only.
    pub class: Option<SmolStr>,
    /// The viewer's rig it is seen from (its slot), or `None` from the pad.
    pub from: Option<u8>,
}

impl Seen {
    /// The frame's row.
    #[must_use]
    pub fn value(&self) -> Value {
        json!({
            "dir": self.dir.map(Octant::abbrev),
            "band": self.band.word(),
            "range": self.band.words(),
            "class": self.class.as_ref().map(SmolStr::as_str),
            "from": self.from,
        })
    }

    /// The words one row prints: `NE 200-500 m`, `S within 200 m, a tv prospector`.
    #[must_use]
    pub fn words(&self) -> String {
        let dir = self.dir.map_or("here", Octant::abbrev);
        match &self.class {
            Some(class) => format!("{dir} {}, a {class}", self.band.words()),
            None => format!("{dir} {}", self.band.words()),
        }
    }
}

/// **The other flights on a site, as one viewer sees them** (doc 32 Q-CF-15): every flight in
/// `flights` that is not one of `own`, each seen from the nearest of `observers` (the viewer's
/// rigs on the ground, `(slot, cell)`; ties to the lower slot) or, with none, from `pad`. Ordered
/// nearest band first, then by bearing — never by exact range, which the band withholds.
#[must_use]
pub fn seen_from(
    flights: &[Flight],
    own: &BTreeSet<ObjId>,
    observers: &[(u8, (i64, i64))],
    pad: (i64, i64),
) -> Vec<Seen> {
    let mut seen: Vec<Seen> = flights
        .iter()
        .filter(|flight| !own.contains(&flight.tv))
        .map(|flight| {
            let (from, at) = observers
                .iter()
                .min_by_key(|(slot, cell)| (cells_apart(*cell, flight.cell), *slot))
                .map_or((None, pad), |(slot, cell)| (Some(*slot), *cell));
            let cells = cells_apart(at, flight.cell);
            let band = Band::of(cells);
            Seen {
                dir: Octant::of(
                    flight.cell.0.saturating_sub(at.0),
                    flight.cell.1.saturating_sub(at.1),
                ),
                band,
                class: (band == Band::Close).then(|| flight.class.clone()),
                from,
            }
        })
        .collect();
    seen.sort_by(|a, b| {
        (a.band, a.dir.map(Octant::abbrev), &a.class).cmp(&(
            b.band,
            b.dir.map(Octant::abbrev),
            &b.class,
        ))
    });
    seen
}

/// **The line `+tv/map` prints**: `Presence: NE 200-500 m; S within 200 m, a tv prospector.`,
/// `None` with nothing seen.
#[must_use]
pub fn presence_line(seen: &[Seen]) -> Option<String> {
    (!seen.is_empty()).then(|| {
        format!(
            "Presence: {}.",
            seen.iter().map(Seen::words).collect::<Vec<_>>().join("; ")
        )
    })
}

/// **A unit's hull class in words**: its type id with the underscores spaced, else `rig`.
#[must_use]
pub fn hull_class(w: &World, tv: ObjId) -> SmolStr {
    mush_tactics::module::unit_of(w, tv).map_or_else(
        || SmolStr::new("rig"),
        |unit| SmolStr::new(unit.schema().key().id.as_str().replace('_', " ")),
    )
}

/// **The lane sites' passive domains** → their entries (one read a fire).
fn site_domains(w: &World) -> BTreeMap<mush_sim::DomainId, CanonicalId> {
    let Some(store) = w.module_state::<crate::TacsimStore>() else {
        return BTreeMap::new();
    };
    let sites: BTreeSet<CanonicalId> = crate::surface::chart::lane_sites(w)
        .into_iter()
        .map(|site| site.entry)
        .collect();
    store
        .port_maps()
        .filter(|map| map.pack.as_str() == crate::definitions::content::LANE_PACK)
        .filter(|map| sites.contains(&map.entry))
        .filter_map(|map| Some((mush_sim::DomainId(map.domain?), map.entry.clone())))
        .collect()
}

/// **Publish the book** (the `tacsim.surface` fire): every world rig standing on a lane site's
/// ground, at the cell its act's frozen leg puts it at `now` on that very site, else its
/// placement (`crate::port::site_map::ground_cell`). `records` is the fire's scan; a rig without a
/// record is placed by its placement alone.
#[must_use]
pub fn publish(w: &World, records: &[RigRecord], now: i64) -> PresenceBook {
    let domains = site_domains(w);
    let mut book = PresenceBook {
        at_ic: now,
        sites: BTreeMap::new(),
    };
    if domains.is_empty() {
        return book;
    }
    let by_tv: BTreeMap<ObjId, &RigRecord> = records.iter().map(|r| (r.tv, r)).collect();
    for tv in crate::surface::act::world_rigs(w) {
        let crate::surface::act::Where::Ground { domain, .. } =
            crate::surface::act::where_is(w, tv)
        else {
            continue;
        };
        let Some(entry) = domains.get(&domain) else {
            continue;
        };
        let record = by_tv.get(&tv).copied();
        let Some(cell) = crate::port::site_map::ground_cell(w, record, tv, (entry, domain), now)
        else {
            continue;
        };
        book.sites.entry(entry.clone()).or_default().push(Flight {
            tv,
            cell,
            class: hull_class(w, tv),
        });
    }
    book
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flight(tv: u32, x: i64, y: i64) -> Flight {
        Flight {
            tv: ObjId(tv),
            cell: (x, y),
            class: SmolStr::new("tv prospector"),
        }
    }

    /// **The bands, per cell** (doc 32 Q-CF-15): the edges belong to the nearer band.
    #[test]
    fn cf1_bands_cut_at_their_edges() {
        assert_eq!(Band::of(0), Band::Close);
        assert_eq!(Band::of(SIGHT_CELLS), Band::Close);
        assert_eq!(Band::of(SIGHT_CELLS + 1), Band::Near);
        assert_eq!(Band::of(NEAR_CELLS), Band::Near);
        assert_eq!(Band::of(NEAR_CELLS + 1), Band::Far);
        assert_eq!(Band::of(FAR_CELLS), Band::Far);
        assert_eq!(Band::of(FAR_CELLS + 1), Band::Distant);
    }

    /// **A viewer sees others, never itself, from its nearest rig** (doc 32 Q-CF-15): the own rig
    /// is left out; a flight is seen from the nearer of two observers; the class only within sight;
    /// with no rig out, from the pad; the line joins the rows nearest band first.
    #[test]
    fn cf1_a_viewer_sees_other_flights_by_band_and_bearing() {
        let flights = vec![
            flight(1, 100, 100),
            flight(2, 110, 100),
            flight(3, 100, 160),
        ];
        let own: BTreeSet<ObjId> = [ObjId(1)].into_iter().collect();
        let seen = seen_from(
            &flights,
            &own,
            &[(1, (100, 100)), (2, (100, 150))],
            (128, 128),
        );
        assert_eq!(seen.len(), 2, "never the viewer's own rig");
        let first = seen.first().unwrap_or_else(|| panic!("a row"));
        assert_eq!(first.band, Band::Close);
        assert_eq!(first.class.as_deref(), Some("tv prospector"));
        // Rig 3 at (100, 160) is ten cells north of observer 2, not sixty of observer 1.
        assert!(seen.iter().all(|s| s.band == Band::Close));
        assert!(
            seen.iter()
                .any(|s| s.from == Some(2) && s.dir == Some(Octant::N))
        );
        assert!(
            seen.iter()
                .any(|s| s.from == Some(1) && s.dir == Some(Octant::E))
        );
        let from_pad = seen_from(&flights, &own, &[], (128, 128));
        assert!(from_pad.iter().all(|s| s.from.is_none()));
        let far = from_pad
            .iter()
            .find(|s| s.dir == Some(Octant::NW))
            .unwrap_or_else(|| panic!("rig 3 from the pad"));
        assert_eq!(far.band, Band::Near);
        assert_eq!(far.class, None, "beyond sight, no class");
        assert_eq!(
            presence_line(&seen_from(
                &[flight(2, 140, 100)],
                &own,
                &[(1, (100, 100))],
                (0, 0)
            ))
            .as_deref(),
            Some("Presence: E 200-500 m.")
        );
        assert_eq!(presence_line(&[]), None);
    }
}
