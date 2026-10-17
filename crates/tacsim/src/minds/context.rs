//! **The think context** (17b cut M2.3; 17b's settled question (ii); doc 17
//! §3, §6, §7; D-UM-9, D-UM-26, D-UM-29, D-UM-31).
//!
//! What one think knows beside its two arguments, held in a thread-local for
//! the length of the think: the view and the map behind the arena's own `Arc`s
//! (a kernel reaches the map from inside the engine without a copy of a
//! sixty-five-thousand-cell terrain), the match seed, the hull, the think's
//! sequence number and its clocks, the counters a kernel's per-think cap
//! reads (`rand`'s draws, `line_to`'s traces), **the intent buffer** the
//! station functions fill (D-UM-9: at most
//! [`crate::arena::mind::MIND_INTENTS_MAX`]) and the refusal flag. The runner
//! (17b cut M2.4) [`set`]s it at the think's start and [`take`]s it at the end
//! — on the `Ok` path too, because a refusal a script could not see is still
//! a fault (`refused`), and an intent buffer is the think's answer.
//!
//! **One think per thread**, the rule `mush_mind`'s budget and trace already
//! rest on: the sim task calls its runner synchronously, and 17b cut M4's pool
//! runs one think per worker.
//!
//! # The context functions
//!
//! The per-think facts and the doctrine's constants a stock mind reads are
//! **zero-argument native functions** — `lands_ms()`, `t_ms()`,
//! `decide_ms()`, `repath_ms()`, `cell_cm()`, `leash_cells()`, `station_cells()`,
//! `screen_half_arc_mdeg()`, `stand_ticks()`, `exposure_front()`,
//! `exposure_flank()`, `exposure_rear()`, `arrive_cells()`, `slots()` (since
//! 17b cut M3.1 the think's own row's, `ThinkJob::profile`), `blocked()`,
//! `cover()` — never scope constants and never script `const`s
//! (17b §5's feas F1 / adv F1, verified against the rhai 1.26 source): under
//! strict variables a function body cannot name a scope variable, a top-level
//! `const` is invisible inside a `fn` and cleared with the statements, a
//! script-to-script call runs on an empty scope, and compiling with a scope
//! would fold the first think's `lands_ms` into an AST cached once. A native
//! function resolves at any call depth and `OptimizationLevel::Simple` never
//! calls one at compile time (`mush_mind`'s test 4b pins the contract), so
//! each think reads its own value. Fee 0. Each is a public fact — the hull's
//! own clock, the map's cell, a doctrine constant — and none names another
//! unit (doc 17 §3 "The map is public"). Called outside a think, each refuses
//! `no think is running`.

use std::cell::RefCell;
use std::sync::Arc;

use mush_core::ObjId;
use mush_mind::rhai::{EvalAltResult, INT, ImmutableString};
use mush_mind::system_error;

use super::Registrar;
use crate::arena::damage::{BLOCKED, COVER};
use crate::arena::doctrine::{
    EXPOSURE_FLANK, EXPOSURE_FRONT, EXPOSURE_REAR, SCREEN_HALF_ARC_MDEG, STAND_TICKS,
    SUPPORT_LEASH_CELLS, SUPPORT_STATION_CELLS,
};
use crate::arena::intents::IntentKind;
use crate::arena::mind::{MindView, ThinkJob};
use crate::arena::motor::ARRIVE_CELLS;
use crate::arena::state::ArenaDefinition;

/// **The cause word of a type refusal** (D-UM-31): an argument of the wrong
/// type, or past its type's bounds — a FLOAT among them (D-UM-28).
pub const REFUSED_TYPE: &str = "type";

/// **The cause word of an unknown id** (D-UM-31): a contact, weapon, slot or
/// spot the view or the map does not hold.
pub const REFUSED_UNKNOWN: &str = "unknown";

/// **The cause word of an intent overflow** (D-UM-9): a seventeenth push.
pub const REFUSED_INTENTS: &str = "intents";

/// **The cause word of a per-think cap** (D-UM-35's `line_to` column): a
/// third `line_to`.
pub const REFUSED_LIMIT: &str = "limit";

/// The sentence a context function answers outside a think.
const NO_THINK: &str = "no think is running";

