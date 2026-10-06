//! **The engine** (17b cut M2.1; doc 17 §7 "The engine", "Limits",
//! "Parameters", "Floats"; D-UM-24, D-UM-25 as amended 2026-10-04, D-UM-27,
//! D-UM-28, D-UM-31).
//!
//! [`Sandbox::new`] builds the one `rhai::Engine` every think of a process
//! runs on (the `sync` feature makes `Engine` and `AST` `Send + Sync`, so 17b
//! cut M4's pool shares one engine and one AST per seal). The builder is the
//! whole of the sandbox's reach: a script can call what the seven packages
//! carry, what this file registers (the integer helpers and the two `sleep`
//! shadows) and what the game's closure registers, and nothing else — no
//! clock, no file, no module, no `eval`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use rhai::module_resolvers::DummyModuleResolver;
use rhai::packages::{
    ArithmeticPackage, BasicArrayPackage, BasicIteratorPackage, BasicMapPackage,
    BasicStringPackage, LanguageCorePackage, LogicPackage, Package,
};
use rhai::{Engine, EvalAltResult, FLOAT, INT};

use crate::cache::{Compiled, Digest};
use crate::memory::Limits;
use crate::{budget, trace};

/// **The rhai this sandbox was built and pinned against** (doc 17 §13,
/// D-UM-24): the first part of the game's `MIND_ABI` (`rhai=1.26.0;api=N;
/// stock=M`). The workspace pins `rhai = "=1.26.0"` exactly, because an
/// operation count — and with it where a budget trips — is a function of the
/// AST and the engine version; a rhai upgrade is a reviewed ABI bump, and
/// `the_abi_pin_matches_the_lockfile` holds this string, the workspace
/// manifest's pin and `Cargo.lock`'s resolved version together.
pub const RHAI_ABI: &str = "rhai=1.26.0";

/// **The largest memory any class may hold** (doc 17 §9's class table, Class
/// III; D-UM-30 as amended): 128 keys of at most 24 characters, string values
/// of at most 32 bytes, integer arrays of at most 32. The engine's data-size
/// backstop below is asserted at twice these totals
/// (`the_backstop_is_twice_the_largest_class`).
pub const MAX_MEMORY_ANY_CLASS: Limits = Limits {
    keys: 128,
    key_chars: 24,
    string_bytes: 32,
    array_len: 32,
};

/// **The largest op ceiling any class may run** (doc 17 §9's class table, Class
/// III's `ops`; 17b cut M3.1): 4,500 operations plus fees a think. The default
/// engine backstop ([`EngineLimits::default`]) — the one engine every class's
/// think runs on — and the ceiling past which [`Sandbox::think`] refuses a
/// think before running it ([`crate::Fault::Limit`]): a caller that hands a
/// think more than the engine was built for is refused, never trusted.
pub const MAX_OPS_ANY_CLASS: u64 = 4_500;

/// **The engine's map-size backstop** (D-UM-25 as amended 2026-10-04 after
/// review, finding 6): the data-size limits are a shared backstop against
/// runaway allocation only — twice the largest valid Class III memory —
/// because Rhai 1.26 counts nested elements and string bytes cumulatively over
/// a whole value at every assignment (the limits as first written refused a
/// Class III `mem` past 64 keys, a third 32-int array in every class and a
/// fifth 32-character string in every class). The per-key, per-value and
/// per-class limits are D-UM-30's, enforced by [`crate::memory::check`] when
/// the think ends.
pub const MAX_MAP_SIZE: usize = 256;

/// **The engine's array-size backstop** (D-UM-25 as amended; see
/// [`MAX_MAP_SIZE`]): elements counted cumulatively over a whole value, so
/// at least twice 128 keys × 32 integers.
pub const MAX_ARRAY_SIZE: usize = 8_192;

/// **The engine's string-size backstop, bytes** (D-UM-25 as amended; see
/// [`MAX_MAP_SIZE`]): bytes counted cumulatively over a whole value, so at
/// least twice 128 keys × 32 bytes.
pub const MAX_STRING_BYTES: usize = 8_192;

