//! # The unit view (interface plan I8) — one projection, three readers
//!
//! [`UnitView`] is the ONE read of a [`TacticalUnit`] the outside world sees:
//! the `tactics.unit` panel keyframe, `@unit/show`'s text and the
//! `tactics.vitals` frame are all rendered from it, so the three can never
//! disagree about which sections, cells, components and capabilities a unit
//! has (10b §4 rule 4; the plan's D-I-9). It is built from the accessors
//! only — never by serializing the unit whole — so the private `rules`
//! value and every capability's rules body have no path onto the wire, and a
//! component's economic asset appears as a presence bit, never as another
//! object's id (D-I8-3).
//!
//! Authority is [`unit_for_view`]: a viewer stationed on the unit (K5a's
//! grant under the `sim` owner, read through
//! [`status::viewer_is_stationed`](crate::status::viewer_is_stationed)), a
//! controller, or staff. Every NAMED failure — no such object, not a unit, no
//! station, a malformed id — is ONE refusal, byte-identical to
//! `@unit/status`'s line, so a query cannot probe existence. The UNNAMED case
//! (no `object` param, no unit of one's own, no station) answers a `null`
//! keyframe: both feeds sit in every hello manifest and `.vitals` is
//! auto-subscribed by both clients, so a refusal there would print in every
//! player's scrollback on every connect.
//!
//! E7k lane L3 (the typed `tactics.unit` family) rewrites exactly one
//! function here — [`unit_source`] — and overrides `query_with_context` on the
//! two handlers so they read owner-scoped handles.

use std::collections::BTreeMap;

use mush_core::output::obj_ref;
use mush_core::{MechanicalState, ObjId, QueryHandler, World, perm};
use mush_proto::{
    MAX_VITAL_NAME_BYTES, ObjRef, OutputEvent, PanelFrameKind, RichText, Vital, VitalState,
    VitalsFrame,
};
use serde_json::{Value, json};
use smol_str::SmolStr;

use crate::module::{UnitStore, is_admin};
use crate::status::{UnitStatus, primary_stationed_unit, viewer_is_stationed};
use crate::unit::{CellAddr, InstallPoint, Journal, JournalEntry, TacticalUnit};

/// The unit-pool vitals feed (`.vitals` suffix convention, interface plan I1).
pub const VITALS_FEED: &str = "tactics.vitals";

/// The soft byte budget of one `tactics.unit` keyframe: past it the walk stops
/// adding components, non-unit cells and capabilities and marks the frame
/// `truncated`. The hard bound stays `MAX_UNIT_RECORD_BYTES` (1 MiB) — the
/// TUI drops any panel frame over that cliff, so a budget with headroom is
/// what keeps a large unit readable rather than silently dark.
pub const MAX_UNIT_VIEW_BYTES: usize = 256 * 1024;

/// The most stationed viewers an after-action delta fans out to (D-I8-2);
/// mirrors mush-sim's seats-per-unit cap by value, not by import.
pub const MAX_UNIT_AUDIENCE: usize = 64;

/// The one refusal for a NAMED subject the viewer may not read — or that does
/// not exist, or is not a unit, or was mis-spelled. Its text is `@unit/status`'s
/// line for the same cases, so the two surfaces cannot be played against each
/// other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewRefusal;

impl ViewRefusal {
    /// The refusal line.
    #[must_use]
    pub fn render(self) -> String {
        UnitStatus::unknown().render()
    }

    fn rich(self) -> RichText {
        RichText::tag("error", self.render())
    }
}

/// Where a unit's state comes from: the active mechanics index, or a held
/// (frozen / migrating / quarantined) record whose mechanics are disabled.
pub enum UnitSource<'w> {
    /// A live unit.
    Active(&'w TacticalUnit),
    /// A record boot discovered but would not activate.
    Held {
        /// The raw persistence key.
        key: &'w SmolStr,
        /// Why it is held.
        state: &'w MechanicalState,
        /// The decoded unit, when the record decoded far enough.
        unit: Option<&'w TacticalUnit>,
    },
}

