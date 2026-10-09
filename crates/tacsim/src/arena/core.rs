//! **The hardware under every mind** (17b cut M3.1; doc 17 §9 "Hardware: the
//! core, the classes, the pool"; D-UM-33, D-UM-35, D-UM-37).
//!
//! A hull's **core** is a component (`crate::definitions::unit::ComponentRole::Core`)
//! of a Compact **class** — `0`, `I`, `II` or `III` — and the class is a row of
//! one table ([`ClassTable`]): how often a mind on it thinks, how many
//! operations a think may spend, how many contact rows and odds rows its view
//! carries, how many condition slots the hull holds, how many memory keys the
//! mind keeps, how many `line_to` traces a think may ask for and how many
//! card profiles it may store. Doc 17 §9's table is [`ClassTable::STOCK`],
//! the compiled-in row set the config defaults to (`[tacsim.minds].classes`),
//! `bind` writes into every minted core's payload, and every fixture runs on.
//!
//! # Read live, never copied
//!
//! [`read_core`] reads the core off the M32 unit **every time** it is asked:
//! the `tacsim.core` capability ([`crate::definitions::unit::CORE_CAPABILITY`])
//! says the hull has one and whether it is governed, and the CURRENT value of
//! its component's `core_class` cell ([`crate::definitions::unit::CORE_CLASS_CELL`])
//! is its class — the fitted class until something lowers the cell. Doc 17 §9
//! wrote `UnitState.core: CoreProfile`, a copy decoded at check-out; **it is
//! not built** (17b cut M3's declared departure): the class is a live fact of
//! the unit that a hit may move mid-match, and a copy would be a second source
//! of it. A hull with no core capability — one that never had one, and one
//! whose core was revoked — reads `None`: Class 0, the ROM alone (D-UM-33).
//!
//! # Three readings of one class
//!
//! * [`Arena::core_of`] — the hardware, off the hull ([`read_core`]).
//! * [`Arena::mind_class`] — the class a **mind** thinks at: the lesser of the
//!   mind's rated class ([`crate::arena::mind::STOCK_RATED_CLASS`] until 17b
//!   cut M3.3's seal names one) and the core's, `Zero` with no core, a core at
//!   Class 0 or a dropped row. Standard Doctrine is rated I, so on any core of
//!   Class I or above it thinks at Class I's row — today's constants — and
//!   nothing of T1 or T2 moves (17b cut M3's ruling, the plan's F4).
//! * [`Arena::slot_capacity`] — the **hull's** figure: its core's row's
//!   `slots`, for a seated pilot and a mind alike.
//!
//! Integers only, no I/O and no clock: the arena library's gate
//! (`crates/mush-contract-tests/tests/arena_library.rs`) reads this file.

use std::collections::BTreeMap;

use mush_core::ObjId;
use mush_tactics::{CapabilityId, CellAddr, CellId, TacticalUnit};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::arena::state::Arena;
use crate::definitions::unit::{CORE_CAPABILITY, CORE_CLASS_CELL};

/// **The most condition slots any class holds** (doc 17 §9's Class III
/// `slots`): the widest a hull's slot row is laid down (17b cut M3.2,
/// `crate::arena::conditions::ConditionTable::arm`), whatever capacity it is
/// handed.
pub const MAX_CONDITION_SLOTS: usize = 6;

/// **A Compact core class** (D-UM-35): `0` — no core, the drive ROM alone —
/// then `I`, `II` and `III`, ordered by [`CoreClass::ordinal`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CoreClass {
    /// **The drive ROM** (D-UM-32, D-UM-33): what every hull has, armed or
    /// not; no core component at all.
    #[serde(rename = "0")]
    Zero,
    /// **The doctrine core**: today's constants.
    #[serde(rename = "I")]
    I,
    /// **The line core**.
    #[serde(rename = "II")]
    II,
    /// **The command core** — named for the command vehicles that carry it,
    /// not for commanding: no core of any class orders (D-UM-52).
    #[serde(rename = "III")]
    III,
}

impl CoreClass {
    /// Every class, lowest first.
    pub const ALL: [CoreClass; 4] = [CoreClass::Zero, CoreClass::I, CoreClass::II, CoreClass::III];