/// **What the caller sets on the engine** (D-UM-25: `max_operations` at the
/// class ceiling, a backstop only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineLimits {
    /// **Rhai's own operation cap** (`set_max_operations`): the highest
    /// ceiling any think on this engine is given. A backstop: the budget of
    /// D-UM-26 ([`crate::budget`]) trips first wherever a think's
    /// `ops_limit` plus its fees is under it. A think whose operations reach
    /// it uncharged ends `limit`, not `ops` — Rhai checks its cap before it
    /// calls `on_progress` — so a caller keeps every think's `ops_limit` at or
    /// under this and charges each a fee (doc 17 §7's fixed 20 per think).
    /// Rhai counts this cap per runtime state, and a package method's
    /// callback runs on a clone that restarts from its caller's count, so it
    /// does not bound a think's operations summed over its callbacks; the
    /// budget, which counts every operation itself, does ([`crate::budget`]).
    pub max_ops_backstop: u64,
}

impl Default for EngineLimits {
    /// The largest class's ceiling (doc 17 §9, D-UM-35; 17b cut M3.1):
    /// [`MAX_OPS_ANY_CLASS`], 4,500 operations — Class I's 1,500 until M3.1.
    fn default() -> Self {
        EngineLimits {
            max_ops_backstop: MAX_OPS_ANY_CLASS,
        }
    }
}

/// **The sandbox**: the one engine and the AST cache (doc 17 §7, D-UM-24,
/// D-UM-27). Built once per process by the game ([`Sandbox::new`]), shared by
/// every think ([`Sandbox::think`]), compiled into at the seal
/// ([`Sandbox::compile`]).
pub struct Sandbox {
    /// The engine every think runs on.
    pub(crate) engine: Engine,
    /// One compiled mind per `(source digest, MIND_ABI)` (D-UM-27): 300 hulls
    /// on 300 differently tuned cards compile once.
    pub(crate) asts: Mutex<BTreeMap<(Digest, &'static str), Arc<Compiled>>>,
    /// **The engine's op backstop** (17b cut M3.1): `limits.max_ops_backstop`
    /// as built, which [`Sandbox::think`] refuses a think's `ops_limit` past.
    pub(crate) max_ops_backstop: u64,
}

impl fmt::Debug for Sandbox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sandbox")
            .field("cached", &self.cached())
            .finish_non_exhaustive()
    }
}