/// The ONE `UnitStore` read of this file — E7k lane L3's edit point.
#[must_use]
pub fn unit_source(w: &World, object: ObjId) -> Option<UnitSource<'_>> {
    let store = w.module_state::<UnitStore>()?;
    if let Some(unit) = store.units.get(&object) {
        return Some(UnitSource::Active(unit));
    }
    store
        .records
        .iter()
        .find(|record| record.claims_object(object))
        .map(|record| UnitSource::Held {
            key: &record.key,
            state: &record.state,
            unit: record.unit.as_ref(),
        })
}

/// Stationed on it, controls it, or staff. `perm::controls` fails closed on
/// a missing object, so a stranger naming a dbref that does not exist takes
/// the same path as one naming a unit they hold no arm on.
fn authorized(w: &World, viewer: ObjId, unit: ObjId) -> bool {
    viewer_is_stationed(w, viewer, unit) || perm::controls(w, viewer, unit) || is_admin(w, viewer)
}

/// The accessor every reader goes through: authority FIRST, then existence,
/// so the two failures are indistinguishable.
///
/// # Errors
///
/// [`ViewRefusal`] — one value, one line — for an unauthorized viewer, a
/// nonexistent object, or an object that is not a unit.
pub fn unit_for_view(
    w: &World,
    viewer: ObjId,
    object: ObjId,
) -> Result<UnitSource<'_>, ViewRefusal> {
    if !authorized(w, viewer, object) {
        return Err(ViewRefusal);
    }
    unit_source(w, object).ok_or(ViewRefusal)
}

/// Which object a query is about. A named `object` is parsed as a dbref only
/// (never `match_object`, which would leak name visibility); absent, the
/// viewer's own object when it carries a unit record, else the viewer's
/// highest-priority `sim` station on an active unit, else `None`.
///
/// # Errors
///
/// [`ViewRefusal`] when `object` is present but not a dbref.
pub fn resolve_subject(
    w: &World,
    viewer: ObjId,
    params: &Value,
) -> Result<Option<ObjId>, ViewRefusal> {
    match params.get("object") {
        None | Some(Value::Null) => {
            if unit_source(w, viewer).is_some() {
                return Ok(Some(viewer));
            }
            Ok(primary_stationed_unit(w, viewer))
        }
        Some(named) => {
            let raw = named.as_str().ok_or(ViewRefusal)?;
            let id = raw
                .trim()
                .trim_start_matches('#')
                .parse::<u32>()
                .map_err(|_| ViewRefusal)?;
            Ok(Some(ObjId(id)))
        }
    }
}

fn state_word(state: &MechanicalState) -> &'static str {
    match state {
        MechanicalState::Active => "active",
        MechanicalState::Frozen { .. } => "frozen",
        MechanicalState::Migrating { .. } => "migrating",
        MechanicalState::Quarantined { .. } => "quarantined",
    }
}

fn vital_word(state: VitalState) -> &'static str {
    match state {
        VitalState::Ok => "ok",
        VitalState::Warn => "warn",
        VitalState::Crit => "crit",
        VitalState::Down => "down",
    }
}

fn rank(state: VitalState) -> u8 {
    match state {
        VitalState::Ok => 0,
        VitalState::Warn => 1,
        VitalState::Crit => 2,
        VitalState::Down => 3,
    }
}

fn worst(states: impl Iterator<Item = VitalState>) -> VitalState {
    states.fold(
        VitalState::Ok,
        |acc, s| if rank(s) > rank(acc) { s } else { acc },
    )
}

/// The scope a cell address lives in — the closed location kind of
/// [`CellAddr`], flattened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    Unit,
    Section,
    Facet,
    Component,
}

/// One cell as projected: a flat address, the id as its label until the
/// rules pack declares metadata, and the server-judged state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellView {
    /// `unit/<cell>` · `section/<s>/<cell>` · `facet/<s>/<f>/<cell>` ·
    /// `component/<c>/<cell>`.
    pub addr: String,
    /// The cell id.
    pub label: String,
    /// Current value.
    pub current: i64,
    /// Inclusive lower bound.
    pub min: i64,
    /// Inclusive upper bound.
    pub max: i64,
    /// The pool ladder's verdict (`mush_ruleset::pool_state`).
    pub state: VitalState,
    scope: Scope,
    section: Option<String>,
    component: Option<String>,
}

