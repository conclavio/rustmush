//! **The TacSim API for unit minds** (17b cut M2.3; doc 17 Â§3 "The
//! observation", Â§4 "The outputs", Â§7 "The sandbox and the budget", Â§8, Â§19
//! "Fit with the crates", Â§23; D-UM-4, D-UM-6, D-UM-7, D-UM-9, D-UM-10,
//! D-UM-11, D-UM-24, D-UM-26, D-UM-27, D-UM-28, D-UM-29, D-UM-31, D-UM-35,
//! D-UM-47).
//!
//! Everything a scripted unit mind can name, registered on `mush_mind`'s
//! sandbox ([`standard_sandbox`](crate::minds::standard_sandbox),
//! [`register_api`](crate::minds::register_api)): the hull's own view as
//! registered custom types with getters and never maps (D-UM-7), its card as a
//! read-only type (D-UM-27), the think's facts as zero-argument native
//! functions, the station functions that fill the think's intent buffer
//! (D-UM-9, D-UM-47), the kernels over the arena's own free functions with
//! their fees (D-UM-26), and `rand(k)` over the sixth draw label (D-UM-29).
//! The arena never names this module or its interpreter: `crate::arena`'s
//! [`crate::arena::mind::MindRunner`] seam is the whole contract, and the
//! runner that drives a think through this API is
//! [`RhaiRunner`](crate::minds::runner::RhaiRunner) (17b cut M2.4).
//!
//! * **The context** (`context.rs`): the think's thread-local
//!   [`ThinkContext`](crate::minds::context::ThinkContext) â€” the view and the
//!   map behind the arena's own `Arc`s, the seed, the clocks, the draw and
//!   `line_to` counters, the intent
//!   buffer and the refusal flag â€” set by the runner at a think's start and
//!   taken at its end, and the fifteen context functions (`lands_ms()` â€¦
//!   `cover()`) that read it from any call depth (17b's settled question
//!   (ii): never scope constants, never script `const`s).
//! * **The view** (`api.rs`): `View`, `Contact`, `Weapon`, `Section`, `Facet`,
//!   `Heat`, `Receiver`, `Order`, `Notice`, `Condition`, `Armed` â€” each a
//!   handle on the one `Arc<MindView>` the arena wrapped (no row copied), each
//!   getter an INT, a bool, a word, a two-INT cell or `()`, never a FLOAT
//!   (D-UM-28), and each with its row in
//!   [`PARITY`](crate::minds::api::PARITY), the pinned table of where the
//!   keyframe carries the same field (D-UM-4 read at the field level, 17b Â§1
//!   (ii)).
//! * **The card** (`card.rs`): `Card`, the 24 stored fields by slider name in
//!   stored integer units (D-UM-27, D-UM-43).
//! * **The kernels** (`kernels.rs`, `fees.rs`): each a thin wrapper over the
//!   ONE free function the native stock mind (T1) calls, charged its fee
//!   through `mush_mind::budget` at the call (D-UM-24, D-UM-26).
//! * **The runner** (`runner.rs`, 17b cut M2.4): `RhaiRunner`, a
//!   `MindRunner` that runs a compiled script synchronously at the think
//!   point, reads the label rule on both ends of the call, and answers a
//!   dropped hull (three faults running) with the native stand-in.
//! * **The think pool** (`pool.rs`, 17b cut M4.1; doc 17 Â§6 "Execution
//!   policy", D-UM-22): [`MindPool`](crate::minds::MindPool), the
//!   process-wide pure-`std` pool of `[tacsim.minds].workers` threads every
//!   sim task sends a pulse's collected thinks to as one batch, served
//!   first-in first-out per batch and answered in job order
//!   ([`BatchHandle`](crate::minds::BatchHandle)); a worker's runner panic
//!   answers [`BatchFailed`](crate::minds::BatchFailed) and the task voids
//!   the match.
//! * **The sealed runners** (`sealed.rs`, 17b cut M3.5):
//!   [`install_sealed_runners`](crate::minds::install_sealed_runners), the
//!   one place a scripted seal becomes a hull's own runner on
//!   `Arena::runners` â€” at check-out and at a resume alike.
//! * **The stock mind** (`content.rs`, 17b cut M2.5): Standard Doctrine in
//!   Rhai (T2, `content/minds/standard.rhai`) with its manifest
//!   (`standard.json`, the role `combat`) and its pinned SHA-256, and
//!   [`standard_runner`](crate::minds::standard_runner) â€” the native stock
//!   mind ported arm for arm: the helm half (17b cut M2.5) and the fire
//!   half (17b cut M2.6).
//! * **The stations** (`stations.rs`): exactly doc 17 Â§4's list â€” `helm`,
//!   `speed`, `startup`, `goto`, `hold`, `goto_off`; `lock`, `emcon`; `fire`,
//!   `arm`, `disarm`; `trace` â€” each pushing the
//!   [`IntentKind`](crate::arena::intents::IntentKind) T1 emits (D-UM-10: a
//!   pilot can type every one of them). `goto` is a reserved keyword of rhai
//!   1.26 and is registered as custom syntax, `goto ( expr )`, with the
//!   spelling unchanged (`stations.rs`).
//!
//! # Refusals
//!
//! Every refusal a station or a kernel raises â€” an argument of the wrong type,
//! an id the view does not hold, a seventeenth intent, a third `line_to` â€” sets
//! the context's flag
//! ([`ThinkContext::refused`](crate::minds::context::ThinkContext::refused),
//! the cause word the runner reads on the `Ok` path too) and is raised through
//! `mush_mind::system_error`, so a script's `try { } catch { }` cannot swallow
//! it (D-UM-31; 17b Â§1 (iv), the adversarial F5). A call that no registered
//! signature matches is Rhai's own error and Rhai's own `type` fault.
//!
//! # Lints
//!
//! `tacsim`'s crate deny omits `indexing_slicing`, so this module adds it here
//! (doc 17 Â§7 "Lints"): every row is reached by `get`, and a handle whose
//! index names no row answers `()`.

#![deny(clippy::indexing_slicing)]

use std::sync::{Arc, OnceLock};

use mush_mind::rhai::Engine;
use mush_mind::{EngineLimits, Sandbox};

/// **Register one native function** of the API on the registrar's engine and
/// record its name â€” the one path a name enters the engine by, so
/// [`registered_names`] is the engine's list by construction.
macro_rules! function {
    ($api:expr, $name:literal, $f:expr) => {{
        let api: &mut $crate::minds::Registrar<'_> = $api;
        api.engine.register_fn($name, $f);
        api.names.push($name);
    }};
}

/// **Register one getter** of a custom type, recorded as `Type.getter`.
macro_rules! getter {
    ($api:expr, $ty:literal, $name:literal, $f:expr) => {{
        let api: &mut $crate::minds::Registrar<'_> = $api;
        api.engine.register_get($name, $f);
        api.names.push(concat!($ty, ".", $name));
    }};
}

/// **Register one method** of a custom type (a function whose first argument
/// is the type), recorded as `Type.method`.
macro_rules! method {
    ($api:expr, $ty:literal, $name:literal, $f:expr) => {{
        let api: &mut $crate::minds::Registrar<'_> = $api;
        api.engine.register_fn($name, $f);
        api.names.push(concat!($ty, ".", $name));
    }};
}

/// **Register one custom type** under its script name, recorded as
/// `type Name`.
macro_rules! custom_type {
    ($api:expr, $t:ty, $name:literal) => {{
        let api: &mut $crate::minds::Registrar<'_> = $api;
        api.engine.register_type_with_name::<$t>($name);
        api.names.push(concat!("type ", $name));
    }};
}

