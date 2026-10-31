//! **The site record** (doc 32 cut CF.1; Q-CF-24, Q-CF-25, Q-CF-27, Q-CF-30; the vocabulary of
//! Q-CF-2: a *site* is the ground and this record, a *field* is the site while promoted).
//!
//! `tacsim.field` ([`SiteRecord`], keyed [`SiteKey`] by the site's port entry) is **the one
//! durable record of what lies on a site's ground between fields**, on the authoritative lane:
//!
//! - the **units left on the ground** ([`LeftUnit`]): an index only — each unit's own record
//!   stays `unit:<obj>` under tactics custody (Q-CF-24) — with the pose and the open epoch it was
//!   left in and whether it is a casualty or a wreck (Q-CF-25 a, b);
//! - the **non-unit objects** ([`GroundObject`]) the record owns outright (Q-CF-25 c, d): the
//!   remains an irrecoverable hull leaves after its epoch closes destroyed, and dropped cargo with
//!   its manifest; at most [`MAX_GROUND_OBJECTS`], the oldest remains merged past it
//!   ([`SiteRecord::drop_object`], Q-CF-27's published rule), cargo never dropped;
//! - the **field ordinal** (the last field opened here, `FieldId { site, ordinal }`), the field
//!   **open now** ([`SiteRecord::promoted`]), and the last demotion's IC instant.
//!
//! **Who writes it.** CF.1 writes it only through the staff fixture
//! (`tacsim_site_record`, `@tacsim/site`, `SiteRecordProvider`): it leaves a unit, drops an
//! object, takes either away, and sets or clears the promoted flag, so the surface's refusals
//! (Q-CF-30) and the persistence gate can be shown before the field host exists. **CF.2's field
//! host is the production writer**: at promotion it reads the record through the family handle
//! (`PortFamilies::sites`) and calls [`SiteRecord::promote`]; at demotion
//! [`SiteRecord::demote`] with the units it leaves, then [`SiteRecord::drop_object`] for each
//! remains or crate the field made — each inside its own authoritative bundle, `txn.replace`
//! under [`SiteRecord::key`].
//!
//! **Who reads it.** The surface act's start refuses every act on a promoted site
//! ([`promoted_refusal`]; the command, the offer and the prepare each read it), the surface tick
//! holds a promoted site's plans and queued starts, `+tv/map` names the open field and shows the
//! ground objects within sight of the viewer's own rigs, and `@tacsim/site <site>` shows staff the
//! whole record. **The record is objective and never published whole to a player** (Q-CF-28).
//!
//! Every pose is a site pose in centimetres on the site's 10 m grid; every number here is per
//! object or per cell, never per window (Q-CF-51), and provisional (doc 32 §2's last row).

use std::collections::BTreeMap;

use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn,
    AuthoritativeActionOutput, CanonicalId, DefinitionRef, ObjId, ProjectedParameterKind,
    ProviderOfferDraft, RulesValue, StateFamilyHandle, World,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::arena::mind::SortieId;
use crate::port_tick::port_error;
use crate::surface::ground::{SITE_CELL_CM, SITE_CELLS};

/// The family's id (doc 32 Q-CF-24: `tacsim.field.<site>`).
pub const SITE_RECORD_FAMILY: &str = "tacsim.field";
/// [`SiteRecord::schema`].
pub const SITE_RECORD_SCHEMA: u16 = 1;
/// **The field cap** (doc 32 Q-CF-21, Q-CF-45; provisionally `[tacsim.field].max_units`): the most
/// units one field admits, and so — enforced at the surface — the most rigs one site holds out.
/// A code constant until CF.2 adds `[tacsim.field]` with its `disengage_s`.
pub const FIELD_MAX_UNITS: u32 = 24;
/// **The most non-unit objects one site keeps** (doc 32 Q-CF-27, provisionally 64).
pub const MAX_GROUND_OBJECTS: usize = 64;
/// **The most units one site's record indexes** (a record-size bound, not a rule of doc 32:
/// casualties and wrecks never count against the field cap, Q-CF-26). A leave past it is refused.
pub const MAX_LEFT_UNITS: usize = 128;
/// The most goods one crate's manifest names.
pub const MAX_MANIFEST_GOODS: usize = 8;
/// A crate's or a remains' widest footprint, cells a side.
pub const MAX_FOOTPRINT_CELLS: u8 = 8;
/// The record's ceiling, canonical JSON bytes (the family's own limit).
pub const MAX_SITE_RECORD_BYTES: usize = 65_536;

// ---- the record -------------------------------------------------------------------------------

/// **The family key**: one record per site, its port entry.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SiteKey(pub CanonicalId);

impl SiteKey {
    /// The site's port entry, as it is.
    #[must_use]
    pub fn key_text(&self) -> String {
        self.0.to_string()
    }

    /// The strict inverse of [`SiteKey::key_text`].
    #[must_use]
    pub fn parse(text: &str) -> Option<SiteKey> {
        CanonicalId::new(text).ok().map(SiteKey)
    }
}

/// **A field: its site and its ordinal** (doc 32 Q-CF-2). Never a match id (D-23-10).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldId {
    /// The site's port entry.
    pub site: CanonicalId,
    /// The field's ordinal on the site, from 1.
    pub ordinal: u32,
}

/// **A pose on the site's grid**: centimetres east and north of the site's south-west corner,
/// and a heading in millidegrees (`< 360_000`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SitePose {
    /// East, centimetres.
    pub x_cm: i64,
    /// North, centimetres.
    pub y_cm: i64,
    /// Heading, millidegrees.
    pub heading_mdeg: u32,
}

impl SitePose {
    /// **The centre of cell `(x, y)`**, facing north.
    #[must_use]
    pub fn at_cell(x: i64, y: i64) -> SitePose {
        let side = i64::from(SITE_CELL_CM);
        SitePose {
            x_cm: x.saturating_mul(side).saturating_add(side / 2),
            y_cm: y.saturating_mul(side).saturating_add(side / 2),
            heading_mdeg: 0,
        }
    }

    /// The cell it stands in.
    #[must_use]
    pub fn cell(&self) -> (i64, i64) {
        crate::surface::excursion::site_cell(self.x_cm, self.y_cm)
    }

    /// On the site's grid with a heading below a full turn.
    fn on_site(&self) -> bool {
        let edge = i64::from(SITE_CELLS) * i64::from(SITE_CELL_CM);
        (0..edge).contains(&self.x_cm)
            && (0..edge).contains(&self.y_cm)
            && self.heading_mdeg < 360_000
    }
}

/// **What a left unit is** (doc 32 Q-CF-25 a, b): its epoch is open either way (Q-H9).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeftCondition {
    /// Disabled, immobile or unarmed; towable (27a H2).
    Casualty,
    /// In the Wreck state.
    Wreck,
}

impl LeftCondition {
    /// The word staff and the record use.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            LeftCondition::Casualty => "casualty",
            LeftCondition::Wreck => "wreck",
        }
    }

    /// The inverse of [`LeftCondition::word`].
    #[must_use]
    pub fn from_word(word: &str) -> Option<LeftCondition> {
        match word {
            "casualty" => Some(LeftCondition::Casualty),
            "wreck" => Some(LeftCondition::Wreck),
            _ => None,
        }
    }
}