    /// The class word content and the config write: `0`, `I`, `II`, `III`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            CoreClass::Zero => "0",
            CoreClass::I => "I",
            CoreClass::II => "II",
            CoreClass::III => "III",
        }
    }

    /// The class a word names, or `None`.
    #[must_use]
    pub fn parse(word: &str) -> Option<CoreClass> {
        CoreClass::ALL
            .into_iter()
            .find(|class| class.as_str() == word)
    }

    /// The class's number, `0` to `3` — the value a core's `core_class` cell
    /// holds.
    #[must_use]
    pub const fn ordinal(self) -> u8 {
        match self {
            CoreClass::Zero => 0,
            CoreClass::I => 1,
            CoreClass::II => 2,
            CoreClass::III => 3,
        }
    }

    /// The class numbered `n`, or `None` past 3.
    #[must_use]
    pub const fn from_ordinal(n: u8) -> Option<CoreClass> {
        match n {
            0 => Some(CoreClass::Zero),
            1 => Some(CoreClass::I),
            2 => Some(CoreClass::II),
            3 => Some(CoreClass::III),
            _ => None,
        }
    }

    /// **One class down** — what a degraded core becomes (D-UM-34, 17b cut
    /// M3.4's `degrade_core`); `Zero` stays `Zero`.
    #[must_use]
    pub const fn lower(self) -> CoreClass {
        match self {
            CoreClass::Zero | CoreClass::I => CoreClass::Zero,
            CoreClass::II => CoreClass::I,
            CoreClass::III => CoreClass::II,
        }
    }
}

/// **One class's row** (D-UM-35): every figure a mind on that hardware is held
/// to. `think_ms` is arena milliseconds; `ops` is operations plus fees a think
/// may spend; the rest are counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreProfile {
    /// The class this row is.
    pub class: CoreClass,
    /// How often a mind thinks, arena milliseconds.
    pub think_ms: i64,
    /// The op ceiling of one think (D-UM-26).
    pub ops: u64,
    /// The contact rows a view carries: the hull's lock and its order's
    /// contact seated, then every contact not known friendly, then the
    /// friendly ones, each group nearest-first, read nearest-first (17b cut
    /// M4.5c; doc 17 §3 as amended 2026-10-09 by the owner's two rulings,
    /// `Arena::mind_view`).
    pub rows: usize,
    /// How many of those carry odds — `0` is the lock's alone.
    pub odds_rows: usize,
    /// The condition slots the hull holds (D-UM-16).
    pub slots: usize,
    /// The memory keys a mind keeps (D-UM-30).
    pub memory_keys: usize,
    /// The `line_to` traces one think may ask for.
    pub line_to: u32,
    /// The card profiles a principal may store (17b cut M6's; carried here).
    pub profiles: u8,
}

/// **Doc 17 §9's four rows, in [`CoreClass::ALL`]'s order** (D-UM-35), one
/// [`CoreProfile`] per class.
///
/// **Its wire shape** (`[tacsim.minds].classes`, 17b cut M3.1): a table keyed
/// by the class word, each value the row's eight figures without its class —
/// `[tacsim.minds.classes.II]` / `think_ms = 500` / … in the deploy file,
/// `{"II": {"think_ms": 500, …}}` in JSON. **An absent class reads its
/// [`ClassTable::STOCK`] row and an absent figure reads that row's figure**,
/// so an operator who moves one number writes one line; a class word that is
/// none of the four, or a figure that is none of the eight, is refused at
/// decode by name. Every value is then held to `MindsConfig::validate`'s
/// bounds (`crate::config::MindsConfig`). Written back out, every class and
/// every figure is spelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ClassTableWire", into = "ClassTableWire")]
pub struct ClassTable {
    rows: [CoreProfile; 4],
}

impl ClassTable {
    /// **D-UM-35's table, exactly** (doc 17 §9): think period, ops, contact
    /// rows, odds rows, slots, memory keys, `line_to`, profiles.
    pub const STOCK: ClassTable = ClassTable {
        rows: [
            row(CoreClass::Zero, 1_000, 400, 8, 0, 1, 16, 1, 1),
            row(CoreClass::I, 1_000, 1_500, 12, 2, 2, 32, 2, 2),
            row(CoreClass::II, 500, 3_000, 16, 4, 4, 64, 4, 3),
            row(CoreClass::III, 250, 4_500, 24, 8, 6, 128, 6, 4),
        ],
    };

