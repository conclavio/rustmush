//! Container, padding, nesting, and comparator guards for `mush-script`
//! (CRIT-6, CRIT-7, HIGH-16, P2-CORE-1, P2-CORE-3).
//!
//! Mirrors `mush-mind::guards` for the synchronous builder-scripting engine:
//!
//! * **`Array::pad`** (CRIT-6): rejects `len > max_array_size` up front, treats
//!   `len <= 0` as a no-op, enforces container depth, and uses checked
//!   arithmetic before resizing.
//! * **Container nesting depth & heap-OOM bounds** (CRIT-7, P2-CORE-3):
//!   enforces [`MAX_CONTAINER_DEPTH`] and key-aware cumulative data-size
//!   limits across every container mutator/builder (`push`, `insert`, `set`,
//!   `append`, `+=`, `+`, `splice`, `mixin`, `fill_with`), every container-
//!   returning or callback method (`extract`, `split`, `for_each`, `map`,
//!   `filter`, `zip`, `reduce`, `reduce_rev`, `drain`, `retain`, `find`,
//!   `find_map`, `some`, `all`, `index_of`, `keys`, `values`, `order`,
//!   `order_desc`), equality (`==`, `!=`), formatting (`print`, `debug`,
//!   `to_string`, `to_debug`, `to_json`), and variable reads (`on_var`).
//!   Every limit violation latches a thread-local flag checked by `on_progress`
//!   and `run_one`, and unwraps system exceptions out of callback
//!   `ErrorInFunctionCall` wrappers so `try`/`catch` cannot swallow them.
//! * **Compile-time source & AST validation** (P2-CORE-1): runs both a
//!   Rhai-accurate lexer/validator (`validate_source`, rejecting unclosed
//!   strings/templates/interpolations/comments/delimiters) and an authoritative
//!   compiled-`AST` structural walk (`validate_ast`) over top-level statements
//!   and all function/closure bodies.
//! * **Iterative teardown**: `drop_dynamic_iteratively`,
//!   `drop_map_iteratively`, `drop_scope_iteratively`, and
//!   `drop_eval_error_iteratively` dismantle nested `Array`/`Map`/`Shared`/
//!   `FnPtr` graphs without call-stack recursion.
//! * **Comparator `sort`/`order`/`dedup`** (HIGH-16): replaces `slice::sort_by`
//!   and `Vec::dedup_by` (which swallow callback `ErrorTerminated` via `.ok()`
//!   / `.unwrap_or`) with fallible iterative implementations that abort
//!   immediately on the first comparator error or budget overrun.

use std::cell::Cell;
use std::cmp::Ordering;
use std::fmt::Write as _;
use std::ops::{Range, RangeInclusive};

use rhai::{
    AST, ASTNode, Array, Dynamic, Engine, EvalAltResult, Expr, FnPtr, FuncRegistration, INT,
    ImmutableString, Map, NativeCallContext, Position, Scope,
};

type ExclusiveRange = Range<INT>;
type InclusiveRange = RangeInclusive<INT>;

use crate::runtime;

/// Maximum permitted nesting depth for `Array` / `Map` / closure containers.
pub(crate) const MAX_CONTAINER_DEPTH: usize = 8;

/// Fallback formatting byte ceiling when `engine.max_string_size()` is `0`.
const FALLBACK_MAX_FORMAT_BYTES: usize = 64 * 1024;

thread_local! {
    /// Latched when any container depth- or size-limit guard fires (including
    /// inside `print_with_func` / string interpolation or callback
    /// `ErrorInFunctionCall` wrappers), preventing `try`/`catch` from
    /// continuing after an overrun.
    static LIMIT_REASON: Cell<Option<&'static str>> = const { Cell::new(None) };
}

/// Clear the limit latch at the start of a script invocation.
pub(crate) fn limit_begin() {
    LIMIT_REASON.with(|flag| flag.set(None));
}

/// Current latched limit reason, if any.
pub(crate) fn limit_reason() -> Option<&'static str> {
    LIMIT_REASON.with(Cell::get)
}

/// Take and clear the latched limit reason at the end of an invocation.
pub(crate) fn limit_take_reason() -> Option<&'static str> {
    LIMIT_REASON.with(Cell::take)
}

fn latch_limit(reason: &'static str) {
    LIMIT_REASON.with(|flag| {
        if flag.get().is_none() {
            flag.set(Some(reason));
        }
    });
}

pub(crate) fn depth_exceeded_err() -> Box<EvalAltResult> {
    latch_limit("Container nesting depth");
    Box::new(EvalAltResult::ErrorDataTooLarge(
        "Container nesting depth".to_owned(),
        Position::NONE,
    ))
}

fn size_exceeded_err(what: &'static str) -> Box<EvalAltResult> {
    latch_limit(what);
    Box::new(EvalAltResult::ErrorDataTooLarge(
        what.to_owned(),
        Position::NONE,
    ))
}

fn budget_terminated_err(reason: &'static str) -> Box<EvalAltResult> {
    Box::new(EvalAltResult::ErrorTerminated(
        Dynamic::from(reason),
        Position::NONE,
    ))
}

fn check_budget() -> Result<(), Box<EvalAltResult>> {
    if let Some(reason) = runtime::budget_tripped() {
        return Err(budget_terminated_err(reason));
    }
    Ok(())
}

/// Handle an error returned by a script callback invoked through `FnPtr`:
///
/// 1. Latch any `ErrorDataTooLarge` so `on_progress` and `run_one` enforce it.
/// 2. Unwrap `ErrorInFunctionCall` / `ErrorInModule` whenever the innermost
///    error is a system exception (`ErrorDataTooLarge`, `ErrorTerminated`,
///    `ErrorStackOverflow`, `ErrorTooManyOperations`, `ErrorTooManyVariables`,
///    `ErrorTooManyModules`, `ErrorSystem`) so `try`/`catch` cannot catch it.
fn handle_callback_err(err: Box<EvalAltResult>) -> Box<EvalAltResult> {
    if matches!(err.unwrap_inner(), EvalAltResult::ErrorDataTooLarge(..)) {
        latch_limit("Container size limit");
    }
    if err.unwrap_inner().is_system_exception() {
        let mut cur = err;
        loop {
            match *cur {
                EvalAltResult::ErrorInFunctionCall(_, _, inner, _)
                | EvalAltResult::ErrorInModule(_, inner, _) => {
                    cur = inner;
                }
                _ => return cur,
            }
        }
    }
    err
}