/// **One unit left on the ground** (doc 32 Q-CF-24): an index entry, never the unit's record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeftUnit {
    /// The unit's object (`unit:<obj>` under tactics custody).
    pub unit: ObjId,
    /// Where it was left.
    pub pose: SitePose,
    /// Casualty or wreck.
    pub condition: LeftCondition,
    /// The epoch it was in when left — open (Q-H9); `None` for a unit that never had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<SortieId>,
    /// The IC instant it was left.
    pub left_ic: i64,
}

/// **What a ground object is** (doc 32 Q-CF-25 c, d).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum GroundKind {
    /// **The remains of an irrecoverable hull** after its epoch closed destroyed: a footprint, a
    /// mass, and (the first version) a manifest of nothing.
    Remains {
        /// The hull it was, when one is known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Option<ObjId>,
        /// The mass, kilograms.
        mass_kg: u64,
        /// How many remains the cap folded into this one (1: none, Q-CF-27).
        merged: u16,
    },
    /// **Dropped cargo**: ore or goods in a container, by a destroyed hopper or an explicit dump.
    Cargo {
        /// The goods and their quantities, each at least 1.
        manifest: BTreeMap<CanonicalId, u64>,
        /// The hull that dropped it, when one is known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<ObjId>,
    },
}

impl GroundKind {
    /// `remains` or `cargo`.
    #[must_use]
    pub fn word(&self) -> &'static str {
        match self {
            GroundKind::Remains { .. } => "remains",
            GroundKind::Cargo { .. } => "cargo",
        }
    }
}

/// **One non-unit object on the ground**, owned by the record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroundObject {
    /// Its number on this site, from 1, never reused.
    pub id: u32,
    /// Where it lies.
    pub pose: SitePose,
    /// Its footprint, cells a side, `1..=`[`MAX_FOOTPRINT_CELLS`] (a static obstacle in a field,
    /// Q-CF-26).
    pub footprint: u8,
    /// Remains or cargo.
    pub kind: GroundKind,
    /// The IC instant it was left.
    pub left_ic: i64,
}

/// **Who opened the field now open.**
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PromotedBy {
    /// The staff fixture (doc 32 cut CF.1): a flag with no field host behind it.
    Staff {
        /// The staff member.
        staff: ObjId,
    },
}

/// **The field open on a site** (doc 32 Q-CF-14): while it is `Some`, every surface act there is
/// refused (Q-CF-30).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Promotion {
    /// The field's ordinal: the record's [`SiteRecord::ordinal`].
    pub ordinal: u32,
    /// The IC instant it opened.
    pub at_ic: i64,
    /// Who opened it.
    pub by: PromotedBy,
}

/// **One site's record in `tacsim.field`** (doc 32 Q-CF-24).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SiteRecord {
    /// [`SITE_RECORD_SCHEMA`].
    pub schema: u16,
    /// The site's port entry: the key.
    pub site: CanonicalId,
    /// The last field ordinal opened here (0: none yet).
    pub ordinal: u32,
    /// The field open now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promoted: Option<Promotion>,
    /// The last demotion's IC instant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub demoted_ic: Option<i64>,
    /// The units left on the ground, strictly increasing by unit.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub units: Vec<LeftUnit>,
    /// The non-unit objects, strictly increasing by id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub objects: Vec<GroundObject>,
    /// The last object number issued (0: none).
    pub next_object: u32,
}

/// **Why a write to the record refuses.**
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SiteRefused {
    /// A field is already open (promote).
    Promoted,
    /// No field is open (demote).
    NotPromoted,
    /// The pose is off the site's grid.
    OffSite,
    /// [`MAX_LEFT_UNITS`] units are indexed.
    UnitsFull,
    /// [`MAX_GROUND_OBJECTS`] objects and no two remains to merge.
    ObjectsFull,
    /// A crate with no goods, more than [`MAX_MANIFEST_GOODS`], a zero quantity, or a footprint
    /// out of bounds.
    Malformed,
    /// No such unit or object on the record.
    Absent,
    /// An ordinal or an object number would overflow.
    Exhausted,
}

impl SiteRefused {
    /// The sentence a refusal reads as.
    #[must_use]
    pub fn sentence(self) -> &'static str {
        match self {
            SiteRefused::Promoted => "a field is already open on that site",
            SiteRefused::NotPromoted => "no field is open on that site",
            SiteRefused::OffSite => "that pose is off the site's ground",
            SiteRefused::UnitsFull => "that site's record indexes as many units as it holds",
            SiteRefused::ObjectsFull => {
                "that site holds as many objects as it keeps and no remains to merge"
            }
            SiteRefused::Malformed => "that object is malformed",
            SiteRefused::Absent => "no such unit or object on that site",
            SiteRefused::Exhausted => "that site's numbering is exhausted",
        }
    }
}

impl SiteRecord {
    /// **An empty record for `site`**: no field yet, nothing on the ground.
    #[must_use]
    pub fn new(site: CanonicalId) -> SiteRecord {
        SiteRecord {
            schema: SITE_RECORD_SCHEMA,
            site,
            ordinal: 0,
            promoted: None,
            demoted_ic: None,
            units: Vec::new(),
            objects: Vec::new(),
            next_object: 0,
        }
    }

    /// The record's key, derived from the value (the families rule: writers derive the key from
    /// the value, readers re-check it).
    #[must_use]
    pub fn key(&self) -> SiteKey {
        SiteKey(self.site.clone())
    }

    /// **The field open now**, as its id.
    #[must_use]
    pub fn open_field(&self) -> Option<FieldId> {
        self.promoted.as_ref().map(|open| FieldId {
            site: self.site.clone(),
            ordinal: open.ordinal,
        })
    }

    /// Whether the record says nothing a fresh one would not: no field ever, nothing on the
    /// ground (a staff `take` of the last thing leaves such a record; it is removed, not kept).
    #[must_use]
    pub fn is_blank(&self) -> bool {
        self.ordinal == 0
            && self.promoted.is_none()
            && self.demoted_ic.is_none()
            && self.units.is_empty()
            && self.objects.is_empty()
            && self.next_object == 0
    }

    /// **Open the site's next field** (doc 32 Q-CF-14; CF.2's promotion, CF.1's staff flag): the
    /// ordinal advances and the field is open from `at_ic`.
    ///
    /// # Errors
    ///
    /// [`SiteRefused::Promoted`] when a field is already open; [`SiteRefused::Exhausted`].
    pub fn promote(&mut self, at_ic: i64, by: PromotedBy) -> Result<FieldId, SiteRefused> {
        if self.promoted.is_some() {
            return Err(SiteRefused::Promoted);
        }
        let ordinal = self.ordinal.checked_add(1).ok_or(SiteRefused::Exhausted)?;
        self.ordinal = ordinal;
        self.promoted = Some(Promotion { ordinal, at_ic, by });
        Ok(FieldId {
            site: self.site.clone(),
            ordinal,
        })
    }