/// A facet and the cell ids it exposes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FacetView {
    /// The facet id.
    pub id: String,
    /// Its cell ids.
    pub cells: Vec<String>,
}

/// One section as projected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionView {
    /// The section id.
    pub id: String,
    /// The ruleset's opaque condition word, verbatim.
    pub condition: String,
    /// The worst state among its own and its facets' cells (`ok` with none).
    pub state: VitalState,
    /// Facets with their cell ids.
    pub facets: Vec<FacetView>,
    /// Install slot ids.
    pub slots: Vec<String>,
}

/// One installed component as projected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentView {
    /// The component id.
    pub id: String,
    /// The definition's identity (`owner:kind:id@version#digest`), never a body.
    pub definition: String,
    /// `unit` · `section/<s>` · `slot/<s>/<slot>`.
    pub install: String,
    /// The ruleset's opaque condition word, verbatim.
    pub condition: String,
    /// The worst state among its cells (`ok` with none).
    pub state: VitalState,
    /// Whether an economic asset is attached — never which one.
    pub asset: bool,
}

/// One capability key as projected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityView {
    /// The capability interface id.
    pub id: String,
    /// The providing component id, or `None` for an intrinsic capability.
    pub source: Option<String>,
}

/// An in-progress mode switch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwitchView {
    /// The mode being left.
    pub from: String,
    /// The mode being entered.
    pub to: String,
    /// Unix seconds when the switch settles.
    pub ready_at: i64,
}

/// One declared mobility mode, minus its rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModeView {
    /// The mode id.
    pub id: String,
    /// How it locomotes.
    pub locomotion: String,
    /// The media it traverses.
    pub mediums: Vec<String>,
    /// Capability tags.
    pub tags: Vec<String>,
    /// Nominal speed, cm/s (`0` = unspecified).
    pub nominal_speed_cms: u32,
    /// Seconds to switch into it (`0` = instant).
    pub transition_s: u32,
}

/// The mobility profile as projected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MobilityView {
    /// The mode in effect now, when settled or a due switch.
    pub mode: Option<String>,
    /// The switch in progress, when any.
    pub switching: Option<SwitchView>,
    /// Every declared mode.
    pub modes: Vec<ModeView>,
}

/// The unit, as every reader sees it. Built from the accessors only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitView {
    /// The unit's object.
    pub object: ObjRef,
    /// The schema's identity (`""` for a held record that did not decode).
    pub schema: String,
    /// The unit revision (`0` for a held record that did not decode).
    pub revision: u64,
    /// `active` · `frozen` · `migrating` · `quarantined`.
    pub state: &'static str,
    /// The record key of a held record; `None` when active.
    pub record: Option<String>,
    /// The mobility profile, when the unit has one.
    pub mobility: Option<MobilityView>,
    /// Sections, in id order.
    pub sections: Vec<SectionView>,
    /// Cells, in address order (unit, section, facet, component scope).
    pub cells: Vec<CellView>,
    /// Installed components, in id order.
    pub components: Vec<ComponentView>,
    /// Capability keys, in key order.
    pub capabilities: Vec<CapabilityView>,
    held_unit: bool,
}

impl UnitView {
    /// The view of `object` from `source`, at `now` (unix seconds).
    #[must_use]
    pub fn of(w: &World, object: ObjId, source: &UnitSource<'_>, now: i64) -> UnitView {
        let object = obj_ref(w, object);
        match source {
            UnitSource::Active(unit) => UnitView::from_unit(object, unit, now),
            UnitSource::Held { key, state, unit } => {
                let mut view = UnitView {
                    object,
                    schema: String::new(),
                    revision: 0,
                    state: state_word(state),
                    record: Some(key.to_string()),
                    mobility: None,
                    sections: Vec::new(),
                    cells: Vec::new(),
                    components: Vec::new(),
                    capabilities: Vec::new(),
                    held_unit: false,
                };
                if let Some(unit) = unit {
                    view.schema = unit.schema().to_string();
                    view.revision = unit.revision();
                    view.held_unit = true;
                }
                view
            }
        }
    }

