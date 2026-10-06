//! **The run** (17b cut M2.1; doc 17 §7 "The budget", §8 "Memory" and
//! "Faults"; D-UM-26, D-UM-28, D-UM-30, D-UM-31).
//!
//! [`Sandbox::think`] runs one think of a compiled mind: the memory decoded
//! and bound as `this`, `think(args…)` called on an empty scope under
//! `catch_unwind`, the budget re-checked after the call, the memory read back
//! out of `this`, checked and encoded. Every way a think ends is an
//! [`Outcome`] or a [`Fault`] with its cause word, and the trace is answered
//! beside either.
//!
//! This crate knows nothing of intents, views or cards: they are the game's
//! `args` and its registered types (17b cut M2.3), and the intents a think
//! emits leave through the game's own station functions, never through this
//! file. The per-think facts are the game's zero-argument native functions,
//! so a think is handed **no scope but an empty one** and no constant: a
//! function body sees no call scope (the parser refuses a name it cannot
//! see, and a script-to-script call runs on an empty scope —
//! `the_scope_contract_a_function_body_sees_no_call_scope_and_a_native_function_resolves_at_depth`).

use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind};

use rhai::{CallFnOptions, Dynamic, EvalAltResult, Map, Scope};

use crate::cache::Compiled;
use crate::engine::{self, Sandbox};
use crate::memory::{self, Limits, MemError, MemFault};
use crate::{budget, trace};

/// **One think's input** (doc 17 §6, §7).
#[derive(Clone, Debug)]
pub struct Think<'a> {
    /// The compiled mind ([`Sandbox::compile`]).
    pub compiled: &'a Compiled,
    /// `think`'s arguments, in order — the game's view and card (17b cut
    /// M2.3). A copy: a script that writes into an argument writes into its
    /// own copy, never into the caller's value or the memory.
    pub args: Vec<Dynamic>,
    /// The memory's bytes as the last landed think left them; empty at a
    /// sortie's start.
    pub memory: &'a [u8],
    /// The class's memory limits (D-UM-30), checked when the think ends.
    pub limits: &'a Limits,
    /// The think's op limit (D-UM-26): operations plus fees may not pass it.
    pub ops_limit: u64,
    /// The fixed fee every think pays before its first operation (doc 17
    /// §7: 20).
    pub fee_per_think: u64,
}

/// **A clean think's answer**: the memory's new bytes and what the think
/// spent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// The canonical bytes of the memory the script left ([`memory::encode`]).
    pub memory: Vec<u8>,
    /// The operations Rhai counted.
    pub ops: u64,
    /// The fees charged, the per-think fee among them.
    pub fee: u64,
}

/// **Why a think faulted** (D-UM-31): a faulted think lands nothing and
/// leaves the memory as it was; the game journals the cause word
/// ([`Fault::cause`]) and counts a strike.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    /// The op budget: operations plus fees passed the limit (`ops`).
    Ops,
    /// An engine limit: Rhai's own operation backstop, a variable or module
    /// count, the call levels (a stack overflow), or a data size (`limit`).
    Limit,
    /// A runtime error — a type mismatch, a missing function or property, an
    /// arithmetic error, a `throw`, a refusal raised through
    /// [`crate::system_error`], a `this` replaced by something not a map —
    /// with its text; a FLOAT in memory, with the key that holds it (the
    /// brief's `Fault::Type(key)`) (`type`). The text is one trace line
    /// (`trace::line`): at most [`crate::trace::TRACE_WIDTH`]
    /// characters, control characters spaced out, and script-authored — a
    /// renderer escapes it as player text.
    Type(String),
    /// A memory of the wrong shape for its class (`memory`).
    Memory(MemError),
    /// A panic in a native function, caught (`panic`).
    Panic,
}

impl Fault {
    /// **The cause word** journalled with the fault (D-UM-31): `ops`,
    /// `limit`, `type`, `memory` or `panic`. (The game adds `intents` and
    /// `unknown` from its own station functions.)
    #[must_use]
    pub const fn cause(&self) -> &'static str {
        match self {
            Fault::Ops => "ops",
            Fault::Limit => "limit",
            Fault::Type(_) => "type",
            Fault::Memory(_) => "memory",
            Fault::Panic => "panic",
        }
    }
}

/// **How a call that answered an error ended** (D-UM-31), read off the
/// innermost error: a package method's callback hands back every error
/// wrapped in `ErrorInFunctionCall` (rhai 1.26 `types/fn_ptr.rs:586-603`),
/// and a wrapped termination is still the budget's, a wrapped limit still the
/// engine's, a wrapped `exit` still a return (17b cut M2.1, review finding
/// F2).
enum Ended {
    /// An `exit` that reached the top wrapped: the think returned.
    Returned,
    /// A fault.
    Faulted(Fault),
}

/// **A Rhai error as a fault** (D-UM-31), classified by its innermost error
/// ([`Ended`]): the budget's termination is `ops`; the engine's limits are
/// `limit`; an `exit` is a return; everything else — a system error raised by
/// [`crate::system_error`] among them — is `type` with the whole error's text
/// as one bounded trace line ([`trace::line`]: script-authored, a thrown
/// value never formatted past the width).
fn ended(error: &EvalAltResult) -> Ended {
    match error.unwrap_inner() {
        EvalAltResult::Exit(..) => Ended::Returned,
        EvalAltResult::ErrorTerminated(..) => Ended::Faulted(Fault::Ops),
        EvalAltResult::ErrorTooManyOperations(..)
        | EvalAltResult::ErrorTooManyVariables(..)
        | EvalAltResult::ErrorTooManyModules(..)
        | EvalAltResult::ErrorStackOverflow(..)
        | EvalAltResult::ErrorDataTooLarge(..) => Ended::Faulted(Fault::Limit),
        _ => Ended::Faulted(Fault::Type(trace::line(error))),
    }
}

