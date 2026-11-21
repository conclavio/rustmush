//! **Container, padding, and comparator guards** (CRIT-6, CRIT-7, HIGH-16,
//! P2-CORE-1, P2-CORE-2, P2-CORE-3).
//!
//! Shadows `rhai 1.26.0` package methods whose unchecked arithmetic, unbounded
//! native recursion, intermediate heap spikes, missing map-key byte accounting,
//! or swallowed callback errors could otherwise abort the process or bypass
//! the op/memory budgets:
//!
//! * **`Array::pad`** (CRIT-6): rejects `len > max_array_size` up front, treats
//!   `len <= 0` as a no-op, enforces container depth, and uses checked
//!   arithmetic before resizing.
//! * **Container nesting depth & pre-mutation size caps** (CRIT-7, P2-CORE-2,
//!   P2-CORE-3): enforces [`MAX_CONTAINER_DEPTH`] and key-aware cumulative
//!   `(array, map, string)` size limits across all `BasicArrayPackage` and
//!   `BasicMapPackage` mutators, builders, container-returning methods
//!   (`push`, `insert`, `set`, `append`, `+=`, `+`, `splice`, `extract`,
//!   `split`, `drain`, `retain`, `filter`, `map`, `zip`, `for_each`, `reduce`,
//!   `reduce_rev`, `find`, `find_map`, `some`, `all`, `index_of`, `order`,
//!   `order_desc`, `mixin`, `fill_with`, `keys`, `values`), equality (`==`,
//!   `!=`), formatting (`print`, `debug`, `to_string`, `to_debug`, `to_json`),
//!   and variable reads (`on_var`, checking both local variables and `this`).
//! * **Uncatchable limit & system-exception latching** (P2-CORE-3): latches
//!   `ErrorDataTooLarge` and unwraps system exceptions out of callback
//!   `ErrorInFunctionCall` wrappers so `try`/`catch` cannot swallow limit or
//!   budget violations.
//! * **Compile-time source & AST validation** (P2-CORE-1, P2-CORE-2): validates
//!   both tokenized source (matching Rhai's lexer, rejecting unclosed strings,
//!   templates, interpolations, comments, and delimiters) and the compiled
//!   `rhai::AST` directly (`validate_ast`), eliminating lexer/parser desync
//!   and closing `switch`-arm, nested `this`, and `Array +=` bypasses.
//! * **Comparator `sort`/`order`/`dedup`** (HIGH-16): replaces `slice::sort_by`
//!   and `Vec::dedup_by` (which swallow callback `ErrorTerminated` via `.ok()`
//!   / `.unwrap_or`) with fallible iterative implementations that abort
//!   immediately on the first comparator error or budget trip.

use std::cell::Cell;
use std::cmp::Ordering;
use std::fmt::Write as _;
use std::ops::{Range, RangeInclusive};

use rhai::{
    AST, ASTFlags, ASTNode, Array, Dynamic, Engine, EvalAltResult, Expr, FnPtr, FuncRegistration,
    INT, ImmutableString, Map, NativeCallContext, Position, Scope, Stmt, Token as RhaiToken,
};

type ExclusiveRange = Range<INT>;
type InclusiveRange = RangeInclusive<INT>;

use crate::budget;

/// Maximum permitted nesting depth for `Array` / `Map` containers.
///
/// A non-container scalar has depth `0`; a flat `Array` or `Map` holding only
/// scalars has depth `1`; wrapping a depth-`d` container adds `1`.
pub const MAX_CONTAINER_DEPTH: usize = 8;

/// Fallback formatting byte ceiling when `engine.max_string_size()` is `0`.
const FALLBACK_MAX_FORMAT_BYTES: usize = 64 * 1024;

thread_local! {
    /// Latched when any depth- or size-limit guard trips (including inside
    /// `print_with_func` / string interpolation or callback paths that might
    /// otherwise wrap or swallow `ErrorDataTooLarge`).
    static LIMIT_TRIPPED: Cell<bool> = const { Cell::new(false) };
}

/// Clear the limit latch at the start of a think.
pub(crate) fn limit_begin() {
    LIMIT_TRIPPED.with(|flag| flag.set(false));
}

/// Whether the limit latch has tripped during the current think.
#[must_use]
pub(crate) fn limit_tripped() -> bool {
    LIMIT_TRIPPED.with(Cell::get)
}

/// Take and clear the limit latch at the end of a think.
#[must_use]
pub(crate) fn limit_take() -> bool {
    LIMIT_TRIPPED.with(|flag| flag.replace(false))
}

/// Latch a limit fault so `on_progress` and `Sandbox::run` abort immediately.
fn latch_limit() {
    LIMIT_TRIPPED.with(|flag| flag.set(true));
}

fn depth_exceeded_err() -> Box<EvalAltResult> {
    latch_limit();
    Box::new(EvalAltResult::ErrorDataTooLarge(
        "Container nesting depth".to_owned(),
        Position::NONE,
    ))
}

fn size_exceeded_err(what: &str) -> Box<EvalAltResult> {
    latch_limit();
    Box::new(EvalAltResult::ErrorDataTooLarge(
        what.to_owned(),
        Position::NONE,
    ))
}

fn ops_terminated_err() -> Box<EvalAltResult> {
    Box::new(EvalAltResult::ErrorTerminated(
        Dynamic::from("ops"),
        Position::NONE,
    ))
}

/// Abort immediately if either the limit latch or the op budget has tripped.
fn check_budget() -> Result<(), Box<EvalAltResult>> {
    if limit_tripped() {
        return Err(size_exceeded_err("Data size limit exceeded"));
    }
    if budget::tripped() {
        return Err(ops_terminated_err());
    }
    Ok(())
}

/// Register all container, padding, equality, formatting, variable-read, and
/// comparator shadows in the engine's global namespace (searched ahead of all
/// standard packages).
pub(crate) fn register(engine: &mut Engine) {
    // CRIT-6 + CRIT-7: safe `Array::pad` with checked arithmetic & depth cap.
    register_mut(engine, "pad", array_pad);

    // CRIT-7 + P2-CORE-2 + P2-CORE-3: container builders and mutators with
    // pre-mutation depth and key-aware size caps.
    register_mut(engine, "push", array_push);
    register_mut(engine, "insert", array_insert);
    register_mut(engine, "set", array_set);
    register_mut(engine, "append", array_append);
    register_mut(engine, "+=", array_append);
    register_mut(engine, "+=", array_push);
    register_pure(engine, "+", array_concat);
    register_mut(engine, "splice", array_splice);
    register_mut(engine, "splice", array_splice_range);
    register_mut(engine, "splice", array_splice_inclusive_range);

    // P2-CORE-2 + P2-CORE-3: container-returning and callback `BasicArrayPackage`
    // methods shadowed with depth/size checks and uncatchable system-exception
    // propagation.
    register_pure(engine, "extract", array_extract);
    register_pure(engine, "extract", array_extract_tail);
    register_pure(engine, "extract", array_extract_range);
    register_pure(engine, "extract", array_extract_inclusive_range);
    register_mut(engine, "split", array_split_at);
    register_mut(engine, "for_each", array_for_each);
    register_pure(engine, "map", array_map);
    register_pure(engine, "filter", array_filter);
    register_pure(engine, "zip", array_zip);
    register_pure(engine, "reduce", array_reduce);
    register_pure(engine, "reduce", array_reduce_with_initial);
    register_pure(engine, "reduce_rev", array_reduce_rev);
    register_pure(engine, "reduce_rev", array_reduce_rev_with_initial);
    register_mut(engine, "drain", array_drain_filter);
    register_mut(engine, "drain", array_drain_range);
    register_mut(engine, "drain", array_drain_exclusive_range);
    register_mut(engine, "drain", array_drain_inclusive_range);
    register_mut(engine, "retain", array_retain_filter);
    register_mut(engine, "retain", array_retain_range);
    register_mut(engine, "retain", array_retain_exclusive_range);
    register_mut(engine, "retain", array_retain_inclusive_range);
    register_pure(engine, "find", array_find);
    register_pure(engine, "find", array_find_starting_from);
    register_pure(engine, "find_map", array_find_map);
    register_pure(engine, "find_map", array_find_map_starting_from);
    register_pure(engine, "some", array_some);
    register_pure(engine, "all", array_all);
    register_pure(engine, "index_of", array_index_of_filter);
    register_pure(engine, "index_of", array_index_of_filter_starting_from);
    register_pure(engine, "order", array_order_builtin);
    register_pure(engine, "order_desc", array_order_builtin_desc);

    // CRIT-7 + P2-CORE-2 + P2-CORE-3: `BasicMapPackage` mutators, builders,
    // container-returning methods, and callbacks.
    register_mut(engine, "set", map_set);
    register_mut(engine, "mixin", map_mixin);
    register_mut(engine, "+=", map_mixin);
    register_pure(engine, "+", map_merge);
    register_mut(engine, "fill_with", map_fill_with);
    register_pure(engine, "keys", map_keys);
    register_pure(engine, "values", map_values);
    register_pure(engine, "map", map_map);
    register_pure(engine, "filter", map_filter);
    register_mut(engine, "drain", map_drain);
    register_mut(engine, "retain", map_retain);

    // CRIT-7: depth-bounded equality without recursive `exec_native_fn_call`.
    register_pure(engine, "==", array_eq);
    register_pure(engine, "!=", array_ne);
    register_pure(engine, "==", map_eq);
    register_pure(engine, "!=", map_ne);

    // CRIT-7: depth- and size-bounded container formatting (`print`, `debug`,
    // `to_string`, `to_debug`, `to_json`).
    register_pure(engine, "print", print_array);
    register_pure(engine, "debug", print_array);
    register_pure(engine, "to_string", to_string_array);
    register_pure(engine, "to_debug", to_string_array);

    register_pure(engine, "print", print_map);
    register_pure(engine, "debug", print_map);
    register_pure(engine, "to_string", to_string_map);
    register_pure(engine, "to_debug", to_string_map);

    register_pure(engine, "to_json", map_to_json);

    // CRIT-7 + P2-CORE-2 + P2-CORE-3: guard literal/index-assignment nesting
    // and map-key byte limits on every variable access (for both the variable
    // itself and `this`).
    #[allow(deprecated)]
    engine.on_var(|name, _index, ctx| {
        check_budget()?;
        if let Some(val) = ctx.scope().get(name)
            && (val.is_array() || val.is_map())
        {
            check_value_limits(ctx.engine(), val)?;
        }
        if let Some(this_val) = ctx.this_ptr()
            && (this_val.is_array() || this_val.is_map())
        {
            check_value_limits(ctx.engine(), this_val)?;
        }
        Ok(None)
    });

    // HIGH-16: fallible iterative sort / order / dedup that abort immediately
    // on comparator errors or budget overrun.
    register_mut(engine, "sort", array_sort_by);
    register_mut(engine, "sort_by", array_sort_by);
    register_mut(engine, "sort", array_sort_by_name);
    register_pure(engine, "order", array_order_by);
    register_pure(engine, "order_by", array_order_by);
    register_mut(engine, "dedup", array_dedup_default);
    register_mut(engine, "dedup", array_dedup_by);
    register_mut(engine, "dedup", array_dedup_by_name);
}

fn register_pure<A: 'static, const N: usize, const X: bool, R, const F: bool, FUNC>(
    engine: &mut Engine,
    name: &str,
    func: FUNC,
) where
    R: Clone + Send + Sync + 'static,
    FUNC: rhai::RhaiNativeFunc<A, N, X, R, F> + Send + Sync + 'static,
{
    FuncRegistration::new(name)
        .in_global_namespace()
        .with_purity(true)
        .register_into_engine(engine, func);
}

fn register_mut<A: 'static, const N: usize, const X: bool, R, const F: bool, FUNC>(
    engine: &mut Engine,
    name: &str,
    func: FUNC,
) where
    R: Clone + Send + Sync + 'static,
    FUNC: rhai::RhaiNativeFunc<A, N, X, R, F> + Send + Sync + 'static,
{
    FuncRegistration::new(name)
        .in_global_namespace()
        .with_purity(false)
        .register_into_engine(engine, func);
}

// ---------------------------------------------------------------------------
// Depth and checked-size accounting (bounded strictly by MAX_CONTAINER_DEPTH)
// ---------------------------------------------------------------------------

/// Side-effect-free depth predicate (does not latch `LIMIT_TRIPPED`).
///
/// Used for compile-time AST validation and safe error formatting so pure
/// inspection never mutates thread-local execution state.
pub(crate) fn exceeds_depth(value: &Dynamic, max_depth: usize) -> bool {
    if let Some(arr) = value.read_lock::<Array>() {
        if max_depth == 0 {
            return true;
        }
        return arr.iter().any(|item| exceeds_depth(item, max_depth - 1));
    }
    if let Some(map) = value.read_lock::<Map>() {
        if max_depth == 0 {
            return true;
        }
        return map.values().any(|val| exceeds_depth(val, max_depth - 1));
    }
    false
}

/// Whether `error` carries any `Dynamic` payload nested deeper than
/// [`MAX_CONTAINER_DEPTH`].
#[must_use]
pub(crate) fn eval_error_has_deep_payload(mut error: &EvalAltResult) -> bool {
    loop {
        match error {
            EvalAltResult::ErrorInFunctionCall(_, _, inner, _)
            | EvalAltResult::ErrorInModule(_, inner, _) => {
                error = inner;
            }
            EvalAltResult::ErrorRuntime(val, _)
            | EvalAltResult::ErrorIndexNotFound(val, _)
            | EvalAltResult::ErrorTerminated(val, _)
            | EvalAltResult::LoopBreak(_, val, _)
            | EvalAltResult::Return(val, _)
            | EvalAltResult::Exit(val, _) => {
                return exceeds_depth(val, MAX_CONTAINER_DEPTH);
            }
            _ => return false,
        }
    }
}

/// Verify `value` has container nesting depth at most `max_depth`.
///
/// Bounded to at most `MAX_CONTAINER_DEPTH` (8) frames with zero cloning, so
/// deeply nested inputs can never overflow the call stack.
pub(crate) fn check_depth(value: &Dynamic, max_depth: usize) -> Result<(), Box<EvalAltResult>> {
    if let Some(arr) = value.read_lock::<Array>() {
        return check_array_depth(&arr, max_depth);
    }
    if let Some(map) = value.read_lock::<Map>() {
        return check_map_depth(&map, max_depth);
    }
    Ok(())
}

fn check_array_depth(array: &Array, max_depth: usize) -> Result<(), Box<EvalAltResult>> {
    if max_depth == 0 {
        return Err(depth_exceeded_err());
    }
    for item in array {
        check_depth(item, max_depth - 1)?;
    }
    Ok(())
}