    fn from_unit(object: ObjRef, unit: &TacticalUnit, now: i64) -> UnitView {
        let mut facet_cells: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
        let mut cells = Vec::new();
        for (addr, cell) in unit.cells() {
            let (scope, section, component, label) = match addr {
                CellAddr::Unit { cell } => (Scope::Unit, None, None, cell.to_string()),
                CellAddr::Section { section, cell } => (
                    Scope::Section,
                    Some(section.to_string()),
                    None,
                    cell.to_string(),
                ),
                CellAddr::Facet {
                    section,
                    facet,
                    cell,
                } => {
                    facet_cells
                        .entry((section.to_string(), facet.to_string()))
                        .or_default()
                        .push(cell.to_string());
                    (
                        Scope::Facet,
                        Some(section.to_string()),
                        None,
                        cell.to_string(),
                    )
                }
                CellAddr::Component { component, cell } => (
                    Scope::Component,
                    None,
                    Some(component.to_string()),
                    cell.to_string(),
                ),
            };
            cells.push(CellView {
                addr: cell_addr_path(addr),
                label,
                current: cell.current,
                min: cell.min,
                max: cell.max,
                state: mush_ruleset::pool_state(cell.current, cell.min, cell.max),
                scope,
                section,
                component,
            });
        }
        let sections = unit
            .sections()
            .map(|(id, s)| {
                let id = id.to_string();
                SectionView {
                    state: worst(
                        cells
                            .iter()
                            .filter(|c| c.section.as_deref() == Some(id.as_str()))
                            .map(|c| c.state),
                    ),
                    condition: s.condition.to_string(),
                    facets: s
                        .facets
                        .iter()
                        .map(|f| FacetView {
                            id: f.to_string(),
                            cells: facet_cells
                                .get(&(id.clone(), f.to_string()))
                                .cloned()
                                .unwrap_or_default(),
                        })
                        .collect(),
                    slots: s.slots.iter().map(ToString::to_string).collect(),
                    id,
                }
            })
            .collect();
        let components = unit
            .components()
            .map(|(id, c)| {
                let id = id.to_string();
                ComponentView {
                    state: worst(
                        cells
                            .iter()
                            .filter(|cell| cell.component.as_deref() == Some(id.as_str()))
                            .map(|cell| cell.state),
                    ),
                    definition: c.definition.to_string(),
                    install: install_path(&c.install),
                    condition: c.condition.to_string(),
                    asset: c.asset.is_some(),
                    id,
                }
            })
            .collect();
        let capabilities = unit
            .capabilities()
            .map(|k| CapabilityView {
                id: k.id.to_string(),
                source: k.source.as_ref().map(ToString::to_string),
            })
            .collect();
        let mobility = unit.mobility().map(|m| MobilityView {
            mode: m.effective_mode(now).map(ToString::to_string),
            switching: m.switch_parts().map(|(from, to, ready_at)| SwitchView {
                from: from.to_string(),
                to: to.to_string(),
                ready_at,
            }),
            modes: m
                .modes()
                .iter()
                .map(|mode| ModeView {
                    id: mode.id.to_string(),
                    locomotion: mode.locomotion.to_string(),
                    mediums: mode.mediums.iter().map(ToString::to_string).collect(),
                    tags: mode.tags.iter().map(ToString::to_string).collect(),
                    nominal_speed_cms: mode.nominal_speed_cms,
                    transition_s: mode.transition_s,
                })
                .collect(),
        });
        UnitView {
            object,
            schema: unit.schema().to_string(),
            revision: unit.revision(),
            state: "active",
            record: None,
            mobility,
            sections,
            cells,
            components,
            capabilities,
            held_unit: true,
        }
    }

