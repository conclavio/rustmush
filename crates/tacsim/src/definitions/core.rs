//! **The core, authored** (17b cut M3.1; doc 17 §9 "The component", D-UM-33,
//! D-UM-35).
//!
//! The ninth definition kind and the seventh a pack may carry: a core is a
//! **class** and a **governed** bit, and nothing else. Its figures — the think
//! period, the op ceiling, the rows, the slots, the memory keys, `line_to`
//! and the profiles — are the class's row of the class table
//! ([`crate::arena::core::ClassTable`]) and are never authored twice: content
//! says which class a box is, the code says what a class can do.
//!
//! A chassis fits one through a `core` component
//! ([`crate::definitions::unit::ComponentRole::Core`]), and `bind` lays down
//! the component's `core_class` cell and the `tacsim.core` capability whose
//! payload is [`profile_payload`] — the ONE function that writes that payload,
//! for `bind` and for every hand-built test hull alike, so the two cannot
//! disagree about what a core carries.
//!
//! **Class 0 is no core** (D-UM-33): a hull with no core component has the
//! ROM alone, so a core definition of class `0` is refused at load.

use mush_core::RulesValue;
use mush_core::mechanics::DefinitionRef;
use mush_tactics::{
    CapabilityId, CapabilityInit, CapabilityKey, CellId, CellSpec, ComponentId, ComponentSpec,
    ConditionId, InstallPoint, SectionId,
};
use serde::{Deserialize, Serialize};

use crate::arena::core::{ClassTable, CoreClass};
use crate::definitions::DefinitionError;

/// **One core** (17b cut M3.1; D-UM-33): what content says about a box.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreDef {
    /// The human-facing name (`line core`).
    pub label: String,
    /// The Compact class: `I`, `II` or `III` (`0` is refused: no core).
    pub class: CoreClass,
    /// Whether the core runs under the governor (doc 17 §10). `true` when
    /// absent — every shipped core is governed.
    #[serde(default = "governed_by_default")]
    pub governed: bool,
    /// Why this box.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// `governed`'s default.
const fn governed_by_default() -> bool {
    true
}

impl CoreDef {
    /// Re-run every bound.
    ///
    /// # Errors
    ///
    /// [`DefinitionError::Value`] naming the entry: the label's and the note's
    /// bounds, and a class of `0`.
    pub fn check(&self, entry: &str) -> Result<(), DefinitionError> {
        crate::definitions::check_label(entry, &self.label)?;
        crate::definitions::check_note(entry, &self.note)?;
        if self.class == CoreClass::Zero {
            return Err(DefinitionError::value(
                entry,
                "Class 0 is no core; a hull with none has the ROM",
            ));
        }
        Ok(())
    }
}

/// **The `tacsim.core` capability's rules payload** (17b cut M3.1; the
/// `tacsim.core.profile` schema's ten keys): the class word, the governed bit
/// and that class's row of [`ClassTable::STOCK`] — `think_ms`, `ops`, `rows`,
/// `odds_rows`, `slots`, `memory_keys`, `line_to`, `profiles`.
///
/// The fitting's record, written once at `bind`: the class a mind thinks at
/// is read off the component's `core_class` cell, live, and the figures off
/// the arena's own table (`Arena::classes`), so neither a lowered cell nor an
/// operator's `[tacsim.minds].classes` rewrites it. The payload's `class` is
/// read only when the cell is missing ([`crate::arena::core::read_core`]).
#[must_use]
pub fn profile_payload(class: CoreClass, governed: bool) -> serde_json::Value {
    let row = ClassTable::STOCK.row(class);
    serde_json::json!({
        "class": class.as_str(),
        "governed": governed,
        "think_ms": row.think_ms,
        "ops": row.ops,
        "rows": row.rows,
        "odds_rows": row.odds_rows,
        "slots": row.slots,
        "memory_keys": row.memory_keys,
        "line_to": row.line_to,
        "profiles": row.profiles,
    })
}

/// **A core's one cell** (17b cut M3.1): [`crate::definitions::unit::CORE_CLASS_CELL`],
/// `0` to `3`, born at `class`'s ordinal — what `bind` lays on a core's
/// component and every hand-built test hull lays on its own ([`core_component`]).
#[must_use]
pub fn core_cell(class: CoreClass) -> Option<CellSpec> {
    Some(CellSpec {
        id: CellId::new(crate::definitions::unit::CORE_CLASS_CELL).ok()?,
        min: 0,
        max: i64::from(CoreClass::III.ordinal()),
        init: i64::from(class.ordinal()),
    })
}

/// **A core's capability** (17b cut M3.1): [`crate::definitions::unit::CORE_CAPABILITY`]
/// sourced on `component`, its rules [`profile_payload`] under `schema` (the
/// minted `tacsim.core.profile`) — what `bind` grants and every hand-built
/// test hull grants ([`core_component`]).
#[must_use]
pub fn core_capability(
    component: &ComponentId,
    class: CoreClass,
    governed: bool,
    schema: DefinitionRef,
) -> Option<CapabilityInit> {
    Some(CapabilityInit {
        key: CapabilityKey {
            id: CapabilityId::new(crate::definitions::unit::CORE_CAPABILITY).ok()?,
            source: Some(component.clone()),
        },
        rules: RulesValue::new(schema, profile_payload(class, governed)).ok()?,
    })
}

/// **A governed core of `class` in `section`, for a unit built by hand**
/// (17b cut M3.1): the component `core` (its one cell, [`core_cell`]) and its
/// capability ([`core_capability`]), the shape `bind` lays down from content,
/// under the caller's own `definition` (the component's) and `schema`
/// references — a test fixture's zero-digest ones. Push the first onto the
/// spec's components and the second onto its capabilities.
#[must_use]
pub fn core_component(
    section: &str,
    class: CoreClass,
    definition: DefinitionRef,
    schema: DefinitionRef,
) -> Option<(ComponentSpec, CapabilityInit)> {
    let id = ComponentId::new("core").ok()?;
    let capability = core_capability(&id, class, true, schema)?;
    Some((
        ComponentSpec {
            id,
            definition,
            install: InstallPoint::Section(SectionId::new(section).ok()?),
            condition: ConditionId::new(crate::definitions::unit::INTACT).ok()?,
            cells: vec![core_cell(class)?],
            asset: None,
        },
        capability,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A core of class `0` is refused by name; `governed` defaults to `true`;
    /// the payload carries the ten keys of the schema.
    #[test]
    fn m3_a_core_is_a_class_and_a_governed_bit() {
        let def: CoreDef = serde_json::from_value(serde_json::json!({
            "label": "line core", "class": "II"
        }))
        .expect("decodes");
        assert!(def.governed, "governed by default");
        assert_eq!(def.check("tacsim.core.line"), Ok(()));
        let zero = CoreDef {
            class: CoreClass::Zero,
            ..def.clone()
        };
        assert_eq!(
            zero.check("tacsim.core.none"),
            Err(DefinitionError::value(
                "tacsim.core.none",
                "Class 0 is no core; a hull with none has the ROM"
            ))
        );
        assert!(
            serde_json::from_value::<CoreDef>(serde_json::json!({
                "label": "x", "class": "IV"
            }))
            .is_err()
        );
        let payload = profile_payload(CoreClass::III, true);
        let keys: Vec<&str> = payload
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys.len(), 10, "{keys:?}");
        assert_eq!(payload["class"], "III");
        assert_eq!(payload["ops"], 4_500);
        assert_eq!(payload["think_ms"], 250);
    }
}