/// Invoke a `FnPtr` callback with Rhai's exact package-method arity-dispatch
/// rules (`call_raw_with_extra_args`) plus budget/exception hardening.
fn call_callback<const N: usize, const M: usize>(
    fn_ptr: &FnPtr,
    caller: &str,
    ctx: &NativeCallContext,
    this_ptr: Option<&mut Dynamic>,
    args: [Dynamic; N],
    extra: [Dynamic; M],
    def_value: Option<usize>,
) -> Result<Dynamic, Box<EvalAltResult>> {
    check_budget()?;
    let res = fn_ptr
        .call_raw_with_extra_args(caller, ctx, this_ptr, args, extra, def_value)
        .map_err(handle_callback_err)?;
    check_budget()?;
    Ok(res)
}

/// Register all container, padding, equality, formatting, variable-read, and
/// comparator shadows in the engine's global namespace (searched ahead of all
/// standard packages).
pub(crate) fn register(engine: &mut Engine) {
    // CRIT-6 + CRIT-7: safe `Array::pad` with checked arithmetic & depth cap.
    register_mut(engine, "pad", array_pad);

    // CRIT-7 + P2-CORE-3: container builders and mutators with pre-mutation
    // depth and key-aware cumulative size checks.
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

    // P2-CORE-3: container-returning & callback array methods with per-step
    // cumulative size/depth checks and uncatchable system-exception propagation.
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

    // CRIT-7 + P2-CORE-3: guard literal/index-assignment nesting, closure
    // capture chains/cycles, and key-size bloat (`m[big_key] = 1`) on every
    // variable read.
    #[allow(deprecated)]
    engine.on_var(|name, _index, ctx| {
        check_budget()?;
        if let Some(val) = ctx.scope().get(name)
            && (val.is_array() || val.is_map() || val.is_fnptr() || val.is_shared())
        {
            check_value_limits(ctx.engine(), val)?;
        }
        if let Some(this_val) = ctx.this_ptr()
            && (this_val.is_array()
                || this_val.is_map()
                || this_val.is_fnptr()
                || this_val.is_shared())
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
    register_pure(engine, "order", array_order_builtin);
    register_pure(engine, "order_desc", array_order_builtin_desc);
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

/// Side-effect-free depth predicate (does not latch `LIMIT_REASON`).
///
/// Used for compile-time AST validation and safe error formatting so pure
/// inspection never mutates thread-local execution state.
pub(crate) fn exceeds_depth(value: &Dynamic, max_depth: usize) -> bool {
    if value.is_shared() && value.read_lock::<Dynamic>().is_none() {
        return true;
    }
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
    if let Some(fp) = value.read_lock::<FnPtr>() {
        let curry = fp.curry();
        if !curry.is_empty() {
            if max_depth == 0 {
                return true;
            }
            return curry.iter().any(|arg| exceeds_depth(arg, max_depth - 1));
        }
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

fn check_depth(value: &Dynamic, max_depth: usize) -> Result<(), Box<EvalAltResult>> {
    if value.is_shared() && value.read_lock::<Dynamic>().is_none() {
        return Err(depth_exceeded_err());
    }
    if let Some(arr) = value.read_lock::<Array>() {
        return check_array_depth(&arr, max_depth);
    }
    if let Some(map) = value.read_lock::<Map>() {
        return check_map_depth(&map, max_depth);
    }
    if let Some(fp) = value.read_lock::<FnPtr>() {
        let curry = fp.curry();
        if !curry.is_empty() {
            if max_depth == 0 {
                return Err(depth_exceeded_err());
            }
            for arg in curry {
                check_depth(arg, max_depth - 1)?;
            }
        }
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
/// accounting for map key bytes as well as values and closure captures.
fn calc_sizes_bounded(
    value: &Dynamic,
    max_depth: usize,
) -> Result<(usize, usize, usize), Box<EvalAltResult>> {
    if value.is_shared() && value.read_lock::<Dynamic>().is_none() {
        return Err(depth_exceeded_err());
    }
    if let Some(arr) = value.read_lock::<Array>() {
        return calc_array_sizes_bounded(&arr, max_depth);
    }
    if let Some(map) = value.read_lock::<Map>() {
        return calc_map_sizes_bounded(&map, max_depth);
    }
    if let Some(fp) = value.read_lock::<FnPtr>() {
        let curry = fp.curry();
        if !curry.is_empty() {
            if max_depth == 0 {
                return Err(depth_exceeded_err());
            }
            let mut total = (0usize, 0usize, 0usize);
            for arg in curry {
                let sz = calc_sizes_bounded(arg, max_depth - 1)?;
                total = add_sizes(total, sz)?;
            }
            return Ok(total);
        }
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

fn check_sizes(engine: &Engine, sizes: (usize, usize, usize)) -> Result<(), Box<EvalAltResult>> {
    let (arr, map, str_len) = sizes;
    let max_arr = engine.max_array_size();
    if max_arr > 0 && arr > max_arr {
        return Err(size_exceeded_err("Size of array/BLOB"));
    }
    let max_map = engine.max_map_size();
    if max_map > 0 && map > max_map {
        return Err(size_exceeded_err("Size of object map"));
    }
    let max_str = engine.max_string_size();
    if max_str > 0 && str_len > max_str {
        return Err(size_exceeded_err("Length of string"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// CRIT-6 + CRIT-7 + P2-CORE-3: Guarded container mutators & builders
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
        return Err(size_exceeded_err("Size of array"));
    };
    let engine = ctx.engine();
    let max_arr = engine.max_array_size();
    if max_arr > 0 && target_len > max_arr {
        return Err(size_exceeded_err("Size of array"));
    }
    if target_len <= array.len() {
        return Ok(());
    }

    let current = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    let per_item = calc_sizes_bounded(&item, MAX_CONTAINER_DEPTH - 1)?;
    let added = target_len - array.len();

    let per_slot_arr = per_item
        .0
        .checked_add(1)
        .ok_or_else(|| size_exceeded_err("Size of array/BLOB"))?;
    let add_arr = added
        .checked_mul(per_slot_arr)
        .ok_or_else(|| size_exceeded_err("Size of array/BLOB"))?;
    let add_map = added
        .checked_mul(per_item.1)
        .ok_or_else(|| size_exceeded_err("Size of object map"))?;
    let add_str = added
        .checked_mul(per_item.2)
        .ok_or_else(|| size_exceeded_err("Length of string"))?;

    let final_sizes = add_sizes(current, (add_arr, add_map, add_str))?;
    check_sizes(engine, final_sizes)?;

    array.resize(target_len, item);
    Ok(())
}

fn array_push(
    ctx: NativeCallContext,
    array: &mut Array,
    item: Dynamic,
) -> Result<(), Box<EvalAltResult>> {
    check_budget()?;
    let current = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    let elem = calc_sizes_bounded(&item, MAX_CONTAINER_DEPTH - 1)?;
    let next = add_sizes(add_sizes(current, elem)?, (1, 0, 0))?;
    check_sizes(ctx.engine(), next)?;
    array.push(item);
    Ok(())
}

fn array_insert(
    ctx: NativeCallContext,
    array: &mut Array,
    position: INT,
    item: Dynamic,
) -> Result<(), Box<EvalAltResult>> {
    check_budget()?;
    let current = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    let elem = calc_sizes_bounded(&item, MAX_CONTAINER_DEPTH - 1)?;
    let next = add_sizes(add_sizes(current, elem)?, (1, 0, 0))?;
    check_sizes(ctx.engine(), next)?;

    let len = array.len();
    let idx = if position < 0 {
        let abs = position.unsigned_abs();
        usize::try_from(abs).map_or(0, |n| len.saturating_sub(n))
    } else {
        usize::try_from(position).map_or(len, |n| n.min(len))
    };
    array.insert(idx, item);
    Ok(())
}

fn array_set(
    ctx: NativeCallContext,
    array: &mut Array,
    index: INT,
    value: Dynamic,
) -> Result<(), Box<EvalAltResult>> {
    check_budget()?;
    let current = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    let new_elem = calc_sizes_bounded(&value, MAX_CONTAINER_DEPTH - 1)?;
    let len = array.len();
    let target_idx = if index < 0 {
        let abs = index.unsigned_abs();
        usize::try_from(abs).ok().and_then(|n| len.checked_sub(n))
    } else {
        usize::try_from(index).ok().filter(|&n| n < len)
    };
    let Some(idx) = target_idx else {
        return Ok(());
    };
    if let Some(slot) = array.get(idx) {
        let old_elem = calc_sizes_bounded(slot, MAX_CONTAINER_DEPTH - 1)?;
        let next = add_sizes(sub_sizes(current, old_elem), new_elem)?;
        check_sizes(ctx.engine(), next)?;
    }
    if let Some(slot) = array.get_mut(idx) {
        *slot = value;
    }
    Ok(())
}

fn array_append(
    ctx: NativeCallContext,
    array: &mut Array,
    mut other: Array,
) -> Result<(), Box<EvalAltResult>> {
    check_budget()?;
    if other.is_empty() {
        return Ok(());
    }
    let lhs = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    let rhs = calc_array_sizes_bounded(&other, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), add_sizes(lhs, rhs)?)?;
    array.append(&mut other);
    Ok(())
}

fn array_concat(
    ctx: NativeCallContext,
    mut array1: Array,
    mut array2: Array,
) -> Result<Array, Box<EvalAltResult>> {
    check_budget()?;
    let lhs = calc_array_sizes_bounded(&array1, MAX_CONTAINER_DEPTH)?;
    let rhs = calc_array_sizes_bounded(&array2, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), add_sizes(lhs, rhs)?)?;
    if array2.is_empty() {
        return Ok(array1);
    }
    if array1.is_empty() {
        return Ok(array2);
    }
    array1.append(&mut array2);
    Ok(array1)
}

fn calc_offset_len(total: usize, start: INT, len: INT) -> (usize, usize) {
    if len <= 0 || total == 0 {
        return (0, 0);
    }
    let start_idx = if start < 0 {
        let abs = start.unsigned_abs();
        usize::try_from(abs).map_or(0, |n| total.saturating_sub(n))
    } else {
        usize::try_from(start).map_or(total, |n| n.min(total))
    };
    let take_len = usize::try_from(len).unwrap_or(usize::MAX);
    let clamped_len = take_len.min(total.saturating_sub(start_idx));
    (start_idx, clamped_len)
}

fn array_splice(
    ctx: NativeCallContext,
    array: &mut Array,
    start: INT,
    len: INT,
    replace: Array,
) -> Result<(), Box<EvalAltResult>> {
    check_budget()?;
    let cur = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    let rep = calc_array_sizes_bounded(&replace, MAX_CONTAINER_DEPTH)?;
    let (start_idx, remove_len) = calc_offset_len(array.len(), start, len);
    let mut removed = (0usize, 0usize, 0usize);
    if let Some(slice) = array.get(start_idx..start_idx.saturating_add(remove_len)) {
        for item in slice {
            let elem = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
            removed = add_sizes(add_sizes(removed, elem)?, (1, 0, 0))?;
        }
    }
    let final_sizes = add_sizes(sub_sizes(cur, removed), rep)?;
    check_sizes(ctx.engine(), final_sizes)?;

    let end_idx = start_idx.saturating_add(remove_len);
    let _ = array.splice(start_idx..end_idx, replace);
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
    let len = end.saturating_sub(start);
    array_splice(ctx, array, start, len, replace)
}

fn array_splice_inclusive_range(
    ctx: NativeCallContext,
    array: &mut Array,
    range: InclusiveRange,
    replace: Array,
) -> Result<(), Box<EvalAltResult>> {
    let start = (*range.start()).max(0);
    let end = (*range.end()).max(start).min(INT::MAX - 1);
    let len = end.saturating_sub(start).saturating_add(1);
    array_splice(ctx, array, start, len, replace)
}

fn map_set(
    ctx: NativeCallContext,
    map: &mut Map,
    key: &str,
    value: Dynamic,
) -> Result<(), Box<EvalAltResult>> {
    check_budget()?;
    let current = calc_map_sizes_bounded(map, MAX_CONTAINER_DEPTH)?;
    let new_val = calc_sizes_bounded(&value, MAX_CONTAINER_DEPTH - 1)?;
    let next = if let Some(existing) = map.get(key) {
        let old_val = calc_sizes_bounded(existing, MAX_CONTAINER_DEPTH - 1)?;
        add_sizes(sub_sizes(current, old_val), new_val)?
    } else {
        add_sizes(add_sizes(current, new_val)?, (0, 1, key.len()))?
    };
    check_sizes(ctx.engine(), next)?;
    map.insert(key.into(), value);
    Ok(())
}

fn calc_map_union_sizes(
    base: &Map,
    overlay: &Map,
    overwrite: bool,
) -> Result<(usize, usize, usize), Box<EvalAltResult>> {
    let mut total = calc_map_sizes_bounded(base, MAX_CONTAINER_DEPTH)?;
    for (key, val) in overlay {
        let new_val = calc_sizes_bounded(val, MAX_CONTAINER_DEPTH - 1)?;
        if let Some(existing) = base.get(key.as_str()) {
            if overwrite {
                let old_val = calc_sizes_bounded(existing, MAX_CONTAINER_DEPTH - 1)?;
                total = add_sizes(sub_sizes(total, old_val), new_val)?;
            }
        } else {
            total = add_sizes(add_sizes(total, new_val)?, (0, 1, key.len()))?;
        }
    }
    Ok(total)
}

fn map_mixin(ctx: NativeCallContext, map: &mut Map, other: Map) -> Result<(), Box<EvalAltResult>> {
    check_budget()?;
    if other.is_empty() {
        return Ok(());
    }
    let final_sizes = calc_map_union_sizes(map, &other, true)?;
    check_sizes(ctx.engine(), final_sizes)?;
    map.extend(other);
    Ok(())
}

fn map_merge(ctx: NativeCallContext, mut map1: Map, map2: Map) -> Result<Map, Box<EvalAltResult>> {
    check_budget()?;
    let final_sizes = calc_map_union_sizes(&map1, &map2, true)?;
    check_sizes(ctx.engine(), final_sizes)?;
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
    other: Map,
) -> Result<(), Box<EvalAltResult>> {
    check_budget()?;
    if other.is_empty() {
        return Ok(());
    }
    let final_sizes = calc_map_union_sizes(map, &other, false)?;
    check_sizes(ctx.engine(), final_sizes)?;
    for (k, v) in other {
        map.entry(k).or_insert(v);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// P2-CORE-3: Guarded BasicArrayPackage & BasicMapPackage container methods
// ---------------------------------------------------------------------------

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
        .map(<[Dynamic]>::to_vec)
        .unwrap_or_default())
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
    start: INT,
) -> Result<Array, Box<EvalAltResult>> {
    check_budget()?;
    let sizes = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), sizes)?;
    if array.is_empty() {
        return Ok(Array::new());
    }
    let len = array.len();
    let idx = if start < 0 {
        let abs = start.unsigned_abs();
        usize::try_from(abs).map_or(0, |n| len.saturating_sub(n))
    } else {
        usize::try_from(start).map_or(len, |n| n.min(len))
    };
    if idx >= len {
        return Ok(Array::new());
    }
    Ok(array.split_off(idx))
}

fn array_for_each(
    ctx: NativeCallContext,
    array: &mut Array,
    mapper: FnPtr,
) -> Result<(), Box<EvalAltResult>> {
    check_budget()?;
    let mut total = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), total)?;
    for (i, item) in array.iter_mut().enumerate() {
        let before = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        let ex = [INT::try_from(i).unwrap_or(INT::MAX).into()];
        let _ = call_callback(&mapper, "for_each", &ctx, Some(item), [], ex, Some(0))?;
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
    let mut out = Array::with_capacity(array.len());
    let mut out_total = (0usize, 0usize, 0usize);
    for (i, item) in array.iter_mut().enumerate() {
        let before = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        let ex = [INT::try_from(i).unwrap_or(INT::MAX).into()];
        let mapped = call_callback(&mapper, "map", &ctx, Some(item), [], ex, Some(0))?;
        let after = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        src_total = add_sizes(sub_sizes(src_total, before), after)?;
        check_sizes(ctx.engine(), src_total)?;
        let mapped_sz = calc_sizes_bounded(&mapped, MAX_CONTAINER_DEPTH - 1)?;
        out_total = add_sizes(add_sizes(out_total, mapped_sz)?, (1, 0, 0))?;
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
    let mut src_total = calc_array_sizes_bounded(array1, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), src_total)?;
    let rhs_total = calc_array_sizes_bounded(&array2, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), rhs_total)?;
    if array1.is_empty() || array2.is_empty() {
        return Ok(Array::new());
    }
    let cap = array1.len().min(array2.len());
    let mut out = Array::with_capacity(cap);
    let mut out_total = (0usize, 0usize, 0usize);
    for (i, (item1, item2)) in array1.iter_mut().zip(array2).enumerate() {
        let before = calc_sizes_bounded(item1, MAX_CONTAINER_DEPTH - 1)?;
        let ex = [INT::try_from(i).unwrap_or(INT::MAX).into()];
        let mapped = call_callback(&mapper, "zip", &ctx, Some(item1), [item2], ex, Some(1))?;
        let after = calc_sizes_bounded(item1, MAX_CONTAINER_DEPTH - 1)?;
        src_total = add_sizes(sub_sizes(src_total, before), after)?;
        check_sizes(ctx.engine(), src_total)?;
        let mapped_sz = calc_sizes_bounded(&mapped, MAX_CONTAINER_DEPTH - 1)?;
        out_total = add_sizes(add_sizes(out_total, mapped_sz)?, (1, 0, 0))?;
        check_sizes(ctx.engine(), out_total)?;
        out.push(mapped);
    }
    Ok(out)
}

fn array_reduce_with_initial(
    ctx: NativeCallContext,
    array: &mut Array,
    reducer: FnPtr,
    initial_value: Dynamic,
) -> Result<Dynamic, Box<EvalAltResult>> {
    check_budget()?;
    let mut src_total = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), src_total)?;
    check_value_limits(ctx.engine(), &initial_value)?;
    let mut result = initial_value;
    for (i, item) in array.iter_mut().enumerate() {
        let before = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        let ex = [INT::try_from(i).unwrap_or(INT::MAX).into()];
        result = call_callback(&reducer, "reduce", &ctx, Some(item), [result], ex, Some(1))?;
        let after = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        src_total = add_sizes(sub_sizes(src_total, before), after)?;
        check_sizes(ctx.engine(), src_total)?;
        check_value_limits(ctx.engine(), &result)?;
    }
    Ok(result)
}

fn array_reduce(
    ctx: NativeCallContext,
    array: &mut Array,
    reducer: FnPtr,
) -> Result<Dynamic, Box<EvalAltResult>> {
    array_reduce_with_initial(ctx, array, reducer, Dynamic::UNIT)
}

fn array_reduce_rev_with_initial(
    ctx: NativeCallContext,
    array: &mut Array,
    reducer: FnPtr,
    initial_value: Dynamic,
) -> Result<Dynamic, Box<EvalAltResult>> {
    check_budget()?;
    let mut src_total = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), src_total)?;
    check_value_limits(ctx.engine(), &initial_value)?;
    let mut result = initial_value;
    let len = array.len();
    for (i, item) in array.iter_mut().rev().enumerate() {
        let before = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        let idx = len.saturating_sub(1).saturating_sub(i);
        let ex = [INT::try_from(idx).unwrap_or(INT::MAX).into()];
        result = call_callback(
            &reducer,
            "reduce_rev",
            &ctx,
            Some(item),
            [result],
            ex,
            Some(1),
        )?;
        let after = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        src_total = add_sizes(sub_sizes(src_total, before), after)?;
        check_sizes(ctx.engine(), src_total)?;
        check_value_limits(ctx.engine(), &result)?;
    }
    Ok(result)
}

fn array_reduce_rev(
    ctx: NativeCallContext,
    array: &mut Array,
    reducer: FnPtr,
) -> Result<Dynamic, Box<EvalAltResult>> {
    array_reduce_rev_with_initial(ctx, array, reducer, Dynamic::UNIT)
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
        let remove = call_callback(&filter, "drain", &ctx, Some(item), [], ex, Some(0))?
            .as_bool()
            .unwrap_or(false);
        let after = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        total = add_sizes(sub_sizes(total, before), after)?;
        check_sizes(ctx.engine(), total)?;
        if remove {
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

fn start_index_for_search(len: usize, start: INT) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let idx = if start < 0 {
        let abs = start.unsigned_abs();
        usize::try_from(abs).map_or(0, |n| len.saturating_sub(n))
    } else {
        usize::try_from(start).ok()?
    };
    (idx < len).then_some(idx)
}

fn array_find_starting_from(
    ctx: NativeCallContext,
    array: &mut Array,
    filter: FnPtr,
    start: INT,
) -> Result<Dynamic, Box<EvalAltResult>> {
    check_budget()?;
    let mut src_total = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), src_total)?;
    let Some(start_idx) = start_index_for_search(array.len(), start) else {
        return Ok(Dynamic::UNIT);
    };
    for (offset, item) in array.iter_mut().skip(start_idx).enumerate() {
        let before = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        let ex = [INT::try_from(start_idx.saturating_add(offset))
            .unwrap_or(INT::MAX)
            .into()];
        let matched = call_callback(&filter, "find", &ctx, Some(item), [], ex, Some(0))?
            .as_bool()
            .unwrap_or(false);
        let after = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        src_total = add_sizes(sub_sizes(src_total, before), after)?;
        check_sizes(ctx.engine(), src_total)?;
        if matched {
            return Ok(item.clone());
        }
    }
    Ok(Dynamic::UNIT)
}