    /// The `tactics.unit` keyframe payload, inside [`MAX_UNIT_VIEW_BYTES`]:
    /// the header, every section and every unit-level cell always; then
    /// components, the remaining cells and the capabilities until the budget
    /// is spent, with `truncated` set and `counts` still the true totals.
    #[must_use]
    pub fn to_frame(&self) -> Value {
        let mut budget = Budget {
            used: 0,
            limit: MAX_UNIT_VIEW_BYTES,
        };
        let header = json!({
            "object": self.object,
            "schema": self.schema,
            "revision": self.revision,
            "state": self.state,
            "record": self.record,
            "mobility": self.mobility.as_ref().map(mobility_json),
            "counts": {
                "sections": self.sections.len(),
                "cells": self.cells.len(),
                "components": self.components.len(),
                "capabilities": self.capabilities.len(),
            },
            "truncated": false,
        });
        budget.charge(&header);
        let sections: Vec<Value> = self.sections.iter().map(section_json).collect();
        for s in &sections {
            budget.charge(s);
        }
        let mut cells = Vec::new();
        for c in self.cells.iter().filter(|c| c.scope == Scope::Unit) {
            let v = cell_json(c);
            budget.charge(&v);
            cells.push(v);
        }
        let mut truncated = false;
        let mut components = Vec::new();
        for c in &self.components {
            let v = component_json(c);
            if !budget.admit(&v) {
                truncated = true;
                break;
            }
            components.push(v);
        }
        for c in self.cells.iter().filter(|c| c.scope != Scope::Unit) {
            let v = cell_json(c);
            if !budget.admit(&v) {
                truncated = true;
                break;
            }
            cells.push(v);
        }
        let mut capabilities = Vec::new();
        for k in &self.capabilities {
            let v = json!({ "id": k.id, "source": k.source });
            if !budget.admit(&v) {
                truncated = true;
                break;
            }
            capabilities.push(v);
        }
        let Value::Object(mut frame) = header else {
            return Value::Null;
        };
        frame.insert("sections".into(), Value::Array(sections));
        frame.insert("cells".into(), Value::Array(cells));
        frame.insert("components".into(), Value::Array(components));
        frame.insert("capabilities".into(), Value::Array(capabilities));
        frame.insert("truncated".into(), Value::Bool(truncated));
        Value::Object(frame)
    }

    /// `@unit/show`'s lines, from the same view. `heading` is the object's
    /// display name as the command shows it.
    #[must_use]
    pub fn render_lines(&self, heading: &str) -> Vec<String> {
        if let Some(record) = &self.record {
            let mut lines = vec![format!(
                "{heading} — tactical record {record} is {}; mechanics are disabled",
                self.state
            )];
            if self.held_unit {
                lines.push(format!(
                    "  schema {}; revision {}",
                    self.schema, self.revision
                ));
            }
            return lines;
        }
        let mobility = match &self.mobility {
            Some(MobilityView {
                mode: Some(mode), ..
            }) => mode.clone(),
            Some(MobilityView {
                switching: Some(sw),
                ..
            }) => format!("{} -> {} (ready {})", sw.from, sw.to, sw.ready_at),
            _ => "none".to_owned(),
        };
        let mut lines = vec![format!(
            "{heading} — schema {}, revision {}, mobility {mobility}",
            self.schema, self.revision
        )];
        for s in &self.sections {
            lines.push(format!("  section {}: {}", s.id, s.condition));
        }
        for c in &self.cells {
            lines.push(format!("  cell {}: {}/{}", c.addr, c.current, c.max));
        }
        for c in &self.components {
            lines.push(format!(
                "  component {}: {} ({})",
                c.id, c.condition, c.definition
            ));
        }
        for k in &self.capabilities {
            lines.push(format!(
                "  capability {} (source {})",
                k.id,
                k.source.as_deref().unwrap_or("none")
            ));
        }
        lines
    }

