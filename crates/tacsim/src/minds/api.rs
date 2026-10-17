//! **The view as a mind reads it** (17b cut M2.3; doc 17 §3 "The
//! observation"; D-UM-4, D-UM-6, D-UM-7, D-UM-28).
//!
//! The hull's [`MindView`] reaches a script as **registered custom types with
//! getters, never maps** (D-UM-7: a getter read 15.05 µs against a map's 24.69
//! µs at twelve contacts on the bench). `View` holds the arena's own
//! `Arc<MindView>` — the one the arena wrapped once for the think
//! (`ThinkJob::view`) — and every row a script reaches is a **handle**, the
//! `Arc` and an index, so reading `v.contacts` copies no row: `Contact`,
//! `Weapon`, `Section`, `Facet`, `Notice`, and the order's `Order`. The small
//! fixed records (`Heat`, `Receiver`, `Condition`, `Armed`) are copied whole.
//!
//! **What a getter answers** (D-UM-28): an INT, a bool, a word (an
//! `ImmutableString`), a cell as a two-INT array `[x, y]`, an array of those,
//! another handle, or `()` where the keyframe writes `null` — never a FLOAT.
//! A contact is named by its `ContactId` read as an INT ([`contact_int`]: the
//! id's 64 bits, the bit-cast `StockMemory` stores `target` with), with the
//! designator and the alias as fields (D-UM-6): an integer costs no string
//! operation, survives a rename and cannot be forged; the trace prints the
//! designator.
//!
//! # Parity at the field level ([`PARITY`])
//!
//! D-UM-4: a mind reads exactly what the hull's keyframe
//! (`crate::match_::cockpit::status_frame_held`) carries and nothing more (one
//! field excepted since 17b cut M4.6b, below). The
//! keyframe and the view carry the same fields at the same resolution, but
//! not always under the same key or in the same form: the miss's reason lives
//! only in the notice's `text`, a drive's outcome is its `kind` word, the
//! order's deadline is printed as the time left (`expires_in_ms`, clamped at
//! nought) where a mind needs the absolute millisecond it ends an order by,
//! and the order's `target`/`of` are printed as designators. So parity is a
//! **pinned table**, one row per getter naming the keyframe's rendering of
//! the same field (17b §5's feas F5), and 17b cut M2.4's parity test reads
//! every row on a live field. A getter with no row fails
//! `every_view_getter_has_a_parity_row`.
//!
//! One field is not the keyframe's: since 17b cut M4.6b the view also reads
//! a hull with no authored speed as `immobile` once its power is `Online`
//! (the owner's STOP B ruling; doc 17 §3 and D-UM-4 as amended), where the
//! keyframe's `immobile` reads motion disabled alone. The parity field
//! fields no such hull, so `View.immobile`'s row still names the key of the
//! same name.

use std::sync::Arc;

use mush_mind::rhai::{Array, Dynamic, EvalAltResult, INT, ImmutableString};

use super::Registrar;
use super::context::{REFUSED_TYPE, refused};
use super::fees::{FEE_READ, charge};
use crate::arena::conditions::{
    Comparison, HeldWhy, LockSelector, Rule, RuleBand, RulePredicate, SectionState, SlotTarget,
    TargetState,
};
use crate::arena::mind::{MindView, ViewContact};
use crate::arena::perception::{
    DriveOutcome, FrameCondition, FrameNotice, MindObjective, Receiver as FrameReceiver,
    RefusalDetail,
};
use crate::arena::state::{Classification, ContactId};
use crate::arena::step::{FrameFacet, FrameHeat, FrameSection};
use crate::arena::weapons::{FrameArming, FrameRule, FrameWeapon};

// ---------------------------------------------------------------------------
// The handles
// ---------------------------------------------------------------------------

/// **The hull's view** (`View`): the arena's own `Arc`.
#[derive(Clone, Debug)]
pub struct View(pub Arc<MindView>);

/// **One contact row** (`Contact`): the view and the row's index in
/// `contacts`.
#[derive(Clone, Debug)]
pub struct Contact(pub Arc<MindView>, pub u32);

/// **One weapon row** (`Weapon`): the view and the row's index in `weapons`.
#[derive(Clone, Debug)]
pub struct Weapon(pub Arc<MindView>, pub u32);

/// **One of the hull's own sections** (`Section`).
#[derive(Clone, Debug)]
pub struct Section(pub Arc<MindView>, pub u32);

/// **One facet of one section** (`Facet`): the section's index, then the
/// facet's.
#[derive(Clone, Debug)]
pub struct Facet(pub Arc<MindView>, pub u32, pub u32);

/// **One notice of the drained buffer** (`Notice`).
#[derive(Clone, Debug)]
pub struct Notice(pub Arc<MindView>, pub u32);

/// **The standing order** (`Order`): the view, whose `order` is held.
#[derive(Clone, Debug)]
pub struct Order(pub Arc<MindView>);

/// **The hull's heat** (`Heat`), copied.
#[derive(Clone, Copy, Debug)]
pub struct Heat(pub FrameHeat);

/// **The hull's receiver** (`Receiver`), copied.
#[derive(Clone, Copy, Debug)]
pub struct Receiver(pub FrameReceiver);

/// **A contact's held reading** (`Condition`), copied.
#[derive(Clone, Copy, Debug)]
pub struct Condition(pub FrameCondition);

/// **A weapon's armed slot** (`Armed`), copied.
#[derive(Clone, Debug)]
pub struct Armed(pub FrameArming);

/// One ordered station rule, sharing the immutable view rather than its row.
#[derive(Clone, Debug)]
pub struct RuleView(pub Arc<MindView>, pub u32);

/// One written predicate. Its values are terms, never hidden target state.
#[derive(Clone, Debug)]
pub struct Predicate(pub RulePredicate);

/// A `u32` index as a `usize`.
fn at(index: u32) -> Option<usize> {
    usize::try_from(index).ok()
}

/// `0..len` as the handles' `u32` indices.
fn indices<T>(rows: &[T]) -> std::ops::Range<u32> {
    0..u32::try_from(rows.len()).unwrap_or(u32::MAX)
}

impl Contact {
    /// The row, or `None` for an index the view does not hold.
    #[must_use]
    pub fn row(&self) -> Option<&ViewContact> {
        self.0.contacts.get(at(self.1)?)
    }
}

impl Weapon {
    /// The row, or `None`.
    #[must_use]
    pub fn row(&self) -> Option<&FrameWeapon> {
        self.0.weapons.get(at(self.1)?)
    }
}

impl RuleView {
    /// The projected rule, or `None` for an index the view does not carry.
    #[must_use]
    pub fn row(&self) -> Option<&FrameRule> {
        self.0.rules.get(at(self.1)?)
    }
}

impl Section {
    /// The section, or `None`.
    #[must_use]
    pub fn row(&self) -> Option<&FrameSection> {
        self.0.sections.get(at(self.1)?)
    }
}

impl Facet {
    /// The facet, or `None`.
    #[must_use]
    pub fn row(&self) -> Option<&FrameFacet> {
        self.0.sections.get(at(self.1)?)?.facets.get(at(self.2)?)
    }
}

impl Notice {
    /// The notice, or `None`.
    #[must_use]
    pub fn row(&self) -> Option<&FrameNotice> {
        self.0.notices.get(at(self.1)?)
    }
}

/// **A contact id as a script's INT** (D-UM-6): the id's 64 bits read as an
/// `i64` — `StockMemory`'s bit-cast for `target`, so a script's stored id and
/// the native mind's are one INT.
#[must_use]
pub const fn contact_int(id: ContactId) -> i64 {
    i64::from_ne_bytes(id.0.to_ne_bytes())
}

/// **A script's INT as a contact id**: [`contact_int`]'s inverse.
#[must_use]
pub const fn contact_of(value: i64) -> ContactId {
    ContactId(u64::from_ne_bytes(value.to_ne_bytes()))
}

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

/// An integer as an INT, saturating at `INT::MAX` (a `u64` past it).
pub(crate) fn int<T>(value: T) -> Dynamic
where
    i64: TryFrom<T>,
{
    Dynamic::from_int(i64::try_from(value).unwrap_or(INT::MAX))
}

/// A word.
pub(crate) fn word(text: &str) -> Dynamic {
    Dynamic::from(ImmutableString::from(text))
}

/// A cell as `[x, y]`, or `()`.
pub(crate) fn cell(at: Option<(u16, u16)>) -> Dynamic {
    at.map_or(Dynamic::UNIT, |(x, y)| {
        Dynamic::from_array(vec![int(x), int(y)])
    })
}

/// `Some` through `f`, `None` as `()`.
pub(crate) fn opt<T>(value: Option<T>, f: impl FnOnce(T) -> Dynamic) -> Dynamic {
    value.map_or(Dynamic::UNIT, f)
}

/// Words as an array of words.
fn words<'w>(items: impl Iterator<Item = &'w smol_str::SmolStr>) -> Dynamic {
    Dynamic::from_array(items.map(|item| word(item)).collect::<Array>())
}

// ---------------------------------------------------------------------------
// Arguments (shared by the kernels and the stations)
// ---------------------------------------------------------------------------