    /// **`class`'s row.**
    #[must_use]
    pub const fn row(&self, class: CoreClass) -> CoreProfile {
        let [zero, one, two, three] = self.rows;
        match class {
            CoreClass::Zero => zero,
            CoreClass::I => one,
            CoreClass::II => two,
            CoreClass::III => three,
        }
    }

    /// **A class's ceiling per arena second** (D-UM-36): `ops × 1000 /
    /// think_ms` — 400, 1,500, 6,000 and 18,000 on the stock table. What 17b
    /// cut M3.5's pool admits against, and never measured use.
    #[must_use]
    pub fn ceiling_per_s(&self, class: CoreClass) -> u64 {
        let profile = self.row(class);
        let period = u64::try_from(profile.think_ms.max(1)).unwrap_or(1);
        profile.ops.saturating_mul(1_000) / period
    }

    /// The four rows, lowest class first.
    #[must_use]
    pub const fn rows(&self) -> &[CoreProfile; 4] {
        &self.rows
    }
}

impl Default for ClassTable {
    /// [`ClassTable::STOCK`].
    fn default() -> Self {
        ClassTable::STOCK
    }
}

/// One stock row, positionally (the const table above reads as doc 17's).
#[allow(clippy::too_many_arguments)]
const fn row(
    class: CoreClass,
    think_ms: i64,
    ops: u64,
    rows: usize,
    odds_rows: usize,
    slots: usize,
    memory_keys: usize,
    line_to: u32,
    profiles: u8,
) -> CoreProfile {
    CoreProfile {
        class,
        think_ms,
        ops,
        rows,
        odds_rows,
        slots,
        memory_keys,
        line_to,
        profiles,
    }
}

/// **One class's figures as the wire writes them** — the row without its
/// class, every figure optional on the way in ([`ClassTable`]'s wire shape).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RowWire {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    think_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ops: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rows: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    odds_rows: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    slots: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    memory_keys: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    line_to: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    profiles: Option<u8>,
}

/// [`ClassTable`]'s wire: the class word to its figures.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
struct ClassTableWire(BTreeMap<String, RowWire>);

impl TryFrom<ClassTableWire> for ClassTable {
    type Error = String;

    /// The stock table with every authored figure written over it; a class
    /// word that names no class is refused by name.
    fn try_from(wire: ClassTableWire) -> Result<Self, Self::Error> {
        let mut table = ClassTable::STOCK;
        for (word, figures) in wire.0 {
            let Some(class) = CoreClass::parse(&word) else {
                return Err(format!(
                    "`{word}` is not a core class; the classes are 0, I, II and III"
                ));
            };
            let at = usize::from(class.ordinal());
            let Some(profile) = table.rows.get_mut(at) else {
                continue;
            };
            let stock = *profile;
            *profile = CoreProfile {
                class,
                think_ms: figures.think_ms.unwrap_or(stock.think_ms),
                ops: figures.ops.unwrap_or(stock.ops),
                rows: figures.rows.unwrap_or(stock.rows),
                odds_rows: figures.odds_rows.unwrap_or(stock.odds_rows),
                slots: figures.slots.unwrap_or(stock.slots),
                memory_keys: figures.memory_keys.unwrap_or(stock.memory_keys),
                line_to: figures.line_to.unwrap_or(stock.line_to),
                profiles: figures.profiles.unwrap_or(stock.profiles),
            };
        }
        Ok(table)
    }
}

impl From<ClassTable> for ClassTableWire {
    /// Every class and every figure, spelled.
    fn from(table: ClassTable) -> Self {
        ClassTableWire(
            table
                .rows
                .iter()
                .map(|profile| {
                    (
                        profile.class.as_str().to_owned(),
                        RowWire {
                            think_ms: Some(profile.think_ms),
                            ops: Some(profile.ops),
                            rows: Some(profile.rows),
                            odds_rows: Some(profile.odds_rows),
                            slots: Some(profile.slots),
                            memory_keys: Some(profile.memory_keys),
                            line_to: Some(profile.line_to),
                            profiles: Some(profile.profiles),
                        },
                    )
                })
                .collect(),
        )
    }
}

/// **A hull's core, as the unit carries it** (D-UM-33): the component that
/// holds it, its class now and whether it is governed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreHardware {
    /// The component the capability's key names as its source (`core` on
    /// every shipped chassis).
    pub component: SmolStr,
    /// The class the component's `core_class` cell reads NOW.
    pub class: CoreClass,
    /// Whether the core runs under the governor (doc 17 §10) — the
    /// capability payload's `governed`, `true` when it is absent.
    pub governed: bool,
}