fn array_find(
    ctx: NativeCallContext,
    array: &mut Array,
    filter: FnPtr,
) -> Result<Dynamic, Box<EvalAltResult>> {
    array_find_starting_from(ctx, array, filter, 0)
}

fn array_find_map_starting_from(
    ctx: NativeCallContext,
    array: &mut Array,
    filter: FnPtr,
    start: INT,
) -> Result<Dynamic, Box<EvalAltResult>> {
    check_budget()?;
    let mut src_total = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), src_total)?;
    let Some(start_idx) = start_index_for_search(array.len(), start) else {
        return Ok(Dynamic::UNIT);
    };
    for (offset, item) in array.iter_mut().skip(start_idx).enumerate() {
        let before = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        let ex = [INT::try_from(start_idx.saturating_add(offset))
            .unwrap_or(INT::MAX)
            .into()];
        let mapped = call_callback(&filter, "find_map", &ctx, Some(item), [], ex, Some(0))?;
        let after = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        src_total = add_sizes(sub_sizes(src_total, before), after)?;
        check_sizes(ctx.engine(), src_total)?;
        if !mapped.is_unit() {
            check_value_limits(ctx.engine(), &mapped)?;
            return Ok(mapped);
        }
    }
    Ok(Dynamic::UNIT)
}