/// **An INT argument within `T`'s range**, else a `type` refusal saying
/// `text`.
pub(crate) fn bounded<T: TryFrom<INT>>(value: INT, text: &str) -> Result<T, Box<EvalAltResult>> {
    T::try_from(value).map_err(|_| refused(REFUSED_TYPE, text))
}

/// **An INT**, else a `type` refusal — a FLOAT among the else (D-UM-28).
pub(crate) fn int_arg(value: &Dynamic, text: &str) -> Result<INT, Box<EvalAltResult>> {
    value.as_int().map_err(|_| refused(REFUSED_TYPE, text))
}

/// **`()` or an INT**, else a `type` refusal.
pub(crate) fn opt_int_arg(value: &Dynamic, text: &str) -> Result<Option<INT>, Box<EvalAltResult>> {
    if value.is_unit() {
        return Ok(None);
    }
    int_arg(value, text).map(Some)
}

/// **A word**, else a `type` refusal.
pub(crate) fn word_arg(value: &Dynamic, text: &str) -> Result<ImmutableString, Box<EvalAltResult>> {
    value
        .clone()
        .into_immutable_string()
        .map_err(|_| refused(REFUSED_TYPE, text))
}

/// **A cell `[x, y]`** of two INTs each in `0..=65535`, else a `type`
/// refusal.
pub(crate) fn cell_arg(value: &Dynamic, text: &str) -> Result<(u16, u16), Box<EvalAltResult>> {
    let items = value
        .clone()
        .into_array()
        .map_err(|_| refused(REFUSED_TYPE, text))?;
    let [x, y] = items.as_slice() else {
        return Err(refused(REFUSED_TYPE, text));
    };
    Ok((
        bounded(int_arg(x, text)?, text)?,
        bounded(int_arg(y, text)?, text)?,
    ))
}

/// **`()` or a cell**, else a `type` refusal.
pub(crate) fn opt_cell_arg(
    value: &Dynamic,
    text: &str,
) -> Result<Option<(u16, u16)>, Box<EvalAltResult>> {
    if value.is_unit() {
        return Ok(None);
    }
    cell_arg(value, text).map(Some)
}

// ---------------------------------------------------------------------------
// The words
// ---------------------------------------------------------------------------

/// A drive's outcome word: the keyframe's kind word less its `drive_`.
const fn outcome_word(outcome: DriveOutcome) -> &'static str {
    match outcome {
        DriveOutcome::Arrived => "arrived",
        DriveOutcome::NoRoute => "no_route",
    }
}

/// A held slot's reason word (`crate::match_::cockpit`'s `held_detail`
/// `why`).
const fn why_word(why: &HeldWhy) -> &'static str {
    match why {
        HeldWhy::Odds { .. } => "odds",
        HeldWhy::Aspect { .. } => "aspect",
        HeldWhy::Range { .. } => "range",
        HeldWhy::Rounds { .. } => "rounds",
    }
}

/// A slot's target word — the keyframe's own
/// (`crate::match_::cockpit::arming_target_word`).
const fn target_word(target: SlotTarget) -> &'static str {
    crate::match_::cockpit::arming_target_word(target)
}

// ---------------------------------------------------------------------------
// The parity table
// ---------------------------------------------------------------------------

/// **Where the keyframe carries each getter's field** (17b §1 (ii), §5's feas
/// F5): `(type, getter, the keyframe's rendering)`. "the key of the same name"
/// is written as the key itself; every other row says how the same field
/// reads there. 17b cut M2.4's parity test reads every row against
/// `status_frame_held` on a live field.
pub const PARITY: &[(&str, &str, &str)] = &[
    ("View", "unit", "unit"),
    ("View", "callsign", "callsign"),
    ("View", "team", "the callsign's side (red / blue)"),
    ("View", "cell", "cell"),
    ("View", "x_cm", "x_cm"),
    ("View", "y_cm", "y_cm"),
    ("View", "z_cm", "z_cm"),
    ("View", "heading_mdeg", "heading_mdeg"),
    ("View", "speed_cms", "speed_cms"),
    ("View", "power", "power"),
    ("View", "held", "blocked"),
    ("View", "wreck", "wreck"),
    // Since 17b cut M4.6b the view also reads a hull with no authored speed
    // as immobile once its power is `Online` (the owner's STOP B ruling; doc
    // 17 §3 and D-UM-4 as amended), where the keyframe reads motion disabled
    // alone; the parity field fields no such hull.
    ("View", "immobile", "immobile"),
    ("View", "sight_m", "sight_m"),
    (
        "View",
        "ecm",
        "ecm (on / off read as true / false; null as ())",
    ),
    ("View", "receiver", "receiver"),
    ("View", "heat", "heat"),
    ("View", "sections", "sections"),
    (
        "View",
        "weapons",
        "weapons (absent without a weapons station: [])",
    ),
    (
        "View",
        "lock",
        "lock (the row key, read as the ContactId's INT)",
    ),
    ("View", "order", "order"),
    ("View", "t_ms", "t_ms"),
    ("View", "notices", "notices"),
    (
        "View",
        "contacts",
        "contacts (the class's rows of them: the lock and the order's contact seated, then not known friendly, then friends; read nearest-first)",
    ),
    ("View", "scan_tables", "scan_tables"),
    ("View", "odds_floor_permille", "odds_floor_permille"),
    ("View", "rules", "rules (all active slots, in slot order)"),
    (
        "Contact",
        "id",
        "id (the row key, read as the ContactId's INT)",
    ),
    ("Contact", "designator", "designator"),
    ("Contact", "alias", "alias (the number of its X<n> word)"),
    ("Contact", "identified", "class != unknown"),
    ("Contact", "friendly", "friendly"),
    ("Contact", "bearing_deg", "bearing_deg"),
    ("Contact", "relative_deg", "relative_deg"),
    ("Contact", "range_m", "range_m"),
    ("Contact", "band", "band"),
    ("Contact", "age_ms", "age_ms"),
    ("Contact", "cell", "cell"),
    ("Contact", "condition", "condition"),
    ("Contact", "moving", "moving"),
    ("Contact", "fatal", "fatal"),
    ("Contact", "aspect", "aspect"),
    ("Contact", "odds", "odds[weapon = the argument].p_permille"),
    (
        "Contact",
        "reads_dead",
        "reads_dead (absent without a weapons station: ())",
    ),
    ("Weapon", "index", "the array position"),
    ("Weapon", "id", "id"),
    ("Weapon", "ready", "ready"),
    ("Weapon", "ready_in_ms", "ready_in_ms"),
    ("Weapon", "rounds", "rounds"),
    ("Weapon", "rounds_max", "rounds_max"),
    (
        "Weapon",
        "range_band",
        "range_band.{index, of, max_range_cm, dispersion_mrad} as four INTs",
    ),
    ("Weapon", "p_permille", "p_permille"),
    ("Weapon", "bands_cm", "bands_cm"),
    ("Weapon", "heat_per_shot", "heat_per_shot"),
    ("Weapon", "armed", "armed"),
    ("Section", "id", "id"),
    ("Section", "condition", "condition"),
    ("Section", "internals", "internals"),
    ("Section", "internals_max", "internals_max"),
    ("Section", "facets", "facets"),
    ("Facet", "id", "id"),
    ("Facet", "armor", "armor"),
    ("Facet", "armor_max", "armor_max"),
    ("Heat", "heat", "heat"),
    ("Heat", "max", "max"),
    ("Heat", "overheated", "overheated"),
    ("Receiver", "budget", "budget"),
    ("Receiver", "budget_max", "budget_max"),
    (
        "Receiver",
        "reach_permille",
        "reach_percent x 10 (the keyframe's whole per cent, in per mille)",
    ),
    ("Order", "squad", "squad"),
    ("Order", "objective", "objective"),
    ("Order", "to", "to"),
    (
        "Order",
        "target",
        "target (the row whose designator it prints, read as its ContactId's INT)",
    ),
    ("Order", "facing_mdeg", "facing_mdeg"),
    (
        "Order",
        "of",
        "of (the row whose designator it prints, read as its ContactId's INT)",
    ),
    ("Order", "hold_fire", "hold_fire"),
    ("Order", "max_range_band", "max_range_band"),
    (
        "Order",
        "expires_ms",
        "expires_in_ms + t_ms, before the clamp at nought",
    ),
    ("Condition", "table", "table"),
    ("Condition", "bands", "bands"),
    ("Condition", "age_ms", "age_ms"),
    ("Armed", "slot", "slot"),
    ("Armed", "target", "target"),
    ("Armed", "min_permille", "min_permille"),
    ("Armed", "aspects", "aspects (the mask of its words)"),
    ("Armed", "keeps", "keeps"),
    ("Armed", "min_range_m", "min_range_m (null read as 0)"),
    (
        "Armed",
        "max_range_m",
        "max_range_m (null read as 4294967295)",
    ),
    ("Rule", "slot", "rules[].slot"),
    ("Rule", "action", "rules[].rule.action"),
    ("Rule", "channel", "rules[].rule.action's channel"),
    ("Rule", "weapon", "rules[].rule.weapon or condition.weapon"),
    ("Rule", "target", "rules[].rule.condition.target.target"),
    (
        "Rule",
        "contact",
        "rules[].rule.condition.target.id, as the ContactId's INT",
    ),
    ("Rule", "selector", "rules[].rule.selector"),
    (
        "Rule",
        "min_permille",
        "rules[].rule.condition.min_permille",
    ),
    ("Rule", "aspects", "rules[].rule.condition.aspects"),
    ("Rule", "keeps", "rules[].rule.condition.min_rounds_left"),
    ("Rule", "min_range_m", "rules[].rule.condition.min_range_m"),
    ("Rule", "max_range_m", "rules[].rule.condition.max_range_m"),
    ("Rule", "implicit", "rules[].rule.implicit"),
    (
        "Rule",
        "independent",
        "rules[].rule.independent (absent on non-fire rules: false)",
    ),
    ("Rule", "predicates", "rules[].rule.predicates"),
    ("Predicate", "kind", "rules[].rule.predicates[].predicate"),
    ("Predicate", "weapon", "rules[].rule.predicates[].weapon"),
    (
        "Predicate",
        "comparison",
        "rules[].rule.predicates[].comparison",
    ),
    (
        "Predicate",
        "value",
        "predicate's min_permille, mask, metres, amount, below_percent or above_percent",
    ),
    ("Predicate", "percent", "rules[].rule.predicates[].percent"),
    ("Predicate", "section", "rules[].rule.predicates[].section"),
    ("Predicate", "state", "rules[].rule.predicates[].state"),
    ("Predicate", "band", "rules[].rule.predicates[].band"),
    ("Predicate", "present", "rules[].rule.predicates[].present"),
    ("Notice", "kind", "kind"),
    (
        "Notice",
        "contact",
        "contact (the row key, read as the ContactId's INT)",
    ),
    ("Notice", "designator", "designator"),
    ("Notice", "was", "was"),
    ("Notice", "section", "section"),
    ("Notice", "cause", "cause"),
    ("Notice", "refusal", "refusal"),
    ("Notice", "order", "order"),
    ("Notice", "weapon", "weapon"),
    ("Notice", "radius_m", "detail.radius_m"),
    ("Notice", "secs", "detail.secs"),
    ("Notice", "heat", "detail.heat"),
    ("Notice", "max", "detail.max"),
    ("Notice", "floor_permille", "detail.floor_permille"),
    ("Notice", "why", "detail.why"),
    ("Notice", "floor", "detail.floor"),
    ("Notice", "range_m", "detail.range_m"),
    ("Notice", "min_m", "detail.min_m"),
    ("Notice", "max_m", "detail.max_m"),
    ("Notice", "rounds", "detail.rounds"),
    ("Notice", "keeps", "detail.keeps"),
    (
        "Notice",
        "friendly",
        "friendly (off the view's own rows; false for a row past them)",
    ),
    ("Notice", "spent", "spent"),
    ("Notice", "reason", "text (the miss's reason word)"),
    ("Notice", "outcome", "kind (drive_arrived / drive_no_route)"),
    ("Notice", "slot", "slot"),
    (
        "Notice",
        "percent",
        "percent (slot_fired), detail.percent (slot_held)",
    ),
    (
        "Notice",
        "aspect",
        "aspect (slot_fired), detail.aspect (slot_held)",
    ),
];