/// **The core a unit carries, read live** (D-UM-33; this file's head).
///
/// The capability whose id is [`CORE_CAPABILITY`] present: `component` is its
/// key's source; `governed` its rules payload's `"governed"` (missing reads
/// `true`); `class` the CURRENT value of that component's [`CORE_CLASS_CELL`]
/// through [`CoreClass::from_ordinal`] — a cell past 3 reads `III`, under 0
/// reads `Zero`, and a missing cell reads the payload's `"class"` word (a
/// missing word, `Zero`). `None` when the unit holds no such capability: a
/// hull that never had a core and a hull whose core was revoked read the same
/// (Class 0, the ROM alone).
///
/// **Never copied onto `UnitState`**: doc 17 §9's `UnitState.core` is not
/// built (17b cut M3's declared departure) — the class is a live fact of the
/// unit, and this function is its one reader.
#[must_use]
pub fn read_core(unit: &TacticalUnit) -> Option<CoreHardware> {
    let id = CapabilityId::new(CORE_CAPABILITY).ok()?;
    let key = unit.capabilities().find(|key| key.id == id)?;
    let payload = unit.capability(key)?.data();
    let governed = payload
        .get("governed")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    let fitted = payload
        .get("class")
        .and_then(serde_json::Value::as_str)
        .and_then(CoreClass::parse)
        .unwrap_or(CoreClass::Zero);
    let component = key.source.as_ref();
    let live = component.and_then(|component| {
        let cell = CellId::new(CORE_CLASS_CELL).ok()?;
        let current = unit
            .cell(&CellAddr::Component {
                component: component.clone(),
                cell,
            })?
            .current;
        let n = u8::try_from(current.clamp(0, 3)).ok()?;
        CoreClass::from_ordinal(n)
    });
    Some(CoreHardware {
        component: component.map_or_else(SmolStr::default, |id| SmolStr::new(id.as_str())),
        class: live.unwrap_or(fitted),
        governed,
    })
}

/// **Whether a unit was fitted with a core at all** (17b cut M3.6): a
/// component carrying a [`CORE_CLASS_CELL`]. Unlike [`read_core`] this stays
/// true once `destroy_core` has revoked the capability — the component and
/// its cell are not taken off the unit — so it tells a hull whose core is
/// GONE from a hull that never had one, which is what the pilot's `its core
/// is gone` needs (`crate::arena::damage::Arena::fired_consequences`,
/// `crate::match_::report::OwnTank`).
#[must_use]
pub fn fitted_with_core(unit: &TacticalUnit) -> bool {
    unit.cells().any(|(addr, _)| {
        matches!(addr, CellAddr::Component { cell, .. } if cell.as_str() == CORE_CLASS_CELL)
    })
}

impl Arena {
    /// **The hull's core** ([`read_core`] over `UnitState.unit`), live; `None`
    /// for a unit this arena does not hold or one with no core.
    #[must_use]
    pub fn core_of(&self, unit: ObjId) -> Option<CoreHardware> {
        read_core(&self.unit(unit)?.unit)
    }

    /// **The class a mind on `unit` thinks at** (D-UM-35; 17b cut M3's
    /// ruling, the plan's F4; **17b cut M4.1: off the seal**): the class the
    /// hull's seal records ([`crate::arena::mind::SealRef::class`] —
    /// `min(rated, core at the seal, cap)`, `crate::match_::seal::seal_for`
    /// under `SealTerms::{rated, class_cap}`) against the core's class NOW
    /// (a `degrade_core` since the seal lowers it); `Zero` for a hull with
    /// no core, a core whose cell reads 0, a hull no seal reached, and a row
    /// that is dropped to doctrine ([`crate::arena::mind::MindState::dropped`]).
    /// Until this cut the rating was the arena-wide `rated_class` (Standard
    /// Doctrine's [`crate::arena::mind::STOCK_RATED_CLASS`], a test's lever
    /// beside it); a rig that rates a hull above the stock rating seals it
    /// so, or writes its `UnitState::seal.class`.
    #[must_use]
    pub fn mind_class(&self, unit: ObjId) -> CoreClass {
        if self
            .minds
            .get(&unit)
            .is_some_and(crate::arena::mind::MindState::dropped)
        {
            return CoreClass::Zero;
        }
        let Some(state) = self.unit(unit) else {
            return CoreClass::Zero;
        };
        read_core(&state.unit).map_or(CoreClass::Zero, |core| core.class.min(state.seal.class))
    }

