//! **The polities, the lane's regions and the control read** (doc 50 cut KC.0; Q-50-1 to
//! Q-50-4, Q-50-52).
//!
//! - **Six polities from content.** Each lane organization carries a `faction` tag
//!   (`freeport`, `republic`, `concord`, `syndicate`, `company`, `townships`). When the lane
//!   applies, every tag becomes a faction of the tag's name ([`polity_name`]: `republic` is
//!   "Republic"), made when missing with no backing organization. The Houses stay organizations.
//! - **The table from lane organization to polity** ([`polity_of_org`]) is read from the tag on
//!   the latched lane pack. It is not `Faction::org`, which binds one organization to one faction.
//! - **The `regions` block** ([`RegionsDef`]): one row per doc 11 §5 row the game uses, naming
//!   its places and its holders in the four kinds at world start. A holder is a polity (by tag)
//!   or a lane organization (by id), whose faction is the organization's own
//!   (`mush_faction::ensure_organization_faction`). A region is seeded once, when the apply makes
//!   it: a later apply leaves its control alone, so a clock's flip is never undone by a lane bump.
//! - **The control read** ([`control_at`], [`papers_authority`]): who holds a place, by kind,
//!   through the faction engine; and whose papers an enforcer reads there (Q-50-52).

use std::collections::{BTreeMap, BTreeSet};

use mush_content::{EntryApplier, Pack, RawEntry};
use mush_core::{CanonicalId, Ctx, ObjId, World};
use mush_faction::{ControlKind, ControlWrite, FactionId, FactionState, RegionId};
use serde::{Deserialize, Serialize};

use crate::pack::{PackEntry, RulesPack};

/// The kind word of the lane's regions block.
pub const REGIONS_KIND: &str = "regions";

/// The most rows a regions block authors.
pub const MAX_REGION_ROWS: usize = 64;

/// The most places one row names.
pub const MAX_ROW_PLACES: usize = 32;

/// The most holds one row seeds.
pub const MAX_ROW_HOLDS: usize = 16;

/// **The lane's regions block** (Q-50-3): one row per doc 11 §5 row the game uses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionsDef {
    /// The rows, each a region of the faction engine.
    pub rows: Vec<RegionRow>,
}

/// One region: its name, its places and its holders at world start.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionRow {
    /// The row's id, unique in the block.
    pub id: CanonicalId,
    /// The region's name in the faction engine and on `+territory`: unique, and a usable
    /// region name (the `@region/create` rule).
    pub name: String,
    /// The places it covers: `location` or `port` entries of this pack. A place is in one row
    /// at most.
    pub places: Vec<CanonicalId>,
    /// Its holds at world start: at most one de facto; one record a (holder, kind).
    pub holds: Vec<HoldDef>,
}

/// One authored hold.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HoldDef {
    /// The kind of hold.
    pub kind: HoldKind,
    /// Who holds it.
    pub holder: Holder,
    /// Who contests it at world start.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contested_by: Vec<Holder>,
}

/// The four kinds of control (doc 11 §5's J, F, C and E).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldKind {
    /// A recognized claim; licenses, permits and claims follow it (Q-50-4).
    DeJure,
    /// Actual control; fees and enforcement follow it (Q-50-4).
    DeFacto,
    /// Control exercised secretly.
    Covert,
    /// An economic grip.
    Economic,
}

impl HoldKind {
    /// The faction engine's kind.
    #[must_use]
    pub fn control(self) -> ControlKind {
        match self {
            HoldKind::DeJure => ControlKind::DeJure,
            HoldKind::DeFacto => ControlKind::DeFacto,
            HoldKind::Covert => ControlKind::Covert,
            HoldKind::Economic => ControlKind::Economic,
        }
    }
}

/// A holder: a polity by its tag, or a lane organization by its id.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Holder {
    /// A polity: a `faction` tag some lane organization carries.
    Polity(CanonicalId),
    /// A lane organization, by its id in a port's `orgs`.
    Org(CanonicalId),
}

