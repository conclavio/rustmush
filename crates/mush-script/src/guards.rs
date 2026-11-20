//! Container, padding, and comparator guards for `mush-script` (CRIT-6,
//! CRIT-7, HIGH-16).
//!
//! Mirrors `mush-mind::guards` for the synchronous builder-scripting engine:
//!
//! * **`Array::pad`** (CRIT-6): rejects `len > max_array_size` up front, treats
//!   `len <= 0` as a no-op, enforces container depth, and uses checked
//!   arithmetic before resizing.
//! * **Container nesting depth** (CRIT-7): enforces [`MAX_CONTAINER_DEPTH`]
//!   across container mutators/builders (`push`, `insert`, `set`, `append`,
//!   `+=`, `+`, `splice`, `mixin`, `fill_with`), equality (`==`, `!=`),
//!   formatting (`print`, `debug`, `to_string`, `to_debug`, `to_json`), and
//!   variable reads (`on_var`). `to_string`/`to_debug` latch the thread-local
//!   limit flag rather than returning `Err` into Rhai's `print_with_func`
//!   (which swallows `Err` and falls back to unbounded recursive `Display`/
//!   `Debug`).
//! * **Comparator `sort`/`order`/`dedup`** (HIGH-16): replaces `slice::sort_by`
//!   and `Vec::dedup_by` (which swallow callback `ErrorTerminated` via `.ok()`
//!   / `.unwrap_or`) with fallible iterative implementations that abort
//!   immediately on the first comparator error or budget overrun.

use std::cell::Cell;
use std::cmp::Ordering;
use std::fmt::Write as _;
use std::ops::{Range, RangeInclusive};

use rhai::{
    Array, Dynamic, Engine, EvalAltResult, FnPtr, FuncRegistration, INT, ImmutableString, Map,
    NativeCallContext, Position,
};

type ExclusiveRange = Range<INT>;
type InclusiveRange = RangeInclusive<INT>;

use crate::runtime;

/// Maximum permitted nesting depth for `Array` / `Map` containers.
pub(crate) const MAX_CONTAINER_DEPTH: usize = 8;

/// Fallback formatting byte ceiling when `engine.max_string_size()` is `0`.
const FALLBACK_MAX_FORMAT_BYTES: usize = 64 * 1024;

thread_local! {
    /// Latched when a depth- or size-bounded formatting shadow hits a limit
    /// inside Rhai's `print_with_func` / string interpolation path.
    static LIMIT_TRIPPED: Cell<bool> = const { Cell::new(false) };
}

/// Clear the limit latch at the start of a script invocation.
pub(crate) fn limit_begin() {
    LIMIT_TRIPPED.with(|flag| flag.set(false));
}

/// Whether the limit latch has tripped during the current invocation.
pub(crate) fn limit_tripped() -> bool {
    LIMIT_TRIPPED.with(Cell::get)
}

/// Take and clear the limit latch at the end of an invocation.
pub(crate) fn limit_take() -> bool {
    LIMIT_TRIPPED.with(|flag| flag.replace(false))
}

fn latch_limit() {
    LIMIT_TRIPPED.with(|flag| flag.set(true));
}

pub(crate) fn depth_exceeded_err() -> Box<EvalAltResult> {
    Box::new(EvalAltResult::ErrorDataTooLarge(
        "Container nesting depth".to_owned(),
        Position::NONE,
    ))
}