impl Sandbox {
    /// **Build the sandbox** (D-UM-24, D-UM-25 as amended, D-UM-27), then
    /// hand the engine to `api` for the game's own types and functions (17b
    /// cut M2.3's `register_api`).
    ///
    /// * `Engine::new_raw()` plus **exactly** `LanguageCorePackage`,
    ///   `ArithmeticPackage`, `LogicPackage`, `BasicIteratorPackage`,
    ///   `BasicArrayPackage`, `BasicMapPackage` and `BasicStringPackage` as
    ///   global modules — never `BasicTimePackage` (a wall clock),
    ///   `BasicMathPackage` (transcendentals and parsing), `MoreStringPackage`
    ///   (string-plus-anything, string `len`) or `BasicFnPackage` (a function
    ///   pointer's `name` and `is_anonymous`; with `Fn`, `call`, `curry` and
    ///   closures gone, a function pointer is reachable only as a bare
    ///   script-function name handed to a package method as its callback). The arithmetic and logic packages carry float
    ///   overloads, reachable in expressions (D-UM-28: floats may appear in
    ///   expressions; no float function is registered by the game, and a
    ///   FLOAT reaching memory is a `type` fault).
    /// * `DummyModuleResolver`, and `eval`, `import`, `export`, `Fn`, `call`
    ///   and `curry` disabled through `disable_symbol`.
    /// * **`sleep` shadowed.** `LanguageCorePackage` carries `sleep(INT)` and
    ///   `sleep(FLOAT)` — a `std::thread::sleep` — and `disable_symbol` acts on
    ///   reserved keywords and literal tokens only, never on an identifier, so
    ///   it cannot remove them. But the engine's own namespace is searched
    ///   before every package, so the two functions registered here, each
    ///   raising [`system_error`], shadow the package's: a mind never reads a
    ///   clock and never waits on one (doc 17 §7).
    /// * **`exit()` is a return**, as the package ships it: see
    ///   [`Sandbox::think`]. `parse_json` and the reflection functions stay as
    ///   the package ships them (a FLOAT or a map they yield faults at the
    ///   memory boundary).
    /// * The limits of D-UM-25 as amended: call levels 12, expression depths
    ///   48 at the top level and 24 in a function body, 64 variables, 32
    ///   functions, no module, strict variables (a typo is a seal-time
    ///   error), a read of a missing map property an error, no anonymous
    ///   function; the data-size backstop [`MAX_MAP_SIZE`],
    ///   [`MAX_ARRAY_SIZE`], [`MAX_STRING_BYTES`]; `max_operations` at
    ///   `limits.max_ops_backstop`.
    /// * The optimisation level left at rhai's default, `Simple` — never
    ///   `Full` (D-UM-27): `Simple` folds the built-in operators on constants
    ///   and never calls a registered function at compile time, which is what
    ///   the game's zero-argument context functions (`lands_ms()` and the
    ///   rest, 17b cut M2.3) rely on to answer each think's own value from an
    ///   AST compiled once.
    /// * `print` and `debug` to the think's trace ([`trace::push`]);
    ///   `on_progress` to the budget ([`budget::check`]) — no clock, ever.
    /// * The integer helpers the packages lack: `clamp(v, lo, hi)` and
    ///   `isqrt(v)`. `abs(INT)` is `ArithmeticPackage`'s and `min(INT, INT)`
    ///   and `max(INT, INT)` are `LogicPackage`'s, so they are not registered
    ///   again: one implementation per name (`the_integer_helpers`).
    #[must_use]
    pub fn new(limits: &EngineLimits, api: impl FnOnce(&mut Engine)) -> Sandbox {
        let mut engine = packaged();

        engine.set_module_resolver(DummyModuleResolver::new());
        for symbol in ["eval", "import", "export", "Fn", "call", "curry"] {
            engine.disable_symbol(symbol);
        }

        // The `sleep` shadows: on the engine's own namespace, searched before
        // the packages (see the doc above).
        engine.register_fn("sleep", |_: INT| -> Result<(), Box<EvalAltResult>> {
            Err(system_error(SLEEP_REFUSED))
        });
        engine.register_fn("sleep", |_: FLOAT| -> Result<(), Box<EvalAltResult>> {
            Err(system_error(SLEEP_REFUSED))
        });

        // D-UM-25 as amended 2026-10-04.
        engine.set_max_call_levels(12);
        engine.set_max_expr_depths(48, 24);
        engine.set_max_variables(64);
        engine.set_max_functions(32);
        engine.set_max_modules(0);
        engine.set_strict_variables(true);
        engine.set_fail_on_invalid_map_property(true);
        engine.set_allow_anonymous_fn(false);
        engine.set_max_map_size(MAX_MAP_SIZE);
        engine.set_max_array_size(MAX_ARRAY_SIZE);
        engine.set_max_string_size(MAX_STRING_BYTES);
        engine.set_max_operations(limits.max_ops_backstop);

        // The trace and the budget (doc 17 §7, §14; D-UM-26).
        engine.on_print(trace::push);
        engine.on_debug(|text, _source, _pos| trace::push(text));
        engine.on_progress(budget::check);

        // The integer helpers the packages do not already carry.
        engine.register_fn("clamp", clamp);
        engine.register_fn("isqrt", isqrt);

        api(&mut engine);

        Sandbox {
            engine,
            asts: Mutex::new(BTreeMap::new()),
            max_ops_backstop: limits.max_ops_backstop,
        }
    }
}

/// **`Engine::new_raw()` with D-UM-24's seven packages, and only these** —
/// the engine before anything of this crate's or the game's is registered on
/// it (the integer-helper test reads which names the packages already carry
/// off this).
fn packaged() -> Engine {
    let mut engine = Engine::new_raw();
    engine.register_global_module(LanguageCorePackage::new().as_shared_module());
    engine.register_global_module(ArithmeticPackage::new().as_shared_module());
    engine.register_global_module(LogicPackage::new().as_shared_module());
    engine.register_global_module(BasicIteratorPackage::new().as_shared_module());
    engine.register_global_module(BasicArrayPackage::new().as_shared_module());
    engine.register_global_module(BasicMapPackage::new().as_shared_module());
    engine.register_global_module(BasicStringPackage::new().as_shared_module());
    engine
}