fn array_find_map(
    ctx: NativeCallContext,
    array: &mut Array,
    filter: FnPtr,
) -> Result<Dynamic, Box<EvalAltResult>> {
    array_find_map_starting_from(ctx, array, filter, 0)
}

fn array_some(
    ctx: NativeCallContext,
    array: &mut Array,
    filter: FnPtr,
) -> Result<bool, Box<EvalAltResult>> {
    check_budget()?;
    let mut src_total = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), src_total)?;
    for (i, item) in array.iter_mut().enumerate() {
        let before = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        let ex = [INT::try_from(i).unwrap_or(INT::MAX).into()];
        let matched = call_callback(&filter, "some", &ctx, Some(item), [], ex, Some(0))?
            .as_bool()
            .unwrap_or(false);
        let after = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        src_total = add_sizes(sub_sizes(src_total, before), after)?;
        check_sizes(ctx.engine(), src_total)?;
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
    let mut src_total = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), src_total)?;
    for (i, item) in array.iter_mut().enumerate() {
        let before = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        let ex = [INT::try_from(i).unwrap_or(INT::MAX).into()];
        let matched = call_callback(&filter, "all", &ctx, Some(item), [], ex, Some(0))?
            .as_bool()
            .unwrap_or(false);
        let after = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        src_total = add_sizes(sub_sizes(src_total, before), after)?;
        check_sizes(ctx.engine(), src_total)?;
        if !matched {
            return Ok(false);
        }
    }
    Ok(true)
}