    /// **Close the open field** (doc 32 Q-CF-20; CF.2's demotion): `units` replaces the index —
    /// every unit the field leaves on the ground, at its field pose (Q-CF-31) — and the instant
    /// is kept. The objects stay; the field adds its own through [`SiteRecord::drop_object`].
    /// CF.1's staff unflag passes the index as it stands.
    ///
    /// # Errors
    ///
    /// [`SiteRefused::NotPromoted`]; [`SiteRefused::OffSite`] or [`SiteRefused::UnitsFull`] for
    /// the index, which is then not applied.
    pub fn demote(&mut self, at_ic: i64, units: Vec<LeftUnit>) -> Result<FieldId, SiteRefused> {
        let field = self.open_field().ok_or(SiteRefused::NotPromoted)?;
        let mut index: BTreeMap<ObjId, LeftUnit> = BTreeMap::new();
        for unit in units {
            if !unit.pose.on_site() {
                return Err(SiteRefused::OffSite);
            }
            index.insert(unit.unit, unit);
        }
        if index.len() > MAX_LEFT_UNITS {
            return Err(SiteRefused::UnitsFull);
        }
        self.units = index.into_values().collect();
        self.promoted = None;
        self.demoted_ic = Some(at_ic);
        Ok(field)
    }

    /// **Leave `unit` on the ground** (or move its entry): kept in unit order.
    ///
    /// # Errors
    ///
    /// [`SiteRefused::OffSite`]; [`SiteRefused::UnitsFull`] for a new entry past the bound.
    pub fn leave_unit(&mut self, unit: LeftUnit) -> Result<(), SiteRefused> {
        if !unit.pose.on_site() {
            return Err(SiteRefused::OffSite);
        }
        match self
            .units
            .binary_search_by_key(&unit.unit, |kept| kept.unit)
        {
            Ok(at) => {
                if let Some(slot) = self.units.get_mut(at) {
                    *slot = unit;
                }
            }
            Err(at) => {
                if self.units.len() >= MAX_LEFT_UNITS {
                    return Err(SiteRefused::UnitsFull);
                }
                self.units.insert(at, unit);
            }
        }
        Ok(())
    }

    /// **Take `unit` off the index** (a recovery, a tow, staff).
    ///
    /// # Errors
    ///
    /// [`SiteRefused::Absent`].
    pub fn take_unit(&mut self, unit: ObjId) -> Result<LeftUnit, SiteRefused> {
        let at = self
            .units
            .binary_search_by_key(&unit, |kept| kept.unit)
            .map_err(|_| SiteRefused::Absent)?;
        Ok(self.units.remove(at))
    }

    /// **Drop an object on the ground** and return its number. Past [`MAX_GROUND_OBJECTS`] the
    /// cap's **published rule** (doc 32 Q-CF-27) runs first: the two oldest remains (by the
    /// instant left, then the number) merge into the older one's number at their mass-weighted
    /// centroid — an unweighted midpoint when both are massless — with the masses and the merge
    /// counts summed, the wider footprint and the older instant; cargo and units are never
    /// merged or dropped, so a cap full of cargo refuses.
    ///
    /// # Errors
    ///
    /// [`SiteRefused::OffSite`], [`SiteRefused::Malformed`], [`SiteRefused::ObjectsFull`],
    /// [`SiteRefused::Exhausted`].
    pub fn drop_object(
        &mut self,
        pose: SitePose,
        footprint: u8,
        kind: GroundKind,
        at_ic: i64,
    ) -> Result<u32, SiteRefused> {
        if !pose.on_site() {
            return Err(SiteRefused::OffSite);
        }
        if !kind_ok(&kind) || !(1..=MAX_FOOTPRINT_CELLS).contains(&footprint) {
            return Err(SiteRefused::Malformed);
        }
        let id = self
            .next_object
            .checked_add(1)
            .ok_or(SiteRefused::Exhausted)?;
        if self.objects.len() >= MAX_GROUND_OBJECTS && !self.merge_oldest_remains() {
            return Err(SiteRefused::ObjectsFull);
        }
        self.next_object = id;
        self.objects.push(GroundObject {
            id,
            pose,
            footprint,
            kind,
            left_ic: at_ic,
        });
        Ok(id)
    }

    /// **Take object `id` off the ground** (a pickup, staff).
    ///
    /// # Errors
    ///
    /// [`SiteRefused::Absent`].
    pub fn take_object(&mut self, id: u32) -> Result<GroundObject, SiteRefused> {
        let at = self
            .objects
            .binary_search_by_key(&id, |kept| kept.id)
            .map_err(|_| SiteRefused::Absent)?;
        Ok(self.objects.remove(at))
    }

    /// Q-CF-27's merge: `false` when fewer than two remains lie here.
    fn merge_oldest_remains(&mut self) -> bool {
        let mut remains: Vec<(i64, u32)> = self
            .objects
            .iter()
            .filter(|object| matches!(object.kind, GroundKind::Remains { .. }))
            .map(|object| (object.left_ic, object.id))
            .collect();
        remains.sort_unstable();
        let (Some(&(_, older)), Some(&(_, newer))) = (remains.first(), remains.get(1)) else {
            return false;
        };
        let Ok(at) = self.objects.binary_search_by_key(&newer, |kept| kept.id) else {
            return false;
        };
        let gone = self.objects.remove(at);
        let Some(kept) = self.objects.iter_mut().find(|object| object.id == older) else {
            return false;
        };
        let (
            GroundKind::Remains {
                mass_kg: kept_kg,
                merged: kept_n,
                ..
            },
            GroundKind::Remains {
                mass_kg: gone_kg,
                merged: gone_n,
                ..
            },
        ) = (&mut kept.kind, &gone.kind)
        else {
            return false;
        };
        let (a, b) = (i128::from(*kept_kg), i128::from(*gone_kg));
        let mean = |p: i64, q: i64| -> i64 {
            let total = a + b;
            let value = if total == 0 {
                (i128::from(p) + i128::from(q)) / 2
            } else {
                (i128::from(p) * a + i128::from(q) * b) / total
            };
            i64::try_from(value).unwrap_or(p)
        };
        kept.pose = SitePose {
            x_cm: mean(kept.pose.x_cm, gone.pose.x_cm),
            y_cm: mean(kept.pose.y_cm, gone.pose.y_cm),
            heading_mdeg: kept.pose.heading_mdeg,
        };
        *kept_kg = kept_kg.saturating_add(*gone_kg);
        *kept_n = kept_n.saturating_add(*gone_n);
        kept.footprint = kept.footprint.max(gone.footprint);
        kept.left_ic = kept.left_ic.min(gone.left_ic);
        true
    }