    /// **The row a mind on `unit` thinks at**: [`Arena::classes`]' row of
    /// [`Arena::mind_class`].
    #[must_use]
    pub fn mind_profile(&self, unit: ObjId) -> CoreProfile {
        self.classes.row(self.mind_class(unit))
    }

    /// **The condition slots the hull holds** (D-UM-35, D-UM-16): its core's
    /// row's `slots` — Class 0's one with no core — for a seated pilot and a
    /// mind alike. Read at every arming (`Arena::resolve_arming` hands it to
    /// `ConditionTable::arm`), which since 17b cut M3.2 lays the hull's row
    /// down this wide, at most [`MAX_CONDITION_SLOTS`].
    #[must_use]
    pub fn slot_capacity(&self, unit: ObjId) -> usize {
        let class = self
            .core_of(unit)
            .map_or(CoreClass::Zero, |core| core.class);
        self.classes.row(class).slots
    }
}

#[cfg(test)]
mod tests {
    use mush_tactics::{
        CapabilityInit, CapabilityKey, CellSpec, ComponentId, ComponentSpec, ConditionId,
        InstallPoint, RulesValue, SectionId, UnitMutationOp, UnitMutationPlan,
    };

    use super::*;
    use crate::arena::test_support::{dref, rules, tactical_unit};

    /// The shipped line tank, bound from the compiled-in content (Class II in
    /// its turret).
    fn shipped(id: &str) -> TacticalUnit {
        let catalog = crate::definitions::Catalog::build_fielded(&[], crate::NS)
            .expect("the fielded content binds");
        let entry = mush_core::CanonicalId::new(id).expect("a canonical id");
        let spec = catalog.specs.get(&entry).expect("the spec");
        let binding = catalog.bindings.get(&entry).expect("the binding");
        crate::definitions::to_tactical_unit(spec, binding, ObjId(4)).expect("it instantiates")
    }

    /// A hull section with one core component whose `core_class` cell holds
    /// `init` under `max`, and the capability over it.
    fn cored(init: i64, max: i64) -> TacticalUnit {
        let mut spec = mush_tactics::UnitSpec {
            definition: dref("unit", "tacsim.unit.line_tank"),
            sections: vec![mush_tactics::SectionSpec {
                id: SectionId::new("hull").expect("a section"),
                condition: ConditionId::new("intact").expect("a condition"),
                facets: Vec::new(),
                slots: Vec::new(),
                cells: Vec::new(),
            }],
            unit_cells: Vec::new(),
            components: Vec::new(),
            capabilities: Vec::new(),
            mobility: None,
            rules: rules(),
        };
        spec.components.push(ComponentSpec {
            id: ComponentId::new("core").expect("a component"),
            definition: dref("component", "tacsim.unit.line_tank.core"),
            install: InstallPoint::Section(SectionId::new("hull").expect("a section")),
            condition: ConditionId::new("intact").expect("a condition"),
            cells: vec![CellSpec {
                id: CellId::new(CORE_CLASS_CELL).expect("a cell"),
                min: 0,
                max,
                init,
            }],
            asset: None,
        });
        spec.capabilities.push(CapabilityInit {
            key: CapabilityKey {
                id: CapabilityId::new(CORE_CAPABILITY).expect("a capability"),
                source: Some(ComponentId::new("core").expect("a component")),
            },
            rules: RulesValue::new(
                dref("schema", crate::definitions::unit::CORE_RULES_SCHEMA),
                crate::definitions::core::profile_payload(CoreClass::II, false),
            )
            .expect("a payload"),
        });
        TacticalUnit::from_spec(&spec, ObjId(4)).expect("a cored unit")
    }

    /// One plan, applied.
    fn applied(unit: &TacticalUnit, op: UnitMutationOp) -> TacticalUnit {
        let plan = UnitMutationPlan::new(vec![op], None, None).expect("a plan");
        let (next, _) = mush_tactics::apply(unit, &plan, 0).expect("the plan applies");
        next
    }