// ---------------------------------------------------------------------------
// The registrations
// ---------------------------------------------------------------------------

/// **The view's types and their getters** (the file head).
#[allow(clippy::too_many_lines)]
pub(crate) fn register(api: &mut Registrar<'_>) {
    custom_type!(api, View, "View");
    custom_type!(api, Contact, "Contact");
    custom_type!(api, Weapon, "Weapon");
    custom_type!(api, Section, "Section");
    custom_type!(api, Facet, "Facet");
    custom_type!(api, Heat, "Heat");
    custom_type!(api, Receiver, "Receiver");
    custom_type!(api, Order, "Order");
    custom_type!(api, Notice, "Notice");
    custom_type!(api, Condition, "Condition");
    custom_type!(api, Armed, "Armed");
    custom_type!(api, RuleView, "Rule");
    custom_type!(api, Predicate, "Predicate");
    register_view(api);
    register_contact(api);
    register_weapon(api);
    register_rules(api);
    register_own(api);
    register_order(api);
    register_notice(api);
}

/// `View`'s getters.
fn register_view(api: &mut Registrar<'_>) {
    getter!(api, "View", "unit", |v: &mut View| { int(v.0.unit) });
    getter!(api, "View", "callsign", |v: &mut View| {
        word(&v.0.callsign.to_string())
    });
    getter!(api, "View", "team", |v: &mut View| {
        word(v.0.team.as_str())
    });
    getter!(api, "View", "cell", |v: &mut View| { cell(v.0.cell) });
    getter!(api, "View", "x_cm", |v: &mut View| { int(v.0.x_cm) });
    getter!(api, "View", "y_cm", |v: &mut View| { int(v.0.y_cm) });
    getter!(api, "View", "z_cm", |v: &mut View| { int(v.0.z_cm) });
    getter!(api, "View", "heading_mdeg", |v: &mut View| {
        int(v.0.heading_mdeg)
    });
    getter!(api, "View", "speed_cms", |v: &mut View| {
        int(v.0.speed_cms)
    });
    getter!(api, "View", "power", |v: &mut View| {
        word(v.0.power.as_str())
    });
    getter!(api, "View", "held", |v: &mut View| {
        Dynamic::from_bool(v.0.held)
    });
    getter!(api, "View", "wreck", |v: &mut View| {
        Dynamic::from_bool(v.0.wreck)
    });
    getter!(api, "View", "immobile", |v: &mut View| {
        Dynamic::from_bool(v.0.immobile)
    });
    getter!(api, "View", "sight_m", |v: &mut View| { int(v.0.sight_m) });
    getter!(api, "View", "ecm", |v: &mut View| {
        opt(v.0.ecm, Dynamic::from_bool)
    });
    getter!(api, "View", "receiver", |v: &mut View| {
        opt(v.0.receiver, |receiver| Dynamic::from(Receiver(receiver)))
    });
    getter!(api, "View", "heat", |v: &mut View| {
        opt(v.0.heat, |heat| Dynamic::from(Heat(heat)))
    });
    getter!(api, "View", "sections", |v: &mut View| {
        Dynamic::from_array(
            indices(&v.0.sections)
                .map(|index| Dynamic::from(Section(Arc::clone(&v.0), index)))
                .collect(),
        )
    });
    getter!(api, "View", "weapons", |v: &mut View| {
        Dynamic::from_array(
            indices(&v.0.weapons)
                .map(|index| Dynamic::from(Weapon(Arc::clone(&v.0), index)))
                .collect(),
        )
    });
    getter!(api, "View", "rules", |v: &mut View| {
        Dynamic::from_array(
            indices(&v.0.rules)
                .map(|index| Dynamic::from(RuleView(Arc::clone(&v.0), index)))
                .collect(),
        )
    });
    getter!(api, "View", "lock", |v: &mut View| {
        opt(v.0.lock, |id| Dynamic::from_int(contact_int(id)))
    });
    getter!(api, "View", "order", |v: &mut View| {
        opt(v.0.order.as_ref(), |_| {
            Dynamic::from(Order(Arc::clone(&v.0)))
        })
    });
    getter!(api, "View", "t_ms", |v: &mut View| { int(v.0.t_ms) });
    getter!(api, "View", "notices", |v: &mut View| {
        Dynamic::from_array(
            indices(&v.0.notices)
                .map(|index| Dynamic::from(Notice(Arc::clone(&v.0), index)))
                .collect(),
        )
    });
    getter!(api, "View", "contacts", |v: &mut View| {
        Dynamic::from_array(
            indices(&v.0.contacts)
                .map(|index| Dynamic::from(Contact(Arc::clone(&v.0), index)))
                .collect(),
        )
    });
    getter!(api, "View", "scan_tables", |v: &mut View| {
        Dynamic::from_array(
            v.0.scan_tables
                .iter()
                .map(|table| words(table.iter()))
                .collect(),
        )
    });
    getter!(api, "View", "odds_floor_permille", |v: &mut View| {
        int(v.0.odds_floor_permille)
    });
}