    /// **The record's own invariants** (the family's validator calls it): the schema; a promotion
    /// is the record's ordinal; units strictly increasing by unit, at most [`MAX_LEFT_UNITS`];
    /// objects strictly increasing by id, at most [`MAX_GROUND_OBJECTS`], none past
    /// `next_object`; every pose on the site; every object well formed.
    ///
    /// # Errors
    ///
    /// The first invariant broken, as a sentence.
    pub fn check(&self) -> Result<(), String> {
        if self.schema != SITE_RECORD_SCHEMA {
            return Err(format!(
                "a site record is schema {SITE_RECORD_SCHEMA}, not {}",
                self.schema
            ));
        }
        if let Some(open) = &self.promoted
            && (open.ordinal != self.ordinal || open.ordinal == 0)
        {
            return Err("an open field is the site's last ordinal".to_owned());
        }
        if self.units.len() > MAX_LEFT_UNITS {
            return Err(format!(
                "a site record indexes at most {MAX_LEFT_UNITS} units"
            ));
        }
        if self.units.windows(2).any(|pair| match pair {
            [a, b] => a.unit >= b.unit,
            _ => false,
        }) {
            return Err("a site record's units are in strictly increasing order".to_owned());
        }
        if self.objects.len() > MAX_GROUND_OBJECTS {
            return Err(format!(
                "a site record keeps at most {MAX_GROUND_OBJECTS} objects"
            ));
        }
        if self.objects.windows(2).any(|pair| match pair {
            [a, b] => a.id >= b.id,
            _ => false,
        }) {
            return Err("a site record's objects are in strictly increasing order".to_owned());
        }
        if self
            .objects
            .iter()
            .any(|object| object.id == 0 || object.id > self.next_object)
        {
            return Err("an object's number is one the record issued".to_owned());
        }
        if self.units.iter().any(|unit| !unit.pose.on_site())
            || self.objects.iter().any(|object| !object.pose.on_site())
        {
            return Err("every pose on a site record is on the site's ground".to_owned());
        }
        if self.objects.iter().any(|object| {
            !kind_ok(&object.kind) || !(1..=MAX_FOOTPRINT_CELLS).contains(&object.footprint)
        }) {
            return Err("an object on a site record is malformed".to_owned());
        }
        Ok(())
    }
}

/// A crate names `1..=`[`MAX_MANIFEST_GOODS`] goods, each at least 1; remains merge at least once.
fn kind_ok(kind: &GroundKind) -> bool {
    match kind {
        GroundKind::Remains { merged, .. } => *merged >= 1,
        GroundKind::Cargo { manifest, .. } => {
            !manifest.is_empty()
                && manifest.len() <= MAX_MANIFEST_GOODS
                && manifest.values().all(|qty| *qty >= 1)
        }
    }
}

// ---- the surface's refusal ------------------------------------------------------------------

/// **The surface act's refusal on a promoted site** (doc 32 Q-CF-30): `{site}`, `{ordinal}` —
/// the fight named, never who marked whom (Q-CF-7).
pub const FIELD_OPEN: &str =
    "Field {ordinal} is being fought on {site}. Surface work there waits until it settles.";
/// The start's message for the same refusal (the prepare cannot name the site's display name).
pub const FIELD_OPEN_CODE: &str = "a field is being fought on that site; surface work waits";
/// `+tv/map`'s line on a promoted site: `{site}`, `{ordinal}`.
pub const MAP_FIELD_OPEN: &str =
    "Field {ordinal} is open on {site}: no surface work until it settles.";

/// **The field open on `site`, if any**, through any reader of the family.
pub(crate) fn open_field<E>(
    read: impl FnOnce(&SiteKey) -> Result<Option<SiteRecord>, E>,
    site: &CanonicalId,
) -> Result<Option<FieldId>, E> {
    let key = SiteKey(site.clone());
    Ok(read(&key)?
        .filter(|record| record.key() == key)
        .and_then(|record| record.open_field()))
}

/// **The player's line for a promoted site** ([`FIELD_OPEN`]).
#[must_use]
pub fn promoted_refusal(field: &FieldId, site_name: &str) -> String {
    FIELD_OPEN
        .replace("{ordinal}", &field.ordinal.to_string())
        .replace("{site}", site_name)
}

/// **The command's gate** (doc 32 Q-CF-30): a `+tv` act, drive, mining run or unload on a rig
/// whose hull is settled on a promoted site is refused with [`FIELD_OPEN`] before it is planned.
/// A hull not on a site, or the family unreadable, passes: the start's prepare reads the record
/// again and refuses there.
///
/// # Errors
///
/// The [`FIELD_OPEN`] line.
pub(crate) fn field_gate(ctx: &mush_core::Ctx<'_>, hull: ObjId) -> Result<(), mush_core::CmdError> {
    let Some((entry, _)) = crate::surface::hull_home(ctx.w, hull) else {
        return Ok(());
    };
    let Some(handles) = ctx.w.module_state::<crate::port::families::PortHandles>() else {
        return Ok(());
    };
    let field = open_field(
        |key| {
            ctx.action_state(&handles.families.sites, key)
                .map(|found| found.cloned())
        },
        &entry,
    )
    .ok()
    .flatten();
    match field {
        Some(field) => Err(mush_core::CmdError::msg(promoted_refusal(
            &field,
            &crate::surface::act::site_name(ctx.w, &entry),
        ))),
        None => Ok(()),
    }
}

// ---- the staff fixture: tacsim_site_record ----------------------------------------------------

/// The staff fixture's action (doc 32 cut CF.1).
pub const SITE_RECORD_ACTION: &str = "tacsim_site_record";
/// Its outcome.
pub const SITE_RECORD_OUTCOME: &str = "tacsim_site_record_result";
/// Its idempotency namespace: `tacsim.site/{actor}/{site}/{op}/{now_ic}/{serial}`, the serial
/// folded from the record as the command read it ([`fixture_key`]).
pub const SITE_KEY_PREFIX: &str = "tacsim.site/";
/// The fixture's operations.
pub const SITE_OPS: [&str; 6] = ["leave", "drop", "take", "remove", "promote", "demote"];

/// **`tacsim_site_record`** — the staff fixture over the site record (doc 32 cut CF.1): leave a
/// unit, drop remains or a crate, take either away, open or close the promoted flag. Offered to
/// staff only, self-scoped; its offer fixes the instant and the lane's sites. Never a player act
/// and never a field host: the flag it sets has no field behind it.
pub(crate) struct SiteRecordProvider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub sites: StateFamilyHandle<SiteKey, SiteRecord>,
}

fn invalid(message: &'static str) -> ActionError {
    port_error(ActionErrorCode::InvalidRequest, message)
}

fn rejected(message: &'static str) -> ActionError {
    port_error(ActionErrorCode::ProviderRejected, message)
}

/// The invocation's parameters: self-scoped, no targets, an object.
fn site_parameters(invocation: &ActionInvocation) -> Result<&Map<String, Value>, ActionError> {
    if invocation.subject() != &mush_core::PerceivedRef::SelfActor
        || !invocation.targets().is_empty()
    {
        return Err(invalid(
            "a site record write is self-scoped and takes no targets",
        ));
    }
    invocation
        .parameters()
        .as_object()
        .ok_or_else(|| invalid("a site record write names a site and an operation"))
}

/// An integer parameter.
fn int(parameters: &Map<String, Value>, name: &str) -> Option<i64> {
    parameters.get(name).and_then(Value::as_i64)
}