fn array_index_of_filter_starting_from(
    ctx: NativeCallContext,
    array: &mut Array,
    filter: FnPtr,
    start: INT,
) -> Result<INT, Box<EvalAltResult>> {
    check_budget()?;
    let mut src_total = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), src_total)?;
    let Some(start_idx) = start_index_for_search(array.len(), start) else {
        return Ok(-1);
    };
    for (offset, item) in array.iter_mut().skip(start_idx).enumerate() {
        let before = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        let idx = start_idx.saturating_add(offset);
        let ex = [INT::try_from(idx).unwrap_or(INT::MAX).into()];
        let matched = call_callback(&filter, "index_of", &ctx, Some(item), [], ex, Some(0))?
            .as_bool()
            .unwrap_or(false);
        let after = calc_sizes_bounded(item, MAX_CONTAINER_DEPTH - 1)?;
        src_total = add_sizes(sub_sizes(src_total, before), after)?;
        check_sizes(ctx.engine(), src_total)?;
        if matched {
            return Ok(INT::try_from(idx).unwrap_or(INT::MAX));
        }
    }
    Ok(-1)
}

fn array_index_of_filter(
    ctx: NativeCallContext,
    array: &mut Array,
    filter: FnPtr,
) -> Result<INT, Box<EvalAltResult>> {
    array_index_of_filter_starting_from(ctx, array, filter, 0)
}