/// `Contact`'s getters, and its `odds` method.
fn register_contact(api: &mut Registrar<'_>) {
    /// A getter over the row.
    fn on(c: &Contact, f: impl FnOnce(&ViewContact) -> Dynamic) -> Dynamic {
        opt(c.row(), f)
    }
    getter!(api, "Contact", "id", |c: &mut Contact| {
        on(c, |row| Dynamic::from_int(contact_int(row.id)))
    });
    getter!(api, "Contact", "designator", |c: &mut Contact| {
        on(c, |row| word(&row.designator.to_string()))
    });
    getter!(api, "Contact", "alias", |c: &mut Contact| {
        on(c, |row| opt(row.alias, int))
    });
    getter!(api, "Contact", "identified", |c: &mut Contact| {
        on(c, |row| {
            Dynamic::from_bool(!matches!(row.class, Classification::Unknown))
        })
    });
    getter!(api, "Contact", "friendly", |c: &mut Contact| {
        on(c, |row| Dynamic::from_bool(row.friendly))
    });
    getter!(api, "Contact", "bearing_deg", |c: &mut Contact| {
        on(c, |row| int(row.bearing_deg))
    });
    getter!(api, "Contact", "relative_deg", |c: &mut Contact| {
        on(c, |row| int(row.relative_deg))
    });
    getter!(api, "Contact", "range_m", |c: &mut Contact| {
        on(c, |row| int(row.range_m))
    });
    getter!(api, "Contact", "band", |c: &mut Contact| {
        on(c, |row| word(&row.band))
    });
    getter!(api, "Contact", "age_ms", |c: &mut Contact| {
        on(c, |row| int(row.age_ms))
    });
    getter!(api, "Contact", "cell", |c: &mut Contact| {
        on(c, |row| cell(row.cell))
    });
    getter!(api, "Contact", "condition", |c: &mut Contact| {
        on(c, |row| {
            opt(row.condition, |reading| Dynamic::from(Condition(reading)))
        })
    });
    getter!(api, "Contact", "moving", |c: &mut Contact| {
        on(c, |row| Dynamic::from_bool(row.moving))
    });
    getter!(api, "Contact", "fatal", |c: &mut Contact| {
        on(c, |row| {
            opt(row.fatal.as_ref(), |fatal| words(fatal.iter()))
        })
    });
    // D-UM-26: an aspect read is a read of the plot, fee 1.
    getter!(api, "Contact", "aspect", |c: &mut Contact| {
        charge(FEE_READ);
        on(c, |row| word(row.aspect.as_str()))
    });
    // D-UM-26: an odds read, fee 1 — the row's figure for that weapon, `()`
    // for a weapon the row carries none for (outside the odds rows, an
    // unidentified or friendly row, a hull with no weapons station).
    method!(
        api,
        "Contact",
        "odds",
        |c: &mut Contact, weapon: ImmutableString| {
            charge(FEE_READ);
            on(c, |row| {
                opt(
                    row.odds
                        .iter()
                        .find(|(id, _)| id.as_str() == weapon.as_str())
                        .map(|(_, p_permille)| *p_permille),
                    int,
                )
            })
        }
    );
    getter!(api, "Contact", "reads_dead", |c: &mut Contact| {
        on(c, |row| opt(row.reads_dead, Dynamic::from_bool))
    });
}

/// `Weapon`'s getters and `Armed`'s.
fn register_weapon(api: &mut Registrar<'_>) {
    /// A getter over the row.
    fn on(w: &Weapon, f: impl FnOnce(&FrameWeapon) -> Dynamic) -> Dynamic {
        opt(w.row(), f)
    }
    getter!(api, "Weapon", "index", |w: &mut Weapon| {
        on(w, |_| int(w.1))
    });
    getter!(api, "Weapon", "id", |w: &mut Weapon| {
        on(w, |row| word(&row.id))
    });
    getter!(api, "Weapon", "ready", |w: &mut Weapon| {
        on(w, |row| Dynamic::from_bool(row.ready))
    });
    getter!(api, "Weapon", "ready_in_ms", |w: &mut Weapon| {
        on(w, |row| int(row.ready_in_ms))
    });
    getter!(api, "Weapon", "rounds", |w: &mut Weapon| {
        on(w, |row| int(row.rounds))
    });
    getter!(api, "Weapon", "rounds_max", |w: &mut Weapon| {
        on(w, |row| int(row.rounds_max))
    });
    getter!(api, "Weapon", "range_band", |w: &mut Weapon| {
        on(w, |row| {
            opt(row.range_band, |band| {
                Dynamic::from_array(vec![
                    int(band.index),
                    int(band.of),
                    int(band.max_range_cm),
                    int(band.dispersion_mrad),
                ])
            })
        })
    });
    getter!(api, "Weapon", "p_permille", |w: &mut Weapon| {
        on(w, |row| opt(row.p_permille, int))
    });
    getter!(api, "Weapon", "bands_cm", |w: &mut Weapon| {
        on(w, |row| {
            Dynamic::from_array(row.bands_cm.iter().copied().map(int).collect())
        })
    });
    getter!(api, "Weapon", "heat_per_shot", |w: &mut Weapon| {
        on(w, |row| int(row.heat_per_shot))
    });
    getter!(api, "Weapon", "armed", |w: &mut Weapon| {
        on(w, |row| {
            opt(row.armed.clone(), |armed| Dynamic::from(Armed(armed)))
        })
    });

    getter!(api, "Armed", "slot", |a: &mut Armed| { int(a.0.slot) });
    getter!(api, "Armed", "target", |a: &mut Armed| {
        word(target_word(a.0.target))
    });
    getter!(api, "Armed", "min_permille", |a: &mut Armed| {
        int(a.0.min_permille)
    });
    getter!(api, "Armed", "aspects", |a: &mut Armed| {
        int(a.0.aspects)
    });
    getter!(api, "Armed", "keeps", |a: &mut Armed| { int(a.0.keeps) });
    getter!(api, "Armed", "min_range_m", |a: &mut Armed| {
        int(a.0.min_range_m)
    });
    getter!(api, "Armed", "max_range_m", |a: &mut Armed| {
        int(a.0.max_range_m)
    });
}

/// The complete written station rules. Every getter is a primitive or a
/// bounded array of predicate records, read from the same rows as the keyframe.
#[allow(clippy::too_many_lines)]
fn register_rules(api: &mut Registrar<'_>) {
    fn on(r: &RuleView, f: impl FnOnce(&FrameRule) -> Dynamic) -> Dynamic {
        opt(r.row(), f)
    }
    getter!(api, "Rule", "slot", |r: &mut RuleView| {
        on(r, |row| int(row.slot))
    });
    getter!(api, "Rule", "action", |r: &mut RuleView| {
        on(r, |row| {
            word(match &row.rule {
                Rule::Fire { .. } => "fire",
                Rule::HoldFire { .. } => "hold_fire",
                Rule::Lock { .. } => "lock",
            })
        })
    });
    getter!(api, "Rule", "channel", |r: &mut RuleView| {
        on(r, |row| word(row.rule.channel().as_str()))
    });
    getter!(api, "Rule", "weapon", |r: &mut RuleView| {
        on(r, |row| opt(row.rule.weapon(), |weapon| word(weapon)))
    });
    getter!(api, "Rule", "target", |r: &mut RuleView| {
        on(r, |row| {
            opt(row.rule.fire(), |fire| {
                word(match fire.target {
                    SlotTarget::Lock => "lock",
                    SlotTarget::Contact { .. } => "contact",
                    SlotTarget::BestHostile => "best_hostile",
                })
            })
        })
    });
    getter!(api, "Rule", "contact", |r: &mut RuleView| {
        on(r, |row| match row.rule.fire().map(|fire| fire.target) {
            Some(SlotTarget::Contact { id }) => Dynamic::from_int(contact_int(id)),
            _ => Dynamic::UNIT,
        })
    });
    getter!(api, "Rule", "selector", |r: &mut RuleView| {
        on(r, |row| match &row.rule {
            Rule::Lock { selector, .. } => word(match selector {
                LockSelector::Nearest => "nearest",
                LockSelector::BestOdds => "best_odds",
                LockSelector::Weakest => "weakest",
                LockSelector::Flank => "flank",
                LockSelector::Newest => "newest",
            }),
            _ => Dynamic::UNIT,
        })
    });
    getter!(api, "Rule", "min_permille", |r: &mut RuleView| {
        on(r, |row| opt(row.rule.fire(), |fire| int(fire.min_permille)))
    });
    getter!(api, "Rule", "aspects", |r: &mut RuleView| {
        on(r, |row| opt(row.rule.fire(), |fire| int(fire.aspects)))
    });
    getter!(api, "Rule", "keeps", |r: &mut RuleView| {
        on(r, |row| {
            opt(row.rule.fire(), |fire| int(fire.min_rounds_left))
        })
    });
    getter!(api, "Rule", "min_range_m", |r: &mut RuleView| {
        on(r, |row| opt(row.rule.fire(), |fire| int(fire.min_range_m)))
    });
    getter!(api, "Rule", "max_range_m", |r: &mut RuleView| {
        on(r, |row| opt(row.rule.fire(), |fire| int(fire.max_range_m)))
    });
    getter!(api, "Rule", "implicit", |r: &mut RuleView| {
        on(r, |row| {
            Dynamic::from_bool(match &row.rule {
                Rule::Fire { implicit, .. }
                | Rule::HoldFire { implicit, .. }
                | Rule::Lock { implicit, .. } => *implicit,
            })
        })
    });
    getter!(api, "Rule", "independent", |r: &mut RuleView| {
        on(r, |row| {
            Dynamic::from_bool(matches!(
                &row.rule,
                Rule::Fire {
                    independent: true,
                    ..
                }
            ))
        })
    });
    getter!(api, "Rule", "predicates", |r: &mut RuleView| {
        on(r, |row| {
            Dynamic::from_array(
                row.rule
                    .predicates()
                    .iter()
                    .cloned()
                    .map(|predicate| Dynamic::from(Predicate(predicate)))
                    .collect(),
            )
        })
    });
    getter!(api, "Predicate", "kind", |p: &mut Predicate| {
        word(match &p.0 {
            RulePredicate::Odds { .. } => "odds",
            RulePredicate::Aspect { .. } => "aspect",
            RulePredicate::Range { .. } => "range",
            RulePredicate::Band { .. } => "band",
            RulePredicate::Target { .. } => "target",
            RulePredicate::TargetSection { .. } => "target_section",
            RulePredicate::Rounds { .. } => "rounds",
            RulePredicate::OwnSection { .. } => "own_section",
            RulePredicate::Heat { .. } => "heat",
            RulePredicate::Lock { .. } => "lock",
        })
    });
    getter!(api, "Predicate", "weapon", |p: &mut Predicate| {
        match &p.0 {
            RulePredicate::Odds { weapon, .. } | RulePredicate::Rounds { weapon, .. } => {
                opt(weapon.as_ref(), |weapon| word(weapon))
            }
            _ => Dynamic::UNIT,
        }
    });
    getter!(api, "Predicate", "comparison", |p: &mut Predicate| {
        match &p.0 {
            RulePredicate::Range { comparison, .. } | RulePredicate::Rounds { comparison, .. } => {
                word(match comparison {
                    Comparison::Less => "less",
                    Comparison::Greater => "greater",
                    Comparison::AtLeast => "at_least",
                    Comparison::Equal => "equal",
                })
            }
            _ => Dynamic::UNIT,
        }
    });
    getter!(api, "Predicate", "value", |p: &mut Predicate| {
        match &p.0 {
            RulePredicate::Odds { min_permille, .. } => int(*min_permille),
            RulePredicate::Aspect { mask } => int(*mask),
            RulePredicate::Range { metres, .. } => int(*metres),
            RulePredicate::Rounds { amount, .. } => int(*amount),
            RulePredicate::OwnSection { below_percent, .. } => int(*below_percent),
            RulePredicate::Heat { above_percent } => int(*above_percent),
            _ => Dynamic::UNIT,
        }
    });
    getter!(api, "Predicate", "percent", |p: &mut Predicate| {
        match &p.0 {
            RulePredicate::Rounds { percent, .. } => Dynamic::from_bool(*percent),
            _ => Dynamic::UNIT,
        }
    });
    getter!(api, "Predicate", "section", |p: &mut Predicate| {
        match &p.0 {
            RulePredicate::TargetSection { section, .. }
            | RulePredicate::OwnSection { section, .. } => word(section),
            _ => Dynamic::UNIT,
        }
    });
    getter!(api, "Predicate", "state", |p: &mut Predicate| {
        match &p.0 {
            RulePredicate::Target { state } => word(match state {
                TargetState::Identified => "identified",
                TargetState::Moving => "moving",
                TargetState::Stopped => "stopped",
                TargetState::Hurt => "hurt",
            }),
            RulePredicate::TargetSection { state, .. } => word(match state {
                SectionState::Green => "green",
                SectionState::Yellow => "yellow",
                SectionState::Red => "red",
                SectionState::Destroyed => "destroyed",
            }),
            _ => Dynamic::UNIT,
        }
    });
    getter!(api, "Predicate", "band", |p: &mut Predicate| {
        match &p.0 {
            RulePredicate::Band { band } => word(match band {
                RuleBand::Close => "close",
                RuleBand::Effective => "effective",
                RuleBand::Far => "far",
            }),
            _ => Dynamic::UNIT,
        }
    });
    getter!(api, "Predicate", "present", |p: &mut Predicate| {
        match &p.0 {
            RulePredicate::Lock { present } => Dynamic::from_bool(*present),
            _ => Dynamic::UNIT,
        }
    });
}