fn check_map_depth(map: &Map, max_depth: usize) -> Result<(), Box<EvalAltResult>> {
    if max_depth == 0 {
        return Err(depth_exceeded_err());
    }
    for val in map.values() {
        check_depth(val, max_depth - 1)?;
    }
    Ok(())
}

/// Verify both container nesting depth and key-aware cumulative sizes in one
/// pass.
fn check_value_limits(engine: &Engine, value: &Dynamic) -> Result<(), Box<EvalAltResult>> {
    let sizes = calc_sizes_bounded(value, MAX_CONTAINER_DEPTH)?;
    check_sizes(engine, sizes)
}

/// Depth-bounded, overflow-checked `(array_elems, map_entries, string_bytes)`,
/// accounting for map key bytes as well as values.
fn calc_sizes_bounded(
    value: &Dynamic,
    max_depth: usize,
) -> Result<(usize, usize, usize), Box<EvalAltResult>> {
    if let Some(arr) = value.read_lock::<Array>() {
        return calc_array_sizes_bounded(&arr, max_depth);
    }
    if let Some(map) = value.read_lock::<Map>() {
        return calc_map_sizes_bounded(&map, max_depth);
    }
    if let Some(s) = value.read_lock::<ImmutableString>() {
        return Ok((0, 0, s.len()));
    }
    if let Some(blob) = value.read_lock::<rhai::Blob>() {
        return Ok((blob.len(), 0, 0));
    }
    Ok((0, 0, 0))
}

fn calc_array_sizes_bounded(
    array: &Array,
    max_depth: usize,
) -> Result<(usize, usize, usize), Box<EvalAltResult>> {
    if max_depth == 0 {
        return Err(depth_exceeded_err());
    }
    let mut ax: usize = 0;
    let mut mx: usize = 0;
    let mut sx: usize = 0;
    for item in array {
        ax = ax
            .checked_add(1)
            .ok_or_else(|| size_exceeded_err("Size of array/BLOB"))?;
        let (a, m, s) = calc_sizes_bounded(item, max_depth - 1)?;
        ax = ax
            .checked_add(a)
            .ok_or_else(|| size_exceeded_err("Size of array/BLOB"))?;
        mx = mx
            .checked_add(m)
            .ok_or_else(|| size_exceeded_err("Size of object map"))?;
        sx = sx
            .checked_add(s)
            .ok_or_else(|| size_exceeded_err("Length of string"))?;
    }
    Ok((ax, mx, sx))
}

fn calc_map_sizes_bounded(
    map: &Map,
    max_depth: usize,
) -> Result<(usize, usize, usize), Box<EvalAltResult>> {
    if max_depth == 0 {
        return Err(depth_exceeded_err());
    }
    let mut ax: usize = 0;
    let mut mx: usize = 0;
    let mut sx: usize = 0;
    for (key, val) in map {
        mx = mx
            .checked_add(1)
            .ok_or_else(|| size_exceeded_err("Size of object map"))?;
        sx = sx
            .checked_add(key.len())
            .ok_or_else(|| size_exceeded_err("Length of string"))?;
        let (a, m, s) = calc_sizes_bounded(val, max_depth - 1)?;
        ax = ax
            .checked_add(a)
            .ok_or_else(|| size_exceeded_err("Size of array/BLOB"))?;
        mx = mx
            .checked_add(m)
            .ok_or_else(|| size_exceeded_err("Size of object map"))?;
        sx = sx
            .checked_add(s)
            .ok_or_else(|| size_exceeded_err("Length of string"))?;
    }
    Ok((ax, mx, sx))
}

fn add_sizes(
    lhs: (usize, usize, usize),
    rhs: (usize, usize, usize),
) -> Result<(usize, usize, usize), Box<EvalAltResult>> {
    let a = lhs
        .0
        .checked_add(rhs.0)
        .ok_or_else(|| size_exceeded_err("Size of array/BLOB"))?;
    let m = lhs
        .1
        .checked_add(rhs.1)
        .ok_or_else(|| size_exceeded_err("Size of object map"))?;
    let s = lhs
        .2
        .checked_add(rhs.2)
        .ok_or_else(|| size_exceeded_err("Length of string"))?;
    Ok((a, m, s))
}

fn sub_sizes(lhs: (usize, usize, usize), rhs: (usize, usize, usize)) -> (usize, usize, usize) {
    (
        lhs.0.saturating_sub(rhs.0),
        lhs.1.saturating_sub(rhs.1),
        lhs.2.saturating_sub(rhs.2),
    )
}

fn check_sizes(
    engine: &Engine,
    (arr, map, str_len): (usize, usize, usize),
) -> Result<(), Box<EvalAltResult>> {
    let max_str = engine.max_string_size();
    if max_str > 0 && str_len > max_str {
        return Err(size_exceeded_err("Length of string"));
    }
    let max_arr = engine.max_array_size();
    if max_arr > 0 && arr > max_arr {
        return Err(size_exceeded_err("Size of array/BLOB"));
    }
    let max_map = engine.max_map_size();
    if max_map > 0 && map > max_map {
        return Err(size_exceeded_err("Size of object map"));
    }
    Ok(())
}

/// Panic-free equivalent of Rhai's `calc_offset_len`.
fn calc_offset_len(length: usize, start: INT, len: INT) -> (usize, usize) {
    let start = if start < 0 {
        usize::try_from(start.unsigned_abs()).map_or(0, |x| length.saturating_sub(x))
    } else {
        match usize::try_from(start) {
            Ok(x) if x < length => x,
            _ => return (length, 0),
        }
    };
    let rem = length.saturating_sub(start);
    let len = if len <= 0 {
        0
    } else {
        match usize::try_from(len) {
            Ok(x) if x <= rem => x,
            _ => rem,
        }
    };
    (start, len)
}

/// Unwrap `ErrorInFunctionCall`/`ErrorInModule` layers around a system
/// exception so `try`/`catch` cannot catch `ErrorDataTooLarge`,
/// `ErrorTerminated`, `ErrorSystem`, `ErrorStackOverflow`, etc.
fn unwrap_system_exception(mut err: Box<EvalAltResult>) -> Box<EvalAltResult> {
    loop {
        match *err {
            EvalAltResult::ErrorInFunctionCall(_, _, inner, _)
            | EvalAltResult::ErrorInModule(_, inner, _) => {
                err = inner;
            }
            _ => return err,
        }
    }
}

/// Normalize callback errors: latch `ErrorDataTooLarge` and unwrap any system
/// exception so `try`/`catch` cannot swallow sandbox limits or refusals.
fn handle_callback_err(err: Box<EvalAltResult>) -> Box<EvalAltResult> {
    let inner = err.unwrap_inner();
    if matches!(inner, EvalAltResult::ErrorDataTooLarge(..)) {
        latch_limit();
        return unwrap_system_exception(err);
    }
    if inner.is_system_exception() {
        return unwrap_system_exception(err);
    }
    err
}

/// Invoke a `FnPtr` callback with Rhai's exact arity dispatch while checking
/// the budget and preventing `try`/`catch` from swallowing system exceptions.
fn call_callback<const N: usize, const E: usize>(
    fp: &FnPtr,
    caller_fn: &str,
    ctx: &NativeCallContext,
    this_ptr: Option<&mut Dynamic>,
    args: [Dynamic; N],
    extras: [Dynamic; E],
    move_this_ptr_to_args: Option<usize>,
) -> Result<Dynamic, Box<EvalAltResult>> {
    check_budget()?;
    let val = fp
        .call_raw_with_extra_args(
            caller_fn,
            ctx,
            this_ptr,
            args,
            extras,
            move_this_ptr_to_args,
        )
        .map_err(handle_callback_err)?;
    check_budget()?;
    Ok(val)
}

// ---------------------------------------------------------------------------
// CRIT-6 + CRIT-7 + P2-CORE-2 + P2-CORE-3: Array and Map mutators/builders
// ---------------------------------------------------------------------------

fn array_pad(
    ctx: NativeCallContext,
    array: &mut Array,
    len: INT,
    item: Dynamic,
) -> Result<(), Box<EvalAltResult>> {
    check_budget()?;
    if len <= 0 {
        return Ok(());
    }
    let Ok(target_len) = usize::try_from(len) else {
        return Err(size_exceeded_err("Size of array/BLOB"));
    };
    if target_len <= array.len() {
        return Ok(());
    }
    let max_arr = ctx.engine().max_array_size();
    if max_arr > 0 && target_len > max_arr {
        return Err(size_exceeded_err("Size of array/BLOB"));
    }

    let (a, m, s) = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    let (ax, mx, sx) = calc_sizes_bounded(&item, MAX_CONTAINER_DEPTH - 1)?;
    let pad = target_len - array.len();

    let total_a = ax
        .checked_mul(pad)
        .and_then(|v| v.checked_add(pad))
        .and_then(|v| v.checked_add(a))
        .ok_or_else(|| size_exceeded_err("Size of array/BLOB"))?;
    let total_m = mx
        .checked_mul(pad)
        .and_then(|v| v.checked_add(m))
        .ok_or_else(|| size_exceeded_err("Size of object map"))?;
    let total_s = sx
        .checked_mul(pad)
        .and_then(|v| v.checked_add(s))
        .ok_or_else(|| size_exceeded_err("Length of string"))?;

    check_sizes(ctx.engine(), (total_a, total_m, total_s))?;
    array.resize(target_len, item);
    Ok(())
}

fn array_push(
    ctx: NativeCallContext,
    array: &mut Array,
    item: Dynamic,
) -> Result<(), Box<EvalAltResult>> {
    check_budget()?;
    let s1 = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    let s2 = calc_sizes_bounded(&item, MAX_CONTAINER_DEPTH - 1)?;
    let total = add_sizes(add_sizes(s1, s2)?, (1, 0, 0))?;
    check_sizes(ctx.engine(), total)?;
    array.push(item);
    Ok(())
}

fn array_insert(
    ctx: NativeCallContext,
    array: &mut Array,
    index: INT,
    item: Dynamic,
) -> Result<(), Box<EvalAltResult>> {
    check_budget()?;
    let s1 = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    let s2 = calc_sizes_bounded(&item, MAX_CONTAINER_DEPTH - 1)?;
    let total = add_sizes(add_sizes(s1, s2)?, (1, 0, 0))?;
    check_sizes(ctx.engine(), total)?;
    if array.is_empty() {
        array.push(item);
        return Ok(());
    }
    let (idx, _) = calc_offset_len(array.len(), index, 0);
    if idx >= array.len() {
        array.push(item);
    } else {
        array.insert(idx, item);
    }
    Ok(())
}

fn array_set(
    ctx: NativeCallContext,
    array: &mut Array,
    index: INT,
    value: Dynamic,
) -> Result<(), Box<EvalAltResult>> {
    check_budget()?;
    let s1 = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    let s2 = calc_sizes_bounded(&value, MAX_CONTAINER_DEPTH - 1)?;
    if array.is_empty() {
        check_sizes(ctx.engine(), s1)?;
        return Ok(());
    }
    let (idx, _) = calc_offset_len(array.len(), index, 0);
    let Some(existing) = array.get(idx) else {
        return Ok(());
    };
    let old = calc_sizes_bounded(existing, MAX_CONTAINER_DEPTH - 1)?;
    let total = add_sizes(sub_sizes(s1, old), s2)?;
    check_sizes(ctx.engine(), total)?;
    if let Some(slot) = array.get_mut(idx) {
        *slot = value;
    }
    Ok(())
}

fn array_append(
    ctx: NativeCallContext,
    array: &mut Array,
    new_array: Array,
) -> Result<(), Box<EvalAltResult>> {
    check_budget()?;
    check_array_depth(array, MAX_CONTAINER_DEPTH)?;
    if new_array.is_empty() {
        return Ok(());
    }
    let s1 = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    let s2 = calc_array_sizes_bounded(&new_array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), add_sizes(s1, s2)?)?;
    if array.is_empty() {
        *array = new_array;
    } else {
        array.extend(new_array);
    }
    Ok(())
}

fn array_concat(
    ctx: NativeCallContext,
    mut array1: Array,
    array2: Array,
) -> Result<Array, Box<EvalAltResult>> {
    check_budget()?;
    let s1 = calc_array_sizes_bounded(&array1, MAX_CONTAINER_DEPTH)?;
    let s2 = calc_array_sizes_bounded(&array2, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), add_sizes(s1, s2)?)?;
    if array2.is_empty() {
        return Ok(array1);
    }
    if array1.is_empty() {
        return Ok(array2);
    }
    array1.extend(array2);
    Ok(array1)
}

fn array_splice(
    ctx: NativeCallContext,
    array: &mut Array,
    start: INT,
    len: INT,
    replace: Array,
) -> Result<(), Box<EvalAltResult>> {
    check_budget()?;
    let s1 = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    let s2 = calc_array_sizes_bounded(&replace, MAX_CONTAINER_DEPTH)?;
    if array.is_empty() {
        check_sizes(ctx.engine(), s2)?;
        *array = replace;
        return Ok(());
    }
    let (start_idx, remove_len) = calc_offset_len(array.len(), start, len);
    let mut removed = (0usize, 0usize, 0usize);
    if let Some(slice) = array.get(start_idx..start_idx.saturating_add(remove_len)) {
        for item in slice {
            let item_sz = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
            removed = add_sizes(add_sizes(removed, item_sz)?, (1, 0, 0))?;
        }
    }
    let total = add_sizes(sub_sizes(s1, removed), s2)?;
    check_sizes(ctx.engine(), total)?;
    if start_idx >= array.len() {
        array.extend(replace);
    } else {
        array.splice(start_idx..start_idx.saturating_add(remove_len), replace);
    }
    Ok(())
}

fn array_splice_range(
    ctx: NativeCallContext,
    array: &mut Array,
    range: ExclusiveRange,
    replace: Array,
) -> Result<(), Box<EvalAltResult>> {
    let start = range.start.max(0);
    let end = range.end.max(start);
    array_splice(ctx, array, start, end.saturating_sub(start), replace)
}

fn array_splice_inclusive_range(
    ctx: NativeCallContext,
    array: &mut Array,
    range: InclusiveRange,
    replace: Array,
) -> Result<(), Box<EvalAltResult>> {
    let start = (*range.start()).max(0);
    let end = (*range.end()).max(start).min(INT::MAX - 1);
    array_splice(
        ctx,
        array,
        start,
        end.saturating_sub(start).saturating_add(1),
        replace,
    )
}