    /// **`read_core` reads the shipped chassis, the target, a revoked core, a
    /// lowered cell, a cell at 0 and a cell past 3** (17b cut M3.1).
    #[test]
    fn m3_read_core_reads_the_hull_live() {
        let tank = shipped(crate::definitions::content::LINE_TANK);
        let core = read_core(&tank).expect("the line tank carries a core");
        assert_eq!(
            core,
            CoreHardware {
                component: SmolStr::new("core"),
                class: CoreClass::II,
                governed: true,
            }
        );
        assert_eq!(
            read_core(&shipped(crate::definitions::content::TARGET_MK1)),
            None,
            "the range's target has the ROM alone"
        );
        assert_eq!(read_core(&tactical_unit(ObjId(4))), None, "a bare hull");

        let key = tank
            .capabilities()
            .find(|key| key.id.as_str() == CORE_CAPABILITY)
            .cloned()
            .expect("the core's key");
        let revoked = applied(&tank, UnitMutationOp::RevokeCapability { key });
        assert_eq!(read_core(&revoked), None, "a revoked core reads none");

        let addr = CellAddr::Component {
            component: ComponentId::new("core").expect("a component"),
            cell: CellId::new(CORE_CLASS_CELL).expect("a cell"),
        };
        let lowered = applied(
            &tank,
            UnitMutationOp::AdjustCell {
                addr: addr.clone(),
                delta: -1,
            },
        );
        assert_eq!(
            read_core(&lowered).map(|core| core.class),
            Some(CoreClass::I)
        );
        let nought = applied(&tank, UnitMutationOp::SetCell { addr, value: 0 });
        assert_eq!(
            read_core(&nought).map(|core| core.class),
            Some(CoreClass::Zero),
            "a core whose cell reads 0 is Class 0 and still a core"
        );

        let past = read_core(&cored(7, 9)).expect("a core");
        assert_eq!(past.class, CoreClass::III, "a cell past 3 reads III");
        assert!(!past.governed, "the payload's governed bit");
        assert_eq!(
            read_core(&cored(1, 3)).map(|core| core.class),
            Some(CoreClass::I)
        );
    }

    /// **The three readings** (17b cut M3's ruling, the plan's F4): a rated-I
    /// mind on a II hull thinks at I (`min`), the hull holds II's slots; a
    /// hull with no core, one whose cell reads 0 and a dropped row think at
    /// `Zero`; rated III, the II hull thinks at II.
    #[test]
    fn m3_a_mind_thinks_at_the_lesser_of_its_rating_and_its_core() {
        use crate::arena::state::{Controller, Team};
        use crate::arena::test_support::{arena_of, line_tank, pose, scout};
        const RED: ObjId = ObjId(4);
        const BLUE: ObjId = ObjId(9);
        let mut red = line_tank(RED, Team::Red, pose(1_500, 4_500));
        red.controller = Controller::Mind;
        red.reseal(&crate::match_::SealTerms::fixture())
            .expect("the fixture terms seal");
        let bare = crate::arena::state::CustodyUnit {
            unit: tactical_unit(BLUE),
            ..scout(BLUE, Team::Blue, pose(6_500, 4_500))
        }
        .sealed();
        let mut arena = arena_of(vec![red, bare]);
        assert_eq!(
            arena.core_of(RED).map(|core| core.class),
            Some(CoreClass::II)
        );
        assert_eq!(arena.mind_class(RED), CoreClass::I, "rated I on a II core");
        assert_eq!(arena.mind_profile(RED), ClassTable::STOCK.row(CoreClass::I));
        assert_eq!(arena.slot_capacity(RED), 4, "the hull's own figure");
        assert_eq!(arena.core_of(BLUE), None);
        assert_eq!(arena.mind_class(BLUE), CoreClass::Zero, "no core");
        assert_eq!(arena.slot_capacity(BLUE), 1);
        // 17b cut M4.1: the rating is the seal's (`SealTerms::rated`), read
        // off `UnitState::seal.class` — `min(rated, core, cap)` at the seal.
        let rated_iii = line_tank(RED, Team::Red, pose(1_500, 4_500));
        let mut rated_iii = crate::arena::state::CustodyUnit {
            controller: Controller::Mind,
            ..rated_iii
        };
        rated_iii
            .reseal(&crate::match_::SealTerms::fixture().rated(CoreClass::III))
            .expect("the fixture terms seal");
        assert_eq!(rated_iii.seal.class, CoreClass::II, "the core caps it");
        if let Some(state) = arena.unit_mut(RED) {
            state.seal.class = rated_iii.seal.class;
        }
        assert_eq!(arena.mind_class(RED), CoreClass::II, "the core caps it");
        if let Some(state) = arena.unit_mut(RED) {
            state.seal.class = CoreClass::I;
        }
        // 17b cut M3.2: the drop is the row's own field, not its strikes.
        if let Some(row) = arena.minds.get_mut(&RED) {
            row.dropped = Some(crate::arena::mind::DropCause::Strikes);
        }
        assert_eq!(arena.mind_class(RED), CoreClass::Zero, "a dropped row");
        if let Some(row) = arena.minds.get_mut(&RED) {
            row.dropped = None;
        }
        let addr = CellAddr::Component {
            component: mush_tactics::ComponentId::new("core").expect("a component"),
            cell: CellId::new(CORE_CLASS_CELL).expect("a cell"),
        };
        let lowered = arena
            .unit(RED)
            .map(|state| applied(&state.unit, UnitMutationOp::SetCell { addr, value: 0 }))
            .expect("red");
        if let Some(state) = arena.unit_mut(RED) {
            state.unit = lowered;
        }
        assert_eq!(arena.mind_class(RED), CoreClass::Zero, "a core at Class 0");
        assert_eq!(arena.slot_capacity(RED), 1);
    }