/// **The polity faction's name for a tag** (Q-50-2): the tag with its first letter capitalized
/// (`republic` is "Republic", `townships` is "Townships").
#[must_use]
pub fn polity_name(tag: &str) -> String {
    let mut c = tag.chars();
    match c.next() {
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Every lane organization's polity tag, by organization id, from a pack's port entries.
fn org_tags<'a>(
    entries: impl Iterator<Item = (&'a CanonicalId, &'a PackEntry)>,
) -> BTreeMap<CanonicalId, CanonicalId> {
    let mut tags = BTreeMap::new();
    for (_, entry) in entries {
        if let PackEntry::Port(def) = entry {
            for org in &def.orgs {
                tags.insert(org.id.clone(), org.faction.clone());
            }
        }
    }
    tags
}

/// **The pack-wide rules of a regions block** (`RulesPack::from_content` runs this): rows,
/// names and places unique and bounded; every place a `location` or `port` entry; every polity
/// a tag some lane organization carries and every organization a lane organization; at most one
/// de facto hold a row; one hold a (holder, kind); no holder contests itself.
///
/// # Errors
///
/// The first broken rule, naming the row.
pub fn check_regions(
    entries: &BTreeMap<CanonicalId, PackEntry>,
    def: &RegionsDef,
) -> Result<(), String> {
    if def.rows.is_empty() || def.rows.len() > MAX_REGION_ROWS {
        return Err(format!("regions: 1 to {MAX_REGION_ROWS} rows"));
    }
    let tags = org_tags(entries.iter());
    let polities: BTreeSet<&CanonicalId> = tags.values().collect();
    let known = |h: &Holder| match h {
        Holder::Polity(tag) => polities.contains(tag),
        Holder::Org(id) => tags.contains_key(id),
    };
    let (mut ids, mut names, mut places) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for row in &def.rows {
        let at = &row.id;
        if !ids.insert(at) {
            return Err(format!("regions: row {at} is authored twice"));
        }
        let name = row.name.trim();
        let usable = !name.is_empty()
            && name.len() <= mush_faction::MAX_NAME_LEN
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '\'' | '.'));
        if !usable || name != row.name {
            return Err(format!(
                "regions: row {at}: {:?} is not a usable region name",
                row.name
            ));
        }
        if !names.insert(name.to_ascii_lowercase()) {
            return Err(format!("regions: row {at}: the name {name:?} is taken"));
        }
        if row.places.is_empty() || row.places.len() > MAX_ROW_PLACES {
            return Err(format!("regions: row {at}: 1 to {MAX_ROW_PLACES} places"));
        }
        for place in &row.places {
            if !matches!(
                entries.get(place),
                Some(PackEntry::Location(_) | PackEntry::Port(_))
            ) {
                return Err(format!("regions: row {at}: {place} is no location or port"));
            }
            if !places.insert(place) {
                return Err(format!("regions: row {at}: {place} is in another row"));
            }
        }
        if row.holds.is_empty() || row.holds.len() > MAX_ROW_HOLDS {
            return Err(format!("regions: row {at}: 1 to {MAX_ROW_HOLDS} holds"));
        }
        let mut seen = BTreeSet::new();
        for hold in &row.holds {
            if !known(&hold.holder) {
                return Err(format!(
                    "regions: row {at}: {:?} is no polity or lane org",
                    hold.holder
                ));
            }
            if !seen.insert((hold.kind, &hold.holder)) {
                return Err(format!(
                    "regions: row {at}: {:?} holds one kind twice",
                    hold.holder
                ));
            }
            for c in &hold.contested_by {
                if !known(c) || c == &hold.holder {
                    return Err(format!("regions: row {at}: a contest names {c:?}"));
                }
            }
        }
        if row
            .holds
            .iter()
            .filter(|h| h.kind == HoldKind::DeFacto)
            .count()
            > 1
        {
            return Err(format!(
                "regions: row {at}: one de facto holder at most (Q-50-50)"
            ));
        }
    }
    Ok(())
}

// ----------------------------------------------------------------------
// The applier
// ----------------------------------------------------------------------

/// **The regions block's applier** (phase 12, after the ports, which make the organizations an
/// org holder names). On every apply: the six polities, then each row's region, seeded with its
/// world-start holds only when this apply makes it.
pub struct RegionsApplier;

/// The faction an authored holder is, made when missing: a polity's by name, an organization's
/// through the faction engine's organization faction.
fn holder_faction(
    ctx: &mut Ctx<'_>,
    pack: &CanonicalId,
    holder: &Holder,
) -> Result<FactionId, String> {
    match holder {
        Holder::Polity(tag) => mush_faction::ensure_faction(ctx, &polity_name(tag.as_str())),
        Holder::Org(id) => {
            let org = ctx
                .w
                .module_state::<crate::store::TacsimStore>()
                .and_then(|store| {
                    store
                        .port_maps()
                        .filter(|map| &map.pack == pack)
                        .find_map(|map| map.orgs.get(id).map(|row| row.org))
                })
                .ok_or_else(|| format!("regions: the lane made no org {id}"))?;
            mush_faction::ensure_organization_faction(ctx, mush_org::OrgId(org))
        }
    }
}