/// **What one think knows beside its arguments** (17b's settled question
/// (ii)). Set by the runner ([`set`]) before `Sandbox::think` and taken back
/// ([`take`]) after it.
#[derive(Clone, Debug)]
pub struct ThinkContext {
    /// The hull's view — the arena's own `Arc` (`ThinkJob::view`), the one
    /// every handle the script holds points into.
    pub view: Arc<MindView>,
    /// The map — the arena's own definition (`ThinkJob::map`).
    pub map: Arc<ArenaDefinition>,
    /// The match seed (`ThinkJob::seed`): `rand`'s key.
    pub seed: u64,
    /// The hull (`ThinkJob::unit`): `rand`'s label.
    pub unit: ObjId,
    /// The think's sequence number: `rand`'s label.
    pub think_seq: u64,
    /// The think's lattice millisecond (`t_ms()`).
    pub t_ms: i64,
    /// The think's landing millisecond (`lands_ms()`).
    pub lands_ms: i64,
    /// The doctrine's think cadence, ms (`decide_ms()`): the stand's dwell is
    /// `stand_ticks() × decide_ms()`.
    pub decide_ms: u32,
    /// The motor's frozen minimum re-plan interval, used by Hold retry.
    pub repath_ms: u32,
    /// The draws taken so far: the next `rand`'s `n`.
    pub draws: u32,
    /// The `line_to` traces asked for so far.
    pub line_to_calls: u32,
    /// **The most `line_to` traces this think may ask for** (17b cut M3.1;
    /// D-UM-35's `line_to` column): the job's row's
    /// ([`ThinkJob::profile`]) — Class I's two,
    /// [`super::STOCK_LINE_TO_PER_THINK`], on every shipped chassis.
    pub line_to_max: u32,
    /// **The condition slots the think is held to** (17b cut M3.1;
    /// D-UM-35's `slots` column): the job's row's — what `slots()` answers
    /// and what `fire(weapon, slot)` bounds its slot number by.
    pub slots: usize,
    /// **The intent buffer** (D-UM-9): what the station functions pushed, in
    /// call order, at most `MIND_INTENTS_MAX`.
    pub intents: Vec<IntentKind>,
    /// A push past `MIND_INTENTS_MAX` was refused (the runner's `intents`).
    pub overflowed: bool,
    /// **The think's first refusal's cause word** — `type`, `unknown`,
    /// `intents` or `limit` — or `None`. Every refusal is also raised through
    /// `mush_mind::system_error`, so the think ends a `type` fault whatever the
    /// script does; the runner reads this word for the fault it journals, on
    /// either path (17b §1 (iv)).
    pub refused: Option<&'static str>,
}

impl ThinkContext {
    /// **A fresh context for one think of `job`** at the doctrine's
    /// `decide_ms`: the job's view, map, seed, hull, sequence and clocks;
    /// nothing drawn, traced, pushed or refused.
    #[must_use]
    pub fn of(job: &ThinkJob<'_>, decide_ms: u32, repath_ms: u32) -> ThinkContext {
        ThinkContext {
            view: Arc::clone(job.view),
            map: Arc::clone(job.map),
            seed: job.seed,
            unit: job.unit,
            think_seq: job.think_seq,
            t_ms: job.t_ms,
            lands_ms: job.lands_ms,
            decide_ms,
            repath_ms,
            draws: 0,
            line_to_calls: 0,
            line_to_max: job.profile.line_to,
            slots: job.profile.slots,
            intents: Vec::new(),
            overflowed: false,
            refused: None,
        }
    }
}

thread_local! {
    /// The think running on this thread, if one is.
    static THINK: RefCell<Option<ThinkContext>> = const { RefCell::new(None) };
}

/// **Open a think's context** on this thread, replacing whatever was left.
pub fn set(ctx: ThinkContext) {
    THINK.with(|think| *think.borrow_mut() = Some(ctx));
}

/// **Close the think's context** and hand it back — the intents, the flags
/// and the counters — or `None` when no think was set.
#[must_use]
pub fn take() -> Option<ThinkContext> {
    THINK.with(|think| think.borrow_mut().take())
}