#[cfg(test)]
mod acceptance;
pub mod api;
pub(crate) mod authoring;
pub mod card;
pub(crate) mod check;
pub mod content;
pub mod context;
#[cfg(test)]
mod coprates_drill;
mod doctrine_library;
/// Doc 17h cut M8g G3: what an author's own sensors earned (no producer until the IC fight).
pub mod earned_contacts;
pub mod fees;
pub(crate) mod ic_sortie;
pub mod kernels;
/// Doc 17h cut M8a: the licence state a seal is read under, a parameter of the line.
pub mod licence_state;
/// 17g C2: the line-mode commands over the composed mind commerce.
pub(crate) mod market_commands;
#[cfg(test)]
mod meridiani_field;
pub(crate) mod metadata;
pub(crate) mod normalize;
pub mod pool;
/// Doc 17h cut M8a: where a core came from, the inspection's provenance read.
pub mod provenance;
pub(crate) mod registry;
/// Doc 17h cut M8a: `@registry/inspect <hull>`, the Registry's one read of a hull.
pub mod registry_inspection;
pub mod resimulation;
pub(crate) mod rule_samples;
pub mod runner;
#[cfg(test)]
mod safe_conduct;
pub mod sealed;
pub mod starter;
pub mod stations;
pub(crate) mod stock;
pub(crate) mod trace_feed;
pub(crate) mod workshop;

pub use content::{
    MindManifest, STANDARD_MANIFEST_JSON, STANDARD_RHAI, STANDARD_SHA256, source_lookup,
    standard_digest, standard_manifest, standard_runner,
};
pub use pool::{BatchFailed, BatchHandle, MindPool};
pub use runner::{DROPPED_LINE, RhaiRunner, ThinkStat};
pub use sealed::{InstallError, install_sealed_runners};

/// **The API's version** (doc 17 Â§13): the `api=` part of [`MIND_ABI`]. A
/// change to any registered name, signature, getter, fee or refusal is a bump.
///
/// **7 since doc 32 cut CF.0** (Q-CF-3, Q-CF-43): `View.team` is gone and
/// `Contact.relation` reads the hull's party's relation to the row, so a mind
/// sees relation and never party. Taken once, with the snapshot and the frame.
pub const API_VERSION: u32 = 7;

/// **The stock content's version** (doc 17 Â§13): the `stock=` part of
/// [`MIND_ABI`]; version 3 distinguishes replacement detachment orders from
/// their optional expiry deadlines.
pub const STOCK_VERSION: u32 = 3;

/// **The ABI a compiled mind is cached and sealed under** (doc 17 Â§7, Â§13;
/// D-UM-27): `mush_mind::RHAI_ABI`, then [`API_VERSION`] and
/// [`STOCK_VERSION`] â€” `mind_abi_is_composed_and_the_sandbox_is_one` composes
/// it and asserts the literal.
pub const MIND_ABI: &str = "rhai=1.26.0;api=7;stock=3";

/// **The stock class's op ceiling per think** (doc 17 Â§9, D-UM-35's class I
/// `ops` column): operations plus fees a think may spend. **The Class I row of
/// `ClassTable::STOCK`; production reads the profile, the tests' arithmetic
/// keeps the constant** (17b cut M3.1: [`RhaiRunner`] runs a think at its
/// row's `ops`, and [`standard_sandbox`]'s engine backstop is the largest
/// class's, `mush_mind::MAX_OPS_ANY_CLASS`).
pub const STOCK_OPS_PER_THINK: u64 = 1_500;

/// **How many `line_to` traces one think may ask for** (D-UM-35's `line_to`
/// column, class I: 2): a third call is refused `limit`
/// (`kernels::line_to`). **The Class I row of `ClassTable::STOCK`; production
/// reads the profile, the tests' arithmetic keeps the constant** (17b cut
/// M3.1: the cap is the think's row's, `context::ThinkContext::line_to_max`).
pub const STOCK_LINE_TO_PER_THINK: u32 = 2;

/// **The largest `k` a `rand(k)` may ask for** (D-UM-29 as read by 17b Â§1 (iv),
/// the adversarial F2): `draw_permille`'s resolution. A raw 64-bit draw is
/// invertible back to the seed-keyed state through a label the script knows
/// (`crate::arena::rng`'s `draw` is two bijections, and the label
/// `mind:{unit}:{think_seq}:{n}` is the script's own), and with that state
/// every other label's draw of the match; ten bits of each of many distinct
/// draws are not. A `k` past it, or under 1, is refused `type`.
pub const RAND_MAX_K: i64 = 1_000;

/// **The names the sandbox itself registers** (17b cut M2.1, `mush-mind`'s
/// `engine.rs`): the two integer helpers the packages lack and the `sleep`
/// shadow (two overloads, one name). Pinned in [`registered_names`].
pub const SANDBOX_NAMES: [&str; 3] = ["clamp", "isqrt", "sleep"];

/// **The packages' names the pin carries** (D-UM-24's seven packages; 17b
/// Â§1's departure (5)): the integer helpers `abs`, `min`, `max`
/// (`ArithmeticPackage`, `LogicPackage`); `LanguageCorePackage`'s own â€”
/// `exit` (a return, M2.1), `take`, the `tag` property and `set_tag`, `sleep`
/// (shadowed by the sandbox, so a call refuses), `parse_json` and the
/// reflection function `get_fn_metadata_list`; and the keyword functions
/// `print`, `debug` (both to the trace), `type_of`, `is_def_fn`,
/// `is_def_var`. The array, map, string and iterator packages' methods are
/// pinned by the package that carries them, not name by name: rhai 1.26
/// enumerates a native function only under its `metadata` feature, which the
/// workspace does not turn on.
pub const PACKAGE_NAMES: [&str; 15] = [
    "abs",
    "min",
    "max",
    "exit",
    "take",
    "tag",
    "set_tag",
    "sleep",
    "parse_json",
    "get_fn_metadata_list",
    "print",
    "debug",
    "type_of",
    "is_def_fn",
    "is_def_var",
];

/// **The one path the API's names enter an engine by**: the engine and the
/// names registered so far, in registration order (`function!`, `getter!`,
/// `method!`, `custom_type!`). Rhai 1.26 lists a native function only under
/// its `metadata` feature, so the list is kept here, beside the registration
/// itself, and nothing registers around it.
pub(crate) struct Registrar<'e> {
    /// The engine the API is registered on.
    pub(crate) engine: &'e mut Engine,
    /// What was registered: `name` for a function, `Type.name` for a getter
    /// or a method, `type Name` for a custom type.
    pub(crate) names: Vec<&'static str>,
}

/// Register the whole API on `engine` and answer what was registered.
fn register(engine: &mut Engine) -> Vec<&'static str> {
    let mut api = Registrar {
        engine,
        names: Vec::new(),
    };
    api::register(&mut api);
    card::register(&mut api);
    kernels::register(&mut api);
    context::register(&mut api);
    stations::register(&mut api);
    stock::register(&mut api);
    api.names
}

/// **Register the TacSim API on an engine** (doc 17 Â§7, Â§19): the custom types
/// and their getters, the card, the kernels and `rand`, the context functions
/// and the station functions with `trace` â€” the closure `mush_mind`'s
/// `Sandbox::new` hands its engine to, last, after every limit, shadow and
/// hook.
pub fn register_api(engine: &mut Engine) {
    let _registered = register(engine);
}

/// **Every name a mind can call or read, pinned** (17b Â§2's M2.3 test 9): the
/// API's registrations in registration order â€” `name` for a function, `Type.name`
/// for a getter or a method, `type Name` for a custom type â€” then
/// [`SANDBOX_NAMES`], then [`PACKAGE_NAMES`]. Read off a scratch engine through
/// the same registrar [`register_api`] runs, so a name cannot be registered
/// without appearing here. D-UM-11 is the pin's other half: no `order`,
/// `release`, `place`, `board` or exit-the-hull function is in it (`exit` is
/// `LanguageCorePackage`'s, a script's early return, never the hull's).
#[must_use]
pub fn registered_names() -> Vec<String> {
    let mut engine = Engine::new_raw();
    register(&mut engine)
        .into_iter()
        .chain(SANDBOX_NAMES)
        .chain(PACKAGE_NAMES)
        .map(str::to_owned)
        .collect()
}