/// The hull's own records: `Section`, `Facet`, `Heat`, `Receiver`, and a
/// contact's `Condition`.
fn register_own(api: &mut Registrar<'_>) {
    /// A getter over the section.
    fn on(s: &Section, f: impl FnOnce(&FrameSection) -> Dynamic) -> Dynamic {
        opt(s.row(), f)
    }
    getter!(api, "Section", "id", |s: &mut Section| {
        on(s, |row| word(&row.id))
    });
    getter!(api, "Section", "condition", |s: &mut Section| {
        on(s, |row| word(&row.condition))
    });
    getter!(api, "Section", "internals", |s: &mut Section| {
        on(s, |row| opt(row.internals, |(current, _)| int(current)))
    });
    getter!(api, "Section", "internals_max", |s: &mut Section| {
        on(s, |row| opt(row.internals, |(_, max)| int(max)))
    });
    getter!(api, "Section", "facets", |s: &mut Section| {
        on(s, |row| {
            Dynamic::from_array(
                indices(&row.facets)
                    .map(|index| Dynamic::from(Facet(Arc::clone(&s.0), s.1, index)))
                    .collect(),
            )
        })
    });

    getter!(api, "Facet", "id", |f: &mut Facet| {
        opt(f.row(), |row| word(&row.id))
    });
    getter!(api, "Facet", "armor", |f: &mut Facet| {
        opt(f.row(), |row| int(row.armor))
    });
    getter!(api, "Facet", "armor_max", |f: &mut Facet| {
        opt(f.row(), |row| int(row.armor_max))
    });

    getter!(api, "Heat", "heat", |h: &mut Heat| { int(h.0.heat) });
    getter!(api, "Heat", "max", |h: &mut Heat| { int(h.0.max) });
    getter!(api, "Heat", "overheated", |h: &mut Heat| {
        Dynamic::from_bool(h.0.overheated())
    });

    getter!(api, "Receiver", "budget", |r: &mut Receiver| {
        int(r.0.budget)
    });
    getter!(api, "Receiver", "budget_max", |r: &mut Receiver| {
        int(r.0.budget_max)
    });
    // At the keyframe's resolution (D-UM-4: a tier never extends the view):
    // the keyframe prints `reach_percent`, the fold's per mille floored to a
    // whole per cent, so the mind reads that figure back in per mille — 755
    // reads 750, as the pilot's 75% — and never the digit the pilot is not
    // shown.
    getter!(api, "Receiver", "reach_permille", |r: &mut Receiver| {
        int(r.0.reach_permille / 10 * 10)
    });

    getter!(api, "Condition", "table", |c: &mut Condition| {
        int(c.0.table)
    });
    getter!(api, "Condition", "bands", |c: &mut Condition| {
        word(&c.0.bands.letters())
    });
    getter!(api, "Condition", "age_ms", |c: &mut Condition| {
        int(c.0.age_ms)
    });
}

/// `Order`'s getters: the objective's payloads where it carries them, `()`
/// elsewhere, as the keyframe's `order_json` writes `null`.
fn register_order(api: &mut Registrar<'_>) {
    /// A getter over the order.
    fn on(o: &Order, f: impl FnOnce(&crate::arena::perception::FrameOrder) -> Dynamic) -> Dynamic {
        opt(o.0.order.as_ref(), f)
    }
    getter!(api, "Order", "squad", |o: &mut Order| {
        on(o, |order| word(&order.squad))
    });
    getter!(api, "Order", "objective", |o: &mut Order| {
        on(o, |order| word(order.objective.as_str()))
    });
    getter!(api, "Order", "to", |o: &mut Order| {
        on(o, |order| match order.objective {
            MindObjective::Advance { to } => cell(Some(to)),
            _ => Dynamic::UNIT,
        })
    });
    getter!(api, "Order", "target", |o: &mut Order| {
        on(o, |order| match order.objective {
            MindObjective::Engage { target } => {
                opt(target, |id| Dynamic::from_int(contact_int(id)))
            }
            _ => Dynamic::UNIT,
        })
    });
    getter!(api, "Order", "facing_mdeg", |o: &mut Order| {
        on(o, |order| match order.objective {
            MindObjective::Screen { facing_mdeg } => int(facing_mdeg),
            _ => Dynamic::UNIT,
        })
    });
    getter!(api, "Order", "of", |o: &mut Order| {
        on(o, |order| match order.objective {
            MindObjective::Support { of } => opt(of, |id| Dynamic::from_int(contact_int(id))),
            _ => Dynamic::UNIT,
        })
    });
    getter!(api, "Order", "hold_fire", |o: &mut Order| {
        on(o, |order| Dynamic::from_bool(order.hold_fire))
    });
    getter!(api, "Order", "max_range_band", |o: &mut Order| {
        on(o, |order| {
            opt(order.max_range_band, |band| word(band.as_str()))
        })
    });
    getter!(api, "Order", "expires_ms", |o: &mut Order| {
        on(o, |order| int(order.expires_ms))
    });
}