fn size_exceeded_err(what: &str) -> Box<EvalAltResult> {
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

/// Register all container, padding, equality, formatting, variable-read, and
/// comparator shadows in the engine's global namespace (searched ahead of all
/// standard packages).
pub(crate) fn register(engine: &mut Engine) {
    // CRIT-6 + CRIT-7: safe `Array::pad` with checked arithmetic & depth cap.
    register_mut(engine, "pad", array_pad);

    // CRIT-7: container builders and mutators with depth (and checked size) caps.
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

    register_mut(engine, "set", map_set);
    register_mut(engine, "mixin", map_mixin);
    register_mut(engine, "+=", map_mixin);
    register_pure(engine, "+", map_merge);
    register_mut(engine, "fill_with", map_fill_with);

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

    // CRIT-7: guard literal/index-assignment nesting (`a = [a]`, `m = #{k: m}`,
    // `b[0] = a`, `n.k = m`) on every variable access.
    #[allow(deprecated)]
    engine.on_var(|name, _index, ctx| {
        if let Some(val) = ctx.scope().get(name)
            && (val.is_array() || val.is_map())
        {
            check_depth(val, MAX_CONTAINER_DEPTH)?;
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

fn check_depth(value: &Dynamic, max_depth: usize) -> Result<(), Box<EvalAltResult>> {
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
    for val in map.values() {
        mx = mx
            .checked_add(1)
            .ok_or_else(|| size_exceeded_err("Size of object map"))?;
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

// ---------------------------------------------------------------------------
// CRIT-6 + CRIT-7: Array and Map mutators/builders
// ---------------------------------------------------------------------------

fn array_pad(
    ctx: NativeCallContext,
    array: &mut Array,
    len: INT,
    item: Dynamic,
) -> Result<(), Box<EvalAltResult>> {
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

fn array_push(array: &mut Array, item: Dynamic) -> Result<(), Box<EvalAltResult>> {
    check_array_depth(array, MAX_CONTAINER_DEPTH)?;
    check_depth(&item, MAX_CONTAINER_DEPTH - 1)?;
    array.push(item);
    Ok(())
}

fn array_insert(array: &mut Array, index: INT, item: Dynamic) -> Result<(), Box<EvalAltResult>> {
    check_array_depth(array, MAX_CONTAINER_DEPTH)?;
    check_depth(&item, MAX_CONTAINER_DEPTH - 1)?;
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

fn array_set(array: &mut Array, index: INT, value: Dynamic) -> Result<(), Box<EvalAltResult>> {
    check_array_depth(array, MAX_CONTAINER_DEPTH)?;
    check_depth(&value, MAX_CONTAINER_DEPTH - 1)?;
    if array.is_empty() {
        return Ok(());
    }
    let (idx, _) = calc_offset_len(array.len(), index, 0);
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
    let s1 = calc_array_sizes_bounded(array, MAX_CONTAINER_DEPTH)?;
    let s2 = calc_array_sizes_bounded(&replace, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), add_sizes(s1, s2)?)?;
    if array.is_empty() {
        *array = replace;
        return Ok(());
    }
    let (start, len) = calc_offset_len(array.len(), start, len);
    if start >= array.len() {
        array.extend(replace);
    } else {
        array.splice(start..start.saturating_add(len), replace);
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

fn map_set(map: &mut Map, property: &str, value: Dynamic) -> Result<(), Box<EvalAltResult>> {
    check_map_depth(map, MAX_CONTAINER_DEPTH)?;
    check_depth(&value, MAX_CONTAINER_DEPTH - 1)?;
    match map.get_mut(property) {
        Some(slot) => *slot = value,
        None => {
            map.insert(property.into(), value);
        }
    }
    Ok(())
}

fn map_mixin(ctx: NativeCallContext, map: &mut Map, map2: Map) -> Result<(), Box<EvalAltResult>> {
    check_map_depth(map, MAX_CONTAINER_DEPTH)?;
    if map2.is_empty() {
        return Ok(());
    }
    let s1 = calc_map_sizes_bounded(map, MAX_CONTAINER_DEPTH)?;
    let s2 = calc_map_sizes_bounded(&map2, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), add_sizes(s1, s2)?)?;
    map.extend(map2);
    Ok(())
}

fn map_merge(ctx: NativeCallContext, mut map1: Map, map2: Map) -> Result<Map, Box<EvalAltResult>> {
    let s1 = calc_map_sizes_bounded(&map1, MAX_CONTAINER_DEPTH)?;
    let s2 = calc_map_sizes_bounded(&map2, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), add_sizes(s1, s2)?)?;
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
    check_map_depth(map, MAX_CONTAINER_DEPTH)?;
    if map2.is_empty() {
        return Ok(());
    }
    let s1 = calc_map_sizes_bounded(map, MAX_CONTAINER_DEPTH)?;
    let s2 = calc_map_sizes_bounded(&map2, MAX_CONTAINER_DEPTH)?;
    check_sizes(ctx.engine(), add_sizes(s1, s2)?)?;
    if map.is_empty() {
        *map = map2;
        return Ok(());
    }
    for (k, v) in map2 {
        map.entry(k).or_insert(v);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// CRIT-7: Depth-bounded equality
// ---------------------------------------------------------------------------

fn array_eq(
    ctx: NativeCallContext,
    array1: &mut Array,
    array2: Array,
) -> Result<bool, Box<EvalAltResult>> {
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
    if let Some(reason) = runtime::budget_tripped() {
        return Err(budget_terminated_err(reason));
    }
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
    if let Some(reason) = runtime::budget_tripped() {
        return Err(budget_terminated_err(reason));
    }
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
    if let Some(reason) = runtime::budget_tripped() {
        return Err(budget_terminated_err(reason));
    }
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
    if let Some(reason) = runtime::budget_tripped() {
        return Err(budget_terminated_err(reason));
    }
    let val = comparer.call_raw(ctx, None, [x.clone(), y.clone()])?;
    if let Some(reason) = runtime::budget_tripped() {
        return Err(budget_terminated_err(reason));
    }
    if let Ok(n) = val.as_int() {
        return Ok(n.cmp(&0));
    }
    if let Ok(b) = val.as_bool() {
        return Ok(if b { Ordering::Less } else { Ordering::Greater });
    }
    Ok(x.type_id().cmp(&y.type_id()))
}

fn merge_sort_by(
    ctx: &NativeCallContext,
    array: &mut Array,
    comparer: &FnPtr,
) -> Result<(), Box<EvalAltResult>> {
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
    check_array_depth(array, MAX_CONTAINER_DEPTH)?;
    let mut copy = array.clone();
    merge_sort_by(&ctx, &mut copy, &comparer)?;
    Ok(copy)
}

fn dedup_with(
    array: &mut Array,
    mut is_dup: impl FnMut(&Dynamic, &Dynamic) -> Result<bool, Box<EvalAltResult>>,
) -> Result<(), Box<EvalAltResult>> {
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
        if let Some(reason) = runtime::budget_tripped() {
            kept.push(curr);
            kept.extend(iter);
            *array = kept;
            return Err(budget_terminated_err(reason));
        }
        let Some(prev) = kept.last() else {
            kept.push(curr);
            continue;
        };
        let dup = match is_dup(prev, &curr) {
            Ok(d) => {
                if let Some(reason) = runtime::budget_tripped() {
                    kept.push(curr);
                    kept.extend(iter);
                    *array = kept;
                    return Err(budget_terminated_err(reason));
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
        let res = comparer.call_raw(&ctx, None, [prev.clone(), curr.clone()])?;
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
// CRIT-7: Compile-time static container-literal nesting guard
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
    Code,
    TemplateString,
    Interp { brace_depth: usize },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Delim {
    ArrayLit,
    Index,
    MapLit,
    Block,
    Paren,
    Interp,
}

fn next_char_len(source: &str, idx: usize) -> usize {
    source
        .get(idx..)
        .and_then(|rest| rest.chars().next())
        .map_or(1, char::len_utf8)
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

fn lex_tokens(source: &str) -> Vec<Token<'_>> {
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
                    i += 1;
                    if modes.len() > 1 {
                        modes.pop();
                    }
                    out.push(Token::Value);
                }
                Some(b'\\') => {
                    i += 1;
                    if i < len {
                        i += next_char_len(source, i);
                    }
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
            continue;
        }

        if b == b'"' {
            i += 1;
            while let Some(c) = bytes.get(i).copied() {
                if c == b'\\' {
                    i += 1;
                    if i < len {
                        i += next_char_len(source, i);
                    }
                } else if c == b'"' {
                    i += 1;
                    break;
                } else {
                    i += next_char_len(source, i);
                }
            }
            out.push(Token::Value);
            continue;
        }

        if b == b'#' {
            if bytes.get(i + 1) == Some(&b'{') {
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
                while i < len {
                    if bytes.get(i) == Some(&b'"') {
                        let mut closing = 0;
                        while closing < hashes && bytes.get(i + 1 + closing) == Some(&b'#') {
                            closing += 1;
                        }
                        if closing == hashes {
                            i += 1 + hashes;
                            break;
                        }
                    }
                    i += 1;
                }
                out.push(Token::Value);
                continue;
            }
            i += 1;
            out.push(Token::Other);
            continue;
        }

        if b == b'\'' {
            i += 1;
            while let Some(c) = bytes.get(i).copied() {
                if c == b'\\' {
                    i += 1;
                    if i < len {
                        i += next_char_len(source, i);
                    }
                } else if c == b'\'' {
                    i += 1;
                    break;
                } else {
                    i += next_char_len(source, i);
                }
            }
            out.push(Token::Value);
            continue;
        }

        if b == b'`' {
            i += 1;
            modes.push(LexMode::TemplateString);
            continue;
        }

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
            _ => {}
        }

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
        out.push(Token::Other);
    }

    out
}

/// Compile-time structural guard (CRIT-7) run after Rhai's parser succeeds:
/// caps static container-literal (`[` / `#{`) nesting depth at
/// [`MAX_CONTAINER_DEPTH`] (8) so recursive functions returning deeply nested
/// literals (e.g. `fn f(n) { [[...50x...f(n-1)...]] }`) cannot construct deep
/// container trees bypassing `on_var`.
pub(crate) fn validate_source(source: &str) -> Result<(), String> {
    let tokens = lex_tokens(source);
    let mut delims: Vec<Delim> = Vec::new();
    let mut container_depth: usize = 0;
    let mut prev_is_value_end = false;

    for tok in tokens {
        match tok {
            Token::HashLBrace => {
                container_depth += 1;
                if container_depth > MAX_CONTAINER_DEPTH {
                    return Err(format!(
                        "Container literal nesting exceeds maximum complexity (limit {MAX_CONTAINER_DEPTH})"
                    ));
                }
                delims.push(Delim::MapLit);
                prev_is_value_end = false;
            }
            Token::LBracket => {
                if prev_is_value_end {
                    delims.push(Delim::Index);
                } else {
                    container_depth += 1;
                    if container_depth > MAX_CONTAINER_DEPTH {
                        return Err(format!(
                            "Container literal nesting exceeds maximum complexity (limit {MAX_CONTAINER_DEPTH})"
                        ));
                    }
                    delims.push(Delim::ArrayLit);
                }
                prev_is_value_end = false;
            }
            Token::RBracket => {
                if let Some(d) = delims.pop()
                    && d == Delim::ArrayLit
                {
                    container_depth = container_depth.saturating_sub(1);
                }
                prev_is_value_end = true;
            }
            Token::LBrace => {
                delims.push(Delim::Block);
                prev_is_value_end = false;
            }
            Token::RBrace => {
                if let Some(d) = delims.pop()
                    && d == Delim::MapLit
                {
                    container_depth = container_depth.saturating_sub(1);
                    prev_is_value_end = true;
                } else {
                    prev_is_value_end = false;
                }
            }
            Token::LParen => {
                delims.push(Delim::Paren);
                prev_is_value_end = false;
            }
            Token::RParen => {
                let _ = delims.pop();
                prev_is_value_end = true;
            }
            Token::InterpStart => {
                delims.push(Delim::Interp);
                prev_is_value_end = false;
            }
            Token::InterpEnd => {
                let _ = delims.pop();
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

    Ok(())
}