impl RegionsApplier {
    fn put(
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
    ) -> Result<(), String> {
        let def: RegionsDef = entry.decode().map_err(|e| format!("{id}: {e}"))?;
        let pack_id = pack.manifest().id().clone();
        // Q-50-2: every lane organization's tag is a polity, made when missing.
        let rules = crate::pack::rules_pack_of(pack.clone()).map_err(|e| e.to_string())?;
        let tags: BTreeSet<CanonicalId> = org_tags(rules.entries().iter()).into_values().collect();
        for tag in &tags {
            mush_faction::ensure_faction(ctx, &polity_name(tag.as_str()))?;
        }
        for row in &def.rows {
            let (region, made) = mush_faction::ensure_region(ctx, &row.name)?;
            if !made {
                continue;
            }
            for hold in &row.holds {
                let faction = holder_faction(ctx, &pack_id, &hold.holder)?;
                let kind = hold.kind.control();
                mush_faction::write_control(
                    ctx,
                    ControlWrite {
                        strength: Some(3),
                        ..ControlWrite::new(region, faction, kind)
                    },
                )?;
                for c in &hold.contested_by {
                    let challenger = holder_faction(ctx, &pack_id, c)?;
                    mush_faction::contest_control(ctx, region, faction, kind, challenger)?;
                }
            }
        }
        seed_customs_front(ctx, &rules, &def)?;
        ctx.out.pin_deferred();
        Ok(())
    }
}

/// **Customs Assertion on the Republic's board, at world start** (doc 50 cut KC.1; Q-50-12,
/// Q-50-13): its flip is the region that holds the picket's place (the `location` with
/// `kirkwood_picket`). A lane with no such row, or no Republic, seeds nothing. Idempotent.
fn seed_customs_front(
    ctx: &mut Ctx<'_>,
    rules: &RulesPack,
    def: &RegionsDef,
) -> Result<(), String> {
    let Some(row) = def.rows.iter().find(|row| {
        row.places.iter().any(|place| {
            matches!(rules.entry(place), Some(PackEntry::Location(loc)) if loc.kirkwood_picket)
        })
    }) else {
        return Ok(());
    };
    let (Some(republic), Some(approach)) = (
        polity(ctx.w, "republic"),
        ctx.w
            .module_state::<FactionState>()
            .and_then(|st| st.region_by_name(&row.name))
            .map(|r| r.id),
    ) else {
        return Ok(());
    };
    let draft = mush_canon::CanonDraft {
        kind: "space.customs_front".into(),
        summary: format!(
            "{}: the Republic presses its claim to the {}.",
            mush_faction::CUSTOMS_ASSERTION_TITLE,
            row.name
        ),
        entities: Vec::new(),
        scope: mush_canon::CanonScope::Public,
        ic_time: ctx.w.world_ic_now().at,
        source: "tacsim.customs.assertion".into(),
        detail: serde_json::json!({ "region": approach.0 }),
    };
    mush_faction::seed_customs_assertion_front(ctx, republic, approach, draft).map(|_| ())
}

/// **The board Kirkwood picket incidents advance** (doc 50 cut KC.1, Q-50-13): the Republic's.
#[must_use]
pub fn customs_board(w: &World) -> Option<FactionId> {
    polity(w, "republic")
}

impl EntryApplier for RegionsApplier {
    fn kinds(&self) -> Vec<CanonicalId> {
        CanonicalId::new(REGIONS_KIND).into_iter().collect()
    }

    fn phase(&self) -> u8 {
        // After the ports (11): an org holder's organization is the port's.
        12
    }

    fn check(
        &self,
        _ctx: &Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        _entry: &RawEntry,
    ) -> Result<(), String> {
        let rules = crate::pack::rules_pack_of(pack.clone()).map_err(|e| e.to_string())?;
        match rules.entry(id) {
            Some(PackEntry::Regions(_)) => Ok(()),
            _ => Err(format!("{id}: not a regions entry")),
        }
    }