/// **The one sandbox every stock think of the process runs on** (doc 17 Â§7,
/// D-UM-24: one shared engine per process): `mush_mind`'s sandbox with its
/// engine backstop at the largest class's ceiling (17b cut M3.1:
/// `mush_mind::MAX_OPS_ANY_CLASS`, Class III's 4,500, so every class's think
/// runs on the one engine and a think asking past it is refused before it
/// runs) and the API registered
/// ([`register_api`]), built once (a process-wide `OnceLock`) and shared â€” two
/// calls answer one `Arc`.
#[must_use]
pub fn standard_sandbox() -> Arc<Sandbox> {
    static SANDBOX: OnceLock<Arc<Sandbox>> = OnceLock::new();
    Arc::clone(SANDBOX.get_or_init(|| {
        Arc::new(Sandbox::new(
            &EngineLimits {
                max_ops_backstop: mush_mind::MAX_OPS_ANY_CLASS,
            },
            register_api,
        ))
    }))
}

/// **The in-source fixture** every test of this module drives: a hand-built
/// view whose every optional field is held (the shapes 17b cut M1b's (j) field
/// gives a live view, rebuilt by hand â€” no arena, no runner), the arena
/// library's own test window as the map, Line as the card, and the two ways a
/// test drives the API: a think on [`standard_sandbox`], and an expression on
/// a scratch engine for reading a getter's value straight back.
#[cfg(test)]
pub(crate) mod fixture {
    use std::sync::Arc;

    use mush_core::ObjId;
    use mush_mind::rhai::packages::{
        ArithmeticPackage, BasicArrayPackage, BasicIteratorPackage, BasicMapPackage,
        BasicStringPackage, LanguageCorePackage, LogicPackage, Package,
    };
    use mush_mind::rhai::{Dynamic, Engine, EvalAltResult, Scope};
    use mush_mind::{Digest, Fault, Outcome, Think};
    use sha2::{Digest as _, Sha256};
    use smol_str::SmolStr;

    use super::api::View;
    use super::card::Card;
    use super::context::{self, ThinkContext};
    use super::fees::FEE_THINK;
    use super::{MIND_ABI, STOCK_OPS_PER_THINK, register_api, standard_sandbox};
    use crate::arena::card::{StockCard, line};
    use crate::arena::condition::Condition;
    use crate::arena::conditions::{FireCondition, HeldWhy, Rule, RulePredicate, SlotTarget};
    use crate::arena::intents::Refusal;
    use crate::arena::mind::{MindView, STOCK_MEMORY_LIMITS, ViewContact};
    use crate::arena::motion::BlockedCause;
    use crate::arena::perception::{
        Aspect, DriveOutcome, FrameCondition, FrameNotice, FrameOrder, MindObjective, Named,
        NoticeKind, Receiver, RefusalDetail,
    };
    use crate::arena::state::{
        ArenaDefinition, Classification, ContactId, Designator, PartyId, Power,
    };
    use crate::arena::step::{FrameFacet, FrameHeat, FrameSection};
    use crate::arena::weapons::{FrameArming, FrameBand, FrameRule, FrameWeapon};
    use crate::config::ReflexConfig;
    use crate::deliberation::order::RangeBand;

    /// The hostile's row key: the high bit set, so the `StockMemory` bit-cast
    /// reads it negative.
    pub(crate) const HOSTILE: ContactId = ContactId(0x8000_0000_0000_0011);
    /// The friend's row key.
    pub(crate) const FRIEND: ContactId = ContactId(0x22);
    /// The unidentified return's row key.
    pub(crate) const BLIP: ContactId = ContactId(0x33);
    /// The thinking hull.
    pub(crate) const UNIT: ObjId = ObjId(4);

    /// A notice's `Named`.
    fn named(id: ContactId, designator: Designator) -> Named {
        Named { id, designator }
    }

    /// The hostile's designator, `BB`.
    pub(crate) const fn hostile_designator() -> Designator {
        Designator::Unit {
            party: PartyId::BLUE,
            letter: 'B',
        }
    }