fn array_order_builtin(
    ctx: NativeCallContext,
    array: &mut Array,
) -> Result<Array, Box<EvalAltResult>> {
    check_budget()?;
    let sizes = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), sizes)?;
    let mut copy = array.clone();
    let mut dyn_copy = Dynamic::from_array(copy);
    let _ = ctx.call_native_fn_raw("sort", false, &mut [&mut dyn_copy])?;
    copy = dyn_copy.try_cast::<Array>().unwrap_or_default();
    Ok(copy)
}

fn array_order_builtin_desc(
    ctx: NativeCallContext,
    array: &mut Array,
) -> Result<Array, Box<EvalAltResult>> {
    check_budget()?;
    let sizes = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), sizes)?;
    let mut copy = array.clone();
    let mut dyn_copy = Dynamic::from_array(copy);
    let _ = ctx.call_native_fn_raw("sort_desc", false, &mut [&mut dyn_copy])?;
    copy = dyn_copy.try_cast::<Array>().unwrap_or_default();
    Ok(copy)
}

fn map_keys(ctx: NativeCallContext, map: &mut Map) -> Result<Array, Box<EvalAltResult>> {
    check_budget()?;
    let sizes = calc_map_sizes_bounded(map, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), sizes)?;
    let mut out = Array::with_capacity(map.len());
    let mut out_total = (0usize, 0usize, 0usize);
    for key in map.keys() {
        out_total = add_sizes(out_total, (1, 0, key.len()))?;
        check_sizes(ctx.engine(), out_total)?;
        out.push(key.clone().into());
    }
    Ok(out)
}

fn map_values(ctx: NativeCallContext, map: &mut Map) -> Result<Array, Box<EvalAltResult>> {
    check_budget()?;
    let sizes = calc_map_sizes_bounded(map, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), sizes)?;
    let mut out = Array::with_capacity(map.len());
    let mut out_total = (0usize, 0usize, 0usize);
    for val in map.values() {
        let val_sz = calc_sizes_bounded(val, MAX_CONTAINER_DEPTH - 1)?;
        out_total = add_sizes(add_sizes(out_total, val_sz)?, (1, 0, 0))?;
        check_sizes(ctx.engine(), out_total)?;
        out.push(val.clone());
    }
    Ok(out)
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
        let mapped_sz = calc_sizes_bounded(&mapped, MAX_CONTAINER_DEPTH - 1)?;
        out_total = add_sizes(add_sizes(out_total, mapped_sz)?, (0, 1, key.len()))?;
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