    fn create(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        _placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<Option<ObjId>, String> {
        RegionsApplier::put(ctx, pack, id, entry)?;
        Ok(None)
    }

    fn update(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        _existing: Option<ObjId>,
        _placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<Option<ObjId>, String> {
        // In place: a region that stands keeps its control; a new row is seeded.
        RegionsApplier::put(ctx, pack, id, entry)?;
        Ok(None)
    }

    fn retire_check(
        &self,
        _ctx: &Ctx<'_>,
        _pack: &CanonicalId,
        _id: &CanonicalId,
        _existing: Option<ObjId>,
        _placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<(), String> {
        Ok(())
    }

    fn retire(
        &self,
        _ctx: &mut Ctx<'_>,
        pack: &CanonicalId,
        id: &CanonicalId,
        _existing: Option<ObjId>,
    ) -> Result<(), String> {
        tracing::info!(
            %pack, %id,
            "tacsim: the regions block retired; the regions, their control and the polities stand"
        );
        Ok(())
    }
}

// ----------------------------------------------------------------------
// The reads
// ----------------------------------------------------------------------

/// The latched lane pack (the readiness report's rule: the applied version, else the shipped).
fn lane(w: &World) -> Option<&RulesPack> {
    let store = w.module_state::<crate::store::TacsimStore>()?;
    crate::port::readiness::report_ports(store, w).0
}

/// The lane's regions block, if it has one.
fn regions_of(pack: &RulesPack) -> Option<&RegionsDef> {
    pack.entries().values().find_map(|e| match e {
        PackEntry::Regions(def) => Some(def),
        _ => None,
    })
}

/// **A polity's faction, by tag** (Q-50-2), once the lane has made it.
#[must_use]
pub fn polity(w: &World, tag: &str) -> Option<FactionId> {
    w.module_state::<FactionState>()?
        .faction_by_name(&polity_name(tag))
        .map(|f| f.id)
}

/// **The polity a lane organization answers to** (Q-50-2's table): the `faction` tag on its
/// row of the latched lane, as that polity's faction. `None` for an organization the lane does
/// not author, or before the polity is made.
#[must_use]
pub fn polity_of_org(w: &World, org: mush_org::OrgId) -> Option<FactionId> {
    let store = w.module_state::<crate::store::TacsimStore>()?;
    let pack = lane(w)?;
    let id = store
        .port_maps()
        .filter(|map| map.pack.as_str() == crate::definitions::content::LANE_PACK)
        .find_map(|map| {
            map.orgs
                .iter()
                .find(|(_, row)| row.org == org.0)
                .map(|(id, _)| id.clone())
        })?;
    let tags = org_tags(pack.entries().iter());
    polity(w, tags.get(&id)?.as_str())
}

/// **Who holds a place, by kind** (Q-50-1, Q-50-4), read from the faction engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaceControl {
    /// The region the place lies in.
    pub region: RegionId,
    /// The de jure claimants, in record order (Q-50-52: may be several).
    pub de_jure: Vec<FactionId>,
    /// The de facto holder: one at most (Q-50-50).
    pub de_facto: Option<FactionId>,
    /// The covert holders.
    pub covert: Vec<FactionId>,
    /// The economic holders.
    pub economic: Vec<FactionId>,
    /// The IC date the de facto hold began (Q-50-9), when there is one.
    pub de_facto_since_ic: Option<i64>,
}

/// **The control read** (Q-50-1): the region a lane place (a `location` or `port` entry id)
/// lies in, by the latched lane's regions block, and who holds it in each kind. This is truth,
/// with no visibility filter: compiled code that sets terms reads it. `None` for a place in no
/// row, or before its region is made.
#[must_use]
pub fn control_at(w: &World, place: &CanonicalId) -> Option<PlaceControl> {
    let row = regions_of(lane(w)?)?
        .rows
        .iter()
        .find(|r| r.places.contains(place))?;
    let st = w.module_state::<FactionState>()?;
    let region = st.region_by_name(&row.name)?.id;
    let recs = st
        .control
        .get(&region)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let of = |kind: ControlKind| -> Vec<FactionId> {
        recs.iter()
            .filter(|r| r.kind == kind)
            .map(|r| r.faction)
            .collect()
    };
    let de_facto = recs.iter().find(|r| r.kind == ControlKind::DeFacto);
    Some(PlaceControl {
        region,
        de_jure: of(ControlKind::DeJure),
        de_facto: de_facto.map(|r| r.faction),
        covert: of(ControlKind::Covert),
        economic: of(ControlKind::Economic),
        de_facto_since_ic: de_facto.map(|r| r.since_ic),
    })
}

/// **Whose papers an enforcer reads at a place** (Q-50-52): its own polity's where that
/// polity is a de jure claimant, otherwise the de jure holder's (the first claimant on record,
/// where several claim and the enforcer is none of them). `None` where nobody claims the place.
#[must_use]
pub fn papers_authority(w: &World, place: &CanonicalId, enforcer: FactionId) -> Option<FactionId> {
    let control = control_at(w, place)?;
    if control.de_jure.contains(&enforcer) {
        return Some(enforcer);
    }
    control.de_jure.first().copied()
}