fn array_extract(
    ctx: NativeCallContext,
    array: &mut Array,
    start: INT,
    len: INT,
) -> Result<Array, Box<EvalAltResult>> {
    check_budget()?;
    let sizes = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), sizes)?;
    if array.is_empty() || len <= 0 {
        return Ok(Array::new());
    }
    let (start_idx, slice_len) = calc_offset_len(array.len(), start, len);
    if slice_len == 0 {
        return Ok(Array::new());
    }
    Ok(array
        .get(start_idx..start_idx.saturating_add(slice_len))
        .map_or_else(Array::new, <[Dynamic]>::to_vec))
}

fn array_extract_tail(
    ctx: NativeCallContext,
    array: &mut Array,
    start: INT,
) -> Result<Array, Box<EvalAltResult>> {
    array_extract(ctx, array, start, INT::MAX)
}

fn array_extract_range(
    ctx: NativeCallContext,
    array: &mut Array,
    range: ExclusiveRange,
) -> Result<Array, Box<EvalAltResult>> {
    let start = range.start.max(0);
    let end = range.end.max(start);
    array_extract(ctx, array, start, end.saturating_sub(start))
}

fn array_extract_inclusive_range(
    ctx: NativeCallContext,
    array: &mut Array,
    range: InclusiveRange,
) -> Result<Array, Box<EvalAltResult>> {
    let start = (*range.start()).max(0);
    let end = (*range.end()).max(start).min(INT::MAX - 1);
    array_extract(
        ctx,
        array,
        start,
        end.saturating_sub(start).saturating_add(1),
    )
}

fn array_split_at(
    ctx: NativeCallContext,
    array: &mut Array,
    index: INT,
) -> Result<Array, Box<EvalAltResult>> {
    check_budget()?;
    let sizes = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), sizes)?;
    if array.is_empty() {
        return Ok(Array::new());
    }
    let (start, len) = calc_offset_len(array.len(), index, INT::MAX);
    if start >= array.len() {
        return Ok(Array::new());
    }
    if start == 0 {
        if len >= array.len() {
            return Ok(std::mem::take(array));
        }
        let cut = array.len().saturating_sub(len);
        return Ok(array.drain(cut..).collect());
    }
    Ok(array.drain(start..).collect())
}

fn array_for_each(
    ctx: NativeCallContext,
    array: &mut Array,
    cb: FnPtr,
) -> Result<(), Box<EvalAltResult>> {
    check_budget()?;
    let mut total = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), total)?;
    if array.is_empty() {
        return Ok(());
    }
    for (i, item) in array.iter_mut().enumerate() {
        let before = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        let ex = [INT::try_from(i).unwrap_or(INT::MAX).into()];
        let _ = call_callback(&cb, "for_each", &ctx, Some(item), [], ex, None)?;
        let after = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        total = add_sizes(sub_sizes(total, before), after)?;
        check_sizes(ctx.engine(), total)?;
    }
    Ok(())
}

fn array_map(
    ctx: NativeCallContext,
    array: &mut Array,
    mapper: FnPtr,
) -> Result<Array, Box<EvalAltResult>> {
    check_budget()?;
    let mut src_total = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), src_total)?;
    if array.is_empty() {
        return Ok(Array::new());
    }
    let mut out = Array::new();
    let mut out_total = (0usize, 0usize, 0usize);
    for (i, item) in array.iter_mut().enumerate() {
        let before = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        let ex = [INT::try_from(i).unwrap_or(INT::MAX).into()];
        let mapped = call_callback(&mapper, "map", &ctx, Some(item), [], ex, Some(0))?;
        let after = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        src_total = add_sizes(sub_sizes(src_total, before), after)?;
        check_sizes(ctx.engine(), src_total)?;

        let item_sz = calc_sizes_bounded(&mapped, MAX_CONTAINER_DEPTH - 1)?;
        out_total = add_sizes(add_sizes(out_total, item_sz)?, (1, 0, 0))?;
        check_sizes(ctx.engine(), out_total)?;
        out.push(mapped);
    }
    Ok(out)
}

fn array_filter(
    ctx: NativeCallContext,
    array: &mut Array,
    filter: FnPtr,
) -> Result<Array, Box<EvalAltResult>> {
    check_budget()?;
    let mut src_total = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), src_total)?;
    if array.is_empty() {
        return Ok(Array::new());
    }
    let mut out = Array::new();
    let mut out_total = (0usize, 0usize, 0usize);
    for (i, item) in array.iter_mut().enumerate() {
        let before = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        let ex = [INT::try_from(i).unwrap_or(INT::MAX).into()];
        let keep = call_callback(&filter, "filter", &ctx, Some(item), [], ex, Some(0))?
            .as_bool()
            .unwrap_or(false);
        let after = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        src_total = add_sizes(sub_sizes(src_total, before), after)?;
        check_sizes(ctx.engine(), src_total)?;
        if keep {
            out_total = add_sizes(add_sizes(out_total, after)?, (1, 0, 0))?;
            check_sizes(ctx.engine(), out_total)?;
            out.push(item.clone());
        }
    }
    Ok(out)
}

fn array_zip(
    ctx: NativeCallContext,
    array1: &mut Array,
    array2: Array,
    mapper: FnPtr,
) -> Result<Array, Box<EvalAltResult>> {
    check_budget()?;
    let s1 = calc_array_sizes_bounded(array1, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), s1)?;
    let s2 = calc_array_sizes_bounded(&array2, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), s2)?;
    if array1.is_empty() || array2.is_empty() {
        return Ok(Array::new());
    }
    let mut out = Array::new();
    let mut out_total = (0usize, 0usize, 0usize);
    for (i, (x, y)) in array1.iter_mut().zip(array2).enumerate() {
        let ex = [INT::try_from(i).unwrap_or(INT::MAX).into()];
        let mapped = call_callback(&mapper, "zip", &ctx, None, [x.clone(), y], ex, None)?;
        let item_sz = calc_sizes_bounded(&mapped, MAX_CONTAINER_DEPTH - 1)?;
        out_total = add_sizes(add_sizes(out_total, item_sz)?, (1, 0, 0))?;
        check_sizes(ctx.engine(), out_total)?;
        out.push(mapped);
    }
    Ok(out)
}

fn array_reduce(
    ctx: NativeCallContext,
    array: &mut Array,
    reducer: FnPtr,
) -> Result<Dynamic, Box<EvalAltResult>> {
    array_reduce_with_initial(ctx, array, reducer, Dynamic::UNIT)
}

fn array_reduce_with_initial(
    ctx: NativeCallContext,
    array: &mut Array,
    reducer: FnPtr,
    initial: Dynamic,
) -> Result<Dynamic, Box<EvalAltResult>> {
    check_budget()?;
    let mut src_total = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), src_total)?;
    check_value_limits(ctx.engine(), &initial)?;
    if array.is_empty() {
        return Ok(initial);
    }
    let mut acc = initial;
    for (i, item) in array.iter_mut().enumerate() {
        let before = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        let ex = [INT::try_from(i).unwrap_or(INT::MAX).into()];
        acc = call_callback(&reducer, "reduce", &ctx, Some(item), [acc], ex, Some(1))?;
        let after = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        src_total = add_sizes(sub_sizes(src_total, before), after)?;
        check_sizes(ctx.engine(), src_total)?;
        check_value_limits(ctx.engine(), &acc)?;
    }
    Ok(acc)
}

fn array_reduce_rev(
    ctx: NativeCallContext,
    array: &mut Array,
    reducer: FnPtr,
) -> Result<Dynamic, Box<EvalAltResult>> {
    array_reduce_rev_with_initial(ctx, array, reducer, Dynamic::UNIT)
}

fn array_reduce_rev_with_initial(
    ctx: NativeCallContext,
    array: &mut Array,
    reducer: FnPtr,
    initial: Dynamic,
) -> Result<Dynamic, Box<EvalAltResult>> {
    check_budget()?;
    let mut src_total = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), src_total)?;
    check_value_limits(ctx.engine(), &initial)?;
    if array.is_empty() {
        return Ok(initial);
    }
    let len = array.len();
    let mut acc = initial;
    for (i, item) in array.iter_mut().rev().enumerate() {
        let before = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        let rev_idx = len.saturating_sub(1).saturating_sub(i);
        let ex = [INT::try_from(rev_idx).unwrap_or(INT::MAX).into()];
        acc = call_callback(&reducer, "reduce_rev", &ctx, Some(item), [acc], ex, Some(1))?;
        let after = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        src_total = add_sizes(sub_sizes(src_total, before), after)?;
        check_sizes(ctx.engine(), src_total)?;
        check_value_limits(ctx.engine(), &acc)?;
    }
    Ok(acc)
}

fn array_drain_filter(
    ctx: NativeCallContext,
    array: &mut Array,
    filter: FnPtr,
) -> Result<Array, Box<EvalAltResult>> {
    check_budget()?;
    let mut total = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), total)?;
    if array.is_empty() {
        return Ok(Array::new());
    }
    let mut drained = Array::new();
    let mut drained_total = (0usize, 0usize, 0usize);
    let mut i: usize = 0;
    let mut x: usize = 0;
    while x < array.len() {
        let Some(item) = array.get_mut(x) else {
            break;
        };
        let before = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        let ex = [INT::try_from(i).unwrap_or(INT::MAX).into()];
        let take = call_callback(&filter, "drain", &ctx, Some(item), [], ex, Some(0))?
            .as_bool()
            .unwrap_or(false);
        let after = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        total = add_sizes(sub_sizes(total, before), after)?;
        check_sizes(ctx.engine(), total)?;
        if take {
            drained_total = add_sizes(add_sizes(drained_total, after)?, (1, 0, 0))?;
            check_sizes(ctx.engine(), drained_total)?;
            let removed = array.remove(x);
            total = sub_sizes(total, add_sizes(after, (1, 0, 0))?);
            drained.push(removed);
        } else {
            x += 1;
        }
        i += 1;
    }
    Ok(drained)
}

fn array_drain_range(
    ctx: NativeCallContext,
    array: &mut Array,
    start: INT,
    len: INT,
) -> Result<Array, Box<EvalAltResult>> {
    check_budget()?;
    let sizes = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), sizes)?;
    if array.is_empty() || len <= 0 {
        return Ok(Array::new());
    }
    let (start_idx, slice_len) = calc_offset_len(array.len(), start, len);
    if slice_len == 0 {
        return Ok(Array::new());
    }
    Ok(array
        .drain(start_idx..start_idx.saturating_add(slice_len))
        .collect())
}

fn array_drain_exclusive_range(
    ctx: NativeCallContext,
    array: &mut Array,
    range: ExclusiveRange,
) -> Result<Array, Box<EvalAltResult>> {
    let start = range.start.max(0);
    let end = range.end.max(start);
    array_drain_range(ctx, array, start, end.saturating_sub(start))
}

fn array_drain_inclusive_range(
    ctx: NativeCallContext,
    array: &mut Array,
    range: InclusiveRange,
) -> Result<Array, Box<EvalAltResult>> {
    let start = (*range.start()).max(0);
    let end = (*range.end()).max(start).min(INT::MAX - 1);
    array_drain_range(
        ctx,
        array,
        start,
        end.saturating_sub(start).saturating_add(1),
    )
}

fn array_retain_filter(
    ctx: NativeCallContext,
    array: &mut Array,
    filter: FnPtr,
) -> Result<Array, Box<EvalAltResult>> {
    check_budget()?;
    let mut total = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), total)?;
    if array.is_empty() {
        return Ok(Array::new());
    }
    let mut drained = Array::new();
    let mut drained_total = (0usize, 0usize, 0usize);
    let mut i: usize = 0;
    let mut x: usize = 0;
    while x < array.len() {
        let Some(item) = array.get_mut(x) else {
            break;
        };
        let before = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        let ex = [INT::try_from(i).unwrap_or(INT::MAX).into()];
        let keep = call_callback(&filter, "retain", &ctx, Some(item), [], ex, Some(0))?
            .as_bool()
            .unwrap_or(false);
        let after = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        total = add_sizes(sub_sizes(total, before), after)?;
        check_sizes(ctx.engine(), total)?;
        if keep {
            x += 1;
        } else {
            drained_total = add_sizes(add_sizes(drained_total, after)?, (1, 0, 0))?;
            check_sizes(ctx.engine(), drained_total)?;
            let removed = array.remove(x);
            total = sub_sizes(total, add_sizes(after, (1, 0, 0))?);
            drained.push(removed);
        }
        i += 1;
    }
    Ok(drained)
}

fn array_retain_range(
    ctx: NativeCallContext,
    array: &mut Array,
    start: INT,
    len: INT,
) -> Result<Array, Box<EvalAltResult>> {
    check_budget()?;
    let sizes = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), sizes)?;
    if array.is_empty() || len <= 0 {
        return Ok(Array::new());
    }
    let (start_idx, slice_len) = calc_offset_len(array.len(), start, len);
    if slice_len == 0 {
        return Ok(Array::new());
    }
    let mut drained: Array = array.drain(..start_idx).collect();
    drained.extend(array.drain(slice_len..));
    Ok(drained)
}

fn array_retain_exclusive_range(
    ctx: NativeCallContext,
    array: &mut Array,
    range: ExclusiveRange,
) -> Result<Array, Box<EvalAltResult>> {
    let start = range.start.max(0);
    let end = range.end.max(start);
    array_retain_range(ctx, array, start, end.saturating_sub(start))
}

fn array_retain_inclusive_range(
    ctx: NativeCallContext,
    array: &mut Array,
    range: InclusiveRange,
) -> Result<Array, Box<EvalAltResult>> {
    let start = (*range.start()).max(0);
    let end = (*range.end()).max(start).min(INT::MAX - 1);
    array_retain_range(
        ctx,
        array,
        start,
        end.saturating_sub(start).saturating_add(1),
    )
}

fn array_index_of_filter(
    ctx: NativeCallContext,
    array: &mut Array,
    filter: FnPtr,
) -> Result<INT, Box<EvalAltResult>> {
    array_index_of_filter_starting_from(ctx, array, filter, 0)
}

fn array_index_of_filter_starting_from(
    ctx: NativeCallContext,
    array: &mut Array,
    filter: FnPtr,
    start: INT,
) -> Result<INT, Box<EvalAltResult>> {
    check_budget()?;
    let mut total = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), total)?;
    if array.is_empty() {
        return Ok(-1);
    }
    let (start_idx, _) = calc_offset_len(array.len(), start, 0);
    for (i, item) in array.iter_mut().enumerate().skip(start_idx) {
        let before = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        let ex = [INT::try_from(i).unwrap_or(INT::MAX).into()];
        let matched = call_callback(&filter, "index_of", &ctx, Some(item), [], ex, Some(0))?
            .as_bool()
            .unwrap_or(false);
        let after = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        total = add_sizes(sub_sizes(total, before), after)?;
        check_sizes(ctx.engine(), total)?;
        if matched {
            return Ok(INT::try_from(i).unwrap_or(INT::MAX));
        }
    }
    Ok(-1)
}