impl Sandbox {
    /// **Run one think** (doc 17 §6–§8): answers the think's result and its
    /// trace ([`crate::trace`]), the trace on every path.
    ///
    /// 1. The budget opens at `ops_limit` and the trace is cleared; **a think
    ///    whose `ops_limit` is past the engine's own backstop
    ///    ([`crate::EngineLimits::max_ops_backstop`] as built) is
    ///    [`Fault::Limit`] here**, nothing charged, nothing run, an empty trace
    ///    (17b cut M3.1: fail closed); otherwise `fee_per_think` is charged.
    /// 2. The memory: empty bytes are the empty map (a fresh sortie); bytes
    ///    that do not decode are the empty map too — the arena's own rule: a
    ///    mind handed them starts over rather than faulting.
    /// 3. The map is bound as `this` and `think(args…)` is called on an empty
    ///    scope, the AST's (cleared) statements not evaluated, under
    ///    `catch_unwind`.
    /// 4. A panic is [`Fault::Panic`]; an error is mapped by its innermost
    ///    kind (a package method's callback wraps every error it hands
    ///    back): `ErrorTerminated` (the budget) is [`Fault::Ops`];
    ///    `ErrorTooManyOperations`, `ErrorTooManyVariables`,
    ///    `ErrorTooManyModules`, `ErrorStackOverflow` and `ErrorDataTooLarge`
    ///    are [`Fault::Limit`]; an `exit` is a return; anything else is
    ///    [`Fault::Type`] with its text. (Rhai 1.26 counts the function limit
    ///    at parse time: a mind past it never compiles.)
    /// 5. A refusal raised in the think ([`crate::system_error`]'s latch) is
    ///    [`Fault::Type`] with the refusal's text, even when the script caught
    ///    it through a callback's wrapping, ahead of whatever came after it
    ///    but a panic.
    /// 6. On a return, the budget is checked once more — a fee charged by
    ///    the last kernel call has no next operation to trip on, and this is
    ///    that operation ([`Fault::Ops`]); then `this` is read back (replaced
    ///    by something not a map: [`Fault::Type`]), converted and checked
    ///    ([`memory::from_map_held`]: a FLOAT is [`Fault::Type`] holding its key,
    ///    a wrong shape [`Fault::Memory`] — and, since 17b cut M3.1, a memory
    ///    past `limits.keys` passes only when its keys are a subset of the keys
    ///    the think was handed, [`memory::check_held`]) and encoded.
    ///
    /// **`exit()` is a return.** `LanguageCorePackage`'s `exit()` and
    /// `exit(value)` end the script at once, and `call_fn` reads an exit as
    /// the function's return: the think ends early, clean, with the memory
    /// it wrote before the call (`exit_is_a_return`). No fault. An `exit`
    /// inside a package method's callback comes back wrapped, and reaching
    /// the top so is a return too; a script that wraps that callback in
    /// `try` catches it and runs on, as it chose.
    ///
    /// A fault leaves nothing behind but the trace: the caller keeps the old
    /// memory (D-UM-31).
    pub fn think(&self, think: Think<'_>) -> (Result<Outcome, Fault>, Vec<String>) {
        budget::begin(think.ops_limit);
        trace::begin();
        engine::refusal_begin();
        // 17b cut M3.1: fail closed — a think asking for more than this
        // engine was built for is refused before it runs, with nothing spent
        // and an empty trace.
        if think.ops_limit > self.max_ops_backstop {
            return (Err(Fault::Limit), trace::take());
        }
        budget::charge(think.fee_per_think);
        let result = self.run(think);
        (result, trace::take())
    }

    /// Steps 2–6 of [`Sandbox::think`].
    fn run(&self, think: Think<'_>) -> Result<Outcome, Fault> {
        let decoded = if think.memory.is_empty() {
            memory::Memory::new()
        } else {
            memory::decode(think.memory).unwrap_or_default()
        };
        // 17b cut M3.1 (D-UM-30): the keys the think was handed, before the
        // script runs — a memory past a lowered cap may keep them and admit
        // no other (`memory::check_held`).
        let held: BTreeSet<String> = decoded.keys().cloned().collect();
        let map = memory::to_map(&decoded);
        let mut this = Dynamic::from_map(map);
        let ast = &think.compiled.ast;
        let args = think.args;
        let called = catch_unwind(AssertUnwindSafe(|| {
            self.engine.call_fn_with_options::<Dynamic>(
                CallFnOptions::new()
                    .bind_this_ptr(&mut this)
                    .eval_ast(false),
                &mut Scope::new(),
                ast,
                "think",
                args,
            )
        }));
        let returned = match called {
            Err(_panic) => return Err(Fault::Panic),
            Ok(Ok(_returned)) => Ended::Returned,
            Ok(Err(error)) => ended(&error),
        };
        // The think's first refusal ends it, caught or not (the latch,
        // `system_error`): it came before whatever the script did after.
        if let Some(refusal) = engine::refusal_take() {
            return Err(Fault::Type(refusal));
        }
        if let Ended::Faulted(fault) = returned {
            return Err(fault);
        }
        let (ops, fee) = budget::spent();
        if budget::tripped() {
            return Err(Fault::Ops);
        }
        let map = this
            .try_cast::<Map>()
            .ok_or_else(|| Fault::Type("this is no longer a map".to_owned()))?;
        let memory =
            memory::from_map_held(&map, think.limits, &held).map_err(|fault| match fault {
                MemFault::Float(key) => Fault::Type(trace::line(&key)),
                MemFault::Shape(error) => Fault::Memory(error),
            })?;
        Ok(Outcome {
            memory: memory::encode(&memory),
            ops,
            fee,
        })
    }
}