/// The pose a write names: `x`, `y` cells and an optional `heading` in millidegrees.
fn pose_of(parameters: &Map<String, Value>) -> Result<SitePose, ActionError> {
    let (Some(x), Some(y)) = (int(parameters, "x"), int(parameters, "y")) else {
        return Err(invalid("a site record write names a cell"));
    };
    let mut pose = SitePose::at_cell(x, y);
    pose.heading_mdeg = int(parameters, "heading")
        .and_then(|h| u32::try_from(h).ok())
        .unwrap_or(0);
    Ok(pose)
}

/// The `n`th unit-or-object parameter.
fn obj_of(parameters: &Map<String, Value>, name: &str) -> Result<u32, ActionError> {
    int(parameters, name)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| invalid("a site record write names its unit or object by number"))
}

/// **The write `op` applies to `record`** (pure; the prepare's body).
///
/// # Errors
///
/// The refusal, as the prepare reports it.
pub(crate) fn apply(
    record: &mut SiteRecord,
    op: &str,
    parameters: &Map<String, Value>,
    (actor, now): (ObjId, i64),
) -> Result<String, ActionError> {
    let refused = |why: SiteRefused| rejected(why.sentence());
    match op {
        "leave" => {
            let unit = ObjId(obj_of(parameters, "unit")?);
            let condition = parameters
                .get("condition")
                .and_then(Value::as_str)
                .and_then(LeftCondition::from_word)
                .ok_or_else(|| invalid("a left unit is a casualty or a wreck"))?;
            record
                .leave_unit(LeftUnit {
                    unit,
                    pose: pose_of(parameters)?,
                    condition,
                    epoch: None,
                    left_ic: now,
                })
                .map_err(refused)?;
            Ok(format!("{} #{} left", condition.word(), unit.0))
        }
        "drop" => {
            let footprint = int(parameters, "footprint")
                .and_then(|n| u8::try_from(n).ok())
                .unwrap_or(1);
            let kind = match parameters.get("kind").and_then(Value::as_str) {
                Some("remains") => GroundKind::Remains {
                    of: int(parameters, "unit")
                        .and_then(|n| u32::try_from(n).ok())
                        .map(ObjId),
                    mass_kg: int(parameters, "mass")
                        .and_then(|n| u64::try_from(n).ok())
                        .unwrap_or(0),
                    merged: 1,
                },
                Some("cargo") => {
                    let good = parameters
                        .get("good")
                        .and_then(Value::as_str)
                        .and_then(|text| CanonicalId::new(text).ok())
                        .ok_or_else(|| invalid("a crate names its good"))?;
                    let qty = int(parameters, "qty")
                        .and_then(|n| u64::try_from(n).ok())
                        .ok_or_else(|| invalid("a crate names its quantity"))?;
                    GroundKind::Cargo {
                        manifest: BTreeMap::from([(good, qty)]),
                        from: None,
                    }
                }
                _ => return Err(invalid("a dropped object is remains or cargo")),
            };
            let word = kind.word();
            let id = record
                .drop_object(pose_of(parameters)?, footprint, kind, now)
                .map_err(refused)?;
            Ok(format!("{word} {id} dropped"))
        }
        "take" => {
            let unit = record
                .take_unit(ObjId(obj_of(parameters, "unit")?))
                .map_err(refused)?;
            Ok(format!("{} #{} taken", unit.condition.word(), unit.unit.0))
        }
        "remove" => {
            let object = record
                .take_object(obj_of(parameters, "object")?)
                .map_err(refused)?;
            Ok(format!("{} {} removed", object.kind.word(), object.id))
        }
        "promote" => {
            let field = record
                .promote(now, PromotedBy::Staff { staff: actor })
                .map_err(refused)?;
            Ok(format!("field {} opened", field.ordinal))
        }
        "demote" => {
            let units = record.units.clone();
            let field = record.demote(now, units).map_err(refused)?;
            Ok(format!("field {} closed", field.ordinal))
        }
        _ => Err(invalid("that is not a site record operation")),
    }
}