    /// **The full view**: a red hull at cell (1, 4) of the test window,
    /// online, holding a lock on the hostile, a standing `Engage` order, two
    /// weapons (the cannon armed in slot 1, the coax idle), one section with
    /// two facets, heat, a receiver and a pod â€” and three contacts: a
    /// classified hostile (blue) with a reading, a classified friend, and an
    /// unidentified return â€” and one notice of each shape the getters read.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn view() -> MindView {
        let hostile = named(HOSTILE, hostile_designator());
        MindView {
            unit: UNIT.0,
            callsign: Designator::Unit {
                party: PartyId::RED,
                letter: 'A',
            },
            party: PartyId::RED,
            cell: Some((1, 4)),
            mode: crate::arena::state::MatchMode::Hold,
            objective: Some(crate::arena::state::HoldObjective {
                spot: (6, 4),
                letter: Some('A'),
                radius_cells: 2,
            }),
            motor: Some(crate::arena::step::FrameMotor {
                requested_goal: (6, 4),
                outcome: Some(DriveOutcome::Arrived),
            }),
            own_last_hit_ms: None,
            x_cm: 1_500,
            y_cm: 4_500,
            z_cm: 0,
            heading_mdeg: 90_000,
            speed_cms: 120,
            power: Power::Online,
            held: false,
            wreck: false,
            immobile: false,
            sight_m: 800,
            rules: vec![FrameRule {
                slot: 1,
                rule: Rule::fire_rule(
                    FireCondition {
                        weapon: SmolStr::new("cannon"),
                        target: SlotTarget::BestHostile,
                        min_permille: 100,
                        aspects: 0b1111,
                        min_rounds_left: 2,
                        min_range_m: 0,
                        max_range_m: 420,
                    },
                    vec![RulePredicate::Odds {
                        weapon: None,
                        min_permille: 100,
                    }],
                ),
            }],
            ecm: Some(false),
            receiver: Some(Receiver {
                budget: 3,
                budget_max: 4,
                // Not a multiple of ten: the getter floors it to the
                // keyframe's whole per cent.
                reach_permille: 755,
            }),
            heat: Some(FrameHeat { heat: 10, max: 100 }),
            sections: vec![FrameSection {
                id: SmolStr::new("hull"),
                condition: SmolStr::new("damaged"),
                internals: Some((40, 50)),
                facets: vec![
                    FrameFacet {
                        id: SmolStr::new("front"),
                        armor: 90,
                        armor_max: 100,
                    },
                    FrameFacet {
                        id: SmolStr::new("rear"),
                        armor: 30,
                        armor_max: 60,
                    },
                ],
            }],
            weapons: vec![
                FrameWeapon {
                    id: SmolStr::new("cannon"),
                    ready: true,
                    ready_in_ms: 0,
                    rounds: 18,
                    rounds_max: 20,
                    range_band: Some(FrameBand {
                        index: 1,
                        of: 2,
                        max_range_cm: 40_000,
                        dispersion_mrad: 3,
                    }),
                    p_permille: Some(620),
                    bands_cm: vec![40_000, 80_000],
                    heat_per_shot: 4,
                    armed: Some(FrameArming {
                        slot: 1,
                        target: SlotTarget::BestHostile,
                        min_permille: 100,
                        aspects: 0b1111,
                        keeps: 2,
                        min_range_m: 0,
                        max_range_m: 420,
                    }),
                },
                FrameWeapon {
                    id: SmolStr::new("coax"),
                    ready: false,
                    ready_in_ms: 700,
                    rounds: 0,
                    rounds_max: 200,
                    range_band: None,
                    p_permille: None,
                    bands_cm: vec![30_000],
                    heat_per_shot: 0,
                    armed: None,
                },
            ],
            lock: Some(HOSTILE),
            order: Some(FrameOrder {
                identity: None,
                squad: SmolStr::new("red-1"),
                objective: MindObjective::Engage {
                    target: Some(HOSTILE),
                },
                hold_fire: false,
                max_range_band: Some(RangeBand::Effective),
                expires_ms: 9_000,
            }),
            t_ms: 5_000,
            notices: vec![
                FrameNotice::Miss {
                    launch_cell: Some((1, 4)),
                    contact: Some(hostile),
                    reason: SmolStr::new("blocked"),
                },
                FrameNotice::Drive {
                    outcome: DriveOutcome::Arrived,
                },
                FrameNotice::Refused {
                    order: SmolStr::new("fire"),
                    refusal: Refusal::Cycling,
                    weapon: Some(SmolStr::new("cannon")),
                    detail: RefusalDetail::Cycling { secs: 2 },
                },
                FrameNotice::Emcon {
                    on: true,
                    radius_m: 300,
                },
                FrameNotice::SlotFired {
                    slot: 1,
                    weapon: SmolStr::new("cannon"),
                    contact: Some(hostile),
                    p_permille: 620,
                    aspect: Aspect::Left,
                },
                FrameNotice::SlotHeld {
                    slot: 2,
                    weapon: SmolStr::new("coax"),
                    why: HeldWhy::Odds {
                        permille: 80,
                        floor: 100,
                    },
                },
                FrameNotice::Contact {
                    kind: NoticeKind::Classified,
                    contact: hostile,
                    class: Classification::Classified {
                        party: PartyId::BLUE,
                    },
                    was: Some(Designator::Provisional(3)),
                },
                FrameNotice::Hit {
                    contact: Some(hostile),
                    section: Some(SmolStr::new("turret")),
                    spent: true,
                },
                FrameNotice::Blocked {
                    cause: BlockedCause::Occupied,
                },
                FrameNotice::Refused {
                    order: SmolStr::new("fire"),
                    refusal: Refusal::Overheated,
                    weapon: Some(SmolStr::new("cannon")),
                    detail: RefusalDetail::Overheated {
                        heat: 100,
                        max: 100,
                    },
                },
                FrameNotice::Refused {
                    order: SmolStr::new("fire"),
                    refusal: Refusal::Unlawful,
                    weapon: Some(SmolStr::new("cannon")),
                    detail: RefusalDetail::Floor { permille: 100 },
                },
                FrameNotice::SlotHeld {
                    slot: 1,
                    weapon: SmolStr::new("cannon"),
                    why: HeldWhy::Range {
                        contact: hostile,
                        range_m: 500,
                        min_m: 0,
                        max_m: 420,
                    },
                },
                FrameNotice::SlotHeld {
                    slot: 1,
                    weapon: SmolStr::new("cannon"),
                    why: HeldWhy::Rounds {
                        rounds: 2,
                        keeps: 2,
                    },
                },
                FrameNotice::SlotHeld {
                    slot: 1,
                    weapon: SmolStr::new("cannon"),
                    why: HeldWhy::Aspect {
                        contact: hostile,
                        aspect: Aspect::Front,
                    },
                },
            ],
            contacts: vec![
                ViewContact {
                    id: HOSTILE,
                    designator: hostile_designator(),
                    alias: Some(3),
                    class: Classification::Classified {
                        party: PartyId::BLUE,
                    },
                    relation: crate::arena::party::Relation::Hostile,
                    friendly: false,
                    moving: false,
                    bearing_deg: 90,
                    relative_deg: 0,
                    range_m: 300,
                    band: SmolStr::new("near"),
                    age_ms: 0,
                    cell: Some((4, 4)),
                    condition: Some(FrameCondition {
                        table: 0,
                        bands: Condition {
                            bands: 0b0100,
                            sections: 2,
                        },
                        age_ms: 1_000,
                    }),
                    fatal: Some(vec![SmolStr::new("hull")]),
                    aspect: Aspect::Left,
                    odds: vec![(SmolStr::new("cannon"), 620)],
                    reads_dead: Some(false),
                },
                ViewContact {
                    id: FRIEND,
                    designator: Designator::Unit {
                        party: PartyId::RED,
                        letter: 'B',
                    },
                    alias: None,
                    class: Classification::Classified {
                        party: PartyId::RED,
                    },
                    relation: crate::arena::party::Relation::Friend,
                    friendly: true,
                    moving: false,
                    bearing_deg: 0,
                    relative_deg: 270,
                    range_m: 450,
                    band: SmolStr::new("far"),
                    age_ms: 500,
                    cell: Some((1, 0)),
                    condition: None,
                    fatal: None,
                    aspect: Aspect::Rear,
                    odds: Vec::new(),
                    reads_dead: Some(false),
                },
                ViewContact {
                    id: BLIP,
                    designator: Designator::Provisional(5),
                    alias: None,
                    class: Classification::Unknown,
                    relation: crate::arena::party::Relation::Unknown,
                    friendly: false,
                    moving: false,
                    bearing_deg: 135,
                    relative_deg: 45,
                    range_m: 600,
                    band: SmolStr::new("far"),
                    age_ms: 2_000,
                    cell: None,
                    condition: None,
                    fatal: None,
                    aspect: Aspect::Front,
                    odds: Vec::new(),
                    reads_dead: None,
                },
            ],
            scan_tables: vec![vec![SmolStr::new("hull"), SmolStr::new("turret")]],
            odds_floor_permille: 100,
        }
    }

    /// The same hull with **no weapons station**: no weapon rows, and the
    /// two weapons-station fields gone from every contact (doc 17 Â§22's
    /// italic line).
    pub(crate) fn unarmed_view() -> MindView {
        let mut view = view();
        view.weapons.clear();
        for row in &mut view.contacts {
            row.odds.clear();
            row.reads_dead = None;
        }
        view
    }

    /// The arena library's own test window: eight by eight cells of 1,000 cm,
    /// red zone west, blue zone east, the spot `The Middle` at (4, 4), two
    /// waypoints joined on row 4.
    pub(crate) fn map() -> ArenaDefinition {
        crate::arena::test_support::definition()
    }

    /// The Line card on the default doctrine.
    pub(crate) fn card() -> StockCard {
        line(&ReflexConfig::default())
    }

    /// Admit exact stock fixture values through the same generic definition path.
    pub(crate) fn runtime_card(card: StockCard) -> crate::arena::runtime_card::RuntimeCard {
        use crate::arena::runtime_card::{CardCatalog, ProfileKey, stock_definition};
        let mut catalog = CardCatalog::default();
        let definition =
            stock_definition(&[("Line".into(), card)], crate::arena::core::CoreClass::I).unwrap();
        let definition = catalog.admit(definition).unwrap();
        catalog
            .resolve(ProfileKey {
                definition,
                index: 0,
            })
            .unwrap()
    }

    pub(crate) fn empty_program() -> crate::arena::profile_rules::BoundProgram {
        crate::arena::profile_rules::BoundProgram::bind(
            &[],
            &[],
            &[],
            &crate::arena::core::ClassTable::STOCK.row(crate::arena::core::CoreClass::I),
            0,
        )
        .unwrap()
    }

    /// **A context for one think** over `view`: think 7 of the hull at 5,000
    /// ms, landing at 5,100, seed 42.
    pub(crate) fn context_on(view: MindView) -> ThinkContext {
        let card = runtime_card(card());
        ThinkContext {
            profile_token: crate::arena::profile_runtime::ProfileToken {
                unit: UNIT,
                generation: 0,
                key: card.key(),
            },
            program: empty_program(),
            next_lands_ms: 6_100,
            rule_sample: None,
            view: Arc::new(view),
            map: Arc::new(map()),
            seed: 42,
            unit: UNIT,
            think_seq: 7,
            t_ms: 5_000,
            lands_ms: 5_100,
            decide_ms: 1_000,
            repath_ms: 4_000,
            draws: 0,
            line_to_calls: 0,
            terrain_calls: 0,
            line_to_max: super::STOCK_LINE_TO_PER_THINK,
            slots: crate::arena::core::ClassTable::STOCK
                .row(crate::arena::core::CoreClass::I)
                .slots,
            intents: Vec::new(),
            overflowed: false,
            refused: None,
        }
    }

    /// One think's answer, its trace and the context it left.
    pub(crate) struct Ran {
        /// What the sandbox answered.
        pub(crate) result: Result<Outcome, Fault>,
        /// The trace.
        pub(crate) trace: Vec<String>,
        /// The context, taken back.
        pub(crate) ctx: ThinkContext,
    }

    /// The source's digest, as the seal would compute it.
    pub(crate) fn digest(source: &str) -> Digest {
        Digest(Sha256::digest(source.as_bytes()).into())
    }

    /// **One think through [`standard_sandbox`]** on `ctx` with `card`,
    /// starting from `memory`, at the stock class's ceiling and the think fee
    /// â€” the way 17b cut M2.4's runner will drive it.
    pub(crate) fn think_with(
        source: &str,
        ctx: ThinkContext,
        card: StockCard,
        memory: &[u8],
    ) -> Ran {
        let sandbox = standard_sandbox();
        let compiled = sandbox
            .compile(source, digest(source), MIND_ABI)
            .unwrap_or_else(|error| panic!("{source}: {error}"));
        let view = Arc::clone(&ctx.view);
        context::set(ctx);
        let (result, trace) = sandbox.think(Think {
            compiled: &compiled,
            args: vec![
                Dynamic::from(View(view)),
                Dynamic::from(Card(runtime_card(card))),
            ],
            memory,
            limits: &STOCK_MEMORY_LIMITS,
            ops_limit: STOCK_OPS_PER_THINK,
            fee_per_think: FEE_THINK,
        });
        let ctx = context::take().expect("the think's context");
        Ran { result, trace, ctx }
    }

    /// [`think_with`] on the full view, Line, empty memory.
    pub(crate) fn think(source: &str) -> Ran {
        think_with(source, context_on(view()), card(), &[])
    }

    /// **A scratch engine** with the seven packages and the API â€” for reading
    /// a getter's value straight back as a `Dynamic` (a think answers only its
    /// memory).
    pub(crate) fn scratch() -> Engine {
        let mut engine = Engine::new_raw();
        // Trusted stock modules use the same parser envelope as production.
        // Rhai's raw default has a shallower function limit than our sandbox.
        engine.set_max_expr_depths(48, 24);
        engine.register_global_module(LanguageCorePackage::new().as_shared_module());
        engine.register_global_module(ArithmeticPackage::new().as_shared_module());
        engine.register_global_module(LogicPackage::new().as_shared_module());
        engine.register_global_module(BasicIteratorPackage::new().as_shared_module());
        engine.register_global_module(BasicArrayPackage::new().as_shared_module());
        engine.register_global_module(BasicMapPackage::new().as_shared_module());
        engine.register_global_module(BasicStringPackage::new().as_shared_module());
        register_api(&mut engine);
        engine
    }

    /// **Evaluate `expr` on [`scratch`]** with `v` the full view and `c` Line
    /// in scope, under a context on the full view (taken back after).
    pub(crate) fn eval(expr: &str) -> Result<Dynamic, Box<EvalAltResult>> {
        eval_in(context_on(view()), expr)
    }

    /// [`eval`] under `ctx`, with `v` its view â€” for a view or a map other
    /// than the full view's.
    pub(crate) fn eval_in(ctx: ThinkContext, expr: &str) -> Result<Dynamic, Box<EvalAltResult>> {
        let view = Arc::clone(&ctx.view);
        context::set(ctx);
        let mut scope = Scope::new();
        scope.push("v", View(view));
        scope.push("c", Card(runtime_card(card())));
        let answer = scratch().eval_with_scope::<Dynamic>(&mut scope, expr);
        let _left = context::take();
        answer
    }

    /// A value's test rendering: rhai's `Debug` (strings quoted, `()` for
    /// unit, arrays bracketed).
    pub(crate) fn shown(value: &Dynamic) -> String {
        format!("{value:?}")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use mush_mind::{Fault, RHAI_ABI};

    use super::fixture::{self, think};
    use super::*;

    /// 17b Â§2's M2.3 test 8: the ABI composed from its three parts equals the
    /// literal, and the sandbox is one per process.
    #[test]
    fn mind_abi_is_composed_and_the_sandbox_is_one() {
        assert_eq!(
            format!("{RHAI_ABI};api={API_VERSION};stock={STOCK_VERSION}"),
            MIND_ABI
        );
        assert_eq!(MIND_ABI, "rhai=1.26.0;api=7;stock=3");
        assert_eq!(STOCK_OPS_PER_THINK, 1_500);
        assert_eq!(STOCK_LINE_TO_PER_THINK, 2);
        assert_eq!(RAND_MAX_K, 1_000);
        let one = standard_sandbox();
        let two = standard_sandbox();
        assert!(Arc::ptr_eq(&one, &two), "two calls built two sandboxes");
    }

    /// **17b cut M3.1** (doc 17 Â§9, D-UM-35): every think-side `STOCK_*`
    /// constant is the Class I row of `ClassTable::STOCK` â€” production reads
    /// the profile, the tests' arithmetic keeps the constant, and the two
    /// cannot part.
    #[test]
    fn m3_every_stock_constant_is_the_class_i_row() {
        use crate::arena::conditions::STOCK_CONDITION_SLOTS;
        use crate::arena::core::{ClassTable, CoreClass};
        use crate::arena::mind::{
            STOCK_CONTACT_ROWS, STOCK_MEMORY_LIMITS, STOCK_RATED_CLASS, STOCK_THINK_MS,
        };
        use crate::arena::step::STOCK_ODDS_ROWS;
        let one = ClassTable::STOCK.row(CoreClass::I);
        assert_eq!(STOCK_THINK_MS, one.think_ms);
        assert_eq!(STOCK_CONTACT_ROWS, one.rows);
        assert_eq!(STOCK_ODDS_ROWS, one.odds_rows);
        assert_eq!(STOCK_CONDITION_SLOTS, one.slots);
        assert_eq!(STOCK_MEMORY_LIMITS.keys, one.memory_keys);
        assert_eq!(STOCK_OPS_PER_THINK, one.ops);
        assert_eq!(STOCK_LINE_TO_PER_THINK, one.line_to);
        assert_eq!(STOCK_RATED_CLASS, CoreClass::I);
        // The engine backstop is the largest class's ceiling.
        assert_eq!(
            CoreClass::ALL
                .map(|class| ClassTable::STOCK.row(class).ops)
                .into_iter()
                .max(),
            Some(mush_mind::MAX_OPS_ANY_CLASS)
        );
    }

    /// **The registered names, pinned** (17b Â§2's M2.3 test 9; D-UM-11): every
    /// type, getter, method, kernel, context function and station function in
    /// registration order, then the sandbox's and the packages' â€” and none of
    /// the five verbs a mind never has.
    #[test]
    fn the_registered_names_are_pinned() {
        let names = registered_names();
        let pinned: Vec<&str> = PINNED.to_vec();
        assert_eq!(
            names.iter().map(String::as_str).collect::<Vec<_>>(),
            pinned,
            "the registered names moved: a change to the API is an API_VERSION bump"
        );
        let unique: BTreeSet<&str> = PINNED.iter().copied().collect();
        // `sleep` is pinned twice: the sandbox's shadow and the package's own.
        assert_eq!(unique.len(), PINNED.len() - 1, "a name registered twice");
        for never in ["order", "release", "place", "board", "exit_hull", "eject"] {
            assert!(
                !names.iter().any(|name| name == never),
                "{never} is a verb a mind never has (D-UM-11)"
            );
        }
    }

    /// **Every pinned name resolves from a script** â€” the list kept beside the
    /// registration proven against the engine: each name called (or read) once
    /// on the full view: every API name on the scratch engine, where each
    /// answers (none is refused on the full view), and every sandbox and
    /// package name in a think on [`standard_sandbox`], where none answers
    /// rhai's "not found" (a refusal there â€” `sleep`'s â€” is an answer: it
    /// proves the name resolved to the shadow).
    #[test]
    fn every_registered_name_resolves_from_a_script() {
        let calls = CALLS;
        let called: BTreeSet<&str> = calls.iter().map(|(name, _)| *name).collect();
        for name in registered_names() {
            if name.starts_with("type ")
                || SANDBOX_NAMES.contains(&name.as_str())
                || PACKAGE_NAMES.contains(&name.as_str())
            {
                continue;
            }
            assert!(called.contains(name.as_str()), "{name} is never called");
        }
        // Every API call answers on the full view: none is refused there.
        for (name, expr) in calls {
            if let Err(error) = fixture::eval(expr) {
                panic!("{name} ({expr}) did not answer: {error}");
            }
        }
        for name in SANDBOX_NAMES.iter().chain(PACKAGE_NAMES.iter()) {
            let expr = PACKAGE_CALLS
                .iter()
                .find(|(called, _)| called == name)
                .map(|(_, expr)| *expr)
                .unwrap_or_else(|| panic!("{name} has no call"));
            let source = format!("fn think(v, c) {{ let x = 1; {expr}; }}");
            let ran = think(&source);
            if let Err(Fault::Type(text)) = &ran.result {
                assert!(
                    !text.contains("not found"),
                    "{name} ({expr}) did not resolve on the sandbox: {text}"
                );
            }
        }
        // The shadow, not the package's: `sleep` refuses at once.
        let ran = think("fn think(v, c) { sleep(1); }");
        assert!(matches!(ran.result, Err(Fault::Type(_))));
        // A custom type reads its registered name.
        let kind = fixture::eval("type_of(v)").expect("type_of");
        assert_eq!(fixture::shown(&kind), "\"View\"");
    }

    /// The pinned list (see [`the_registered_names_are_pinned`]).
    const PINNED: [&str; 281] = [
        "type View",
        "type Objective",
        "type Motor",
        "type Contact",
        "type Weapon",
        "type Section",
        "type Facet",
        "type Heat",
        "type Receiver",
        "type Order",
        "type Notice",
        "type Condition",
        "type Armed",
        "type Rule",
        "type Predicate",
        "View.unit",
        "View.callsign",
        "View.cell",
        "View.mode",
        "View.objective",
        "View.motor",
        "Objective.cell",
        "Objective.letter",
        "Objective.radius_cells",
        "Motor.goal",
        "Motor.outcome",
        "View.x_cm",
        "View.y_cm",
        "View.z_cm",
        "View.heading_mdeg",
        "View.speed_cms",
        "View.power",
        "View.held",
        "View.wreck",
        "View.immobile",
        "View.sight_m",
        "View.ecm",
        "View.receiver",
        "View.heat",
        "View.sections",
        "View.weapons",
        "View.rules",
        "View.lock",
        "View.order",
        "View.t_ms",
        "View.notices",
        "View.contacts",
        "View.scan_tables",
        "View.odds_floor_permille",
        "Contact.id",
        "Contact.designator",
        "Contact.alias",
        "Contact.identified",
        "Contact.friendly",
        "Contact.relation",
        "Contact.bearing_deg",
        "Contact.relative_deg",
        "Contact.range_m",
        "Contact.band",
        "Contact.age_ms",
        "Contact.cell",
        "Contact.condition",
        "Contact.moving",
        "Contact.fatal",
        "Contact.aspect",
        "Contact.odds",
        "Contact.reads_dead",
        "Weapon.index",
        "Weapon.id",
        "Weapon.ready",
        "Weapon.ready_in_ms",
        "Weapon.rounds",
        "Weapon.rounds_max",
        "Weapon.range_band",
        "Weapon.p_permille",
        "Weapon.bands_cm",
        "Weapon.heat_per_shot",
        "Weapon.armed",
        "Armed.slot",
        "Armed.target",
        "Armed.min_permille",
        "Armed.aspects",
        "Armed.keeps",
        "Armed.min_range_m",
        "Armed.max_range_m",
        "Rule.slot",
        "Rule.action",
        "Rule.channel",
        "Rule.weapon",
        "Rule.target",
        "Rule.contact",
        "Rule.selector",
        "Rule.min_permille",
        "Rule.aspects",
        "Rule.keeps",
        "Rule.min_range_m",
        "Rule.max_range_m",
        "Rule.implicit",
        "Rule.independent",
        "Rule.predicates",
        "Predicate.kind",
        "Predicate.weapon",
        "Predicate.comparison",
        "Predicate.value",
        "Predicate.percent",
        "Predicate.section",
        "Predicate.state",
        "Predicate.band",
        "Predicate.present",
        "Section.id",
        "Section.condition",
        "Section.internals",
        "Section.internals_max",
        "Section.facets",
        "Facet.id",
        "Facet.armor",
        "Facet.armor_max",
        "Heat.heat",
        "Heat.max",
        "Heat.overheated",
        "Receiver.budget",
        "Receiver.budget_max",
        "Receiver.reach_permille",
        "Condition.table",
        "Condition.bands",
        "Condition.age_ms",
        "Order.squad",
        "Order.identity",
        "Order.objective",
        "Order.to",
        "Order.target",
        "Order.facing_mdeg",
        "Order.of",
        "Order.hold_fire",
        "Order.max_range_band",
        "Order.expires_ms",
        "Notice.kind",
        "Notice.contact",
        "Notice.designator",
        "Notice.was",
        "Notice.section",
        "Notice.cause",
        "Notice.refusal",
        "Notice.order",
        "Notice.weapon",
        "Notice.radius_m",
        "Notice.secs",
        "Notice.heat",
        "Notice.max",
        "Notice.floor_permille",
        "Notice.why",
        "Notice.floor",
        "Notice.range_m",
        "Notice.min_m",
        "Notice.max_m",
        "Notice.rounds",
        "Notice.keeps",
        "Notice.friendly",
        "Notice.spent",
        "Notice.reason",
        "Notice.launch_cell",
        "Notice.outcome",
        "Notice.slot",
        "Notice.percent",
        "Notice.aspect",
        "type Card",
        "Card.stance",
        "Card.band_permille",
        "Card.close_m",
        "Card.orbit_sense",
        "Card.turn_into_hits",
        "Card.turn_s",
        "Card.screen_bearing",
        "Card.screen_arc",
        "Card.opening",
        "Card.min_odds_permille",
        "Card.aspect",
        "Card.reserve_pct",
        "Card.reserve_odds_pct",
        "Card.prefer",
        "Card.weight_range",
        "Card.weight_facet",
        "Card.weight_health",
        "Card.switch_m",
        "Card.withdraw_permille",
        "Card.stand_min_permille",
        "Card.rally_ms",
        "Card.search_ms",
        "Card.hunt_after_ms",
        "Card.stop_unseen",
        "Card.get",
        "type Ranges",
        "Ranges.engage_cm",
        "Ranges.release_cm",
        "Ranges.stand_cm",
        "Ranges.limit_cm",
        "type Walk",
        "Walk.waypoint",
        "Walk.from_waypoint",
        "Walk.goal",
        "Walk.clear_route",
        "best_hostile",
        "health_permille",
        "is_dry",
        "holds_a_hostile",
        "would_overheat",
        "fight_limit_cm",
        "contact",
        "ranges",
        "line_to",
        "terrain_at",
        "spot_cell",
        "entry_cell",
        "entry_interior_cell",
        "far_entry_cell",
        "stand_off_cell",
        "patrol_step",
        "hold_arrived",
        "hold_contains",
        "hold_advance_arrived",
        "fall_back_to_waypoint",
        "sin_m",
        "cos_m",
        "bearing_to",
        "range_to",
        "arc",
        "stand_off_want",
        "rand",
        "lands_ms",
        "t_ms",
        "decide_ms",
        "repath_ms",
        "cell_cm",
        "leash_cells",
        "station_cells",
        "screen_half_arc_mdeg",
        "stand_ticks",
        "exposure_front",
        "exposure_flank",
        "exposure_rear",
        "arrive_cells",
        "slots",
        "blocked",
        "cover",
        "helm",
        "speed",
        "startup",
        "goto",
        "hold",
        "goto_off",
        "lock",
        "emcon",
        "fire",
        "arm",
        "disarm",
        "trace",
        "stock::think",
        "stock::rules",
        "stock::fire_discipline",
        "stock::best_hostile",
        "stock::hold_band",
        "stock::steer_to",
        "stock::hunt",
        "clamp",
        "isqrt",
        "sleep",
        "abs",
        "min",
        "max",
        "exit",
        "take",
        "tag",
        "set_tag",
        "sleep",
        "parse_json",
        "get_fn_metadata_list",
        "print",
        "debug",
        "type_of",
        "is_def_fn",
        "is_def_var",
    ];

    /// One call of every API name on the full view (`v`, `c` in scope).
    const CALLS: [(&str, &str); 245] = [
        ("View.unit", "v.unit"),
        ("View.callsign", "v.callsign"),
        ("View.cell", "v.cell"),
        ("View.mode", "v.mode"),
        ("View.objective", "v.objective"),
        ("View.motor", "v.motor"),
        ("Objective.cell", "v.objective.cell"),
        ("Objective.letter", "v.objective.letter"),
        ("Objective.radius_cells", "v.objective.radius_cells"),
        ("Motor.goal", "v.motor.goal"),
        ("Motor.outcome", "v.motor.outcome"),
        ("View.x_cm", "v.x_cm"),
        ("View.y_cm", "v.y_cm"),
        ("View.z_cm", "v.z_cm"),
        ("View.heading_mdeg", "v.heading_mdeg"),
        ("View.speed_cms", "v.speed_cms"),
        ("View.power", "v.power"),
        ("View.held", "v.held"),
        ("View.wreck", "v.wreck"),
        ("View.immobile", "v.immobile"),
        ("View.sight_m", "v.sight_m"),
        ("View.ecm", "v.ecm"),
        ("View.receiver", "v.receiver"),
        ("View.heat", "v.heat"),
        ("View.sections", "v.sections"),
        ("View.weapons", "v.weapons"),
        ("View.rules", "v.rules"),
        ("View.lock", "v.lock"),
        ("View.order", "v.order"),
        ("View.t_ms", "v.t_ms"),
        ("View.notices", "v.notices"),
        ("View.contacts", "v.contacts"),
        ("View.scan_tables", "v.scan_tables"),
        ("View.odds_floor_permille", "v.odds_floor_permille"),
        ("Contact.id", "v.contacts[0].id"),
        ("Contact.designator", "v.contacts[0].designator"),
        ("Contact.alias", "v.contacts[0].alias"),
        ("Contact.identified", "v.contacts[0].identified"),
        ("Contact.moving", "v.contacts[0].moving"),
        ("Contact.friendly", "v.contacts[0].friendly"),
        ("Contact.relation", "v.contacts[0].relation"),
        ("Contact.bearing_deg", "v.contacts[0].bearing_deg"),
        ("Contact.relative_deg", "v.contacts[0].relative_deg"),
        ("Contact.range_m", "v.contacts[0].range_m"),
        ("Contact.band", "v.contacts[0].band"),
        ("Contact.age_ms", "v.contacts[0].age_ms"),
        ("Contact.cell", "v.contacts[0].cell"),
        ("Contact.condition", "v.contacts[0].condition"),
        ("Contact.fatal", "v.contacts[0].fatal"),
        ("Contact.aspect", "v.contacts[0].aspect"),
        ("Contact.odds", "v.contacts[0].odds(\"cannon\")"),
        ("Contact.reads_dead", "v.contacts[0].reads_dead"),
        ("Weapon.index", "v.weapons[0].index"),
        ("Weapon.id", "v.weapons[0].id"),
        ("Weapon.ready", "v.weapons[0].ready"),
        ("Weapon.ready_in_ms", "v.weapons[0].ready_in_ms"),
        ("Weapon.rounds", "v.weapons[0].rounds"),
        ("Weapon.rounds_max", "v.weapons[0].rounds_max"),
        ("Weapon.range_band", "v.weapons[0].range_band"),
        ("Weapon.p_permille", "v.weapons[0].p_permille"),
        ("Weapon.bands_cm", "v.weapons[0].bands_cm"),
        ("Weapon.heat_per_shot", "v.weapons[0].heat_per_shot"),
        ("Weapon.armed", "v.weapons[0].armed"),
        ("Armed.slot", "v.weapons[0].armed.slot"),
        ("Armed.target", "v.weapons[0].armed.target"),
        ("Armed.min_permille", "v.weapons[0].armed.min_permille"),
        ("Armed.aspects", "v.weapons[0].armed.aspects"),
        ("Armed.keeps", "v.weapons[0].armed.keeps"),
        ("Armed.min_range_m", "v.weapons[0].armed.min_range_m"),
        ("Armed.max_range_m", "v.weapons[0].armed.max_range_m"),
        ("Rule.slot", "v.rules[0].slot"),
        ("Rule.action", "v.rules[0].action"),
        ("Rule.channel", "v.rules[0].channel"),
        ("Rule.weapon", "v.rules[0].weapon"),
        ("Rule.target", "v.rules[0].target"),
        ("Rule.contact", "v.rules[0].contact"),
        ("Rule.selector", "v.rules[0].selector"),
        ("Rule.min_permille", "v.rules[0].min_permille"),
        ("Rule.aspects", "v.rules[0].aspects"),
        ("Rule.keeps", "v.rules[0].keeps"),
        ("Rule.min_range_m", "v.rules[0].min_range_m"),
        ("Rule.max_range_m", "v.rules[0].max_range_m"),
        ("Rule.implicit", "v.rules[0].implicit"),
        ("Rule.independent", "v.rules[0].independent"),
        ("Rule.predicates", "v.rules[0].predicates"),
        ("Predicate.kind", "v.rules[0].predicates[0].kind"),
        ("Predicate.weapon", "v.rules[0].predicates[0].weapon"),
        (
            "Predicate.comparison",
            "v.rules[0].predicates[0].comparison",
        ),
        ("Predicate.value", "v.rules[0].predicates[0].value"),
        ("Predicate.percent", "v.rules[0].predicates[0].percent"),
        ("Predicate.section", "v.rules[0].predicates[0].section"),
        ("Predicate.state", "v.rules[0].predicates[0].state"),
        ("Predicate.band", "v.rules[0].predicates[0].band"),
        ("Predicate.present", "v.rules[0].predicates[0].present"),
        ("Section.id", "v.sections[0].id"),
        ("Section.condition", "v.sections[0].condition"),
        ("Section.internals", "v.sections[0].internals"),
        ("Section.internals_max", "v.sections[0].internals_max"),
        ("Section.facets", "v.sections[0].facets"),
        ("Facet.id", "v.sections[0].facets[0].id"),
        ("Facet.armor", "v.sections[0].facets[0].armor"),
        ("Facet.armor_max", "v.sections[0].facets[0].armor_max"),
        ("Heat.heat", "v.heat.heat"),
        ("Heat.max", "v.heat.max"),
        ("Heat.overheated", "v.heat.overheated"),
        ("Receiver.budget", "v.receiver.budget"),
        ("Receiver.budget_max", "v.receiver.budget_max"),
        ("Receiver.reach_permille", "v.receiver.reach_permille"),
        ("Condition.table", "v.contacts[0].condition.table"),
        ("Condition.bands", "v.contacts[0].condition.bands"),
        ("Condition.age_ms", "v.contacts[0].condition.age_ms"),
        ("Order.squad", "v.order.squad"),
        ("Order.identity", "v.order.identity"),
        ("Order.objective", "v.order.objective"),
        ("Order.to", "v.order.to"),
        ("Order.target", "v.order.target"),
        ("Order.facing_mdeg", "v.order.facing_mdeg"),
        ("Order.of", "v.order.of"),
        ("Order.hold_fire", "v.order.hold_fire"),
        ("Order.max_range_band", "v.order.max_range_band"),
        ("Order.expires_ms", "v.order.expires_ms"),
        ("Notice.kind", "v.notices[0].kind"),
        ("Notice.contact", "v.notices[0].contact"),
        ("Notice.designator", "v.notices[0].designator"),
        ("Notice.was", "v.notices[0].was"),
        ("Notice.section", "v.notices[0].section"),
        ("Notice.cause", "v.notices[0].cause"),
        ("Notice.refusal", "v.notices[0].refusal"),
        ("Notice.order", "v.notices[0].order"),
        ("Notice.weapon", "v.notices[0].weapon"),
        ("Notice.radius_m", "v.notices[0].radius_m"),
        ("Notice.secs", "v.notices[0].secs"),
        ("Notice.heat", "v.notices[0].heat"),
        ("Notice.max", "v.notices[0].max"),
        ("Notice.floor_permille", "v.notices[0].floor_permille"),
        ("Notice.why", "v.notices[0].why"),
        ("Notice.floor", "v.notices[0].floor"),
        ("Notice.range_m", "v.notices[0].range_m"),
        ("Notice.min_m", "v.notices[0].min_m"),
        ("Notice.max_m", "v.notices[0].max_m"),
        ("Notice.rounds", "v.notices[0].rounds"),
        ("Notice.keeps", "v.notices[0].keeps"),
        ("Notice.friendly", "v.notices[0].friendly"),
        ("Notice.spent", "v.notices[0].spent"),
        ("Notice.reason", "v.notices[0].reason"),
        ("Notice.launch_cell", "v.notices[0].launch_cell"),
        ("Notice.outcome", "v.notices[0].outcome"),
        ("Notice.slot", "v.notices[0].slot"),
        ("Notice.percent", "v.notices[0].percent"),
        ("Notice.aspect", "v.notices[0].aspect"),
        ("Card.stance", "c.stance"),
        ("Card.band_permille", "c.band_permille"),
        ("Card.close_m", "c.close_m"),
        ("Card.orbit_sense", "c.orbit_sense"),
        ("Card.turn_into_hits", "c.turn_into_hits"),
        ("Card.turn_s", "c.turn_s"),
        ("Card.screen_bearing", "c.screen_bearing"),
        ("Card.screen_arc", "c.screen_arc"),
        ("Card.opening", "c.opening"),
        ("Card.min_odds_permille", "c.min_odds_permille"),
        ("Card.aspect", "c.aspect"),
        ("Card.reserve_pct", "c.reserve_pct"),
        ("Card.reserve_odds_pct", "c.reserve_odds_pct"),
        ("Card.prefer", "c.prefer"),
        ("Card.weight_range", "c.weight_range"),
        ("Card.weight_facet", "c.weight_facet"),
        ("Card.weight_health", "c.weight_health"),
        ("Card.switch_m", "c.switch_m"),
        ("Card.withdraw_permille", "c.withdraw_permille"),
        ("Card.stand_min_permille", "c.stand_min_permille"),
        ("Card.rally_ms", "c.rally_ms"),
        ("Card.search_ms", "c.search_ms"),
        ("Card.hunt_after_ms", "c.hunt_after_ms"),
        ("Card.stop_unseen", "c.stop_unseen"),
        ("Card.get", "c.get(\"stance\")"),
        ("Ranges.engage_cm", "ranges(v, 800, true).engage_cm"),
        ("Ranges.release_cm", "ranges(v, 800, true).release_cm"),
        ("Ranges.stand_cm", "ranges(v, 800, true).stand_cm"),
        ("Ranges.limit_cm", "ranges(v, 800, true).limit_cm"),
        (
            "Walk.waypoint",
            "patrol_step(v, false, (), (), ()).waypoint",
        ),
        (
            "Walk.from_waypoint",
            "patrol_step(v, false, (), (), ()).from_waypoint",
        ),
        ("Walk.goal", "patrol_step(v, false, (), (), ()).goal"),
        (
            "Walk.clear_route",
            "patrol_step(v, false, (), (), ()).clear_route",
        ),
        (
            "best_hostile",
            "best_hostile(v, 1, 1, 1, 1000, (), (), (), ())",
        ),
        ("health_permille", "health_permille(v)"),
        ("is_dry", "is_dry(v)"),
        ("holds_a_hostile", "holds_a_hostile(v)"),
        ("would_overheat", "would_overheat(v, \"cannon\")"),
        ("fight_limit_cm", "fight_limit_cm(v)"),
        ("contact", "contact(v, v.lock)"),
        ("ranges", "ranges(v, 800, true)"),
        ("line_to", "line_to(v, v.lock)"),
        ("terrain_at", "terrain_at(v.cell)"),
        ("spot_cell", "spot_cell(\"A\")"),
        ("entry_cell", "entry_cell(v)"),
        ("entry_interior_cell", "entry_interior_cell(v)"),
        ("far_entry_cell", "far_entry_cell(v)"),
        ("stand_off_cell", "stand_off_cell(v, v.lock, 500)"),
        ("patrol_step", "patrol_step(v, false, (), (), ())"),
        ("hold_arrived", "hold_arrived(v)"),
        ("hold_contains", "hold_contains(v, [6, 4])"),
        ("hold_advance_arrived", "hold_advance_arrived(v, [6, 4])"),
        (
            "fall_back_to_waypoint",
            "fall_back_to_waypoint(v, [6, 4], (), ())",
        ),
        ("sin_m", "sin_m(90000)"),
        ("cos_m", "cos_m(90000)"),
        ("bearing_to", "bearing_to(v, [4, 4])"),
        ("range_to", "range_to(v, [4, 4])"),
        ("arc", "arc(0, 90000)"),
        ("stand_off_want", "stand_off_want(4000, 1000, 300)"),
        ("rand", "rand(10)"),
        ("lands_ms", "lands_ms()"),
        ("t_ms", "t_ms()"),
        ("decide_ms", "decide_ms()"),
        ("repath_ms", "repath_ms()"),
        ("cell_cm", "cell_cm()"),
        ("leash_cells", "leash_cells()"),
        ("station_cells", "station_cells()"),
        ("screen_half_arc_mdeg", "screen_half_arc_mdeg()"),
        ("stand_ticks", "stand_ticks()"),
        ("exposure_front", "exposure_front()"),
        ("exposure_flank", "exposure_flank()"),
        ("exposure_rear", "exposure_rear()"),
        ("arrive_cells", "arrive_cells()"),
        ("slots", "slots()"),
        ("blocked", "blocked()"),
        ("cover", "cover()"),
        ("helm", "helm(0)"),
        ("speed", "speed(0)"),
        ("startup", "startup()"),
        ("goto", "goto([1, 1])"),
        ("hold", "hold()"),
        ("goto_off", "goto_off()"),
        ("lock", "lock(v.lock)"),
        ("emcon", "emcon(true)"),
        ("fire", "fire(\"cannon\", 1)"),
        ("arm", "arm(\"cannon\", \"best\", 100, 15, 0, 0, 420)"),
        ("disarm", "disarm(())"),
        ("trace", "trace(\"t\")"),
        ("stock::think", "stock::think(v,c,#{})"),
        ("stock::rules", "stock::rules()"),
        ("stock::fire_discipline", "stock::fire_discipline(v,c,())"),
        ("stock::best_hostile", "stock::best_hostile(v,c)"),
        ("stock::hold_band", "stock::hold_band(v,c,v.lock)"),
        ("stock::steer_to", "stock::steer_to(v,[1,1])"),
        ("stock::hunt", "stock::hunt(v,c,#{})"),
    ];

    /// One call of every sandbox and package name, inside `think`.
    const PACKAGE_CALLS: [(&str, &str); 17] = [
        ("clamp", "clamp(15, 0, 10)"),
        ("isqrt", "isqrt(17)"),
        ("sleep", "sleep(1)"),
        ("abs", "abs(-3)"),
        ("min", "min(2, 5)"),
        ("max", "max(2, 5)"),
        ("exit", "exit()"),
        ("take", "take(x)"),
        ("tag", "x.tag"),
        ("set_tag", "x.set_tag(2)"),
        ("parse_json", "parse_json(\"{}\")"),
        ("get_fn_metadata_list", "get_fn_metadata_list()"),
        ("print", "print(\"p\")"),
        ("debug", "debug(\"d\")"),
        ("type_of", "type_of(x)"),
        ("is_def_fn", "is_def_fn(\"think\", 2)"),
        ("is_def_var", "is_def_var(\"x\")"),
    ];
}

pub(crate) mod bulk;