    /// The unit-level pools as `Pool` vitals, in cell order. A cell whose
    /// prefixed id or label would exceed the wire's name cap is skipped (one
    /// authored id costs one gauge, never the feed).
    #[must_use]
    pub fn pool_vitals(&self) -> Vec<Vital> {
        self.cells
            .iter()
            .filter(|c| c.scope == Scope::Unit)
            .filter_map(|c| {
                let id = format!("tactics.{}", c.label);
                if id.len() > MAX_VITAL_NAME_BYTES || c.label.len() > MAX_VITAL_NAME_BYTES {
                    tracing::warn!(cell = %c.label, "tactics.vitals: cell id over the vital name cap; skipped");
                    return None;
                }
                Some(mush_ruleset::pool(
                    SmolStr::new(id),
                    c.label.clone(),
                    c.current,
                    c.min,
                    c.max,
                ))
            })
            .collect()
    }

    /// The `tactics.vitals` frame: `revision` is the unit revision — the same
    /// number `@unit/show` prints and the delta carries.
    #[must_use]
    pub fn vitals_frame(&self) -> Option<VitalsFrame> {
        mush_ruleset::vitals_frame(self.object.clone(), self.revision, self.pool_vitals())
    }

    /// [`Self::vitals_frame`] as a panel payload (`null` when it refused).
    #[must_use]
    pub fn vitals_value(&self) -> Value {
        self.vitals_frame()
            .and_then(|f| serde_json::to_value(f).ok())
            .unwrap_or(Value::Null)
    }
}

struct Budget {
    used: usize,
    limit: usize,
}

impl Budget {
    fn size(v: &Value) -> usize {
        serde_json::to_string(v)
            .map_or(0, |s| s.len())
            .saturating_add(1)
    }

    fn charge(&mut self, v: &Value) {
        self.used = self.used.saturating_add(Self::size(v));
    }

    fn admit(&mut self, v: &Value) -> bool {
        let n = Self::size(v);
        if self.used.saturating_add(n) > self.limit {
            return false;
        }
        self.used += n;
        true
    }
}

fn cell_json(c: &CellView) -> Value {
    json!({
        "addr": c.addr,
        "label": c.label,
        "current": c.current,
        "min": c.min,
        "max": c.max,
        "state": vital_word(c.state),
    })
}

fn section_json(s: &SectionView) -> Value {
    json!({
        "id": s.id,
        "condition": s.condition,
        "state": vital_word(s.state),
        "facets": s.facets.iter().map(|f| json!({ "id": f.id, "cells": f.cells })).collect::<Vec<_>>(),
        "slots": s.slots,
    })
}

fn component_json(c: &ComponentView) -> Value {
    json!({
        "id": c.id,
        "definition": c.definition,
        "install": c.install,
        "condition": c.condition,
        "state": vital_word(c.state),
        "asset": c.asset,
    })
}

fn mobility_json(m: &MobilityView) -> Value {
    json!({
        "mode": m.mode,
        "switching": m.switching.as_ref().map(|sw| json!({ "from": sw.from, "to": sw.to, "ready_at": sw.ready_at })),
        "modes": m.modes.iter().map(|mode| json!({
            "id": mode.id,
            "locomotion": mode.locomotion,
            "mediums": mode.mediums,
            "tags": mode.tags,
            "nominal_speed_cms": mode.nominal_speed_cms,
            "transition_s": mode.transition_s,
        })).collect::<Vec<_>>(),
    })
}

/// The flat path of a cell address.
#[must_use]
pub fn cell_addr_path(addr: &CellAddr) -> String {
    match addr {
        CellAddr::Unit { cell } => format!("unit/{cell}"),
        CellAddr::Section { section, cell } => format!("section/{section}/{cell}"),
        CellAddr::Facet {
            section,
            facet,
            cell,
        } => format!("facet/{section}/{facet}/{cell}"),
        CellAddr::Component { component, cell } => format!("component/{component}/{cell}"),
    }
}

/// The flat path of an install point.
#[must_use]
pub fn install_path(install: &InstallPoint) -> String {
    match install {
        InstallPoint::Unit => "unit".to_owned(),
        InstallPoint::Section(section) => format!("section/{section}"),
        InstallPoint::Slot { section, slot } => format!("slot/{section}/{slot}"),
    }
}