impl ActionProvider for SiteRecordProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        let parameters = site_parameters(invocation)?;
        parameters
            .get("site")
            .and_then(Value::as_str)
            .and_then(SiteKey::parse)
            .ok_or_else(|| invalid("a site record write names a site"))?;
        parameters
            .get("op")
            .and_then(Value::as_str)
            .filter(|op| SITE_OPS.contains(op))
            .ok_or_else(|| invalid("that is not a site record operation"))?;
        Ok(())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        if !crate::surface::act::self_query(action, &self.action, query) {
            return Ok(None);
        }
        let w: &World = &engine.world;
        if !w
            .module_state::<crate::TacsimStore>()
            .is_some_and(crate::TacsimStore::enabled)
            || !crate::commands::is_staff(w, query.actor())
        {
            return Ok(None);
        }
        let sites: Vec<String> = crate::surface::chart::lane_sites(w)
            .iter()
            .map(|site| site.entry.to_string())
            .collect();
        if sites.is_empty() {
            return Ok(None);
        }
        let cell = i64::from(SITE_CELLS) - 1;
        let integer =
            |minimum: i64, maximum: i64| ProjectedParameterKind::Integer { minimum, maximum };
        let parameters = vec![
            crate::surface::act::parameter(
                "site",
                true,
                ProjectedParameterKind::Choice { choices: sites },
            )?,
            crate::surface::act::parameter(
                "op",
                true,
                ProjectedParameterKind::Choice {
                    choices: SITE_OPS.iter().map(|op| (*op).to_owned()).collect(),
                },
            )?,
            crate::surface::act::parameter("x", false, integer(0, cell))?,
            crate::surface::act::parameter("y", false, integer(0, cell))?,
            crate::surface::act::parameter("heading", false, integer(0, 359_999))?,
            crate::surface::act::parameter("unit", false, integer(1, i64::from(u32::MAX)))?,
            crate::surface::act::parameter("object", false, integer(1, i64::from(u32::MAX)))?,
            crate::surface::act::parameter(
                "condition",
                false,
                ProjectedParameterKind::Choice {
                    choices: vec!["casualty".to_owned(), "wreck".to_owned()],
                },
            )?,
            crate::surface::act::parameter(
                "kind",
                false,
                ProjectedParameterKind::Choice {
                    choices: vec!["remains".to_owned(), "cargo".to_owned()],
                },
            )?,
            crate::surface::act::parameter(
                "good",
                false,
                ProjectedParameterKind::Text { maximum_bytes: 64 },
            )?,
            crate::surface::act::parameter("qty", false, integer(1, i64::from(u32::MAX)))?,
            crate::surface::act::parameter("mass", false, integer(0, i64::from(u32::MAX)))?,
            crate::surface::act::parameter(
                "footprint",
                false,
                integer(1, i64::from(MAX_FOOTPRINT_CELLS)),
            )?,
        ];
        crate::surface::act::offer_draft(
            action,
            engine,
            json!({ "now": w.world_ic_now().at }),
            parameters,
            (
                "tacsim-site-record",
                "Edit a site record",
                "Staff fixture: leave units and objects on a site's ground, or flag it promoted.",
            ),
        )
        .map(Some)
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.validate_invocation(invocation)?;
        let parameters = site_parameters(invocation)?;
        let now = int(parameters, "now")
            .ok_or_else(|| invalid("a site record write's offer fixes the instant"))?;
        let key = parameters
            .get("site")
            .and_then(Value::as_str)
            .and_then(SiteKey::parse)
            .ok_or_else(|| invalid("a site record write names a site"))?;
        let op = parameters
            .get("op")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("that is not a site record operation"))?;
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        let found = txn
            .get(&self.sites, &key)
            .map_err(|_| integrity("the site records are unavailable"))?
            .cloned();
        if found.as_ref().is_some_and(|record| record.key() != key) {
            return Err(integrity("a site record disagrees with its own key"));
        }
        let existed = found.is_some();
        let mut record = found.unwrap_or_else(|| SiteRecord::new(key.0.clone()));
        let said = apply(&mut record, op, parameters, (invocation.actor(), now))?;
        record
            .check()
            .map_err(|_| integrity("the site record is out of bounds"))?;
        if record.is_blank() {
            if existed {
                txn.remove(&self.sites, &key)
                    .map_err(|_| integrity("the site record did not stage"))?;
            }
        } else if existed {
            txn.replace(&self.sites, &key, record)
                .map_err(|_| integrity("the site record did not stage"))?;
        } else {
            txn.insert(&self.sites, &key, record)
                .map_err(|_| integrity("the site record did not stage"))?;
        }
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({ "actor": invocation.actor().0, "site": key.key_text(), "said": said }),
        )
        .map_err(|_| {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "the site record's outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

// ---- the staff command: @tacsim/site ---------------------------------------------------------

/// The one usage line of `@tacsim/site`.
pub const SITE_USAGE: &str = "Usage: @tacsim/site <site> | <site> leave #<unit> casualty|wreck <x>,<y> \
     | <site> drop crate <qty> <good> <x>,<y> | <site> drop remains <x>,<y> [<kg>] \
     | <site> take #<unit> | <site> remove <n> | <site> promote | <site> demote";

/// **The fixture's idempotency key**: one write a staff member, a site, an op, an IC instant and
/// a serial folded from the record the command read, so two different writes in one IC second
/// never share a key.
#[must_use]
pub fn fixture_key(
    actor: ObjId,
    site: &CanonicalId,
    op: &str,
    now_ic: i64,
    serial: u64,
) -> Option<mush_core::IdempotencyKeyText> {
    mush_core::IdempotencyKeyText::parse_canonical(format!(
        "{SITE_KEY_PREFIX}{}/{site}/{op}/{now_ic}/{serial}",
        actor.0
    ))
    .ok()
}

/// The record's serial for [`fixture_key`]: its ordinal, issued objects and index size, folded.
fn serial_of(record: Option<&SiteRecord>) -> u64 {
    record.map_or(0, |record| {
        (u64::from(record.ordinal) << 40)
            ^ (u64::from(record.next_object) << 16)
            ^ u64::try_from(record.units.len()).unwrap_or(0)
            ^ u64::from(record.promoted.is_some())
    })
}

/// `#<n>`, strictly.
fn hash_object(word: Option<&str>) -> Option<u32> {
    word.and_then(|word| word.strip_prefix('#'))
        .and_then(crate::port::families::strict_decimal_u32)
}

/// `<x>,<y>`, each on the site's grid.
fn cell_word(word: Option<&str>) -> Option<(i64, i64)> {
    let (x, y) = word?.split_once(',')?;
    let side = i64::from(SITE_CELLS);
    let x: i64 = x.trim().parse().ok().filter(|x| (0..side).contains(x))?;
    let y: i64 = y.trim().parse().ok().filter(|y| (0..side).contains(y))?;
    Some((x, y))
}

/// **The whole record, staff truth** (`@tacsim/site <site>`): never a player's view (Q-CF-28).
#[must_use]
pub fn record_lines(name: &str, site: &CanonicalId, record: Option<&SiteRecord>) -> Vec<String> {
    let Some(record) = record else {
        return vec![format!(
            "Site record of {name} ({site}): no field yet, nothing on the ground."
        )];
    };
    let field = match (&record.promoted, record.demoted_ic) {
        (Some(open), _) => {
            let PromotedBy::Staff { staff } = &open.by;
            format!(
                "field {} open since IC {} (staff #{} fixture)",
                open.ordinal, open.at_ic, staff.0
            )
        }
        (None, Some(at)) => format!(
            "no field open; {} so far, the last closed at IC {at}",
            record.ordinal
        ),
        (None, None) => format!("no field open; {} so far", record.ordinal),
    };
    let mut lines = vec![format!("Site record of {name} ({site}): {field}.")];
    if record.units.is_empty() {
        lines.push("Units left: none.".to_owned());
    }
    for unit in &record.units {
        let (x, y) = unit.pose.cell();
        lines.push(format!(
            "Unit #{}: {} at ({x}, {y}), left at IC {}{}.",
            unit.unit.0,
            unit.condition.word(),
            unit.left_ic,
            unit.epoch
                .map_or_else(String::new, |epoch| format!(", epoch {epoch} open"))
        ));
    }
    if record.objects.is_empty() {
        lines.push("Objects: none.".to_owned());
    }
    for object in &record.objects {
        let (x, y) = object.pose.cell();
        lines.push(format!(
            "Object {}: {} at ({x}, {y}), {} cells a side, left at IC {}.",
            object.id,
            object_words(&object.kind),
            object.footprint,
            object.left_ic
        ));
    }
    lines
}

/// What an object is, in words: `remains, 40 t` or `a crate of 2 metal_ore`.
#[must_use]
pub fn object_words(kind: &GroundKind) -> String {
    match kind {
        GroundKind::Remains {
            mass_kg, merged, ..
        } => format!(
            "remains, {} t{}",
            mass_kg / 1_000,
            if *merged > 1 {
                format!(", {merged} merged")
            } else {
                String::new()
            }
        ),
        GroundKind::Cargo { manifest, .. } => format!(
            "a crate of {}",
            manifest
                .iter()
                .map(|(good, qty)| format!("{qty} {good}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// **`@tacsim/site`** (doc 32 cut CF.1; staff, the caller gates): show a site's record, or submit
/// one fixture write through `tacsim_site_record`. The committed result is the record; the command
/// promises nothing before the prepare.
///
/// # Errors
///
/// The usage line, a site that is not the lane's, a unit with no tactics record, or the fixture
/// unregistered.
pub(crate) fn command(ctx: &mut mush_core::Ctx<'_>) -> Result<(), mush_core::CmdError> {
    use mush_core::CmdError;
    let usage = || CmdError::msg(SITE_USAGE);
    let raw = ctx.args.raw().to_owned();
    let mut words = raw.split_whitespace();
    let site = words
        .next()
        .and_then(|word| CanonicalId::new(word).ok())
        .ok_or_else(usage)?;
    if !crate::surface::chart::lane_sites(ctx.w)
        .iter()
        .any(|lane| lane.entry == site)
    {
        return Err(CmdError::msg(format!("{site} is not a site on the lane.")));
    }
    let handles = ctx
        .w
        .module_state::<crate::port::families::PortHandles>()
        .cloned()
        .ok_or_else(|| CmdError::msg("The site records are unavailable."))?;
    let key = SiteKey(site.clone());
    let record = ctx
        .action_state(&handles.families.sites, &key)
        .map_err(|error| CmdError::msg(error.to_string()))?
        .cloned()
        .filter(|record| record.key() == key);
    let name = crate::surface::act::site_name(ctx.w, &site);
    let Some(op) = words.next() else {
        for line in record_lines(&name, &site, record.as_ref()) {
            ctx.out.sys_to_actor(ctx.actor, "tacsim", line);
        }
        return Ok(());
    };
    let mut fields: Vec<(&str, Value)> = Vec::new();
    match op {
        "leave" => {
            let unit = hash_object(words.next()).ok_or_else(usage)?;
            let condition = words
                .next()
                .and_then(LeftCondition::from_word)
                .ok_or_else(usage)?;
            let (x, y) = cell_word(words.next()).ok_or_else(usage)?;
            if mush_tactics::module::unit_of(ctx.w, ObjId(unit)).is_none() {
                return Err(CmdError::msg(format!(
                    "#{unit} has no tactical unit record."
                )));
            }
            fields.extend([
                ("unit", json!(unit)),
                ("condition", json!(condition.word())),
                ("x", json!(x)),
                ("y", json!(y)),
            ]);
        }
        "drop" => match words.next() {
            Some("crate") => {
                let qty: u64 = words
                    .next()
                    .and_then(|word| word.parse().ok())
                    .filter(|qty| *qty >= 1)
                    .ok_or_else(usage)?;
                let good = words
                    .next()
                    .and_then(|word| CanonicalId::new(word).ok())
                    .ok_or_else(usage)?;
                let (x, y) = cell_word(words.next()).ok_or_else(usage)?;
                fields.extend([
                    ("kind", json!("cargo")),
                    ("qty", json!(qty)),
                    ("good", json!(good.to_string())),
                    ("x", json!(x)),
                    ("y", json!(y)),
                ]);
            }
            Some("remains") => {
                let (x, y) = cell_word(words.next()).ok_or_else(usage)?;
                let kg: u64 = match words.next() {
                    None => 0,
                    Some(word) => word.parse().map_err(|_| usage())?,
                };
                fields.extend([
                    ("kind", json!("remains")),
                    ("mass", json!(kg)),
                    ("x", json!(x)),
                    ("y", json!(y)),
                ]);
            }
            _ => return Err(usage()),
        },
        "take" => {
            let unit = hash_object(words.next()).ok_or_else(usage)?;
            fields.push(("unit", json!(unit)));
        }
        "remove" => {
            let n = words
                .next()
                .and_then(crate::port::families::strict_decimal_u32)
                .ok_or_else(usage)?;
            fields.push(("object", json!(n)));
        }
        "promote" | "demote" => {}
        _ => return Err(usage()),
    }
    if words.next().is_some() {
        return Err(usage());
    }
    let mut parameters = Map::new();
    parameters.insert("site".to_owned(), json!(site.to_string()));
    parameters.insert("op".to_owned(), json!(op));
    for (name, value) in fields {
        parameters.insert(name.to_owned(), value);
    }
    let action = handles
        .site_record
        .clone()
        .ok_or_else(|| CmdError::msg("The site record fixture is unavailable."))?;
    let now = ctx.w.world_ic_now().at;
    let key = fixture_key(ctx.actor, &site, op, now, serial_of(record.as_ref()))
        .ok_or_else(|| CmdError::msg("The site record key is unavailable."))?;
    ctx.submit_authoritative(
        action,
        mush_core::PerceivedRef::SelfActor,
        Vec::new(),
        Value::Object(parameters),
        key,
    );
    ctx.out.info(
        ctx.actor,
        format!("Site record write requested: {op} on {name}. Its committed result is the record."),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cid(text: &str) -> CanonicalId {
        CanonicalId::new(text).unwrap_or_else(|_| panic!("{text} is canonical"))
    }

    fn record() -> SiteRecord {
        SiteRecord::new(cid("ahuna_port"))
    }

    fn remains(mass_kg: u64) -> GroundKind {
        GroundKind::Remains {
            of: None,
            mass_kg,
            merged: 1,
        }
    }

    fn crate_of(good: &str, qty: u64) -> GroundKind {
        GroundKind::Cargo {
            manifest: BTreeMap::from([(cid(good), qty)]),
            from: None,
        }
    }

    fn left(unit: u32, x: i64, y: i64, condition: LeftCondition) -> LeftUnit {
        LeftUnit {
            unit: ObjId(unit),
            pose: SitePose::at_cell(x, y),
            condition,
            epoch: Some(SortieId::ic(ObjId(unit), 1_000)),
            left_ic: 2_000,
        }
    }

    /// **The record round-trips strictly and keeps its order** (doc 32 Q-CF-24, Q-CF-25): units in
    /// unit order whatever the leave order, a re-leave moves the entry, objects numbered from 1 and
    /// never reused; `check` refuses an out-of-order, off-site or foreign-numbered record; an
    /// unknown field is refused on the wire.
    #[test]
    fn cf1_the_site_record_round_trips_and_keeps_its_order() {
        let mut r = record();
        assert!(r.is_blank());
        assert_eq!(r.leave_unit(left(9, 10, 10, LeftCondition::Wreck)), Ok(()));
        assert_eq!(
            r.leave_unit(left(4, 11, 10, LeftCondition::Casualty)),
            Ok(())
        );
        assert_eq!(r.leave_unit(left(9, 12, 12, LeftCondition::Wreck)), Ok(()));
        assert_eq!(
            r.units.iter().map(|u| u.unit.0).collect::<Vec<_>>(),
            vec![4, 9]
        );
        assert_eq!(r.units.get(1).map(|u| u.pose.cell()), Some((12, 12)));
        assert_eq!(
            r.drop_object(SitePose::at_cell(5, 5), 1, crate_of("metal_ore", 3), 10),
            Ok(1)
        );
        assert_eq!(
            r.drop_object(SitePose::at_cell(6, 5), 2, remains(40_000), 11),
            Ok(2)
        );
        assert_eq!(r.take_object(1).map(|o| o.id), Ok(1));
        assert_eq!(
            r.drop_object(SitePose::at_cell(7, 5), 1, crate_of("ice", 1), 12),
            Ok(3)
        );
        assert_eq!(r.check(), Ok(()));
        let wire = serde_json::to_value(&r).unwrap_or_else(|e| panic!("{e}"));
        let back: SiteRecord =
            serde_json::from_value(wire.clone()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, r);
        let mut extra = wire;
        extra["surprise"] = json!(1);
        assert!(serde_json::from_value::<SiteRecord>(extra).is_err());
        // What `check` refuses.
        let mut disordered = r.clone();
        disordered.units.reverse();
        assert!(disordered.check().is_err());
        let mut off = r.clone();
        if let Some(object) = off.objects.first_mut() {
            object.pose.x_cm = -1;
        }
        assert!(off.check().is_err());
        let mut foreign = r.clone();
        foreign.next_object = 2;
        assert!(foreign.check().is_err());
        let mut empty_crate = r.clone();
        if let Some(object) = empty_crate.objects.last_mut() {
            object.kind = GroundKind::Cargo {
                manifest: BTreeMap::new(),
                from: None,
            };
        }
        assert!(empty_crate.check().is_err());
        assert_eq!(
            r.leave_unit(left(5, 256, 0, LeftCondition::Wreck)),
            Err(SiteRefused::OffSite)
        );
        assert_eq!(r.take_unit(ObjId(77)), Err(SiteRefused::Absent));
        assert_eq!(SiteKey::parse("ahuna_port"), Some(r.key()));
        assert_eq!(SiteKey::parse("Ahuna Port"), None);
    }

    /// **Promotion opens the next field; demotion closes it and keeps the ground** (doc 32 Q-CF-14,
    /// Q-CF-20): ordinals advance and never repeat, a second promote and a bare demote refuse, the
    /// demotion's units replace the index, the objects stay, and `check` ties an open field to
    /// the last ordinal.
    #[test]
    fn cf1_promotion_opens_the_next_field_and_demotion_keeps_the_ground() {
        let mut r = record();
        let staff = PromotedBy::Staff { staff: ObjId(1) };
        assert_eq!(r.demote(5, Vec::new()), Err(SiteRefused::NotPromoted));
        let field = r
            .promote(10, staff.clone())
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(
            field,
            FieldId {
                site: cid("ahuna_port"),
                ordinal: 1
            }
        );
        assert_eq!(r.open_field(), Some(field.clone()));
        assert_eq!(r.promote(11, staff.clone()), Err(SiteRefused::Promoted));
        let _ = r.drop_object(SitePose::at_cell(1, 1), 1, crate_of("ice", 2), 12);
        let closed = r
            .demote(
                20,
                vec![
                    left(9, 3, 3, LeftCondition::Casualty),
                    left(4, 2, 2, LeftCondition::Wreck),
                ],
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(closed, field);
        assert_eq!(r.open_field(), None);
        assert_eq!(r.demoted_ic, Some(20));
        assert_eq!(
            r.units.iter().map(|u| u.unit.0).collect::<Vec<_>>(),
            vec![4, 9]
        );
        assert_eq!(r.objects.len(), 1);
        assert_eq!(r.promote(30, staff).map(|f| f.ordinal), Ok(2));
        assert_eq!(r.check(), Ok(()));
        let mut stale = r.clone();
        stale.ordinal = 3;
        assert!(stale.check().is_err(), "an open field is the last ordinal");
    }

    /// **The object cap merges the two oldest remains and never drops cargo** (doc 32 Q-CF-27):
    /// at 64 objects a new drop merges the two oldest remains at their mass-weighted centroid into
    /// the older number, the masses and merge counts summed; a cap full of cargo refuses.
    #[test]
    fn cf1_the_object_cap_merges_the_oldest_remains_and_never_drops_cargo() {
        let mut r = record();
        assert_eq!(
            r.drop_object(SitePose::at_cell(10, 10), 1, remains(1_000), 5),
            Ok(1)
        );
        assert_eq!(
            r.drop_object(SitePose::at_cell(20, 10), 3, remains(3_000), 6),
            Ok(2)
        );
        for n in 0..62_i64 {
            let at = r.drop_object(SitePose::at_cell(n, 40), 1, crate_of("ice", 1), 100 + n);
            assert!(at.is_ok(), "{at:?}");
        }
        assert_eq!(r.objects.len(), MAX_GROUND_OBJECTS);
        assert_eq!(
            r.drop_object(SitePose::at_cell(1, 50), 1, crate_of("ice", 1), 999),
            Ok(65)
        );
        assert_eq!(r.objects.len(), MAX_GROUND_OBJECTS);
        let merged = r
            .objects
            .first()
            .unwrap_or_else(|| panic!("the merged remains"));
        assert_eq!(merged.id, 1);
        assert_eq!(merged.footprint, 3);
        assert_eq!(merged.left_ic, 5);
        // (10·1000 + 20·3000) / 4000 cells → 17.5 cells' centres: x = (10_500·1 + 20_500·3) / 4.
        assert_eq!(merged.pose.x_cm, (10_500 + 20_500 * 3) / 4);
        assert_eq!(
            merged.kind,
            GroundKind::Remains {
                of: None,
                mass_kg: 4_000,
                merged: 2
            }
        );
        assert!(r.objects.iter().all(|o| o.id != 2));
        assert_eq!(r.check(), Ok(()));
        // Only one remains left: the next drop refuses rather than drop cargo.
        assert_eq!(
            r.drop_object(SitePose::at_cell(2, 50), 1, crate_of("ice", 1), 1_000),
            Err(SiteRefused::ObjectsFull)
        );
    }

    /// **The fixture's operations** (doc 32 cut CF.1): each op through `apply`, the refusals
    /// as sentences, an unknown op invalid.
    #[test]
    fn cf1_the_fixture_operations_apply() {
        let mut r = record();
        let p = |value: Value| value.as_object().cloned().unwrap_or_default();
        let said = apply(
            &mut r,
            "leave",
            &p(json!({"unit": 900, "x": 100, "y": 101, "condition": "wreck"})),
            (ObjId(1), 50),
        );
        assert_eq!(said.ok().as_deref(), Some("wreck #900 left"));
        let said = apply(
            &mut r,
            "drop",
            &p(json!({"kind": "cargo", "good": "metal_ore", "qty": 2, "x": 102, "y": 101})),
            (ObjId(1), 51),
        );
        assert_eq!(said.ok().as_deref(), Some("cargo 1 dropped"));
        assert_eq!(
            apply(&mut r, "promote", &p(json!({})), (ObjId(1), 52))
                .ok()
                .as_deref(),
            Some("field 1 opened")
        );
        assert!(apply(&mut r, "promote", &p(json!({})), (ObjId(1), 53)).is_err());
        assert_eq!(
            apply(&mut r, "demote", &p(json!({})), (ObjId(1), 54))
                .ok()
                .as_deref(),
            Some("field 1 closed")
        );
        assert_eq!(r.units.len(), 1, "a staff demote keeps the index");
        assert!(
            apply(
                &mut r,
                "drop",
                &p(json!({"kind": "cargo", "x": 1, "y": 1})),
                (ObjId(1), 55)
            )
            .is_err()
        );
        assert!(apply(&mut r, "dance", &p(json!({})), (ObjId(1), 56)).is_err());
        assert_eq!(
            apply(&mut r, "remove", &p(json!({"object": 1})), (ObjId(1), 57))
                .ok()
                .as_deref(),
            Some("cargo 1 removed")
        );
        assert_eq!(
            apply(&mut r, "take", &p(json!({"unit": 900})), (ObjId(1), 58))
                .ok()
                .as_deref(),
            Some("wreck #900 taken")
        );
        assert_eq!(r.check(), Ok(()));
        assert!(!r.is_blank(), "a field was opened here once");
        assert_eq!(
            promoted_refusal(
                &FieldId {
                    site: cid("ahuna_port"),
                    ordinal: 2
                },
                "Ahuna Flank"
            ),
            "Field 2 is being fought on Ahuna Flank. Surface work there waits until it settles."
        );
    }
}