/// The sentence a `sleep` call ends its think with.
const SLEEP_REFUSED: &str = "sleep is not a thing a mind does";

/// **`clamp(v, lo, hi)`**: `v` held inside `lo..=hi`. A `lo` above `hi` is
/// refused ([`system_error`], a `type` fault) rather than read as some
/// range the script never wrote — refuse, never clamp the clamp.
fn clamp(v: INT, lo: INT, hi: INT) -> Result<INT, Box<EvalAltResult>> {
    if lo > hi {
        return Err(system_error("clamp's low bound is above its high bound"));
    }
    Ok(v.max(lo).min(hi))
}

/// **`isqrt(v)`**: the floor of the square root of a non-negative INT. A
/// negative argument is refused ([`system_error`], a `type` fault).
fn isqrt(v: INT) -> Result<INT, Box<EvalAltResult>> {
    if v < 0 {
        return Err(system_error("isqrt takes a number at or above zero"));
    }
    Ok(v.isqrt())
}

/// **Why a mind's call was refused** (D-UM-31): the error inside every
/// [`system_error`]. It carries the refusal's sentence; the game reads the
/// fault's text, never this type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MindRefusal(pub String);

impl fmt::Display for MindRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("refused")
    }
}

impl std::error::Error for MindRefusal {}

/// **The one constructor of a refusal** (D-UM-31; doc 17 §4, §8): an
/// `EvalAltResult::ErrorSystem` named by `text`, wrapping a [`MindRefusal`].
///
/// A system error is the one error a script cannot swallow: `try { } catch
/// { }` does not catch it, and a script function it passes through hands it
/// on unwrapped (a plain runtime error is caught by `try` and is wrapped in
/// `ErrorInFunctionCall` on its way out of a function). So the game's station
/// functions and kernels raise every refusal through this function, and
/// [`Sandbox::think`] reads it as a `type` fault whose text holds `text`
/// (`a_system_error_is_not_caught_by_try`). Rendered, the fault's text is
/// exactly `<text>: refused` (a system error carries no position).
///
/// **The latch.** One path wraps even a system error: a callback a package
/// method runs (`[1].map(f)` and the rest) hands back every error wrapped in
/// a catchable `ErrorInFunctionCall` (rhai 1.26 `types/fn_ptr.rs:586-603`).
/// So constructing a refusal also latches it, on this thread, for the think
/// running there: the FIRST refusal of a think ends that think a `type` fault
/// with its text whether or not the script caught it on the way out
/// ([`Sandbox::think`] clears the latch at the start and reads it at the end;
/// 17b cut M2.1, review finding F2). Constructing one is raising one: the
/// game builds a refusal only to return it.
#[must_use]
pub fn system_error(text: &str) -> Box<EvalAltResult> {
    let error = Box::new(EvalAltResult::ErrorSystem(
        text.to_owned(),
        Box::new(MindRefusal(text.to_owned())),
    ));
    REFUSAL.with(|latched| {
        let mut latched = latched.borrow_mut();
        if latched.is_none() {
            *latched = Some(trace::line(&error));
        }
    });
    error
}