/// The `tactics.unit` delta payload for an applied plan: the journal with
/// every entry tagged by `kind`, flat addresses, and the asset pointer
/// reduced to `attached` (D-I8-3). `to_revision` stays at the root.
#[must_use]
pub fn journal_frame(w: &World, journal: &Journal) -> Value {
    json!({
        "object": obj_ref(w, journal.object),
        "from_revision": journal.from_revision,
        "to_revision": journal.to_revision,
        "now": journal.now,
        "cause": journal.cause.as_deref(),
        "entries": journal.entries.iter().map(entry_json).collect::<Vec<_>>(),
    })
}

fn entry_json(entry: &JournalEntry) -> Value {
    match entry {
        JournalEntry::Cell { addr, from, to } => json!({
            "kind": "cell", "addr": cell_addr_path(addr), "from": from, "to": to,
        }),
        JournalEntry::SectionCondition { section, from, to } => json!({
            "kind": "section", "section": section.to_string(),
            "from": from.to_string(), "to": to.to_string(),
        }),
        JournalEntry::ComponentCondition {
            component,
            from,
            to,
        } => json!({
            "kind": "component", "component": component.to_string(),
            "from": from.to_string(), "to": to.to_string(),
        }),
        JournalEntry::ComponentAsset { component, to, .. } => json!({
            "kind": "asset", "component": component.to_string(), "attached": to.is_some(),
        }),
        JournalEntry::CapabilityGranted { key, replaced } => json!({
            "kind": "capability_granted", "id": key.id.to_string(),
            "source": key.source.as_ref().map(ToString::to_string), "replaced": replaced,
        }),
        JournalEntry::CapabilityRevoked { key, was_present } => json!({
            "kind": "capability_revoked", "id": key.id.to_string(),
            "source": key.source.as_ref().map(ToString::to_string), "was_present": was_present,
        }),
    }
}

/// Resolve the subject and build its payload: a named failure is the one
/// refusal; the unnamed no-subject case is `null`.
fn subject_frame(
    w: &World,
    actor: ObjId,
    params: &Value,
    build: impl Fn(&UnitView) -> Value,
) -> Result<Value, RichText> {
    let named = params.get("object").is_some_and(|v| !v.is_null());
    let subject = resolve_subject(w, actor, params).map_err(ViewRefusal::rich)?;
    let Some(object) = subject else {
        return Ok(Value::Null);
    };
    match unit_for_view(w, actor, object) {
        Ok(source) => Ok(build(&UnitView::of(w, object, &source, w.now()))),
        Err(refusal) if named => Err(refusal.rich()),
        Err(_) => Ok(Value::Null),
    }
}

/// The `tactics.unit` query: a keyframe of the subject's view.
pub struct UnitQuery {
    feed: SmolStr,
}

impl UnitQuery {
    /// A handler answering on `feed`.
    #[must_use]
    pub fn new(feed: &str) -> UnitQuery {
        UnitQuery {
            feed: SmolStr::new(feed),
        }
    }
}

impl QueryHandler for UnitQuery {
    fn query(&self, w: &World, actor: ObjId, params: &Value) -> Result<OutputEvent, RichText> {
        let data = subject_frame(w, actor, params, UnitView::to_frame)?;
        Ok(OutputEvent::Panel {
            feed: self.feed.clone(),
            kind: PanelFrameKind::Keyframe,
            data,
        })
    }
}

/// The `tactics.vitals` query: the subject's unit-level pools as a frame.
pub struct UnitVitalsQuery {
    feed: SmolStr,
}

impl UnitVitalsQuery {
    /// A handler answering on `feed`.
    #[must_use]
    pub fn new(feed: &str) -> UnitVitalsQuery {
        UnitVitalsQuery {
            feed: SmolStr::new(feed),
        }
    }
}

impl QueryHandler for UnitVitalsQuery {
    fn query(&self, w: &World, actor: ObjId, params: &Value) -> Result<OutputEvent, RichText> {
        let data = subject_frame(w, actor, params, UnitView::vitals_value)?;
        Ok(OutputEvent::Panel {
            feed: self.feed.clone(),
            kind: PanelFrameKind::Keyframe,
            data,
        })
    }
}