    /// **The classes: their words, their numbers, one down, the ceilings and
    /// the rows** (D-UM-35, D-UM-36).
    #[test]
    fn m3_the_classes_and_the_table() {
        for class in CoreClass::ALL {
            assert_eq!(CoreClass::parse(class.as_str()), Some(class));
            assert_eq!(CoreClass::from_ordinal(class.ordinal()), Some(class));
            assert_eq!(ClassTable::STOCK.row(class).class, class);
        }
        assert_eq!(CoreClass::from_ordinal(4), None);
        assert_eq!(CoreClass::parse("IV"), None);
        assert_eq!(CoreClass::Zero.lower(), CoreClass::Zero);
        assert_eq!(CoreClass::I.lower(), CoreClass::Zero);
        assert_eq!(CoreClass::II.lower(), CoreClass::I);
        assert_eq!(CoreClass::III.lower(), CoreClass::II);
        assert!(CoreClass::Zero < CoreClass::I && CoreClass::II < CoreClass::III);
        assert_eq!(
            CoreClass::ALL.map(|class| ClassTable::STOCK.ceiling_per_s(class)),
            [400, 1_500, 6_000, 18_000]
        );
        assert_eq!(
            ClassTable::STOCK.row(CoreClass::II),
            CoreProfile {
                class: CoreClass::II,
                think_ms: 500,
                ops: 3_000,
                rows: 16,
                odds_rows: 4,
                slots: 4,
                memory_keys: 64,
                line_to: 4,
                profiles: 3,
            }
        );
        assert_eq!(ClassTable::default(), ClassTable::STOCK);
        assert_eq!(
            ClassTable::STOCK.row(CoreClass::III).slots,
            MAX_CONDITION_SLOTS
        );
        assert_eq!(
            serde_json::to_value(CoreClass::Zero).expect("a word"),
            serde_json::json!("0")
        );
    }

    /// **The wire shape**: an absent class and an absent figure read the stock
    /// row; a word that names no class and a figure that is none of the eight
    /// are refused; written back out, the table round-trips.
    #[test]
    fn m3_the_class_table_wire_reads_the_stock_row_where_absent() {
        let empty: ClassTable = serde_json::from_str("{}").expect("an empty table");
        assert_eq!(empty, ClassTable::STOCK);
        let moved: ClassTable =
            serde_json::from_str(r#"{"II": {"think_ms": 750}}"#).expect("one figure");
        assert_eq!(moved.row(CoreClass::II).think_ms, 750);
        assert_eq!(moved.row(CoreClass::II).ops, 3_000, "the rest is stock");
        assert_eq!(moved.row(CoreClass::I), ClassTable::STOCK.row(CoreClass::I));
        let wrong = serde_json::from_str::<ClassTable>(r#"{"IV": {}}"#)
            .expect_err("no class IV")
            .to_string();
        assert!(wrong.contains("`IV` is not a core class"), "{wrong}");
        assert!(serde_json::from_str::<ClassTable>(r#"{"I": {"speed": 1}}"#).is_err());
        let text = serde_json::to_string(&moved).expect("encodes");
        let back: ClassTable = serde_json::from_str(&text).expect("decodes");
        assert_eq!(back, moved);
    }
}