/// **Run `f` on the running think's context**, or refuse `no think is
/// running` (a system error, never a panic). `f` must not call back into
/// this module's thread-local: a refusal inside it is built with [`refuse`]
/// on the context it was handed.
pub(crate) fn with<R>(f: impl FnOnce(&mut ThinkContext) -> R) -> Result<R, Box<EvalAltResult>> {
    THINK.with(|think| match think.try_borrow_mut() {
        Ok(mut held) => match held.as_mut() {
            Some(ctx) => Ok(f(ctx)),
            None => Err(system_error(NO_THINK)),
        },
        Err(_) => Err(system_error(NO_THINK)),
    })
}

/// **A refusal on the running think** (D-UM-31): the think's first refusal
/// word is kept on `ctx.refused`, and the error is `mush_mind::system_error`
/// — uncatchable by `try`, latched for the think. Constructing one is raising
/// one: build it only to return it.
pub(crate) fn refuse(ctx: &mut ThinkContext, word: &'static str, text: &str) -> Box<EvalAltResult> {
    if ctx.refused.is_none() {
        ctx.refused = Some(word);
    }
    system_error(text)
}

/// **A refusal from outside a [`with`]** — a kernel's or a station's argument
/// check before it reaches the context: [`refuse`] on the running think when
/// one is set (and its context is not already held), else the system error
/// alone.
pub(crate) fn refused(word: &'static str, text: &str) -> Box<EvalAltResult> {
    THINK.with(|think| {
        if let Ok(mut held) = think.try_borrow_mut()
            && let Some(ctx) = held.as_mut()
            && ctx.refused.is_none()
        {
            ctx.refused = Some(word);
        }
    });
    system_error(text)
}

/// A context function's INT, read off the running think.
fn read(f: impl FnOnce(&ThinkContext) -> INT) -> Result<INT, Box<EvalAltResult>> {
    with(|ctx| f(ctx))
}

/// A doctrine constant's INT, answered only inside a think.
fn constant(value: INT) -> Result<INT, Box<EvalAltResult>> {
    read(|_| value)
}

/// A doctrine word, answered only inside a think.
fn word(value: &'static str) -> Result<ImmutableString, Box<EvalAltResult>> {
    with(|_| ImmutableString::from(value))
}

/// **The fifteen context functions** (the file head), each zero-argument,
/// fee 0.
pub(crate) fn register(api: &mut Registrar<'_>) {
    function!(api, "lands_ms", || { read(|ctx| ctx.lands_ms) });
    function!(api, "t_ms", || { read(|ctx| ctx.t_ms) });
    function!(api, "decide_ms", || {
        read(|ctx| INT::from(ctx.decide_ms))
    });
    function!(api, "repath_ms", || {
        read(|ctx| INT::from(ctx.repath_ms))
    });
    function!(api, "cell_cm", || {
        read(|ctx| INT::from(ctx.map.terrain.cell_cm()))
    });
    function!(api, "leash_cells", || {
        constant(INT::from(SUPPORT_LEASH_CELLS))
    });
    function!(api, "station_cells", || {
        constant(INT::from(SUPPORT_STATION_CELLS))
    });
    function!(api, "screen_half_arc_mdeg", || {
        constant(INT::from(SCREEN_HALF_ARC_MDEG))
    });
    function!(api, "stand_ticks", || { constant(STAND_TICKS) });
    function!(api, "exposure_front", || {
        constant(INT::from(EXPOSURE_FRONT))
    });
    function!(api, "exposure_flank", || {
        constant(INT::from(EXPOSURE_FLANK))
    });
    function!(api, "exposure_rear", || {
        constant(INT::from(EXPOSURE_REAR))
    });
    function!(api, "arrive_cells", || {
        constant(INT::from(ARRIVE_CELLS))
    });
    // 17b cut M3.1: the think's own row's slots, no longer a constant.
    function!(api, "slots", || {
        read(|ctx| INT::try_from(ctx.slots).unwrap_or(INT::MAX))
    });
    function!(api, "blocked", || { word(BLOCKED) });
    function!(api, "cover", || { word(COVER) });
}

#[cfg(test)]
mod tests {
    use mush_mind::memory::{Value, decode};

    use super::super::fixture::{self, card, context_on, think_with, view};
    use super::*;