/// `Notice`'s getters: one per field a `FrameNotice` variant carries that
/// the keyframe prints, `()` on a notice that carries none.
#[allow(clippy::too_many_lines)]
fn register_notice(api: &mut Registrar<'_>) {
    /// A getter over the notice.
    fn on(n: &Notice, f: impl FnOnce(&FrameNotice) -> Dynamic) -> Dynamic {
        opt(n.row(), f)
    }
    getter!(api, "Notice", "kind", |n: &mut Notice| {
        on(n, |notice| word(notice.kind().as_str()))
    });
    getter!(api, "Notice", "contact", |n: &mut Notice| {
        on(n, |notice| {
            opt(notice.contact(), |id| Dynamic::from_int(contact_int(id)))
        })
    });
    getter!(api, "Notice", "designator", |n: &mut Notice| {
        on(n, |notice| {
            opt(notice.designator(), |named| word(&named.to_string()))
        })
    });
    getter!(api, "Notice", "was", |n: &mut Notice| {
        on(n, |notice| opt(notice.was(), |was| word(&was.to_string())))
    });
    getter!(api, "Notice", "section", |n: &mut Notice| {
        on(n, |notice| opt(notice.section(), |section| word(section)))
    });
    getter!(api, "Notice", "cause", |n: &mut Notice| {
        on(n, |notice| {
            opt(notice.cause(), |cause| word(cause.as_str()))
        })
    });
    getter!(api, "Notice", "refusal", |n: &mut Notice| {
        on(n, |notice| {
            opt(notice.refusal(), |refusal| word(refusal.as_str()))
        })
    });
    getter!(api, "Notice", "order", |n: &mut Notice| {
        on(n, |notice| opt(notice.order(), |order| word(order)))
    });
    getter!(api, "Notice", "weapon", |n: &mut Notice| {
        on(n, |notice| opt(notice.weapon(), |weapon| word(weapon)))
    });
    getter!(api, "Notice", "radius_m", |n: &mut Notice| {
        on(n, |notice| match notice {
            FrameNotice::Emcon { radius_m, .. } => int(*radius_m),
            _ => Dynamic::UNIT,
        })
    });
    getter!(api, "Notice", "secs", |n: &mut Notice| {
        on(n, |notice| match notice.detail() {
            RefusalDetail::Cycling { secs } => int(secs),
            _ => Dynamic::UNIT,
        })
    });
    getter!(api, "Notice", "heat", |n: &mut Notice| {
        on(n, |notice| match notice.detail() {
            RefusalDetail::Overheated { heat, .. } => int(heat),
            _ => Dynamic::UNIT,
        })
    });
    getter!(api, "Notice", "max", |n: &mut Notice| {
        on(n, |notice| match notice.detail() {
            RefusalDetail::Overheated { max, .. } => int(max),
            _ => Dynamic::UNIT,
        })
    });
    getter!(api, "Notice", "floor_permille", |n: &mut Notice| {
        on(n, |notice| match notice.detail() {
            RefusalDetail::Floor { permille } => int(permille),
            _ => Dynamic::UNIT,
        })
    });
    getter!(api, "Notice", "why", |n: &mut Notice| {
        on(n, |notice| match notice {
            FrameNotice::SlotHeld { why, .. } => word(why_word(why)),
            _ => Dynamic::UNIT,
        })
    });
    getter!(api, "Notice", "floor", |n: &mut Notice| {
        on(n, |notice| match notice {
            FrameNotice::SlotHeld {
                why: HeldWhy::Odds { floor, .. },
                ..
            } => int(floor / 10),
            _ => Dynamic::UNIT,
        })
    });
    getter!(api, "Notice", "range_m", |n: &mut Notice| {
        on(n, |notice| match notice {
            FrameNotice::SlotHeld {
                why: HeldWhy::Range { range_m, .. },
                ..
            } => int(*range_m),
            _ => Dynamic::UNIT,
        })
    });
    getter!(api, "Notice", "min_m", |n: &mut Notice| {
        on(n, |notice| match notice {
            FrameNotice::SlotHeld {
                why: HeldWhy::Range { min_m, .. },
                ..
            } => int(*min_m),
            _ => Dynamic::UNIT,
        })
    });
    getter!(api, "Notice", "max_m", |n: &mut Notice| {
        on(n, |notice| match notice {
            FrameNotice::SlotHeld {
                why: HeldWhy::Range { max_m, .. },
                ..
            } => int(*max_m),
            _ => Dynamic::UNIT,
        })
    });
    getter!(api, "Notice", "rounds", |n: &mut Notice| {
        on(n, |notice| match notice {
            FrameNotice::SlotHeld {
                why: HeldWhy::Rounds { rounds, .. },
                ..
            } => int(*rounds),
            _ => Dynamic::UNIT,
        })
    });
    getter!(api, "Notice", "keeps", |n: &mut Notice| {
        on(n, |notice| match notice {
            FrameNotice::SlotHeld {
                why: HeldWhy::Rounds { keeps, .. },
                ..
            } => int(*keeps),
            _ => Dynamic::UNIT,
        })
    });
    // The keyframe reads `friendly` off the frame's own rows; the view holds
    // the class's `rows` of them (17b cut M4.5c: the lock and the order's
    // contact seated, then every row not known friendly, then the friends, so
    // the rows a full view cuts are friends first), and a row past those
    // reads false here.
    getter!(api, "Notice", "friendly", |n: &mut Notice| {
        on(n, |notice| {
            Dynamic::from_bool(
                notice
                    .contact()
                    .is_some_and(|id| n.0.contacts.iter().any(|row| row.id == id && row.friendly)),
            )
        })
    });
    getter!(api, "Notice", "spent", |n: &mut Notice| {
        on(n, |notice| Dynamic::from_bool(notice.spent()))
    });
    getter!(api, "Notice", "reason", |n: &mut Notice| {
        on(n, |notice| match notice {
            FrameNotice::Miss { reason, .. } => word(reason),
            _ => Dynamic::UNIT,
        })
    });
    getter!(api, "Notice", "outcome", |n: &mut Notice| {
        on(n, |notice| match notice {
            FrameNotice::Drive { outcome } => word(outcome_word(*outcome)),
            _ => Dynamic::UNIT,
        })
    });
    getter!(api, "Notice", "slot", |n: &mut Notice| {
        on(n, |notice| match notice {
            FrameNotice::SlotFired { slot, .. } | FrameNotice::SlotHeld { slot, .. } => int(*slot),
            _ => Dynamic::UNIT,
        })
    });
    getter!(api, "Notice", "percent", |n: &mut Notice| {
        on(n, |notice| match notice {
            FrameNotice::SlotFired { p_permille, .. } => int(p_permille / 10),
            FrameNotice::SlotHeld {
                why: HeldWhy::Odds { permille, .. },
                ..
            } => int(permille / 10),
            _ => Dynamic::UNIT,
        })
    });
    getter!(api, "Notice", "aspect", |n: &mut Notice| {
        on(n, |notice| match notice {
            FrameNotice::SlotFired { aspect, .. }
            | FrameNotice::SlotHeld {
                why: HeldWhy::Aspect { aspect, .. },
                ..
            } => word(aspect.as_str()),
            _ => Dynamic::UNIT,
        })
    });
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::super::fixture::{self, HOSTILE, eval, shown};
    use super::super::registered_names;
    use super::*;

    /// 17b §2's M2.3 test 1: **every custom type is `Clone + Send + Sync`**
    /// (the `sync` engine shares them across the pool's workers), and the
    /// handles share the one view: reading `contacts` clones the `Arc` once a
    /// row and copies no row.
    #[test]
    fn every_custom_type_is_shared_and_sendable() {
        fn shared<T: Clone + Send + Sync + 'static>() {}
        shared::<View>();
        shared::<Contact>();
        shared::<Weapon>();
        shared::<Section>();
        shared::<Facet>();
        shared::<Notice>();
        shared::<Order>();
        shared::<Heat>();
        shared::<Receiver>();
        shared::<Condition>();
        shared::<Armed>();
        shared::<RuleView>();
        shared::<Predicate>();
        shared::<super::super::card::Card>();
        shared::<super::super::kernels::Ranges>();
        shared::<super::super::kernels::Walk>();

        let view = Arc::new(fixture::view());
        let mut scope = mush_mind::rhai::Scope::new();
        scope.push("v", View(Arc::clone(&view)));
        assert_eq!(Arc::strong_count(&view), 2);
        let rows = fixture::scratch()
            .eval_with_scope::<Array>(&mut scope, "v.contacts")
            .expect("the contacts");
        assert_eq!(rows.len(), 3);
        // The view, the scope's handle and one handle per row: no copy.
        assert_eq!(Arc::strong_count(&view), 2 + 3);
        let first = rows
            .first()
            .and_then(|row| row.clone().try_cast::<Contact>())
            .expect("a Contact handle");
        assert!(Arc::ptr_eq(&first.0, &view));
        assert_eq!(first.1, 0);
        drop(rows);
        drop(first);
        assert_eq!(Arc::strong_count(&view), 2);
    }

    /// **Every getter answers its view field** at the keyframe's resolution:
    /// each getter read once on the full view, the INTs, words, cells and
    /// `()`s it answers pinned — every type's every getter, so the parity
    /// table's rows each have a read here.
    #[test]
    #[allow(clippy::too_many_lines)]
    fn every_getter_answers_its_view_field() {
        let hostile = contact_int(HOSTILE).to_string();
        let reads: Vec<(&str, String)> = vec![
            ("v.unit", "4".into()),
            ("v.callsign", "\"RA\"".into()),
            ("v.team", "\"red\"".into()),
            ("v.cell", "[1, 4]".into()),
            ("v.x_cm", "1500".into()),
            ("v.y_cm", "4500".into()),
            ("v.z_cm", "0".into()),
            ("v.heading_mdeg", "90000".into()),
            ("v.speed_cms", "120".into()),
            ("v.power", "\"online\"".into()),
            ("v.held", "false".into()),
            ("v.wreck", "false".into()),
            ("v.immobile", "false".into()),
            ("v.sight_m", "800".into()),
            ("v.ecm", "false".into()),
            ("type_of(v.receiver)", "\"Receiver\"".into()),
            ("type_of(v.heat)", "\"Heat\"".into()),
            ("v.sections.len()", "1".into()),
            ("v.weapons.len()", "2".into()),
            ("v.rules.len()", "1".into()),
            ("v.lock", hostile.clone()),
            ("type_of(v.order)", "\"Order\"".into()),
            ("v.t_ms", "5000".into()),
            ("v.notices.len()", "14".into()),
            ("v.contacts.len()", "3".into()),
            ("v.scan_tables", "[[\"hull\", \"turret\"]]".into()),
            ("v.odds_floor_permille", "100".into()),
            // Contacts: the hostile, the friend, the unidentified return.
            ("v.contacts[0].id", hostile.clone()),
            ("v.contacts[1].id", "34".into()),
            ("v.contacts[0].designator", "\"BB\"".into()),
            ("v.contacts[2].designator", "\"X5\"".into()),
            ("v.contacts[0].alias", "3".into()),
            ("v.contacts[1].alias", "()".into()),
            ("v.contacts[0].identified", "true".into()),
            ("v.contacts[2].identified", "false".into()),
            ("v.contacts[0].friendly", "false".into()),
            ("v.contacts[1].friendly", "true".into()),
            ("v.contacts[0].bearing_deg", "90".into()),
            ("v.contacts[1].relative_deg", "270".into()),
            ("v.contacts[0].range_m", "300".into()),
            ("v.contacts[0].band", "\"near\"".into()),
            ("v.contacts[2].age_ms", "2000".into()),
            ("v.contacts[0].cell", "[4, 4]".into()),
            ("v.contacts[2].cell", "()".into()),
            ("type_of(v.contacts[0].condition)", "\"Condition\"".into()),
            ("v.contacts[1].condition", "()".into()),
            ("v.contacts[0].fatal", "[\"hull\"]".into()),
            ("v.contacts[1].fatal", "()".into()),
            ("v.contacts[0].aspect", "\"left\"".into()),
            ("v.contacts[0].odds(\"cannon\")", "620".into()),
            ("v.contacts[0].odds(\"coax\")", "()".into()),
            ("v.contacts[0].reads_dead", "false".into()),
            ("v.contacts[2].reads_dead", "()".into()),
            // Weapons.
            ("v.weapons[1].index", "1".into()),
            ("v.weapons[0].id", "\"cannon\"".into()),
            ("v.weapons[0].ready", "true".into()),
            ("v.weapons[1].ready_in_ms", "700".into()),
            ("v.weapons[0].rounds", "18".into()),
            ("v.weapons[0].rounds_max", "20".into()),
            ("v.weapons[0].range_band", "[1, 2, 40000, 3]".into()),
            ("v.weapons[1].range_band", "()".into()),
            ("v.weapons[0].p_permille", "620".into()),
            ("v.weapons[1].p_permille", "()".into()),
            ("v.weapons[0].bands_cm", "[40000, 80000]".into()),
            ("v.weapons[0].heat_per_shot", "4".into()),
            ("v.weapons[1].armed", "()".into()),
            ("v.weapons[0].armed.slot", "1".into()),
            ("v.weapons[0].armed.target", "\"best\"".into()),
            ("v.weapons[0].armed.min_permille", "100".into()),
            ("v.weapons[0].armed.aspects", "15".into()),
            ("v.weapons[0].armed.keeps", "2".into()),
            ("v.weapons[0].armed.min_range_m", "0".into()),
            ("v.weapons[0].armed.max_range_m", "420".into()),
            // The hull's own records.
            ("v.sections[0].id", "\"hull\"".into()),
            ("v.sections[0].condition", "\"damaged\"".into()),
            ("v.sections[0].internals", "40".into()),
            ("v.sections[0].internals_max", "50".into()),
            ("v.sections[0].facets.len()", "2".into()),
            ("v.sections[0].facets[1].id", "\"rear\"".into()),
            ("v.sections[0].facets[1].armor", "30".into()),
            ("v.sections[0].facets[1].armor_max", "60".into()),
            ("v.heat.heat", "10".into()),
            ("v.heat.max", "100".into()),
            ("v.heat.overheated", "false".into()),
            ("v.receiver.budget", "3".into()),
            ("v.receiver.budget_max", "4".into()),
            // The fixture's 755 at the keyframe's resolution (75%).
            ("v.receiver.reach_permille", "750".into()),
            ("v.contacts[0].condition.table", "0".into()),
            ("v.contacts[0].moving", "false".into()),
            ("v.contacts[0].condition.bands", "\"gy\"".into()),
            ("v.contacts[0].condition.age_ms", "1000".into()),
            // The order: an Engage on the hostile.
            ("v.order.squad", "\"red-1\"".into()),
            ("v.order.objective", "\"engage\"".into()),
            ("v.order.to", "()".into()),
            ("v.order.target", hostile.clone()),
            ("v.order.facing_mdeg", "()".into()),
            ("v.order.of", "()".into()),
            ("v.order.hold_fire", "false".into()),
            ("v.order.max_range_band", "\"effective\"".into()),
            ("v.order.expires_ms", "9000".into()),
            // Notices, in the fixture's order.
            ("v.notices[0].kind", "\"miss\"".into()),
            ("v.notices[0].contact", hostile.clone()),
            ("v.notices[0].designator", "\"BB\"".into()),
            ("v.notices[0].reason", "\"blocked\"".into()),
            ("v.notices[0].friendly", "false".into()),
            ("v.notices[1].kind", "\"drive_arrived\"".into()),
            ("v.notices[1].outcome", "\"arrived\"".into()),
            ("v.notices[1].reason", "()".into()),
            ("v.notices[2].refusal", "\"cycling\"".into()),
            ("v.notices[2].order", "\"fire\"".into()),
            ("v.notices[2].weapon", "\"cannon\"".into()),
            ("v.notices[2].secs", "2".into()),
            ("v.notices[3].radius_m", "300".into()),
            ("v.notices[4].slot", "1".into()),
            ("v.notices[4].percent", "62".into()),
            ("v.notices[4].aspect", "\"left\"".into()),
            ("v.notices[5].why", "\"odds\"".into()),
            ("v.notices[5].percent", "8".into()),
            ("v.notices[5].floor", "10".into()),
            ("v.notices[5].slot", "2".into()),
            ("v.notices[6].was", "\"X3\"".into()),
            ("v.notices[6].kind", "\"classified\"".into()),
            ("v.notices[7].section", "\"turret\"".into()),
            ("v.notices[7].spent", "true".into()),
            ("v.notices[8].cause", "\"occupied\"".into()),
            ("v.notices[8].contact", "()".into()),
            ("v.notices[9].heat", "100".into()),
            ("v.notices[9].max", "100".into()),
            ("v.notices[10].floor_permille", "100".into()),
            ("v.notices[11].range_m", "500".into()),
            ("v.notices[11].min_m", "0".into()),
            ("v.notices[11].max_m", "420".into()),
            ("v.notices[12].rounds", "2".into()),
            ("v.notices[12].keeps", "2".into()),
            ("v.notices[13].aspect", "\"front\"".into()),
            ("v.notices[13].why", "\"aspect\"".into()),
        ];
        for (expr, want) in &reads {
            let got = eval(expr).unwrap_or_else(|error| panic!("{expr}: {error}"));
            assert_eq!(&shown(&got), want, "{expr}");
        }
        // Every getter of every view type is read above.
        for (ty, getter, _) in PARITY {
            if matches!(*ty, "Rule" | "Predicate") {
                // These require authored rules and are read by the dedicated
                // complete-rule projection fixture below.
                continue;
            }
            let dotted = format!(".{getter}");
            assert!(
                reads.iter().any(|(expr, _)| expr.contains(&dotted)),
                "{ty}.{getter} is never read"
            );
        }
    }

    /// **Every view getter has a `PARITY` row, and every row a getter**
    /// (17b §2's M2.3 test 9's second half): the table is the parity
    /// assertion 17b cut M2.4 reads, so a getter without a row is a getter
    /// whose keyframe rendering nobody wrote down.
    #[test]
    fn every_view_getter_has_a_parity_row() {
        let view_types = [
            "View",
            "Contact",
            "Weapon",
            "Section",
            "Facet",
            "Heat",
            "Receiver",
            "Order",
            "Notice",
            "Condition",
            "Armed",
            "Rule",
            "Predicate",
        ];
        let registered: BTreeSet<String> = registered_names()
            .into_iter()
            .filter(|name| {
                name.split_once('.')
                    .is_some_and(|(ty, _)| view_types.contains(&ty))
            })
            .collect();
        let rows: BTreeSet<String> = PARITY
            .iter()
            .map(|(ty, getter, _)| format!("{ty}.{getter}"))
            .collect();
        assert_eq!(rows.len(), PARITY.len(), "a parity row written twice");
        // The table's size, for 17b cut M2.4's parity test to count on.
        assert_eq!(PARITY.len(), 140);
        assert_eq!(
            registered, rows,
            "a view getter without a parity row, or a row without a getter"
        );
        for (ty, getter, rendering) in PARITY {
            assert!(!rendering.is_empty(), "{ty}.{getter} names no rendering");
        }
    }

    /// All action kinds and predicate payloads cross the same observation
    /// boundary. Slot gaps and two fire rules for one weapon stay visible;
    /// fields belonging to another action or predicate return unit, not a
    /// guessed default or a target-state lookup.
    #[test]
    #[allow(clippy::too_many_lines)]
    fn ordered_rules_and_their_predicates_are_read_at_keyframe_resolution() {
        use crate::arena::conditions::FireCondition;

        let mut seen = fixture::view();
        let fire = FireCondition {
            weapon: "cannon".into(),
            target: SlotTarget::Contact { id: HOSTILE },
            min_permille: 600,
            aspects: 14,
            min_rounds_left: 2,
            min_range_m: 50,
            max_range_m: 420,
        };
        seen.rules = vec![
            FrameRule {
                slot: 1,
                rule: Rule::Fire {
                    condition: fire.clone(),
                    predicates: vec![
                        RulePredicate::Odds {
                            weapon: Some("cannon".into()),
                            min_permille: 600,
                        },
                        RulePredicate::Aspect { mask: 14 },
                        RulePredicate::Range {
                            comparison: Comparison::Less,
                            metres: 421,
                        },
                        RulePredicate::Target {
                            state: TargetState::Moving,
                        },
                    ],
                    implicit: false,
                    independent: false,
                },
            },
            FrameRule {
                slot: 3,
                rule: Rule::HoldFire {
                    weapon: "cannon".into(),
                    predicates: vec![
                        RulePredicate::OwnSection {
                            section: "hull".into(),
                            below_percent: 50,
                        },
                        RulePredicate::Heat { above_percent: 70 },
                        RulePredicate::Rounds {
                            weapon: None,
                            comparison: Comparison::AtLeast,
                            amount: 25,
                            percent: true,
                        },
                    ],
                    implicit: true,
                },
            },
            FrameRule {
                slot: 4,
                rule: Rule::Lock {
                    selector: LockSelector::BestOdds,
                    weapon: Some("cannon".into()),
                    predicates: vec![
                        RulePredicate::TargetSection {
                            section: "hull".into(),
                            state: SectionState::Red,
                        },
                        RulePredicate::Band {
                            band: RuleBand::Effective,
                        },
                        RulePredicate::Lock { present: false },
                    ],
                    implicit: false,
                },
            },
            FrameRule {
                slot: 5,
                rule: Rule::Fire {
                    condition: FireCondition {
                        target: SlotTarget::BestHostile,
                        ..fire
                    },
                    predicates: vec![],
                    implicit: true,
                    independent: true,
                },
            },
        ];
        let read = |expr: &str| {
            shown(
                &fixture::eval_in(fixture::context_on(seen.clone()), expr)
                    .unwrap_or_else(|error| panic!("{expr}: {error}")),
            )
        };
        let reads = [
            ("v.rules.len()", "4"),
            ("type_of(v.rules[0])", "\"Rule\""),
            ("v.rules[1].slot", "3"),
            ("v.rules[0].action", "\"fire\""),
            ("v.rules[1].action", "\"hold_fire\""),
            ("v.rules[2].action", "\"lock\""),
            ("v.rules[0].channel", "\"fire\""),
            ("v.rules[2].channel", "\"sensors\""),
            ("v.rules[0].weapon", "\"cannon\""),
            ("v.rules[0].target", "\"contact\""),
            ("v.rules[3].target", "\"best_hostile\""),
            ("v.rules[2].target", "()"),
            ("v.rules[0].selector", "()"),
            ("v.rules[2].selector", "\"best_odds\""),
            ("v.rules[0].min_permille", "600"),
            ("v.rules[0].aspects", "14"),
            ("v.rules[0].keeps", "2"),
            ("v.rules[0].min_range_m", "50"),
            ("v.rules[0].max_range_m", "420"),
            ("v.rules[1].min_permille", "()"),
            ("v.rules[1].implicit", "true"),
            ("v.rules[0].independent", "false"),
            ("v.rules[1].independent", "false"),
            ("v.rules[3].independent", "true"),
            ("v.rules[0].predicates.len()", "4"),
            ("type_of(v.rules[0].predicates[0])", "\"Predicate\""),
            ("v.rules[0].predicates[0].kind", "\"odds\""),
            ("v.rules[0].predicates[0].weapon", "\"cannon\""),
            ("v.rules[0].predicates[0].value", "600"),
            ("v.rules[0].predicates[1].value", "14"),
            ("v.rules[0].predicates[2].comparison", "\"less\""),
            ("v.rules[0].predicates[2].value", "421"),
            ("v.rules[0].predicates[3].state", "\"moving\""),
            ("v.rules[1].predicates[0].section", "\"hull\""),
            ("v.rules[1].predicates[0].value", "50"),
            ("v.rules[1].predicates[1].value", "70"),
            ("v.rules[1].predicates[2].weapon", "()"),
            ("v.rules[1].predicates[2].comparison", "\"at_least\""),
            ("v.rules[1].predicates[2].value", "25"),
            ("v.rules[1].predicates[2].percent", "true"),
            ("v.rules[2].predicates[0].state", "\"red\""),
            ("v.rules[2].predicates[1].band", "\"effective\""),
            ("v.rules[2].predicates[2].present", "false"),
            ("v.rules[2].predicates[2].value", "()"),
            ("v.rules[0].predicates[0].percent", "()"),
            ("v.rules[0].predicates[0].state", "()"),
            ("v.rules[0].predicates[0].section", "()"),
            ("v.rules[0].predicates[0].band", "()"),
            ("v.rules[0].predicates[0].present", "()"),
        ];
        for (expr, expected) in reads {
            assert_eq!(read(expr), expected, "{expr}");
        }
        assert_eq!(read("v.rules[0].contact"), contact_int(HOSTILE).to_string());
        assert_eq!(read("v.rules[2].contact"), "()");
        for (ty, getter, _) in PARITY
            .iter()
            .filter(|(ty, _, _)| matches!(*ty, "Rule" | "Predicate"))
        {
            assert!(
                getter == &"contact"
                    || reads
                        .iter()
                        .any(|(expr, _)| expr.contains(&format!(".{getter}"))),
                "{ty}.{getter} is never read"
            );
        }
    }

    /// **The order's payload getters on every objective that carries one**:
    /// `to` on an `Advance` (the cell as `[x, y]`), `facing_mdeg` on a
    /// `Screen`, `of` on a `Support` (the row key's INT, or `()` when the
    /// re-key found none) — and every other payload getter `()` on each, as
    /// the keyframe's `order_json` writes `null`.
    #[test]
    fn the_order_getters_read_each_objectives_payload() {
        use crate::arena::perception::MindObjective;
        let friend = contact_int(fixture::FRIEND).to_string();
        let cases: [(MindObjective, &str, [&str; 4]); 5] = [
            (
                MindObjective::Advance { to: (6, 3) },
                "advance",
                ["[6, 3]", "()", "()", "()"],
            ),
            (
                MindObjective::Screen {
                    facing_mdeg: 135_000,
                },
                "screen",
                ["()", "()", "135000", "()"],
            ),
            (
                MindObjective::Support {
                    of: Some(fixture::FRIEND),
                },
                "support",
                ["()", "()", "()", friend.as_str()],
            ),
            (
                MindObjective::Support { of: None },
                "support",
                ["()", "()", "()", "()"],
            ),
            (
                MindObjective::Withdraw,
                "withdraw",
                ["()", "()", "()", "()"],
            ),
        ];
        for (objective, word, [to, target, facing, of]) in cases {
            let mut seen = fixture::view();
            if let Some(order) = seen.order.as_mut() {
                order.objective = objective;
            }
            let read = |expr: &str| {
                shown(
                    &fixture::eval_in(fixture::context_on(seen.clone()), expr)
                        .unwrap_or_else(|error| panic!("{expr}: {error}")),
                )
            };
            assert_eq!(read("v.order.objective"), format!("{word:?}"));
            assert_eq!(read("v.order.to"), to, "{word}");
            assert_eq!(read("v.order.target"), target, "{word}");
            assert_eq!(read("v.order.facing_mdeg"), facing, "{word}");
            assert_eq!(read("v.order.of"), of, "{word}");
        }
    }

    /// The contact id's INT is `StockMemory`'s bit-cast, both ways.
    #[test]
    fn a_contact_id_crosses_as_its_bits() {
        for id in [ContactId(0), ContactId(1), HOSTILE, ContactId(u64::MAX)] {
            assert_eq!(contact_of(contact_int(id)), id);
            assert_eq!(contact_int(id), i64::from_ne_bytes(id.0.to_ne_bytes()));
        }
        assert!(contact_int(HOSTILE) < 0);
    }
}