fn array_find(
    ctx: NativeCallContext,
    array: &mut Array,
    filter: FnPtr,
) -> Result<Dynamic, Box<EvalAltResult>> {
    array_find_starting_from(ctx, array, filter, 0)
}

fn array_find_starting_from(
    ctx: NativeCallContext,
    array: &mut Array,
    filter: FnPtr,
    start: INT,
) -> Result<Dynamic, Box<EvalAltResult>> {
    let idx = array_index_of_filter_starting_from(ctx, array, filter, start)?;
    if idx < 0 {
        return Ok(Dynamic::UNIT);
    }
    let Ok(i) = usize::try_from(idx) else {
        return Ok(Dynamic::UNIT);
    };
    Ok(array.get(i).cloned().unwrap_or(Dynamic::UNIT))
}

fn array_find_map(
    ctx: NativeCallContext,
    array: &mut Array,
    filter: FnPtr,
) -> Result<Dynamic, Box<EvalAltResult>> {
    array_find_map_starting_from(ctx, array, filter, 0)
}

fn array_find_map_starting_from(
    ctx: NativeCallContext,
    array: &mut Array,
    filter: FnPtr,
    start: INT,
) -> Result<Dynamic, Box<EvalAltResult>> {
    check_budget()?;
    let mut total = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), total)?;
    if array.is_empty() {
        return Ok(Dynamic::UNIT);
    }
    let (start_idx, _) = calc_offset_len(array.len(), start, 0);
    for (i, item) in array.iter_mut().enumerate().skip(start_idx) {
        let before = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        let ex = [INT::try_from(i).unwrap_or(INT::MAX).into()];
        let val = call_callback(&filter, "find_map", &ctx, Some(item), [], ex, Some(0))?;
        let after = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        total = add_sizes(sub_sizes(total, before), after)?;
        check_sizes(ctx.engine(), total)?;
        if !val.is_unit() {
            check_value_limits(ctx.engine(), &val)?;
            return Ok(val);
        }
    }
    Ok(Dynamic::UNIT)
}

fn array_some(
    ctx: NativeCallContext,
    array: &mut Array,
    filter: FnPtr,
) -> Result<bool, Box<EvalAltResult>> {
    check_budget()?;
    let mut total = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), total)?;
    if array.is_empty() {
        return Ok(false);
    }
    for (i, item) in array.iter_mut().enumerate() {
        let before = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        let ex = [INT::try_from(i).unwrap_or(INT::MAX).into()];
        let matched = call_callback(&filter, "some", &ctx, Some(item), [], ex, Some(0))?
            .as_bool()
            .unwrap_or(false);
        let after = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        total = add_sizes(sub_sizes(total, before), after)?;
        check_sizes(ctx.engine(), total)?;
        if matched {
            return Ok(true);
        }
    }
    Ok(false)
}

fn array_all(
    ctx: NativeCallContext,
    array: &mut Array,
    filter: FnPtr,
) -> Result<bool, Box<EvalAltResult>> {
    check_budget()?;
    let mut total = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), total)?;
    if array.is_empty() {
        return Ok(true);
    }
    for (i, item) in array.iter_mut().enumerate() {
        let before = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        let ex = [INT::try_from(i).unwrap_or(INT::MAX).into()];
        let matched = call_callback(&filter, "all", &ctx, Some(item), [], ex, Some(0))?
            .as_bool()
            .unwrap_or(false);
        let after = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        total = add_sizes(sub_sizes(total, before), after)?;
        check_sizes(ctx.engine(), total)?;
        if !matched {
            return Ok(false);
        }
    }
    Ok(true)
}

fn array_order_builtin(
    ctx: NativeCallContext,
    array: &mut Array,
) -> Result<Array, Box<EvalAltResult>> {
    check_budget()?;
    let sizes = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), sizes)?;
    let mut copy = Dynamic::from_array(array.clone());
    let _ = ctx.call_native_fn_raw("sort", false, &mut [&mut copy])?;
    Ok(copy.try_cast::<Array>().unwrap_or_default())
}

fn array_order_builtin_desc(
    ctx: NativeCallContext,
    array: &mut Array,
) -> Result<Array, Box<EvalAltResult>> {
    check_budget()?;
    let sizes = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), sizes)?;
    let mut copy = Dynamic::from_array(array.clone());
    let _ = ctx.call_native_fn_raw("sort_desc", false, &mut [&mut copy])?;
    Ok(copy.try_cast::<Array>().unwrap_or_default())
}

fn map_set(
    ctx: NativeCallContext,
    map: &mut Map,
    property: &str,
    value: Dynamic,
) -> Result<(), Box<EvalAltResult>> {
    check_budget()?;
    let max_str = ctx.engine().max_string_size();
    if max_str > 0 && property.len() > max_str {
        return Err(size_exceeded_err("Length of string"));
    }
    let s1 = calc_map_sizes_bounded(map, MAX_CONTAINER_DEPTH)?;
    let s2 = calc_sizes_bounded(&value, MAX_CONTAINER_DEPTH - 1)?;
    let total = if let Some(existing) = map.get(property) {
        let old = calc_sizes_bounded(existing, MAX_CONTAINER_DEPTH - 1)?;
        add_sizes(sub_sizes(s1, old), s2)?
    } else {
        add_sizes(add_sizes(s1, s2)?, (0, 1, property.len()))?
    };
    check_sizes(ctx.engine(), total)?;
    match map.get_mut(property) {
        Some(slot) => *slot = value,
        None => {
            map.insert(property.into(), value);
        }
    }
    Ok(())
}

fn calc_map_union_sizes(
    base: &Map,
    overlay: &Map,
    overwrite: bool,
) -> Result<(usize, usize, usize), Box<EvalAltResult>> {
    let mut total = calc_map_sizes_bounded(base, MAX_CONTAINER_DEPTH)?;
    for (k, v) in overlay {
        let v_sz = calc_sizes_bounded(v, MAX_CONTAINER_DEPTH - 1)?;
        if let Some(existing) = base.get(k) {
            if overwrite {
                let old_sz = calc_sizes_bounded(existing, MAX_CONTAINER_DEPTH - 1)?;
                total = add_sizes(sub_sizes(total, old_sz), v_sz)?;
            }
        } else {
            total = add_sizes(add_sizes(total, v_sz)?, (0, 1, k.len()))?;
        }
    }
    Ok(total)
}

fn map_mixin(ctx: NativeCallContext, map: &mut Map, map2: Map) -> Result<(), Box<EvalAltResult>> {
    check_budget()?;
    check_map_depth(map, MAX_CONTAINER_DEPTH)?;
    if map2.is_empty() {
        return Ok(());
    }
    let total = calc_map_union_sizes(map, &map2, true)?;
    check_sizes(ctx.engine(), total)?;
    map.extend(map2);
    Ok(())
}

fn map_merge(ctx: NativeCallContext, mut map1: Map, map2: Map) -> Result<Map, Box<EvalAltResult>> {
    check_budget()?;
    let total = calc_map_union_sizes(&map1, &map2, true)?;
    check_sizes(ctx.engine(), total)?;
    if map2.is_empty() {
        return Ok(map1);
    }
    if map1.is_empty() {
        return Ok(map2);
    }
    map1.extend(map2);
    Ok(map1)
}

fn map_fill_with(
    ctx: NativeCallContext,
    map: &mut Map,
    map2: Map,
) -> Result<(), Box<EvalAltResult>> {
    check_budget()?;
    check_map_depth(map, MAX_CONTAINER_DEPTH)?;
    if map2.is_empty() {
        return Ok(());
    }
    let total = calc_map_union_sizes(map, &map2, false)?;
    check_sizes(ctx.engine(), total)?;
    if map.is_empty() {
        *map = map2;
        return Ok(());
    }
    for (k, v) in map2 {
        map.entry(k).or_insert(v);
    }
    Ok(())
}

fn map_keys(ctx: NativeCallContext, map: &mut Map) -> Result<Array, Box<EvalAltResult>> {
    check_budget()?;
    let sizes = calc_map_sizes_bounded(map, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), sizes)?;
    if map.is_empty() {
        return Ok(Array::new());
    }
    let mut key_bytes: usize = 0;
    for key in map.keys() {
        key_bytes = key_bytes
            .checked_add(key.len())
            .ok_or_else(|| size_exceeded_err("Length of string"))?;
    }
    check_sizes(ctx.engine(), (map.len(), 0, key_bytes))?;
    Ok(map.keys().cloned().map(Into::into).collect())
}

fn map_values(ctx: NativeCallContext, map: &mut Map) -> Result<Array, Box<EvalAltResult>> {
    check_budget()?;
    let (ax, mx, sx) = calc_map_sizes_bounded(map, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), (ax, mx, sx))?;
    if map.is_empty() {
        return Ok(Array::new());
    }
    let out_ax = ax
        .checked_add(map.len())
        .ok_or_else(|| size_exceeded_err("Size of array/BLOB"))?;
    check_sizes(ctx.engine(), (out_ax, mx, sx))?;
    Ok(map.values().cloned().collect())
}

fn map_map(
    ctx: NativeCallContext,
    map: &mut Map,
    mapper: FnPtr,
) -> Result<Map, Box<EvalAltResult>> {
    check_budget()?;
    let mut src_total = calc_map_sizes_bounded(map, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), src_total)?;
    if map.is_empty() {
        return Ok(Map::new());
    }
    let mut out = Map::new();
    let mut out_total = (0usize, 0usize, 0usize);
    for (key, item) in map.iter_mut() {
        let before = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        let mapped = call_callback(&mapper, "map", &ctx, Some(item), [key.into()], [], Some(1))?;
        let after = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        src_total = add_sizes(sub_sizes(src_total, before), after)?;
        check_sizes(ctx.engine(), src_total)?;

        let val_sz = calc_sizes_bounded(&mapped, MAX_CONTAINER_DEPTH - 1)?;
        out_total = add_sizes(add_sizes(out_total, val_sz)?, (0, 1, key.len()))?;
        check_sizes(ctx.engine(), out_total)?;
        out.insert(key.clone(), mapped);
    }
    Ok(out)
}

fn map_filter(
    ctx: NativeCallContext,
    map: &mut Map,
    filter: FnPtr,
) -> Result<Map, Box<EvalAltResult>> {
    check_budget()?;
    let mut src_total = calc_map_sizes_bounded(map, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), src_total)?;
    if map.is_empty() {
        return Ok(Map::new());
    }
    let mut out = Map::new();
    let mut out_total = (0usize, 0usize, 0usize);
    for (key, item) in map.iter_mut() {
        let before = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        let keep = call_callback(
            &filter,
            "filter",
            &ctx,
            Some(item),
            [key.into()],
            [],
            Some(1),
        )?
        .as_bool()
        .unwrap_or(false);
        let after = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        src_total = add_sizes(sub_sizes(src_total, before), after)?;
        check_sizes(ctx.engine(), src_total)?;
        if keep {
            out_total = add_sizes(add_sizes(out_total, after)?, (0, 1, key.len()))?;
            check_sizes(ctx.engine(), out_total)?;
            out.insert(key.clone(), item.clone());
        }
    }
    Ok(out)
}

fn map_drain_or_retain(
    ctx: NativeCallContext,
    map: &mut Map,
    filter: FnPtr,
    retain_matching: bool,
) -> Result<Map, Box<EvalAltResult>> {
    check_budget()?;
    let sizes = calc_map_sizes_bounded(map, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), sizes)?;
    if map.is_empty() {
        return Ok(Map::new());
    }
    let caller = if retain_matching { "retain" } else { "drain" };
    let mut drained = Map::new();
    let mut retained = Map::new();
    let mut drained_total = (0usize, 0usize, 0usize);
    let mut retained_total = (0usize, 0usize, 0usize);
    let mut error: Option<Box<EvalAltResult>> = None;

    for (key, mut value) in std::mem::take(map) {
        if error.is_some() {
            retained.insert(key, value);
            continue;
        }
        match call_callback(
            &filter,
            caller,
            &ctx,
            Some(&mut value),
            [key.clone().into()],
            [],
            Some(1),
        ) {
            Ok(res) => match calc_sizes_bounded(&value, MAX_CONTAINER_DEPTH - 1) {
                Ok(val_sz) => {
                    let keep = res.as_bool().unwrap_or(false) == retain_matching;
                    if keep {
                        match add_sizes(add_sizes(retained_total, val_sz)?, (0, 1, key.len()))
                            .and_then(|next| {
                                check_sizes(ctx.engine(), next)?;
                                Ok(next)
                            }) {
                            Ok(next) => {
                                retained_total = next;
                                retained.insert(key, value);
                            }
                            Err(err) => {
                                drop_dynamic_iteratively(value);
                                error = Some(err);
                            }
                        }
                    } else {
                        match add_sizes(add_sizes(drained_total, val_sz)?, (0, 1, key.len()))
                            .and_then(|next| {
                                check_sizes(ctx.engine(), next)?;
                                Ok(next)
                            }) {
                            Ok(next) => {
                                drained_total = next;
                                drained.insert(key, value);
                            }
                            Err(err) => {
                                drop_dynamic_iteratively(value);
                                error = Some(err);
                            }
                        }
                    }
                }
                Err(err) => {
                    drop_dynamic_iteratively(value);
                    error = Some(err);
                }
            },
            Err(err) => {
                drop_dynamic_iteratively(value);
                error = Some(err);
            }
        }
    }

    *map = retained;
    match error {
        Some(err) => {
            drop_map_iteratively(drained);
            Err(err)
        }
        None => Ok(drained),
    }
}

fn map_drain(
    ctx: NativeCallContext,
    map: &mut Map,
    filter: FnPtr,
) -> Result<Map, Box<EvalAltResult>> {
    map_drain_or_retain(ctx, map, filter, false)
}

fn map_retain(
    ctx: NativeCallContext,
    map: &mut Map,
    filter: FnPtr,
) -> Result<Map, Box<EvalAltResult>> {
    map_drain_or_retain(ctx, map, filter, true)
}

// ---------------------------------------------------------------------------
// CRIT-7: Depth-bounded equality
// ---------------------------------------------------------------------------

fn array_eq(
    ctx: NativeCallContext,
    array1: &mut Array,
    array2: Array,
) -> Result<bool, Box<EvalAltResult>> {
    check_budget()?;
    check_array_depth(array1, MAX_CONTAINER_DEPTH)?;
    check_array_depth(&array2, MAX_CONTAINER_DEPTH)?;
    array_slice_eq(&ctx, array1, &array2, 1)
}