thread_local! {
    /// The think's first refusal, rendered ([`system_error`]'s latch).
    static REFUSAL: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// **Clear the refusal latch** for a new think.
pub(crate) fn refusal_begin() {
    REFUSAL.with(|latched| *latched.borrow_mut() = None);
}

/// **The think's first refusal**, if one was raised, taken off the latch.
pub(crate) fn refusal_take() -> Option<String> {
    REFUSAL.with(|latched| latched.borrow_mut().take())
}

#[cfg(test)]
mod tests {
    use rhai::{EvalAltResult, INT, OptimizationLevel};

    use super::*;

    /// `the_integer_helpers`' half that needs the engine before this crate
    /// registers anything: `abs`, `min` and `max` over INT are the packages'
    /// own, `clamp` and `isqrt` are not — so the sandbox registers exactly
    /// the two the packages lack, and every helper name has one
    /// implementation.
    #[test]
    fn the_packages_carry_abs_min_max_and_not_clamp_or_isqrt() {
        let engine = packaged();
        assert_eq!(engine.eval::<INT>("abs(-3)").unwrap(), 3);
        assert_eq!(engine.eval::<INT>("min(2, 5)").unwrap(), 2);
        assert_eq!(engine.eval::<INT>("max(2, 5)").unwrap(), 5);
        for missing in ["clamp(15, 0, 10)", "isqrt(17)"] {
            let error = engine.eval::<INT>(missing).unwrap_err();
            assert!(
                matches!(*error, EvalAltResult::ErrorFunctionNotFound(..)),
                "{missing} resolved on the packages alone: {error}"
            );
        }
        // And none of the excluded packages' names resolves: the time
        // package's clock, the math package's transcendentals and parsing,
        // `MoreStringPackage`'s string-plus-anything and string `len` (a mind
        // writes `"k" + i.to_string()`), and `BasicFnPackage`'s function-
        // pointer properties (a bare script-function name is still a
        // function pointer — the callbacks the budget counts — but nothing
        // reads one).
        for absent in [
            "timestamp()",
            "sin(1.0)",
            "to_int(1.5)",
            "parse_int(\"1\")",
            "\"k\" + 1",
            "\"abc\".len()",
            "fn f() { } let p = f; p.name()",
        ] {
            let error = engine.eval::<rhai::Dynamic>(absent).unwrap_err();
            assert!(
                matches!(*error, EvalAltResult::ErrorFunctionNotFound(..)),
                "{absent} resolved on the seven packages: {error}"
            );
        }
    }

    /// D-UM-25 as amended: the data-size backstop at twice the largest valid
    /// class's totals ([`MAX_MEMORY_ANY_CLASS`], Class III).
    #[test]
    fn the_backstop_is_twice_the_largest_class() {
        let class = MAX_MEMORY_ANY_CLASS;
        assert_eq!((class.keys, class.key_chars), (128, 24));
        assert_eq!((class.string_bytes, class.array_len), (32, 32));
        assert!(MAX_MAP_SIZE >= 2 * class.keys);
        assert!(MAX_ARRAY_SIZE >= 2 * class.keys * class.array_len);
        assert!(MAX_STRING_BYTES >= 2 * class.keys * class.string_bytes);
        let sandbox = Sandbox::new(&EngineLimits::default(), |_| {});
        assert_eq!(sandbox.engine.max_map_size(), MAX_MAP_SIZE);
        assert_eq!(sandbox.engine.max_array_size(), MAX_ARRAY_SIZE);
        assert_eq!(sandbox.engine.max_string_size(), MAX_STRING_BYTES);
    }

    /// The rest of D-UM-25 as amended and D-UM-27's optimisation level, read
    /// back off the built engine.
    #[test]
    fn the_engine_carries_the_amended_limits() {
        let sandbox = Sandbox::new(
            &EngineLimits {
                max_ops_backstop: 777,
            },
            |_| {},
        );
        let engine = &sandbox.engine;
        assert_eq!(engine.max_call_levels(), 12);
        assert_eq!(engine.max_expr_depth(), 48);
        assert_eq!(engine.max_function_expr_depth(), 24);
        assert_eq!(engine.max_variables(), 64);
        assert_eq!(engine.max_functions(), 32);
        assert_eq!(engine.max_modules(), 0);
        assert_eq!(engine.max_operations(), 777);
        assert!(engine.strict_variables());
        assert!(engine.fail_on_invalid_map_property());
        assert!(!engine.allow_anonymous_fn());
        assert_eq!(engine.optimization_level(), OptimizationLevel::Simple);
        assert_eq!(EngineLimits::default().max_ops_backstop, 4_500);
        assert_eq!(MAX_OPS_ANY_CLASS, 4_500);
    }

    /// The pool shares one sandbox across workers (17b cut M4).
    #[test]
    fn the_sandbox_is_send_and_sync() {
        fn shared<T: Send + Sync>() {}
        shared::<Sandbox>();
        shared::<crate::Compiled>();
    }
}