/// Depth- and size-bounded `to_string` / `to_debug` for `Array`.
///
/// Latches the thread-local limit flag and returns an empty string on limit
/// violation so Rhai's `print_with_func` cannot swallow `Err` and fall back
/// to unbounded recursive `Debug::fmt`.
fn to_string_array(
    ctx: NativeCallContext,
    array: &mut Array,
) -> Result<ImmutableString, Box<EvalAltResult>> {
    let mut buf = BoundedBuf::new(ctx.engine());
    match format_array_into(&ctx, &mut buf, array, 1) {
        Ok(()) => Ok(buf.out.into()),
        Err(err) if matches!(*err, EvalAltResult::ErrorDataTooLarge(..)) => {
            latch_limit("Container formatting limit");
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
            latch_limit("Container formatting limit");
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
// CRIT-7 + P2-CORE-3: Iterative teardown
// ---------------------------------------------------------------------------

/// Iteratively dismantle any nested `Array`/`Map`/`Shared`/`FnPtr` graph
/// inside `root` so dropping `Scope` variables or error payloads never
/// overflows the call stack or leaks shared closure cycles.
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
// P2-CORE-1: Compile-time source lexer/validator + authoritative AST validator
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
    Value,
    InterpStart,
    InterpEnd,
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LexMode {
    Normal,
    TemplateString,
    Interp { brace_depth: usize },
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn byte_at(bytes: &[u8], i: usize) -> Option<u8> {
    bytes.get(i).copied()
}

/// Skip a Rhai standard string (`"..."`) or char (`'...'`) literal starting
/// right after the opening quote. Mirrors Rhai's tokenizer: handles `\x`
/// escapes and doubled delimiters (`""` / `''`), and fails closed on EOF.
fn skip_quoted_literal(bytes: &[u8], mut i: usize, quote: u8) -> Result<usize, String> {
    while let Some(b) = byte_at(bytes, i) {
        if b == b'\\' {
            i = i.saturating_add(1);
            let Some(_) = byte_at(bytes, i) else {
                return Err("Unterminated escape in string or char literal".to_owned());
            };
            i = i.saturating_add(1);
            continue;
        }
        if b == quote {
            if byte_at(bytes, i + 1) == Some(quote) {
                i = i.saturating_add(2);
                continue;
            }
            return Ok(i.saturating_add(1));
        }
        i = i.saturating_add(1);
    }
    Err("Unterminated string or char literal".to_owned())
}

/// Skip a Rhai raw string literal (`#"..."#`, `##"..."##`, etc.). `i` points
/// to the first `#`. Returns `Ok(Some(next_i))` if a raw string starts here,
/// `Ok(None)` if this `#` does not begin a raw string, or `Err` if unclosed.
fn try_skip_raw_string(bytes: &[u8], i: usize) -> Result<Option<usize>, String> {
    let mut hashes = 0usize;
    while byte_at(bytes, i.saturating_add(hashes)) == Some(b'#') {
        hashes += 1;
    }
    if hashes == 0 || byte_at(bytes, i.saturating_add(hashes)) != Some(b'"') {
        return Ok(None);
    }
    let mut j = i.saturating_add(hashes).saturating_add(1);
    while let Some(b) = byte_at(bytes, j) {
        if b == b'"' {
            let mut matched = 0usize;
            while matched < hashes
                && byte_at(bytes, j.saturating_add(1).saturating_add(matched)) == Some(b'#')
            {
                matched += 1;
            }
            if matched == hashes {
                return Ok(Some(j.saturating_add(1).saturating_add(hashes)));
            }
        }
        j = j.saturating_add(1);
    }
    Err("Unterminated raw string literal".to_owned())
}

/// Skip a nested Rhai block comment (`/* ... */`). `i` points to the opening
/// `/`. Fails closed if the comment reaches EOF unclosed.
fn skip_block_comment(bytes: &[u8], mut i: usize) -> Result<usize, String> {
    i = i.saturating_add(2);
    let mut depth = 1usize;
    while depth > 0 {
        let Some(b) = byte_at(bytes, i) else {
            return Err("Unterminated block comment".to_owned());
        };
        if b == b'/' && byte_at(bytes, i + 1) == Some(b'*') {
            depth += 1;
            i = i.saturating_add(2);
        } else if b == b'*' && byte_at(bytes, i + 1) == Some(b'/') {
            depth -= 1;
            i = i.saturating_add(2);
        } else {
            i = i.saturating_add(1);
        }
    }
    Ok(i)
}

/// Tokenize Rhai source into structural tokens matching Rhai's lexer, failing
/// closed on any unterminated string, raw string, char, block comment,
/// template string, or `${...}` interpolation (P2-CORE-1).
fn lex_tokens(source: &str) -> Result<Vec<Token<'_>>, String> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut modes = vec![LexMode::Normal];
    let mut i = 0usize;

    while let Some(b) = byte_at(bytes, i) {
        let mode = modes.last().copied().unwrap_or(LexMode::Normal);
        match mode {
            LexMode::TemplateString => {
                // Rhai interpolated backtick strings are verbatim: `\` only
                // escapes `${` (`\${`), NOT backticks; `` `` `` is an escaped
                // backtick.
                if b == b'\\'
                    && byte_at(bytes, i + 1) == Some(b'$')
                    && byte_at(bytes, i + 2) == Some(b'{')
                {
                    i = i.saturating_add(3);
                    continue;
                }
                if b == b'`' {
                    if byte_at(bytes, i + 1) == Some(b'`') {
                        i = i.saturating_add(2);
                        continue;
                    }
                    modes.pop();
                    tokens.push(Token::Value);
                    i = i.saturating_add(1);
                    continue;
                }
                if b == b'$' && byte_at(bytes, i + 1) == Some(b'{') {
                    modes.push(LexMode::Interp { brace_depth: 0 });
                    tokens.push(Token::InterpStart);
                    i = i.saturating_add(2);
                    continue;
                }
                i = i.saturating_add(1);
                continue;
            }
            LexMode::Normal | LexMode::Interp { .. } => {
                if b.is_ascii_whitespace() {
                    i = i.saturating_add(1);
                    continue;
                }
                if b == b'/' && byte_at(bytes, i + 1) == Some(b'/') {
                    i = i.saturating_add(2);
                    while let Some(c) = byte_at(bytes, i) {
                        i = i.saturating_add(1);
                        if c == b'\n' {
                            break;
                        }
                    }
                    continue;
                }
                if b == b'/' && byte_at(bytes, i + 1) == Some(b'*') {
                    i = skip_block_comment(bytes, i)?;
                    continue;
                }
                if b == b'#' {
                    if let Some(next_i) = try_skip_raw_string(bytes, i)? {
                        tokens.push(Token::Value);
                        i = next_i;
                        continue;
                    }
                    if byte_at(bytes, i + 1) == Some(b'{') {
                        // P2-CORE-1: `#{` opens a brace pair inside `${...}`
                        // interpolation just like `{`!
                        if let Some(LexMode::Interp { brace_depth }) = modes.last_mut() {
                            *brace_depth += 1;
                        }
                        tokens.push(Token::HashLBrace);
                        i = i.saturating_add(2);
                        continue;
                    }
                }
                if b == b'"' || b == b'\'' {
                    i = skip_quoted_literal(bytes, i.saturating_add(1), b)?;
                    tokens.push(Token::Value);
                    continue;
                }
                if b == b'`' {
                    modes.push(LexMode::TemplateString);
                    i = i.saturating_add(1);
                    continue;
                }
                if is_ident_start(b) {
                    let start = i;
                    i = i.saturating_add(1);
                    while byte_at(bytes, i).is_some_and(is_ident_continue) {
                        i = i.saturating_add(1);
                    }
                    if let Some(ident) = source.get(start..i) {
                        tokens.push(Token::Ident(ident));
                    }
                    continue;
                }
                if b.is_ascii_digit() {
                    i = i.saturating_add(1);
                    while byte_at(bytes, i)
                        .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'.')
                    {
                        i = i.saturating_add(1);
                    }
                    tokens.push(Token::Value);
                    continue;
                }

                match b {
                    b'[' => tokens.push(Token::LBracket),
                    b']' => tokens.push(Token::RBracket),
                    b'(' => tokens.push(Token::LParen),
                    b')' => tokens.push(Token::RParen),
                    b'{' => {
                        if let Some(LexMode::Interp { brace_depth }) = modes.last_mut() {
                            *brace_depth += 1;
                        }
                        tokens.push(Token::LBrace);
                    }
                    b'}' => {
                        if let Some(LexMode::Interp { brace_depth }) = modes.last_mut() {
                            if *brace_depth == 0 {
                                modes.pop();
                                tokens.push(Token::InterpEnd);
                                i = i.saturating_add(1);
                                continue;
                            }
                            *brace_depth -= 1;
                        }
                        tokens.push(Token::RBrace);
                    }
                    _ => tokens.push(Token::Other),
                }
                i = i.saturating_add(1);
            }
        }
    }

    if modes.len() != 1 || modes.first() != Some(&LexMode::Normal) {
        return Err("Unterminated template string or interpolation".to_owned());
    }

    Ok(tokens)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DelimKind {
    ArrayLiteral,
    IndexBracket,
    MapLiteral,
    Block,
    Paren,
    Interp,
}

impl DelimKind {
    fn is_container(self) -> bool {
        matches!(self, Self::ArrayLiteral | Self::MapLiteral)
    }
}

fn is_prefix_keyword(word: &str) -> bool {
    matches!(
        word,
        "return"
            | "throw"
            | "let"
            | "const"
            | "if"
            | "else"
            | "while"
            | "for"
            | "in"
            | "switch"
            | "break"
            | "continue"
            | "try"
            | "catch"
            | "loop"
            | "do"
            | "until"
            | "fn"
            | "private"
    )
}

/// Validate compiled script source against static container-literal nesting
/// deeper than [`MAX_CONTAINER_DEPTH`] (defense-in-depth alongside
/// [`validate_ast`]).
pub(crate) fn validate_source(source: &str) -> Result<(), String> {
    let tokens = lex_tokens(source)?;
    let mut delims: Vec<DelimKind> = Vec::new();
    let mut container_depth: usize = 0;
    let mut prev_is_value_end = false;

    for tok in tokens {
        match tok {
            Token::HashLBrace => {
                delims.push(DelimKind::MapLiteral);
                container_depth += 1;
                if container_depth > MAX_CONTAINER_DEPTH {
                    return Err(format!(
                        "Container literal nesting exceeds maximum complexity (limit {MAX_CONTAINER_DEPTH})"
                    ));
                }
                prev_is_value_end = false;
            }
            Token::LBracket => {
                let kind = if prev_is_value_end {
                    DelimKind::IndexBracket
                } else {
                    DelimKind::ArrayLiteral
                };
                delims.push(kind);
                if kind.is_container() {
                    container_depth += 1;
                    if container_depth > MAX_CONTAINER_DEPTH {
                        return Err(format!(
                            "Container literal nesting exceeds maximum complexity (limit {MAX_CONTAINER_DEPTH})"
                        ));
                    }
                }
                prev_is_value_end = false;
            }
            Token::RBracket => {
                match delims.pop() {
                    Some(kind @ (DelimKind::ArrayLiteral | DelimKind::IndexBracket)) => {
                        if kind.is_container() {
                            container_depth = container_depth.saturating_sub(1);
                        }
                    }
                    _ => return Err("Unmatched `]` delimiter in source".to_owned()),
                }
                prev_is_value_end = true;
            }
            Token::LBrace => {
                delims.push(DelimKind::Block);
                prev_is_value_end = false;
            }
            Token::RBrace => {
                let Some(kind) = delims.pop() else {
                    return Err("Unmatched `}` delimiter in source".to_owned());
                };
                match kind {
                    DelimKind::MapLiteral => {
                        container_depth = container_depth.saturating_sub(1);
                        prev_is_value_end = true;
                    }
                    DelimKind::Block => {
                        prev_is_value_end = true;
                    }
                    _ => return Err("Mismatched `}` delimiter in source".to_owned()),
                }
            }
            Token::LParen => {
                delims.push(DelimKind::Paren);
                prev_is_value_end = false;
            }
            Token::RParen => {
                if delims.pop() != Some(DelimKind::Paren) {
                    return Err("Mismatched `)` delimiter in source".to_owned());
                }
                prev_is_value_end = true;
            }
            Token::InterpStart => {
                delims.push(DelimKind::Interp);
                prev_is_value_end = false;
            }
            Token::InterpEnd => {
                if delims.pop() != Some(DelimKind::Interp) {
                    return Err("Mismatched interpolation delimiter in source".to_owned());
                }
                prev_is_value_end = false;
            }
            Token::Ident(word) => {
                prev_is_value_end = !is_prefix_keyword(word);
            }
            Token::Value => {
                prev_is_value_end = true;
            }
            Token::Other => {
                prev_is_value_end = false;
            }
        }
    }

    if !delims.is_empty() {
        return Err("Unclosed delimiter in source".to_owned());
    }

    Ok(())
}

/// Authoritative compile-time structural validator run directly over Rhai's
/// compiled [`AST`] (covering top-level statements and every function/closure
/// body, including `switch` arms, interpolations, and optimizer-folded
/// constants).
///
/// Enforces static container-literal nesting depth `<= MAX_CONTAINER_DEPTH`
/// across `Expr::Array`, `Expr::Map`, and `Expr::DynamicConstant` so no lexer
/// or syntax quirk can ever bypass the depth ceiling (P2-CORE-1).
pub(crate) fn validate_ast(ast: &AST) -> Result<(), String> {
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
            _ => {}
        }

        true
    });

    match error {
        Some(msg) => Err(msg),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p2_core_1_validate_source_fails_closed_on_unclosed_forms_and_interp_maps() {
        let must_reject = [
            "let s = \"unterminated;",
            "let s = #\"unterminated;",
            "let c = 'a;",
            "/* unclosed comment",
            "/* outer /* inner */",
            "let s = `unclosed template;",
            "let s = `${ 1;",
            "let a = [1, 2;",
            "let s = `${ #{}; \"`\" }`; let x = [[[[[[[[[1]]]]]]]]];",
            "let s = `\\` + \"`\"; let x = [[[[[[[[[1]]]]]]]]];",
        ];
        for src in must_reject {
            assert!(
                validate_source(src).is_err(),
                "expected validate_source refusal for: {src}"
            );
        }

        let must_accept = [
            "let s = `${ #{ k: 1 }; \"`\" }`;",
            "let s = `hello ${ `nested ${ #{ a: [1] } }` }`;",
            "let s = #\"raw \" quote\"#; let q = \"a\"\"b\";",
            "let s = `\\${not_interp} ``escaped`` \\`;",
        ];
        for src in must_accept {
            assert!(
                validate_source(src).is_ok(),
                "expected validate_source acceptance for: {src}"
            );
        }
    }

    #[test]
    fn p2_core_3_iterative_teardown_handles_deep_scope_and_error_runtime_on_small_stack() {
        let handle = std::thread::Builder::new()
            .name("script-p2-core-3-teardown".to_owned())
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