fn array_ne(
    ctx: NativeCallContext,
    array1: &mut Array,
    array2: Array,
) -> Result<bool, Box<EvalAltResult>> {
    array_eq(ctx, array1, array2).map(|eq| !eq)
}

fn map_eq(ctx: NativeCallContext, map1: &mut Map, map2: Map) -> Result<bool, Box<EvalAltResult>> {
    check_budget()?;
    check_map_depth(map1, MAX_CONTAINER_DEPTH)?;
    check_map_depth(&map2, MAX_CONTAINER_DEPTH)?;
    map_ref_eq(&ctx, map1, &map2, 1)
}

fn map_ne(ctx: NativeCallContext, map1: &mut Map, map2: Map) -> Result<bool, Box<EvalAltResult>> {
    map_eq(ctx, map1, map2).map(|eq| !eq)
}

fn array_slice_eq(
    ctx: &NativeCallContext,
    a1: &[Dynamic],
    a2: &[Dynamic],
    depth: usize,
) -> Result<bool, Box<EvalAltResult>> {
    if depth > MAX_CONTAINER_DEPTH {
        return Err(depth_exceeded_err());
    }
    if a1.len() != a2.len() {
        return Ok(false);
    }
    for (x, y) in a1.iter().zip(a2.iter()) {
        if !dynamic_equals(ctx, x, y, depth)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn map_ref_eq(
    ctx: &NativeCallContext,
    m1: &Map,
    m2: &Map,
    depth: usize,
) -> Result<bool, Box<EvalAltResult>> {
    if depth > MAX_CONTAINER_DEPTH {
        return Err(depth_exceeded_err());
    }
    if m1.len() != m2.len() {
        return Ok(false);
    }
    for (k, v1) in m1 {
        let Some(v2) = m2.get(k) else {
            return Ok(false);
        };
        if !dynamic_equals(ctx, v1, v2, depth)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn dynamic_equals(
    ctx: &NativeCallContext,
    lhs: &Dynamic,
    rhs: &Dynamic,
    depth: usize,
) -> Result<bool, Box<EvalAltResult>> {
    check_budget()?;
    if let (Some(a1), Some(a2)) = (lhs.read_lock::<Array>(), rhs.read_lock::<Array>()) {
        return array_slice_eq(ctx, &a1, &a2, depth + 1);
    }
    if let (Some(m1), Some(m2)) = (lhs.read_lock::<Map>(), rhs.read_lock::<Map>()) {
        return map_ref_eq(ctx, &m1, &m2, depth + 1);
    }
    if lhs.is_array() || rhs.is_array() || lhs.is_map() || rhs.is_map() {
        return Ok(false);
    }
    let mut a = lhs.clone();
    let mut b = rhs.clone();
    ctx.call_native_fn_raw("==", true, &mut [&mut a, &mut b])
        .or_else(|err| match *err {
            EvalAltResult::ErrorFunctionNotFound(ref sig, ..) if sig.starts_with("==") => {
                if a.type_id() == b.type_id() {
                    Err(err)
                } else {
                    Ok(Dynamic::FALSE)
                }
            }
            _ => Err(err),
        })
        .map(|val| val.as_bool().unwrap_or(false))
}

// ---------------------------------------------------------------------------
// CRIT-7: Depth- and size-bounded container formatting
// ---------------------------------------------------------------------------

struct BoundedBuf {
    out: String,
    max_bytes: usize,
}

impl BoundedBuf {
    fn new(engine: &Engine) -> Self {
        let cap = engine.max_string_size();
        Self {
            out: String::new(),
            max_bytes: if cap > 0 {
                cap
            } else {
                FALLBACK_MAX_FORMAT_BYTES
            },
        }
    }

    fn push_str(&mut self, text: &str) -> Result<(), Box<EvalAltResult>> {
        if self.out.len().saturating_add(text.len()) > self.max_bytes {
            return Err(size_exceeded_err("Length of string"));
        }
        self.out.push_str(text);
        Ok(())
    }
}

fn format_leaf_debug(
    ctx: &NativeCallContext,
    value: &Dynamic,
) -> Result<String, Box<EvalAltResult>> {
    if value.is_unit() {
        return Ok("()".to_owned());
    }
    if let Ok(n) = value.as_int() {
        return Ok(n.to_string());
    }
    if let Ok(b) = value.as_bool() {
        return Ok(if b { "true" } else { "false" }.to_owned());
    }
    if let Ok(ch) = value.as_char() {
        return Ok(ch.to_string());
    }
    if let Some(s) = value.read_lock::<ImmutableString>() {
        let mut out = String::new();
        let _ = write!(out, "{:?}", s.as_str());
        return Ok(out);
    }
    let mut copy = value.clone();
    match ctx.call_native_fn_raw("to_debug", true, &mut [&mut copy]) {
        Ok(res) if res.is_string() => Ok(res
            .into_immutable_string()
            .map(|s| s.to_string())
            .unwrap_or_default()),
        Ok(res) => Ok(ctx.engine().map_type_name(res.type_name()).to_owned()),
        Err(_) => Ok(ctx.engine().map_type_name(value.type_name()).to_owned()),
    }
}

fn format_dynamic_into(
    ctx: &NativeCallContext,
    buf: &mut BoundedBuf,
    value: &Dynamic,
    depth: usize,
) -> Result<(), Box<EvalAltResult>> {
    check_budget()?;
    if let Some(arr) = value.read_lock::<Array>() {
        return format_array_into(ctx, buf, &arr, depth);
    }
    if let Some(map) = value.read_lock::<Map>() {
        return format_map_into(ctx, buf, &map, depth);
    }
    let leaf = format_leaf_debug(ctx, value)?;
    buf.push_str(&leaf)
}

fn format_array_into(
    ctx: &NativeCallContext,
    buf: &mut BoundedBuf,
    array: &Array,
    depth: usize,
) -> Result<(), Box<EvalAltResult>> {
    if depth > MAX_CONTAINER_DEPTH {
        return Err(depth_exceeded_err());
    }
    buf.push_str("[")?;
    for (i, item) in array.iter().enumerate() {
        if i > 0 {
            buf.push_str(", ")?;
        }
        format_dynamic_into(ctx, buf, item, depth + 1)?;
    }
    buf.push_str("]")
}

fn format_map_into(
    ctx: &NativeCallContext,
    buf: &mut BoundedBuf,
    map: &Map,
    depth: usize,
) -> Result<(), Box<EvalAltResult>> {
    if depth > MAX_CONTAINER_DEPTH {
        return Err(depth_exceeded_err());
    }
    buf.push_str("#{")?;
    for (i, (key, val)) in map.iter().enumerate() {
        if i > 0 {
            buf.push_str(", ")?;
        }
        let mut quoted = String::new();
        let _ = write!(quoted, "{:?}", key.as_str());
        buf.push_str(&quoted)?;
        buf.push_str(": ")?;
        format_dynamic_into(ctx, buf, val, depth + 1)?;
    }
    buf.push_str("}")
}

fn print_array(
    ctx: NativeCallContext,
    array: &mut Array,
) -> Result<ImmutableString, Box<EvalAltResult>> {
    let mut buf = BoundedBuf::new(ctx.engine());
    format_array_into(&ctx, &mut buf, array, 1)?;
    Ok(buf.out.into())
}

fn print_map(ctx: NativeCallContext, map: &mut Map) -> Result<ImmutableString, Box<EvalAltResult>> {
    let mut buf = BoundedBuf::new(ctx.engine());
    format_map_into(&ctx, &mut buf, map, 1)?;
    Ok(buf.out.into())
}

/// `to_string`/`to_debug` for `Array`: latches the limit fault instead of
/// returning `Err` into Rhai's `print_with_func` (string interpolation), which
/// would otherwise swallow `Err` and fall back to unbounded recursive `Display`.
fn to_string_array(
    ctx: NativeCallContext,
    array: &mut Array,
) -> Result<ImmutableString, Box<EvalAltResult>> {
    let mut buf = BoundedBuf::new(ctx.engine());
    match format_array_into(&ctx, &mut buf, array, 1) {
        Ok(()) => Ok(buf.out.into()),
        Err(err) if matches!(*err, EvalAltResult::ErrorDataTooLarge(..)) => {
            latch_limit();
            Ok(ImmutableString::new())
        }
        Err(err) if matches!(*err, EvalAltResult::ErrorTerminated(..)) => {
            Ok(ImmutableString::new())
        }
        Err(err) => Err(err),
    }
}

fn to_string_map(
    ctx: NativeCallContext,
    map: &mut Map,
) -> Result<ImmutableString, Box<EvalAltResult>> {
    let mut buf = BoundedBuf::new(ctx.engine());
    match format_map_into(&ctx, &mut buf, map, 1) {
        Ok(()) => Ok(buf.out.into()),
        Err(err) if matches!(*err, EvalAltResult::ErrorDataTooLarge(..)) => {
            latch_limit();
            Ok(ImmutableString::new())
        }
        Err(err) if matches!(*err, EvalAltResult::ErrorTerminated(..)) => {
            Ok(ImmutableString::new())
        }
        Err(err) => Err(err),
    }
}

fn format_json_value_into(
    buf: &mut BoundedBuf,
    value: &Dynamic,
    depth: usize,
) -> Result<(), Box<EvalAltResult>> {
    check_budget()?;
    if value.is_unit() {
        return buf.push_str("null");
    }
    if let Some(arr) = value.read_lock::<Array>() {
        if depth > MAX_CONTAINER_DEPTH {
            return Err(depth_exceeded_err());
        }
        buf.push_str("[")?;
        for (i, item) in arr.iter().enumerate() {
            if i > 0 {
                buf.push_str(",")?;
            }
            format_json_value_into(buf, item, depth + 1)?;
        }
        return buf.push_str("]");
    }
    if let Some(map) = value.read_lock::<Map>() {
        return format_json_map_into(buf, &map, depth);
    }
    if let Ok(n) = value.as_int() {
        return buf.push_str(&n.to_string());
    }
    if let Ok(b) = value.as_bool() {
        return buf.push_str(if b { "true" } else { "false" });
    }
    if let Ok(f) = value.as_float() {
        let mut s = String::new();
        let _ = write!(s, "{f:?}");
        return buf.push_str(&s);
    }
    if let Some(s) = value.read_lock::<ImmutableString>() {
        let mut quoted = String::new();
        let _ = write!(quoted, "{:?}", s.as_str());
        return buf.push_str(&quoted);
    }
    if let Ok(ch) = value.as_char() {
        let mut quoted = String::new();
        let _ = write!(quoted, "{ch:?}");
        return buf.push_str(&quoted);
    }
    let mut s = String::new();
    let _ = write!(s, "{value:?}");
    buf.push_str(&s)
}

fn format_json_map_into(
    buf: &mut BoundedBuf,
    map: &Map,
    depth: usize,
) -> Result<(), Box<EvalAltResult>> {
    if depth > MAX_CONTAINER_DEPTH {
        return Err(depth_exceeded_err());
    }
    buf.push_str("{")?;
    for (i, (key, val)) in map.iter().enumerate() {
        if i > 0 {
            buf.push_str(",")?;
        }
        let mut quoted = String::new();
        let _ = write!(quoted, "{:?}", key.as_str());
        buf.push_str(&quoted)?;
        buf.push_str(":")?;
        format_json_value_into(buf, val, depth + 1)?;
    }
    buf.push_str("}")
}

fn map_to_json(
    ctx: NativeCallContext,
    map: &mut Map,
) -> Result<ImmutableString, Box<EvalAltResult>> {
    let mut buf = BoundedBuf::new(ctx.engine());
    format_json_map_into(&mut buf, map, 1)?;
    Ok(buf.out.into())
}

// ---------------------------------------------------------------------------
// HIGH-16: Fallible iterative sort, order, and dedup
// ---------------------------------------------------------------------------

fn compare_with_fn_ptr(
    ctx: &NativeCallContext,
    comparer: &FnPtr,
    x: &Dynamic,
    y: &Dynamic,
) -> Result<Ordering, Box<EvalAltResult>> {
    check_budget()?;
    let val = comparer
        .call_raw(ctx, None, [x.clone(), y.clone()])
        .map_err(handle_callback_err)?;
    check_budget()?;
    if let Ok(n) = val.as_int() {
        return Ok(n.cmp(&0));
    }
    if let Ok(b) = val.as_bool() {
        return Ok(if b { Ordering::Less } else { Ordering::Greater });
    }
    Ok(x.type_id().cmp(&y.type_id()))
}

/// Stable, iterative, bottom-up merge sort that aborts on the first comparator
/// error or budget trip without panicking on non-transitive comparators.
fn merge_sort_by(
    ctx: &NativeCallContext,
    array: &mut Array,
    comparer: &FnPtr,
) -> Result<(), Box<EvalAltResult>> {
    check_budget()?;
    check_array_depth(array, MAX_CONTAINER_DEPTH)?;
    let n = array.len();
    if n <= 1 {
        return Ok(());
    }
    let mut src = std::mem::take(array);
    let mut width: usize = 1;
    while width < n {
        let mut dst = Array::with_capacity(n);
        let mut iter = src.into_iter();
        loop {
            let mut left: Vec<Dynamic> = iter.by_ref().take(width).collect();
            if left.is_empty() {
                break;
            }
            let right: Vec<Dynamic> = iter.by_ref().take(width).collect();
            if right.is_empty() {
                dst.append(&mut left);
                break;
            }
            let mut li = left.into_iter().peekable();
            let mut ri = right.into_iter().peekable();
            while let (Some(l), Some(r)) = (li.peek(), ri.peek()) {
                let ord = match compare_with_fn_ptr(ctx, comparer, l, r) {
                    Ok(ord) => ord,
                    Err(err) => {
                        dst.extend(li);
                        dst.extend(ri);
                        dst.extend(iter);
                        *array = dst;
                        return Err(err);
                    }
                };
                if matches!(ord, Ordering::Less | Ordering::Equal) {
                    if let Some(item) = li.next() {
                        dst.push(item);
                    }
                } else if let Some(item) = ri.next() {
                    dst.push(item);
                }
            }
            dst.extend(li);
            dst.extend(ri);
        }
        src = dst;
        width = width.saturating_mul(2);
    }
    *array = src;
    Ok(())
}

fn array_sort_by(
    ctx: NativeCallContext,
    array: &mut Array,
    comparer: FnPtr,
) -> Result<(), Box<EvalAltResult>> {
    merge_sort_by(&ctx, array, &comparer)
}

fn array_sort_by_name(
    ctx: NativeCallContext,
    array: &mut Array,
    comparer: &str,
) -> Result<(), Box<EvalAltResult>> {
    let fp = FnPtr::new(comparer)?;
    merge_sort_by(&ctx, array, &fp)
}

fn array_order_by(
    ctx: NativeCallContext,
    array: &mut Array,
    comparer: FnPtr,
) -> Result<Array, Box<EvalAltResult>> {
    check_budget()?;
    let sizes = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), sizes)?;
    let mut copy = array.clone();
    merge_sort_by(&ctx, &mut copy, &comparer)?;
    Ok(copy)
}

fn dedup_with(
    array: &mut Array,
    mut is_dup: impl FnMut(&Dynamic, &Dynamic) -> Result<bool, Box<EvalAltResult>>,
) -> Result<(), Box<EvalAltResult>> {
    check_budget()?;
    check_array_depth(array, MAX_CONTAINER_DEPTH)?;
    if array.len() <= 1 {
        return Ok(());
    }
    let mut iter = std::mem::take(array).into_iter();
    let mut kept = Array::with_capacity(iter.len());
    if let Some(first) = iter.next() {
        kept.push(first);
    }
    while let Some(curr) = iter.next() {
        if let Err(err) = check_budget() {
            kept.push(curr);
            kept.extend(iter);
            *array = kept;
            return Err(err);
        }
        let Some(prev) = kept.last() else {
            kept.push(curr);
            continue;
        };
        let dup = match is_dup(prev, &curr) {
            Ok(d) => {
                if let Err(err) = check_budget() {
                    kept.push(curr);
                    kept.extend(iter);
                    *array = kept;
                    return Err(err);
                }
                d
            }
            Err(err) => {
                kept.push(curr);
                kept.extend(iter);
                *array = kept;
                return Err(err);
            }
        };
        if !dup {
            kept.push(curr);
        }
    }
    *array = kept;
    Ok(())
}

fn array_dedup_default(
    ctx: NativeCallContext,
    array: &mut Array,
) -> Result<(), Box<EvalAltResult>> {
    dedup_with(array, |prev, curr| dynamic_equals(&ctx, prev, curr, 1))
}

fn array_dedup_by(
    ctx: NativeCallContext,
    array: &mut Array,
    comparer: FnPtr,
) -> Result<(), Box<EvalAltResult>> {
    dedup_with(array, |prev, curr| {
        let res = comparer
            .call_raw(&ctx, None, [prev.clone(), curr.clone()])
            .map_err(handle_callback_err)?;
        Ok(res.as_bool().unwrap_or(false))
    })
}

fn array_dedup_by_name(
    ctx: NativeCallContext,
    array: &mut Array,
    comparer: &str,
) -> Result<(), Box<EvalAltResult>> {
    let fp = FnPtr::new(comparer)?;
    array_dedup_by(ctx, array, fp)
}

// ---------------------------------------------------------------------------
// CRIT-7 + P2-CORE-2: Iterative teardown
// ---------------------------------------------------------------------------

/// Iteratively dismantle any nested `Array`/`Map` tree (including `Shared`
/// wrappers) inside `root` so dropping `this`, `Scope` variables, return
/// values, or error payloads never overflows the stack.
pub(crate) fn drop_dynamic_iteratively(root: Dynamic) {
    let mut stack = vec![root];
    while let Some(mut val) = stack.pop() {
        if val.is_shared() {
            if let Some(mut guard) = val.write_lock::<Dynamic>() {
                let inner = std::mem::take(&mut *guard);
                drop(guard);
                if !inner.is_unit() {
                    stack.push(inner);
                }
            }
        } else if val.is_array() {
            if let Some(arr) = val.try_cast::<Array>() {
                stack.extend(arr);
            }
        } else if val.is_map() {
            if let Some(map) = val.try_cast::<Map>() {
                stack.extend(map.into_values());
            }
        } else if val.is_fnptr()
            && let Some(mut fp) = val.try_cast::<FnPtr>()
        {
            for arg in fp.iter_curry_mut() {
                let inner = std::mem::take(arg);
                if !inner.is_unit() {
                    stack.push(inner);
                }
            }
        }
    }
}

/// Iteratively dismantle a `Map` whose values may contain nested containers.
pub(crate) fn drop_map_iteratively(map: Map) {
    let mut stack: Vec<Dynamic> = map.into_values().collect();
    while let Some(val) = stack.pop() {
        drop_dynamic_iteratively(val);
    }
}

/// Iteratively dismantle every variable stored inside a `Scope`.
pub(crate) fn drop_scope_iteratively(scope: Scope<'_>) {
    for (_name, val, _aliases) in scope {
        drop_dynamic_iteratively(val);
    }
}

/// Iteratively dismantle any `Dynamic` payload (and nested `ErrorInFunctionCall`
/// / `ErrorInModule` wrappers) inside an `EvalAltResult`.
#[allow(clippy::boxed_local)]
pub(crate) fn drop_eval_error_iteratively(mut err: Box<EvalAltResult>) {
    loop {
        match *err {
            EvalAltResult::ErrorInFunctionCall(_, _, inner, _)
            | EvalAltResult::ErrorInModule(_, inner, _) => {
                err = inner;
            }
            EvalAltResult::ErrorRuntime(val, _)
            | EvalAltResult::ErrorIndexNotFound(val, _)
            | EvalAltResult::ErrorTerminated(val, _)
            | EvalAltResult::LoopBreak(_, val, _)
            | EvalAltResult::Return(val, _)
            | EvalAltResult::Exit(val, _) => {
                drop_dynamic_iteratively(val);
                return;
            }
            _ => return,
        }
    }
}

// ---------------------------------------------------------------------------
// P2-CORE-1 + P2-CORE-2: Compile-time lexer/source & AST structural guards
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Token<'a> {
    Ident(&'a str),
    HashLBrace,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    LParen,
    RParen,
    Dot,
    Assign,
    EqEq,
    FatArrow,
    Semi,
    Comma,
    Colon,
    Value,
    InterpStart,
    InterpEnd,
    Op,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LexMode {
    Code,
    TemplateString,
    Interp { brace_depth: usize },
}

fn next_char_len(source: &str, idx: usize) -> usize {
    source
        .get(idx..)
        .and_then(|rest| rest.chars().next())
        .map_or(1, char::len_utf8)
}

/// Tokenize `source` faithfully following Rhai 1.26's lexer rules (verbatim
/// backtick templates with `\${` and `` `` `` escapes, double-quote `""`
/// escapes, raw strings `#"..."#`, char literals with `''`, nested `/* */`
/// comments, and `#{` inside `${...}` interpolations). Fails closed on any
/// unclosed literal, comment, template, or interpolation at EOF.
fn lex_tokens(source: &str) -> Result<Vec<Token<'_>>, String> {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut modes = vec![LexMode::Code];
    let mut out = Vec::new();

    while i < len {
        let mode = modes.last().copied().unwrap_or(LexMode::Code);
        if mode == LexMode::TemplateString {
            match bytes.get(i).copied() {
                Some(b'`') => {
                    // Rhai `parse_string_literal`: double backtick `` `` `` is
                    // an escaped literal backtick inside a template string.
                    if bytes.get(i + 1) == Some(&b'`') {
                        i += 2;
                    } else {
                        i += 1;
                        if modes.len() > 1 {
                            modes.pop();
                        }
                        out.push(Token::Value);
                    }
                }
                // Template strings are verbatim in Rhai (`verbatim = true`):
                // `\` does NOT escape `` ` ``; only `\${` escapes `$`.
                Some(b'\\')
                    if bytes.get(i + 1) == Some(&b'$') && bytes.get(i + 2) == Some(&b'{') =>
                {
                    i += 2;
                }
                Some(b'$') if bytes.get(i + 1) == Some(&b'{') => {
                    i += 2;
                    modes.push(LexMode::Interp { brace_depth: 0 });
                    out.push(Token::InterpStart);
                }
                _ => {
                    i += next_char_len(source, i);
                }
            }
            continue;
        }

        let Some(b) = bytes.get(i).copied() else {
            break;
        };

        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // Line comment `//...`
        if b == b'/' && bytes.get(i + 1) == Some(&b'/') {
            i += 2;
            while let Some(c) = bytes.get(i).copied() {
                i += 1;
                if c == b'\n' {
                    break;
                }
            }
            continue;
        }

        // Nested block comment `/* ... */`
        if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            let mut depth: usize = 1;
            while i < len && depth > 0 {
                if bytes.get(i) == Some(&b'/') && bytes.get(i + 1) == Some(&b'*') {
                    depth += 1;
                    i += 2;
                } else if bytes.get(i) == Some(&b'*') && bytes.get(i + 1) == Some(&b'/') {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if depth != 0 {
                return Err("Unterminated block comment".to_owned());
            }
            continue;
        }

        // Double-quoted string literal (`\"`, `\\`, line continuation, and `""`).
        if b == b'"' {
            i += 1;
            let mut closed = false;
            while let Some(c) = bytes.get(i).copied() {
                if c == b'\\' {
                    i += 1;
                    if i < len {
                        if bytes.get(i) == Some(&b'\r') && bytes.get(i + 1) == Some(&b'\n') {
                            i += 2;
                        } else {
                            i += next_char_len(source, i);
                        }
                    }
                } else if c == b'"' {
                    if bytes.get(i + 1) == Some(&b'"') {
                        i += 2;
                    } else {
                        i += 1;
                        closed = true;
                        break;
                    }
                } else {
                    i += next_char_len(source, i);
                }
            }
            if !closed {
                return Err("Unterminated string literal".to_owned());
            }
            out.push(Token::Value);
            continue;
        }

        // Map literal `#{` or raw string `#"..."#`
        if b == b'#' {
            if bytes.get(i + 1) == Some(&b'{') {
                if let Some(LexMode::Interp { brace_depth }) = modes.last_mut() {
                    *brace_depth += 1;
                }
                i += 2;
                out.push(Token::HashLBrace);
                continue;
            }
            let mut hashes = 0;
            while bytes.get(i + hashes) == Some(&b'#') {
                hashes += 1;
            }
            if bytes.get(i + hashes) == Some(&b'"') {
                i += hashes + 1;
                let mut closed = false;
                while i < len {
                    if bytes.get(i) == Some(&b'"') {
                        let mut closing = 0;
                        while closing < hashes && bytes.get(i + 1 + closing) == Some(&b'#') {
                            closing += 1;
                        }
                        if closing == hashes {
                            i += 1 + hashes;
                            closed = true;
                            break;
                        }
                    }
                    i += 1;
                }
                if !closed {
                    return Err("Unterminated raw string literal".to_owned());
                }
                out.push(Token::Value);
                continue;
            }
            i += 1;
            out.push(Token::Op);
            continue;
        }

        // Char literal `'...'` (with `\'` and `''` escapes).
        if b == b'\'' {
            i += 1;
            let mut closed = false;
            while let Some(c) = bytes.get(i).copied() {
                if c == b'\\' {
                    i += 1;
                    if i < len {
                        i += next_char_len(source, i);
                    }
                } else if c == b'\'' {
                    if bytes.get(i + 1) == Some(&b'\'') {
                        i += 2;
                    } else {
                        i += 1;
                        closed = true;
                        break;
                    }
                } else {
                    i += next_char_len(source, i);
                }
            }
            if !closed {
                return Err("Unterminated character literal".to_owned());
            }
            out.push(Token::Value);
            continue;
        }

        // Backtick template string `` `...` ``
        if b == b'`' {
            i += 1;
            modes.push(LexMode::TemplateString);
            continue;
        }

        // Braces (tracking interpolation boundaries)
        if b == b'{' {
            if let Some(LexMode::Interp { brace_depth }) = modes.last_mut() {
                *brace_depth += 1;
            }
            i += 1;
            out.push(Token::LBrace);
            continue;
        }
        if b == b'}' {
            if let Some(LexMode::Interp { brace_depth }) = modes.last_mut() {
                if *brace_depth == 0 {
                    modes.pop();
                    i += 1;
                    out.push(Token::InterpEnd);
                    continue;
                }
                *brace_depth -= 1;
            }
            i += 1;
            out.push(Token::RBrace);
            continue;
        }

        // Single-byte delimiters & punctuation
        match b {
            b'[' => {
                i += 1;
                out.push(Token::LBracket);
                continue;
            }
            b']' => {
                i += 1;
                out.push(Token::RBracket);
                continue;
            }
            b'(' => {
                i += 1;
                out.push(Token::LParen);
                continue;
            }
            b')' => {
                i += 1;
                out.push(Token::RParen);
                continue;
            }
            b';' => {
                i += 1;
                out.push(Token::Semi);
                continue;
            }
            b',' => {
                i += 1;
                out.push(Token::Comma);
                continue;
            }
            b'.' => {
                if bytes.get(i + 1) == Some(&b'.') {
                    i += if bytes.get(i + 2) == Some(&b'=') {
                        3
                    } else {
                        2
                    };
                    out.push(Token::Op);
                } else {
                    i += 1;
                    out.push(Token::Dot);
                }
                continue;
            }
            b':' => {
                if bytes.get(i + 1) == Some(&b':') {
                    i += 2;
                    out.push(Token::Op);
                } else {
                    i += 1;
                    out.push(Token::Colon);
                }
                continue;
            }
            _ => {}
        }

        // Equality, arrows, and assignments
        if b == b'=' {
            match bytes.get(i + 1).copied() {
                Some(b'=') => {
                    i += 2;
                    out.push(Token::EqEq);
                }
                Some(b'>') => {
                    i += 2;
                    out.push(Token::FatArrow);
                }
                _ => {
                    i += 1;
                    out.push(Token::Assign);
                }
            }
            continue;
        }
        if b == b'!' && bytes.get(i + 1) == Some(&b'=') {
            i += 2;
            out.push(Token::EqEq);
            continue;
        }
        if matches!(
            (b, bytes.get(i + 1).copied(), bytes.get(i + 2).copied()),
            (b'*', Some(b'*'), Some(b'='))
                | (b'<', Some(b'<'), Some(b'='))
                | (b'>', Some(b'>'), Some(b'='))
                | (b'?', Some(b'?'), Some(b'='))
        ) {
            i += 3;
            out.push(Token::Assign);
            continue;
        }
        if matches!(
            (b, bytes.get(i + 1).copied()),
            (b'+', Some(b'='))
                | (b'-', Some(b'='))
                | (b'*', Some(b'='))
                | (b'/', Some(b'='))
                | (b'%', Some(b'='))
                | (b'&', Some(b'='))
                | (b'|', Some(b'='))
                | (b'^', Some(b'='))
        ) {
            i += 2;
            out.push(Token::Assign);
            continue;
        }
        if matches!(
            (b, bytes.get(i + 1).copied()),
            (b'<', Some(b'=')) | (b'>', Some(b'='))
        ) {
            i += 2;
            out.push(Token::Op);
            continue;
        }

        // Identifiers
        if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            i += 1;
            while let Some(c) = bytes.get(i).copied() {
                if c.is_ascii_alphanumeric() || c == b'_' {
                    i += 1;
                } else {
                    break;
                }
            }
            let ident = source.get(start..i).unwrap_or_default();
            out.push(Token::Ident(ident));
            continue;
        }

        // Numeric literals
        if b.is_ascii_digit() {
            i += 1;
            while let Some(c) = bytes.get(i).copied() {
                if c.is_ascii_alphanumeric()
                    || c == b'_'
                    || (c == b'.'
                        && bytes
                            .get(i + 1)
                            .copied()
                            .is_some_and(|n| n.is_ascii_digit()))
                {
                    i += 1;
                } else {
                    break;
                }
            }
            out.push(Token::Value);
            continue;
        }

        i += next_char_len(source, i);
        out.push(Token::Op);
    }

    if modes.len() != 1 || modes.first() != Some(&LexMode::Code) {
        return Err("Unterminated template string or interpolation".to_owned());
    }

    Ok(out)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Delim {
    ArrayLit,
    Index,
    MapLit,
    Block,
    CallParen,
    GroupParen,
    Interp,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StmtState {
    Start,
    LetDecl,
    LhsLocal,
    LhsThisBare,
    LhsThisMember,
    RhsLocal,
    RhsThisBare,
    RhsThisMember,
    Other,
}

#[derive(Clone, Copy)]
struct StmtFrame {
    state: StmtState,
    base_delim_depth: usize,
}

fn is_prefix_keyword(word: &str) -> bool {
    matches!(
        word,
        "return"
            | "throw"
            | "in"
            | "else"
            | "if"
            | "while"
            | "until"
            | "loop"
            | "do"
            | "switch"
            | "break"
            | "continue"
            | "let"
            | "const"
            | "fn"
            | "private"
            | "try"
            | "catch"
    )
}

fn is_stmt_starter_keyword(word: &str) -> bool {
    matches!(
        word,
        "let"
            | "const"
            | "fn"
            | "private"
            | "return"
            | "break"
            | "continue"
            | "for"
            | "while"
            | "loop"
            | "do"
            | "until"
            | "throw"
    )
}

/// Compile-time structural source guard (CRIT-7, P2-CORE-1, P2-CORE-2) paired
/// with [`validate_ast`]:
///
/// 1. Caps static container-literal (`[` / `#{`) nesting depth at
///    [`MAX_CONTAINER_DEPTH`] (8).
/// 2. Rejects unclosed literals, comments, templates, interpolations, and
///    unmatched delimiters.
/// 3. Rejects `this` self-nesting container constructs across all statements
///    (including `switch` arm bodies).
pub(crate) fn validate_source(source: &str) -> Result<(), String> {
    let tokens = lex_tokens(source)?;
    let mut delims: Vec<Delim> = Vec::new();
    let mut container_depth: usize = 0;
    let mut stmt_stack: Vec<StmtFrame> = vec![StmtFrame {
        state: StmtState::Start,
        base_delim_depth: 0,
    }];
    let mut prev_is_value_end = false;

    for (idx, &tok) in tokens.iter().enumerate() {
        let prev = idx.checked_sub(1).and_then(|p| tokens.get(p).copied());
        let next = tokens.get(idx + 1).copied();

        let at_base = stmt_stack
            .last()
            .is_some_and(|frame| delims.len() == frame.base_delim_depth);
        let in_this_rhs = stmt_stack
            .iter()
            .any(|f| matches!(f.state, StmtState::RhsThisBare | StmtState::RhsThisMember));
        let in_this_member_rhs = stmt_stack
            .iter()
            .any(|f| f.state == StmtState::RhsThisMember);
        let in_local_rhs =
            stmt_stack.iter().any(|f| f.state == StmtState::RhsLocal) && !in_this_rhs;
        let in_call_paren = delims.contains(&Delim::CallParen);

        match tok {
            Token::HashLBrace => {
                container_depth += 1;
                if container_depth > MAX_CONTAINER_DEPTH {
                    return Err(format!(
                        "Container literal nesting exceeds maximum complexity (limit {MAX_CONTAINER_DEPTH})"
                    ));
                }
                delims.push(Delim::MapLit);
                if at_base
                    && let Some(frame) = stmt_stack.last_mut()
                    && frame.state == StmtState::Start
                {
                    frame.state = StmtState::Other;
                }
                prev_is_value_end = false;
            }
            Token::LBracket => {
                if prev_is_value_end {
                    delims.push(Delim::Index);
                    if at_base
                        && let Some(frame) = stmt_stack.last_mut()
                        && frame.state == StmtState::LhsThisBare
                    {
                        frame.state = StmtState::LhsThisMember;
                    }
                } else {
                    container_depth += 1;
                    if container_depth > MAX_CONTAINER_DEPTH {
                        return Err(format!(
                            "Container literal nesting exceeds maximum complexity (limit {MAX_CONTAINER_DEPTH})"
                        ));
                    }
                    delims.push(Delim::ArrayLit);
                    if at_base
                        && let Some(frame) = stmt_stack.last_mut()
                        && frame.state == StmtState::Start
                    {
                        frame.state = StmtState::Other;
                    }
                }
                prev_is_value_end = false;
            }
            Token::RBracket => {
                let Some(d) = delims.pop() else {
                    return Err("Unmatched closing bracket".to_owned());
                };
                if d == Delim::ArrayLit {
                    container_depth = container_depth.saturating_sub(1);
                } else if d != Delim::Index {
                    return Err("Mismatched closing bracket".to_owned());
                }
                prev_is_value_end = true;
            }
            Token::LBrace => {
                delims.push(Delim::Block);
                stmt_stack.push(StmtFrame {
                    state: StmtState::Start,
                    base_delim_depth: delims.len(),
                });
                prev_is_value_end = false;
            }
            Token::RBrace => match delims.pop() {
                Some(Delim::MapLit) => {
                    container_depth = container_depth.saturating_sub(1);
                    prev_is_value_end = true;
                }
                Some(Delim::Block) => {
                    if stmt_stack.len() > 1 {
                        stmt_stack.pop();
                    }
                    if let Some(frame) = stmt_stack.last_mut()
                        && delims.len() == frame.base_delim_depth
                        && !matches!(
                            frame.state,
                            StmtState::RhsLocal | StmtState::RhsThisBare | StmtState::RhsThisMember
                        )
                    {
                        frame.state = StmtState::Start;
                    }
                    prev_is_value_end = false;
                }
                _ => {
                    return Err("Unmatched closing brace".to_owned());
                }
            },
            Token::LParen => {
                let is_call = matches!(prev, Some(Token::Ident(word)) if !is_prefix_keyword(word));
                if at_base
                    && let Some(frame) = stmt_stack.last_mut()
                    && matches!(
                        frame.state,
                        StmtState::Start
                            | StmtState::LhsLocal
                            | StmtState::LhsThisBare
                            | StmtState::LhsThisMember
                    )
                {
                    frame.state = StmtState::Other;
                }
                delims.push(if is_call {
                    Delim::CallParen
                } else {
                    Delim::GroupParen
                });
                prev_is_value_end = false;
            }
            Token::RParen => {
                if !matches!(delims.pop(), Some(Delim::CallParen | Delim::GroupParen)) {
                    return Err("Unmatched closing parenthesis".to_owned());
                }
                prev_is_value_end = true;
            }
            Token::InterpStart => {
                delims.push(Delim::Interp);
                stmt_stack.push(StmtFrame {
                    state: StmtState::Start,
                    base_delim_depth: delims.len(),
                });
                prev_is_value_end = false;
            }
            Token::InterpEnd => {
                if delims.pop() != Some(Delim::Interp) {
                    return Err("Unmatched interpolation closing brace".to_owned());
                }
                if stmt_stack.len() > 1 {
                    stmt_stack.pop();
                }
                prev_is_value_end = false;
            }
            Token::Semi | Token::FatArrow => {
                if at_base && let Some(frame) = stmt_stack.last_mut() {
                    frame.state = StmtState::Start;
                }
                prev_is_value_end = false;
            }
            Token::Comma => {
                // At block base, `,` separates `switch` arms; reset to `Start`
                // so the next arm pattern/body tracks `this` assignments cleanly.
                if at_base && let Some(frame) = stmt_stack.last_mut() {
                    frame.state = StmtState::Start;
                }
                prev_is_value_end = false;
            }
            Token::Dot => {
                if at_base
                    && let Some(frame) = stmt_stack.last_mut()
                    && frame.state == StmtState::LhsThisBare
                {
                    frame.state = StmtState::LhsThisMember;
                }
                prev_is_value_end = false;
            }
            Token::Assign => {
                if at_base && let Some(frame) = stmt_stack.last_mut() {
                    frame.state = match frame.state {
                        StmtState::LhsThisBare => StmtState::RhsThisBare,
                        StmtState::LhsThisMember => StmtState::RhsThisMember,
                        StmtState::LhsLocal => StmtState::RhsLocal,
                        _ => StmtState::Other,
                    };
                }
                prev_is_value_end = false;
            }
            Token::Ident("this") => {
                let is_member_access = matches!(next, Some(Token::Dot | Token::LBracket));
                let is_bare_this = !is_member_access;
                let after_in = matches!(prev, Some(Token::Ident("in")));
                let near_eq =
                    matches!(prev, Some(Token::EqEq)) || matches!(next, Some(Token::EqEq));
                let is_lhs_start = at_base
                    && stmt_stack
                        .last()
                        .is_some_and(|frame| frame.state == StmtState::Start);

                if is_bare_this {
                    if container_depth > 0 {
                        return Err("Container literal cannot embed bare `this`".to_owned());
                    }
                    if after_in {
                        // `"key" in this` is a safe membership test.
                    } else if is_lhs_start && matches!(next, Some(Token::Assign)) {
                        if let Some(frame) = stmt_stack.last_mut() {
                            frame.state = StmtState::LhsThisBare;
                        }
                    } else if in_this_member_rhs {
                        return Err("Cannot assign `this` into a member of `this`".to_owned());
                    } else if in_call_paren || near_eq || in_local_rhs {
                        // Call argument (`stock::think(..., this)`), equality, or local var.
                    } else {
                        return Err("Bare `this` cannot be returned directly".to_owned());
                    }
                } else {
                    if container_depth > 0 && (in_this_rhs || container_depth > 1 || !in_local_rhs)
                    {
                        return Err(
                            "Cannot nest `this` member inside a container literal".to_owned()
                        );
                    }
                    if is_lhs_start && let Some(frame) = stmt_stack.last_mut() {
                        frame.state = StmtState::LhsThisBare;
                    }
                }
                prev_is_value_end = true;
            }
            Token::Ident(word) => {
                if at_base && let Some(frame) = stmt_stack.last_mut() {
                    match frame.state {
                        StmtState::Start => {
                            if matches!(word, "let" | "const") {
                                frame.state = StmtState::LetDecl;
                            } else if is_prefix_keyword(word) {
                                frame.state = StmtState::Other;
                            } else {
                                frame.state = StmtState::LhsLocal;
                            }
                        }
                        StmtState::LetDecl => {
                            frame.state = StmtState::LhsLocal;
                        }
                        StmtState::LhsLocal | StmtState::LhsThisBare | StmtState::LhsThisMember => {
                            if !matches!(prev, Some(Token::Dot)) {
                                frame.state = StmtState::Other;
                            }
                        }
                        StmtState::RhsLocal | StmtState::RhsThisBare | StmtState::RhsThisMember => {
                            if is_stmt_starter_keyword(word) {
                                frame.state = if matches!(word, "let" | "const") {
                                    StmtState::LetDecl
                                } else {
                                    StmtState::Other
                                };
                            }
                        }
                        StmtState::Other => {}
                    }
                }
                prev_is_value_end = !is_prefix_keyword(word);
            }
            Token::Value => {
                if at_base
                    && let Some(frame) = stmt_stack.last_mut()
                    && frame.state == StmtState::Start
                {
                    frame.state = StmtState::Other;
                }
                prev_is_value_end = true;
            }
            Token::EqEq | Token::Colon | Token::Op => {
                if at_base
                    && let Some(frame) = stmt_stack.last_mut()
                    && matches!(
                        frame.state,
                        StmtState::Start
                            | StmtState::LhsLocal
                            | StmtState::LhsThisBare
                            | StmtState::LhsThisMember
                    )
                {
                    frame.state = StmtState::Other;
                }
                prev_is_value_end = false;
            }
        }
    }

    if !delims.is_empty() {
        return Err("Unclosed delimiter in source".to_owned());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Authoritative compiled-AST structural validator (P2-CORE-1, P2-CORE-2)
// ---------------------------------------------------------------------------

/// Unwrap trivial single-expression `Expr::Stmt([Stmt::Expr(e)])` wrappers
/// produced by parenthesized expressions like `(this)` or `([this.k])`.
fn unwrap_paren_expr(mut expr: &Expr) -> &Expr {
    while let Expr::Stmt(block) = expr {
        let stmts = block.statements();
        if stmts.len() == 1
            && let Some(Stmt::Expr(inner)) = stmts.first()
        {
            expr = inner;
        } else {
            break;
        }
    }
    expr
}

fn rhs_chain_steps(expr: &Expr) -> usize {
    match expr {
        Expr::Dot(x, options, ..) | Expr::Index(x, options, ..) => {
            if options.contains(ASTFlags::BREAK) {
                1
            } else {
                1 + rhs_chain_steps(&x.rhs)
            }
        }
        _ => 0,
    }
}

/// If `expr` is `this` or a dot/index chain rooted at `this`, returns its
/// nesting depth (`0` for bare `this`, `1` for `this.k` / `this["k"]`,
/// `>= 2` for `this.a[0]` / `this.a.k` / `this["a"]["k"]`, etc.).
fn this_chain_depth(expr: &Expr) -> Option<usize> {
    match unwrap_paren_expr(expr) {
        Expr::ThisPtr(..) => Some(0),
        Expr::Dot(x, options, ..) | Expr::Index(x, options, ..) => {
            let lhs_depth = this_chain_depth(&x.lhs)?;
            if options.contains(ASTFlags::BREAK) {
                Some(lhs_depth + 1)
            } else {
                Some(lhs_depth + 1 + rhs_chain_steps(&x.rhs))
            }
        }
        _ => None,
    }
}

/// Whether `call` is a safe scalar-producing predicate (`in`, `!in`, `==`, `!=`).
///
/// `==` and `!=` are native-only operators (`op_token` set). `in` and `!in`
/// lower to a 2-arg call to `contains`, which can resolve to a user script
/// function if the script defines `fn contains(...)` — so we only treat
/// `contains` as safe when the AST defines no script function named `contains`.
fn is_safe_this_predicate_call(call: &rhai::FnCallExpr, has_script_contains: bool) -> bool {
    if matches!(
        call.op_token,
        Some(RhaiToken::EqualsTo | RhaiToken::NotEqualsTo)
    ) {
        return true;
    }
    !has_script_contains
        && call.namespace.is_empty()
        && call.name == "contains"
        && call.args.len() == 2
}

/// Whether `expr` references `this` (bare or member/method) anywhere outside a
/// safe scalar predicate (`"k" in this`, `this == x`, `this != x`).
fn expr_contains_value_this(expr: &Expr, has_script_contains: bool) -> bool {
    let mut found = false;
    let mut path = Vec::new();
    let _ = expr.walk(&mut path, &mut |nodes| {
        if let Some(ASTNode::Expr(Expr::ThisPtr(..))) = nodes.last() {
            // Allow bare `this` only as a direct operand of `in`, `==`, `!=`.
            let direct_safe_pred = nodes.len() >= 2
                && matches!(
                    nodes.get(nodes.len() - 2),
                    Some(ASTNode::Expr(Expr::FnCall(f, ..)))
                        if is_safe_this_predicate_call(f, has_script_contains)
                );
            if !direct_safe_pred {
                found = true;
                return false;
            }
        }
        true
    });
    found
}

/// Whether `expr` contains a bare `this` (i.e. `Expr::ThisPtr` NOT directly
/// accessed as `this.prop` / `this[idx]` / `this.method(...)` and NOT a direct
/// operand of `in` / `==` / `!=`).
fn expr_contains_bare_this(expr: &Expr, has_script_contains: bool) -> bool {
    let mut found = false;
    let mut path = Vec::new();
    let _ = expr.walk(&mut path, &mut |nodes| {
        if let Some(ASTNode::Expr(cur @ Expr::ThisPtr(..))) = nodes.last() {
            let parent = nodes.len().checked_sub(2).and_then(|i| nodes.get(i));
            let is_member_lhs = matches!(
                parent,
                Some(ASTNode::Expr(Expr::Dot(x, ..) | Expr::Index(x, ..)))
                    if std::ptr::eq(&x.lhs, *cur)
            );
            let is_safe_pred = matches!(
                parent,
                Some(ASTNode::Expr(Expr::FnCall(f, ..)))
                    if is_safe_this_predicate_call(f, has_script_contains)
            );
            if !is_member_lhs && !is_safe_pred {
                found = true;
                return false;
            }
        }
        true
    });
    found
}

/// Authoritative compile-time structural validator run directly over Rhai's
/// compiled [`AST`] (covering top-level statements and every function body,
/// including `switch` arms, interpolations, and optimizer-folded constants):
///
/// 1. Enforces static container-literal nesting depth `<= MAX_CONTAINER_DEPTH`
///    across `Expr::Array`, `Expr::Map`, and `Expr::DynamicConstant`.
/// 2. Rejects all `this` self-nesting paths (`this = [this]`, `this.k = [this.k]`,
///    `this.k = this`, `this.a[0] = this.a`, `this.a.k = this.b`,
///    `this.a += this.b`, `this.a += [this.a]`, `fn wrap() { this }`,
///    `fn make() { [this.k] }`, and `switch`-arm equivalents) while preserving
///    legitimate mind patterns (`this = pass_mem(this)`, `"k" in this`,
///    `this.before = this.last`, `this.hits += 1`,
///    `let at = [this.last[0], this.last[1]]`,
///    `this.marks_x[this.mark_index] = mark[0]`).
pub(crate) fn validate_ast(ast: &AST) -> Result<(), String> {
    let has_script_contains = ast.iter_functions().any(|f| f.name == "contains");
    let mut error: Option<String> = None;

    let _ = ast.walk(&mut |path| {
        let Some(&last) = path.last() else {
            return true;
        };

        let container_depth = path
            .iter()
            .filter(|node| matches!(node, ASTNode::Expr(Expr::Array(..) | Expr::Map(..))))
            .count();

        if container_depth > MAX_CONTAINER_DEPTH {
            error = Some(format!(
                "Container literal nesting exceeds maximum complexity (limit {MAX_CONTAINER_DEPTH})"
            ));
            return false;
        }

        match last {
            ASTNode::Expr(Expr::DynamicConstant(val, ..)) => {
                let allowed = MAX_CONTAINER_DEPTH.saturating_sub(container_depth);
                if exceeds_depth(val, allowed) {
                    error = Some(format!(
                        "Container literal nesting exceeds maximum complexity (limit {MAX_CONTAINER_DEPTH})"
                    ));
                    return false;
                }
            }
            ASTNode::Expr(Expr::Map(x, ..)) => {
                let allowed = MAX_CONTAINER_DEPTH.saturating_sub(container_depth);
                for val in x.1.values() {
                    if exceeds_depth(val, allowed) {
                        error = Some(format!(
                            "Container literal nesting exceeds maximum complexity (limit {MAX_CONTAINER_DEPTH})"
                        ));
                        return false;
                    }
                }
            }
            ASTNode::Stmt(Stmt::Assignment(x)) => {
                let (op_info, bin_expr) = x.as_ref();
                if let Some(lhs_depth) = this_chain_depth(&bin_expr.lhs) {
                    if lhs_depth >= 1 && expr_contains_bare_this(&bin_expr.rhs, has_script_contains)
                    {
                        error = Some("Cannot assign `this` into a member of `this`".to_owned());
                        return false;
                    }
                    // Op-assignments on `this` / `this.*` (`+=`, etc.) bypass
                    // `array_push` under `fast_operators = true`: forbid any
                    // `this`-referencing RHS (e.g. `this.a += this.b`,
                    // `this.a += [this.a]`) while keeping `this.hits += 1`.
                    if op_info.is_op_assignment()
                        && expr_contains_value_this(&bin_expr.rhs, has_script_contains)
                    {
                        error = Some(
                            "Cannot op-assign a `this`-derived value into `this`".to_owned(),
                        );
                        return false;
                    }
                    // Nested member/index writes into `this` (`this.a[0] = ...`,
                    // `this.a.k = ...`) mutate containers in place without
                    // `on_var`: forbid any `this`-derived RHS (`this.a[0] = this.a`,
                    // `this.a.k = this.b`) while allowing `this.marks_x[this.mark_index] = mark[0]`.
                    if lhs_depth >= 2
                        && expr_contains_value_this(&bin_expr.rhs, has_script_contains)
                    {
                        error = Some(
                            "Cannot assign a `this`-derived value into a nested member of `this`"
                                .to_owned(),
                        );
                        return false;
                    }
                }
            }
            ASTNode::Expr(cur @ Expr::ThisPtr(..)) => {
                let parent = path.len().checked_sub(2).and_then(|i| path.get(i).copied());
                let is_member_lhs = matches!(
                    parent,
                    Some(ASTNode::Expr(Expr::Dot(x, ..) | Expr::Index(x, ..)))
                        if std::ptr::eq(&x.lhs, cur)
                );
                let is_assign_lhs = is_inside_this_assignment_lhs(path);
                let is_in_this_assign_rhs = is_inside_this_assignment_rhs(path);
                let is_in_local_rhs = is_inside_local_assignment_rhs(path) && !is_in_this_assign_rhs;

                if !is_member_lhs {
                    // Bare `this`
                    if container_depth > 0 {
                        error = Some("Container literal cannot embed bare `this`".to_owned());
                        return false;
                    }
                    let is_safe_pred = matches!(
                        parent,
                        Some(ASTNode::Expr(Expr::FnCall(f, ..)))
                            if is_safe_this_predicate_call(f, has_script_contains)
                    );
                    let is_call_arg = matches!(
                        parent,
                        Some(
                            ASTNode::Expr(Expr::FnCall(..) | Expr::MethodCall(..))
                                | ASTNode::Stmt(Stmt::FnCall(..))
                        )
                    );
                    if is_safe_pred || is_assign_lhs {
                        // Safe `"k" in this`, `this == x`, or `this = ...` LHS.
                    } else if is_in_this_assign_rhs {
                        // Allow `this = pass_mem(this)` (bare `this` LHS, call arg RHS),
                        // refuse assigning bare `this` into `this.member` (handled above too).
                        let lhs_is_bare_this = enclosing_this_assignment_lhs_depth(path) == Some(0);
                        if !(lhs_is_bare_this && is_call_arg) {
                            error = Some("Cannot assign `this` into a member of `this`".to_owned());
                            return false;
                        }
                    } else if is_call_arg || is_in_local_rhs {
                        // Call argument (`pass_mem(this)`) or local-variable RHS (`let m = this`).
                    } else {
                        error = Some("Bare `this` cannot be returned directly".to_owned());
                        return false;
                    }
                } else {
                    // Member or method access on `this` (`this.k`, `this["k"]`, `this.m()`).
                    if container_depth > 0
                        && (is_in_this_assign_rhs || container_depth > 1 || !is_in_local_rhs)
                    {
                        error =
                            Some("Cannot nest `this` member inside a container literal".to_owned());
                        return false;
                    }
                }
            }
            _ => {}
        }

        true
    });

    match error {
        Some(msg) => Err(msg),
        None => Ok(()),
    }
}

/// Whether the node at the end of `path` lies inside the `lhs` of an enclosing
/// `Stmt::Assignment` whose `lhs` is rooted at `this`.
fn is_inside_this_assignment_lhs(path: &[ASTNode<'_>]) -> bool {
    for (idx, node) in path.iter().enumerate() {
        if let ASTNode::Stmt(Stmt::Assignment(x)) = node
            && this_chain_depth(&x.1.lhs).is_some()
            && let Some(ASTNode::Expr(child)) = path.get(idx + 1)
            && std::ptr::eq(*child, &x.1.lhs)
        {
            return true;
        }
    }
    false
}

/// Whether the node at the end of `path` lies inside the `rhs` of an enclosing
/// `Stmt::Assignment` whose `lhs` is rooted at `this`.
fn is_inside_this_assignment_rhs(path: &[ASTNode<'_>]) -> bool {
    enclosing_this_assignment_lhs_depth(path).is_some()
}

/// Returns the `lhs` `this`-chain depth of the innermost enclosing
/// `Stmt::Assignment` whose `rhs` contains the end of `path`.
fn enclosing_this_assignment_lhs_depth(path: &[ASTNode<'_>]) -> Option<usize> {
    for (idx, node) in path.iter().enumerate().rev() {
        if let ASTNode::Stmt(Stmt::Assignment(x)) = node
            && let Some(depth) = this_chain_depth(&x.1.lhs)
            && let Some(ASTNode::Expr(child)) = path.get(idx + 1)
            && std::ptr::eq(*child, &x.1.rhs)
        {
            return Some(depth);
        }
    }
    None
}

/// Whether the node at the end of `path` lies inside the RHS of a local
/// `Stmt::Var` (`let x = ...`) or a non-`this` `Stmt::Assignment` (`x = ...`).
fn is_inside_local_assignment_rhs(path: &[ASTNode<'_>]) -> bool {
    for (idx, node) in path.iter().enumerate().rev() {
        match node {
            ASTNode::Stmt(Stmt::Var(..)) => return true,
            ASTNode::Stmt(Stmt::Assignment(x)) => {
                if this_chain_depth(&x.1.lhs).is_none()
                    && let Some(ASTNode::Expr(child)) = path.get(idx + 1)
                    && std::ptr::eq(*child, &x.1.rhs)
                {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p2_core_1_validate_source_fails_closed_on_unclosed_forms_and_interp_maps() {
        let must_reject = [
            // Unclosed strings, raw strings, char literals, comments, templates, interps, delims
            "fn think(a, b) { let s = \"unterminated; }",
            "fn think(a, b) { let s = #\"unterminated; }",
            "fn think(a, b) { let c = 'a; }",
            "fn think(a, b) { /* unclosed comment }",
            "fn think(a, b) { /* outer /* inner */ }",
            "fn think(a, b) { let s = `unclosed template; }",
            "fn think(a, b) { let s = `${ 1; }",
            "fn think(a, b) { let s = [1, 2; }",
            // Exact P2-CORE-1 map-literal-in-interpolation desync payloads
            "fn think(a, b) { let s = `${ #{}; \"`\" }`; loop { this = [this]; } }",
            "fn think(a, b) { let s = `${ #{}; \"`\" }`; let x = [[[[[[[[[1]]]]]]]]]; }",
            // Verbatim backtick backslash desync
            "fn think(a, b) { let s = `\\` + \"`\"; loop { this = [this]; } }",
        ];
        for src in must_reject {
            assert!(
                validate_source(src).is_err(),
                "expected validate_source refusal for: {src}"
            );
        }

        let must_accept = [
            "fn think(a, b) { let s = `${ #{ k: 1 }; \"`\" }`; }",
            "fn think(a, b) { let s = `hello ${ `nested ${ #{ a: [1] } }` }`; }",
            "fn think(a, b) { let s = #\"raw \" quote\"#; let q = \"a\"\"b\"; }",
            "fn think(a, b) { let s = `\\${not_interp} ``escaped`` \\`; }",
        ];
        for src in must_accept {
            assert!(
                validate_source(src).is_ok(),
                "expected validate_source acceptance for: {src}"
            );
        }
    }

    #[test]
    fn p2_core_2_iterative_teardown_handles_deep_scope_and_error_runtime_on_small_stack() {
        let handle = std::thread::Builder::new()
            .name("mind-p2-core-2-teardown".to_owned())
            .stack_size(2 * 1024 * 1024)
            .spawn(|| {
                let mut deep_scope_val = Dynamic::from_int(1);
                let mut deep_err_val = Dynamic::from_int(2);
                for _ in 0..5_000 {
                    deep_scope_val = Dynamic::from_array(vec![deep_scope_val]);
                    let mut m = Map::new();
                    m.insert("k".into(), deep_err_val);
                    deep_err_val = Dynamic::from_map(m);
                }

                let mut scope = Scope::new();
                scope.push_dynamic("deep", deep_scope_val);
                drop_scope_iteratively(scope);

                let err = Box::new(EvalAltResult::ErrorInFunctionCall(
                    "inner".to_owned(),
                    String::new(),
                    Box::new(EvalAltResult::ErrorRuntime(deep_err_val, Position::NONE)),
                    Position::NONE,
                ));
                assert!(eval_error_has_deep_payload(&err));
                drop_eval_error_iteratively(err);
            });
        assert!(handle.is_ok(), "failed to spawn teardown thread");
        if let Ok(h) = handle {
            assert!(
                h.join().is_ok(),
                "iterative teardown panicked or overflowed"
            );
        }
    }
}