    /// 17b §2's M2.3 test 5: **every context function read from a helper `fn`
    /// equals the context's value**, and two thinks on one cached AST with
    /// different `lands_ms` read different values — the native call is made
    /// per think, never folded into the AST.
    #[test]
    fn the_context_functions_answer_the_context_inside_a_helper() {
        let source = "
            fn deep() {
                [lands_ms(), t_ms(), decide_ms(), repath_ms(), cell_cm(), leash_cells(), station_cells(),
                 screen_half_arc_mdeg(), stand_ticks(), exposure_front(), exposure_flank(),
                 exposure_rear(), arrive_cells(), slots()]
            }
            fn helper() { deep() }
            fn think(v, c) {
                this.facts = helper();
                this.blocked = blocked() == \"blocked\";
                this.cover = cover() == \"cover\";
            }
        ";
        let ran = think_with(source, context_on(view()), card(), &[]);
        let outcome = ran.result.expect("a clean think");
        let memory = decode(&outcome.memory).expect("canonical memory");
        let expected: Vec<i64> = vec![
            5_100,
            5_000,
            1_000,
            4_000,
            1_000,
            i64::from(SUPPORT_LEASH_CELLS),
            i64::from(SUPPORT_STATION_CELLS),
            i64::from(SCREEN_HALF_ARC_MDEG),
            STAND_TICKS,
            i64::from(EXPOSURE_FRONT),
            i64::from(EXPOSURE_FLANK),
            i64::from(EXPOSURE_REAR),
            i64::from(ARRIVE_CELLS),
            i64::try_from(
                crate::arena::core::ClassTable::STOCK
                    .row(crate::arena::core::CoreClass::I)
                    .slots,
            )
            .unwrap(),
        ];
        assert_eq!(memory.get("facts"), Some(&Value::Ints(expected)));
        assert_eq!(memory.get("blocked"), Some(&Value::Bool(true)));
        assert_eq!(memory.get("cover"), Some(&Value::Bool(true)));
        assert_eq!(ran.ctx.refused, None);

        // Two thinks, one cached AST, two landing milliseconds.
        let source = "fn at() { lands_ms() } fn think(v, c) { this.at = at(); }";
        let mut early = context_on(view());
        early.lands_ms = 1_100;
        let mut late = context_on(view());
        late.lands_ms = 2_100;
        let sandbox = super::super::standard_sandbox();
        let compiled = |source: &str| {
            sandbox
                .compile(source, fixture::digest(source), super::super::MIND_ABI)
                .expect("compiles")
        };
        // One AST: the second compile is the cache's answer.
        assert!(Arc::ptr_eq(&compiled(source), &compiled(source)));
        let first = think_with(source, early, card(), &[]);
        let second = think_with(source, late, card(), &[]);
        let read = |ran: &fixture::Ran| {
            let outcome = ran.result.as_ref().expect("a clean think");
            decode(&outcome.memory)
                .expect("canonical")
                .get("at")
                .cloned()
        };
        assert_eq!(read(&first), Some(Value::Int(1_100)));
        assert_eq!(read(&second), Some(Value::Int(2_100)));
    }

    /// Outside a think a context function refuses, and `take` with nothing
    /// set answers `None` — never a panic.
    #[test]
    fn a_context_function_outside_a_think_refuses() {
        let _left = take();
        assert!(with(|ctx| ctx.t_ms).is_err());
        assert!(take().is_none());
        let error = fixture::scratch()
            .eval::<INT>("lands_ms()")
            .expect_err("no think is running");
        assert!(error.to_string().contains(NO_THINK), "{error}");
    }

    /// The first refusal's word is the one kept.
    #[test]
    fn the_first_refusal_is_the_word_kept() {
        let mut ctx = context_on(view());
        let _first = refuse(&mut ctx, REFUSED_UNKNOWN, "first");
        let _second = refuse(&mut ctx, REFUSED_LIMIT, "second");
        assert_eq!(ctx.refused, Some(REFUSED_UNKNOWN));
        set(ctx);
        let _third = refused(REFUSED_TYPE, "third");
        assert_eq!(take().and_then(|ctx| ctx.refused), Some(REFUSED_UNKNOWN));
    }
}
