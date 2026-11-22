//! Lock expressions: a small declarative AST with TinyMUX-compatible syntax.
//!
//! No softcode survives in rustmush, but classic lock keys do:
//!
//! | syntax            | meaning                                        |
//! |-------------------|------------------------------------------------|
//! | `me`, `*Nick`, `Bob` | resolves (via caller closure) to an object → passes if `who` **is** that object |
//! | `#123`            | passes if `who` is `#123`                      |
//! | `+#123`           | passes if `who` **carries** `#123`             |
//! | `$#123`           | passes if `who` is **owned by** `#123`         |
//! | `flag^WIZARD`     | passes if `who` has the flag (or marker)       |
//! | `sex:m*le`        | passes if `who`'s attribute glob-matches       |
//! | `@#123/enter`     | **indirect**: evaluates `#123`'s enter lock    |
//! | `#TRUE` / `#FALSE`| constants                                      |
//! | `!` `&` `\|` `( )`| boolean operators, precedence `!` > `&` > `\|` |
//!
//! Attribute locks split on the **last** `:` so namespaced attribute names
//! (`wod:clan:V*` → attr `wod:clan`, glob `V*`) round-trip; glob patterns
//! therefore must not contain `:`, and — because the parser breaks terms at
//! the operator characters — must not contain `&`, `|`, `(`, or `)` either.
//! (The parser cannot produce such globs; programmatically built
//! [`LockExpr::AttrMatch`] values violating this print as text that
//! reparses into a different tree.)
//!
//! The [`Display`](std::fmt::Display) impl prints an expression back in this
//! syntax such that `parse(print(e)) == e` (a round-trip test enforces it).
//!
//! Three bounds keep a deep tree from becoming a stack overflow, and they are
//! not the same bound: [`MAX_LOCK_PARSE_FRAMES`] and [`MAX_LOCK_EXPR_DEPTH`]
//! stop the parser from *building* one; `MAX_LOCK_STRUCTURAL_DEPTH` stops
//! [`eval_lock`] and `Display` reading one that got into RAM by some other
//! road (a decode, a module), and they fail closed at it;
//! [`LockExpr::clamp_depth`] repairs such a tree in place. Freeing one is
//! bounded too — `Drop` is hand-written and iterative — and
//! [`LockExpr::depth`] measures without recursing. What is *not* bounded is
//! the derived `Clone`/`PartialEq`/`Debug`/`Serialize`; they are safe only
//! because nothing outside this file builds a compound node, which
//! `tests/lock_construction_ratchet.rs` holds in place.

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::attr::{Attr, AttrFlags, AttrValue, UncasedName};
use crate::flags::Flags;
use crate::obj::ObjId;
use crate::object_read::{BoundedRead, ObjectRead};
use crate::world::World;

/// The kinds of lock an object can carry (`@lock/<type>`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum LockType {
    /// The default lock: who may pick this up / pass this exit.
    Basic,
    /// Who may enter (things with `ENTER_OK`, vehicles).
    Enter,
    /// Who may `use` the object.
    Use,
    /// Who may page the player.
    Page,
    /// Who may teleport to/into it.
    Teleport,
    /// Who may speak in the room.
    Speech,
    /// Zone control: passing grants `controls()` over zone members.
    Zone,
    /// Who may drop the object.
    Drop,
    /// Who may give the object away / give to this object.
    Give,
    /// Who may @mail the player.
    Mail,
}

impl LockType {
    /// Lowercase name used in `@lock/<type>` and `@#123/<type>` syntax.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            LockType::Basic => "basic",
            LockType::Enter => "enter",
            LockType::Use => "use",
            LockType::Page => "page",
            LockType::Teleport => "teleport",
            LockType::Speech => "speech",
            LockType::Zone => "zone",
            LockType::Drop => "drop",
            LockType::Give => "give",
            LockType::Mail => "mail",
        }
    }
}

impl fmt::Display for LockType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for LockType {
    type Err = LockParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lt = match s.to_ascii_lowercase().as_str() {
            "basic" => LockType::Basic,
            "enter" => LockType::Enter,
            "use" => LockType::Use,
            "page" => LockType::Page,
            "teleport" => LockType::Teleport,
            "speech" => LockType::Speech,
            "zone" => LockType::Zone,
            "drop" => LockType::Drop,
            "give" => LockType::Give,
            "mail" => LockType::Mail,
            _ => return Err(LockParseError::UnknownLockType(s.to_string())),
        };
        Ok(lt)
    }
}

/// A parsed lock expression. Names (`me`, `*player`) are resolved to
/// [`ObjId`]s at parse time, so the AST is purely structural.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LockExpr {
    /// Always passes (`#TRUE`) / always fails (`#FALSE`).
    Const(bool),
    /// `who` is exactly this object.
    Is(ObjId),
    /// `who` carries this object (`+#123`).
    Carry(ObjId),
    /// `who` is owned by this object (`$#123`).
    OwnedBy(ObjId),
    /// `who` has this core flag or marker flag (`flag^WIZARD`).
    HasFlag(SmolStr),
    /// `who`'s attribute `name` glob-matches `glob` (`sex:m*le`).
    AttrMatch {
        /// Attribute name (inheritance-aware lookup).
        name: UncasedName,
        /// Glob pattern (`*` and `?`), matched case-insensitively.
        ///
        /// Must not contain `:` or the operator characters `& | ( )` —
        /// such a pattern would not survive the documented
        /// `parse(print(e)) == e` round-trip (see the module docs).
        glob: String,
    },
    /// Evaluate another object's lock of the given type (`@#123/enter`).
    Indirect(ObjId, LockType),
    /// Negation (`!`).
    Not(Box<LockExpr>),
    /// Conjunction (`&`).
    And(Box<LockExpr>, Box<LockExpr>),
    /// Disjunction (`|`).
    Or(Box<LockExpr>, Box<LockExpr>),
}

/// Default / maximum indirection depth for [`eval_lock`].
pub const MAX_LOCK_DEPTH: u8 = 10;

/// Maximum total AST evaluation steps [`eval_lock`] will perform across all
/// branches and indirect lock hops before failing closed to `false`. Bounds
/// DAG/branching blowup (`k^MAX_LOCK_DEPTH`) even on locks that bypassed
/// [`parse_lock`].
pub const MAX_LOCK_STEPS: u32 = 512;

/// Maximum metered work units [`eval_lock`] will spend on a single evaluation
/// (attribute/JSON inspection, glob matching, marker scans, inventory walks)
/// before failing closed to `false`.
pub const MAX_LOCK_WORK: usize = 512_000;

/// Maximum byte length of a glob pattern accepted in [`LockExpr::AttrMatch`]
/// (enforced in [`parse_lock`] and re-checked in [`eval_lock`]) and `@find`.
pub const MAX_LOCK_GLOB_LEN: usize = 256;

/// Does this node own children — and so cost a level of every walk over the
/// tree, `Drop` included?
fn compound(e: &LockExpr) -> bool {
    matches!(e, LockExpr::Not(_) | LockExpr::And(..) | LockExpr::Or(..))
}

/// How many *structural* levels ([`LockExpr::Not`], [`And`](LockExpr::And),
/// [`Or`](LockExpr::Or)) [`eval_lock`] and [`Display`](fmt::Display) will
/// descend before they give up and fail closed.
///
/// This is a stack fence, not a policy: it exists so that a tree which never
/// went through [`parse_lock`] — one decoded from Redis, or built by a module
/// — cannot walk the guard page on the world task. Two independent bounds
/// already stand in front of it ([`MAX_LOCK_EXPR_DEPTH`] = 48 on the parse
/// path, serde_json's own 128-level recursion limit on the decode path, which
/// caps a decodable tree at roughly 127 `Not` levels or 63 `And`/`Or` levels),
/// so no tree that can legitimately reach RAM comes anywhere near 1024 and the
/// truncation below is unreachable in practice. That headroom is the point:
/// the budget must stay strictly above every decodable tree, or `Display`
/// would truncate a legal key and break the module's load-bearing
/// `parse(print(e)) == e` invariant.
const MAX_LOCK_STRUCTURAL_DEPTH: u32 = 1024;

/// Evaluate `expr` against `who`.
///
/// `depth` bounds **indirect** lock chains ([`LockExpr::Indirect`]); each
/// indirection decrements it and an expression at depth 0 — or any active
/// `@lock`-cycle — poisons the evaluation so even enclosing `!` fails closed
/// to `false`. Callers normally pass [`MAX_LOCK_DEPTH`].
///
/// Semantics of the leaves:
/// - `Carry`: the target object is directly in `who`'s contents.
/// - `OwnedBy`: `who.owner` is the target.
/// - `HasFlag`: matched against core [`Flags`] by (case-folded) name first,
///   then against the object's marker set, case-insensitively.
/// - `AttrMatch`: attribute looked up with parent-chain inheritance
///   ([`World::attr`]); `Text` values match their text, `Json` values match
///   their compact serialization; a missing attribute fails, and an attribute
///   the lock may not *read* ([`AttrFlags::WIZARD_ONLY`], [`AttrFlags::SELF_ONLY`])
///   poisons the whole key — see `unreadable`.
/// - `Indirect`: a missing target object fails; a target without that lock
///   type passes (an unset lock is unlocked, as in TinyMUX).
///
/// Structural nesting (`!`, `&`, `|`) is bounded separately by
/// `MAX_LOCK_STRUCTURAL_DEPTH`; a tree taller than that fails closed at the
/// bound instead of overflowing the world task's stack.
#[must_use]
pub fn eval_lock(w: &World, expr: &LockExpr, who: ObjId, depth: u8) -> bool {
    eval_lock_with(&mut BoundedRead::new(w, MAX_LOCK_WORK), expr, who, depth).unwrap_or(false)
}

/// Resource errors propagate through every evaluated operator, including
/// negation. Evaluation-cutoff conditions (unreadable attributes, indirect
/// cycles, indirect-depth exhaustion, structural-depth exhaustion, and total
/// step-budget exhaustion) poison evaluation as `Ok(None)` so enclosing
/// operators — including `!` — fail closed to `false`, while remaining
/// distinct from a tracked-read `Err`.
pub(crate) fn eval_lock_with<'w, R: ObjectRead<'w>>(
    read: &mut R,
    expr: &LockExpr,
    who: ObjId,
    depth: u8,
) -> Result<bool, R::Error> {
    // `None` — the key read an attribute it is not allowed to read, hit a
    // cycle, or exhausted the indirect/structural/step budget — is not a third
    // answer anyone outside can see: it lands as "does not pass", failing
    // closed even through `!`.
    let mut steps = MAX_LOCK_STEPS;
    let mut active: Vec<(ObjId, LockType)> = Vec::new();
    let mut memo: HashMap<(ObjId, LockType), (bool, u8)> = HashMap::new();
    Ok(eval_with(
        read,
        expr,
        who,
        depth,
        MAX_LOCK_STRUCTURAL_DEPTH,
        &mut steps,
        &mut active,
        &mut memo,
    )?
    .0
    .unwrap_or(false))
}

/// Is this attribute one a lock key may not read?
///
/// `@lock/speech here=wod:secret:V*` is a **read path**: setting a key and
/// watching whether it passes is a one-bit probe of any attribute the key can
/// name. [`World::attr`] filters only [`AttrFlags::NO_INHERIT`], and the read
/// permissions are enforced nowhere else but the `examine` printer, so without
/// this test any player who can set a lock could spell out an attribute
/// `examine` refuses them, one glob at a time.
///
/// [`AttrFlags::HIDDEN`] is deliberately **not** in the set, on two grounds.
/// It is a display flag, not a confidentiality boundary — `examine/all` shows
/// a hidden attribute to *any* examiner, with no role test — so a lock reading
/// it breaches nothing. And it is a flag a player may set on their own
/// attributes, so honouring it would hand every subject of an attribute lock
/// the means to defeat it (and to flip every negated one) by flagging the
/// attribute the key names.
fn unreadable(a: &Attr) -> bool {
    a.flags
        .intersects(AttrFlags::WIZARD_ONLY | AttrFlags::SELF_ONLY)
}

/// Is this attribute name an internal core or module-owned attribute that
/// `@lock` `AttrMatch` must never read (nor even probe for existence)?
///
/// Checking the name **before** reading the attribute closes both value leaks
/// (on legacy unflagged records or module-written records without explicit
/// `WIZARD_ONLY`/`SELF_ONLY` flags) and presence oracles (e.g. `!ignore:*`
/// revealing whether a player has muted anyone at all).
fn is_restricted_lock_attr(name: &str) -> bool {
    if name.eq_ignore_ascii_case("ignore") || name.eq_ignore_ascii_case("macros") {
        return true;
    }
    let Some((ns, _)) = name.split_once(':') else {
        return false;
    };
    ns.eq_ignore_ascii_case("sheet")
        || ns.eq_ignore_ascii_case("agent")
        || ns.eq_ignore_ascii_case("tacsim")
        || ns.eq_ignore_ascii_case("script")
        || ns.eq_ignore_ascii_case("atlas")
}

/// [`eval_lock`] with both budgets explicit, and with the third answer.
///
/// `indirect` is the public `depth`: it counts [`LockExpr::Indirect`] hops and
/// is what [`MAX_LOCK_DEPTH`] sizes. `structural` counts `Not`/`And`/`Or`
/// levels within one object's key and is reseeded on every indirection,
/// because each hop starts a fresh (and separately bounded) tree — the two
/// budgets multiply into the true stack ceiling, which is why the indirect
/// budget stays small.
///
/// Returns `(outcome, indirect_hops_used)` where `Some(b)` is the key's answer
/// and `None` is the fail-closed poison produced when the key tries to read an
/// attribute it may not (`unreadable` / `is_restricted_lock_attr`), detects an
/// indirect cycle, or exhausts its indirect, structural, or step budget.
/// `None` is not `Some(false)`: it **absorbs** through every operator,
/// including `!`, so neither a restricted-attribute probe nor a cycle/budget
/// cutoff can be negated into a pass. For restricted attributes, `None` does
/// not depend on the attribute's value, so the refusal is byte-identical
/// whether the value would have matched the glob or not — a lock that failed
/// for the ordinary reason and a lock that was not allowed to look must be
/// indistinguishable, or the refusal is itself the leaked bit. The one thing
/// that does not poison is a branch never taken: `&`/`|` keep their
/// short-circuit, so a term that is never evaluated is never read either.
///
/// Visited `(ObjId, LockType)` outcomes are memoized alongside the number of
/// nested indirect hops they required so diamond DAGs across an object's lock
/// slots evaluate each node at most once while preserving exact `indirect`
/// depth-cutoff semantics.
#[allow(clippy::too_many_arguments)]
fn eval_with<'w, R: ObjectRead<'w>>(
    read: &mut R,
    expr: &LockExpr,
    who: ObjId,
    indirect: u8,
    structural: u32,
    steps: &mut u32,
    active: &mut Vec<(ObjId, LockType)>,
    memo: &mut HashMap<(ObjId, LockType), (bool, u8)>,
) -> Result<(Option<bool>, u8), R::Error> {
    if *steps == 0 {
        // Total evaluation step budget exhausted: poison so enclosing `!`
        // cannot invert the refusal into a pass (fail closed).
        return Ok((None, 0));
    }
    *steps -= 1;
    read.charge_work(1)?;
    Ok(match expr {
        LockExpr::Const(b) => (Some(*b), 0),
        LockExpr::Is(id) => (Some(who == *id), 0),
        LockExpr::Carry(id) => {
            let contents = read
                .object(who)?
                .map_or(&[][..], |object| object.contents.as_slice());
            read.charge_work(contents.len())?;
            (Some(contents.contains(id)), 0)
        }
        LockExpr::OwnedBy(id) => (Some(read.object(who)?.is_some_and(|o| o.owner == *id)), 0),
        LockExpr::HasFlag(name) => (Some(eval_flag_with(read, who, name)?), 0),
        LockExpr::AttrMatch { name, glob } => (eval_attr_with(read, who, name, glob)?, 0),
        LockExpr::Indirect(obj, lt) => {
            if indirect == 0 {
                // Indirect-depth budget exhausted: poison so enclosing `!`
                // cannot invert the refusal into permission (EXT-P2-16).
                return Ok((None, 0));
            }
            let key = (*obj, *lt);
            if active.contains(&key) {
                // Active recursion cycle detected: poison so enclosing `!`
                // cannot turn a cyclic lock into permission (EXT-P2-16).
                return Ok((None, 0));
            }
            if let Some(&(cached, body_hops)) = memo.get(&key) {
                if indirect - 1 < body_hops {
                    return Ok((None, 0));
                }
                return Ok((Some(cached), body_hops.saturating_add(1)));
            }
            match read.object(*obj)? {
                None => (Some(false), 1),
                Some(o) => match o.locks.get(lt) {
                    None => (Some(true), 1),
                    // A fresh tree: the structural budget starts over, while
                    // the indirect budget, total step budget, active cycle
                    // set, and DAG memo carry through.
                    Some(inner) => {
                        active.push(key);
                        let res = eval_with(
                            read,
                            inner,
                            who,
                            indirect - 1,
                            MAX_LOCK_STRUCTURAL_DEPTH,
                            steps,
                            active,
                            memo,
                        );
                        active.pop();
                        let (val, body_hops) = res?;
                        if let Some(b) = val {
                            memo.insert(key, (b, body_hops));
                        }
                        (val, body_hops.saturating_add(1))
                    }
                },
            }
        }
        LockExpr::Not(e) => {
            if structural == 0 {
                return Ok((None, 0));
            }
            let (val, hops) =
                eval_with(read, e, who, indirect, structural - 1, steps, active, memo)?;
            (val.map(|v| !v), hops)
        }
        LockExpr::And(a, b) => {
            if structural == 0 {
                return Ok((None, 0));
            }
            let (va, ha) = eval_with(read, a, who, indirect, structural - 1, steps, active, memo)?;
            match va {
                Some(true) => {
                    let (vb, hb) =
                        eval_with(read, b, who, indirect, structural - 1, steps, active, memo)?;
                    (vb, ha.max(hb))
                }
                short => (short, ha),
            }
        }
        LockExpr::Or(a, b) => {
            if structural == 0 {
                return Ok((None, 0));
            }
            let (va, ha) = eval_with(read, a, who, indirect, structural - 1, steps, active, memo)?;
            match va {
                Some(false) => {
                    let (vb, hb) =
                        eval_with(read, b, who, indirect, structural - 1, steps, active, memo)?;
                    (vb, ha.max(hb))
                }
                short => (short, ha),
            }
        }
    })
}

// Keep allocation-bearing leaf work outside the recursive evaluator's stack
// frame; the legacy structural limit also protects directly constructed ASTs.
fn eval_flag_with<'w, R: ObjectRead<'w>>(
    read: &mut R,
    who: ObjId,
    name: &str,
) -> Result<bool, R::Error> {
    let Some(object) = read.object(who)? else {
        return Ok(false);
    };
    read.charge_work(name.len())?;
    if let Some(flag) = Flags::from_name(&name.to_uppercase()) {
        return Ok(object.flags.contains(flag));
    }
    for marker in &object.markers {
        read.charge_work(marker.len().saturating_add(name.len()).saturating_add(1))?;
        if marker.eq_ignore_ascii_case(name) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn eval_attr_with<'w, R: ObjectRead<'w>>(
    read: &mut R,
    who: ObjId,
    name: &UncasedName,
    glob: &str,
) -> Result<Option<bool>, R::Error> {
    // Fail closed before even checking existence on restricted internal/module
    // attributes (closing presence oracles such as `!ignore:*`) and on
    // unparsed oversized globs.
    if is_restricted_lock_attr(name.as_str()) || glob.len() > MAX_LOCK_GLOB_LEN {
        return Ok(None);
    }
    Ok(match crate::world::attr_with(read, who, name.as_str())? {
        // Checked before the value is so much as looked at.
        Some(attr) if unreadable(attr) => None,
        Some(attr) => Some(match &attr.value {
            AttrValue::Text(text) => glob_with(glob, text, |units| read.charge_work(units))?,
            AttrValue::Json(value) => {
                charge_json_text(read, value)?;
                glob_with(glob, &value.to_string(), |units| read.charge_work(units))?
            }
        }),
        None => Some(false),
    })
}

/// Charge a conservative serialized size before compact JSON allocation.
/// Iterators keep the pending work proportional to depth, not array width.
fn charge_json_text<'w, R: ObjectRead<'w>>(
    read: &mut R,
    root: &serde_json::Value,
) -> Result<(), R::Error> {
    use serde_json::Value;
    enum Frame<'a> {
        Value(&'a Value),
        Array(std::slice::Iter<'a, Value>),
        Object(serde_json::map::Iter<'a>),
    }
    let mut stack = vec![Frame::Value(root)];
    while let Some(frame) = stack.pop() {
        read.charge_work(1)?;
        match frame {
            Frame::Value(value) => match value {
                Value::String(text) => {
                    read.charge_work(text.len().saturating_mul(6).saturating_add(2))?
                }
                Value::Array(values) => stack.push(Frame::Array(values.iter())),
                Value::Object(values) => stack.push(Frame::Object(values.iter())),
                _ => read.charge_work(32)?,
            },
            Frame::Array(mut values) => {
                if let Some(value) = values.next() {
                    stack.push(Frame::Array(values));
                    stack.push(Frame::Value(value));
                }
            }
            Frame::Object(mut values) => {
                if let Some((key, value)) = values.next() {
                    read.charge_work(key.len().saturating_mul(6).saturating_add(3))?;
                    stack.push(Frame::Object(values));
                    stack.push(Frame::Value(value));
                }
            }
        }
    }
    Ok(())
}

/// Case-insensitive glob match supporting `*` (any run, including empty) and
/// `?` (exactly one character). No regex, no escapes.
///
/// ```
/// use mush_core::lock::glob_match;
/// assert!(glob_match("m*le", "Male"));
/// assert!(glob_match("a?c", "abc"));
/// assert!(!glob_match("a?c", "ac"));
/// ```
#[must_use]
pub fn glob_match(pattern: &str, text: &str) -> bool {
    match glob_with(pattern, text, |_| Ok::<(), std::convert::Infallible>(())) {
        Ok(value) => value,
        Err(never) => match never {},
    }
}

/// Case-insensitive glob match with an explicit work budget. Returns `None`
/// if `budget` is exhausted before matching finishes.
pub(crate) fn glob_match_bounded(pattern: &str, text: &str, budget: &mut usize) -> Option<bool> {
    glob_with(pattern, text, |units| match budget.checked_sub(units) {
        Some(left) => {
            *budget = left;
            Ok(())
        }
        None => {
            *budget = 0;
            Err(())
        }
    })
    .ok()
}

fn chars_match_fixed<E>(
    seg: &[char],
    slice: &[char],
    charge: &mut impl FnMut(usize) -> Result<(), E>,
) -> Result<bool, E> {
    if seg.len() != slice.len() {
        return Ok(false);
    }
    for (&pc, &tc) in seg.iter().zip(slice.iter()) {
        charge(1)?;
        if pc != '?' && pc != tc {
            return Ok(false);
        }
    }
    Ok(true)
}

fn kmp_lps<E>(
    needle: &[char],
    charge: &mut impl FnMut(usize) -> Result<(), E>,
) -> Result<Vec<usize>, E> {
    let mut lps = vec![0usize; needle.len()];
    let mut len = 0usize;
    let mut i = 1usize;
    while i < needle.len() {
        charge(1)?;
        if needle.get(i) == needle.get(len) {
            len += 1;
            if let Some(slot) = lps.get_mut(i) {
                *slot = len;
            }
            i += 1;
        } else if len > 0 {
            len = lps.get(len - 1).copied().unwrap_or(0);
        } else {
            i += 1;
        }
    }
    Ok(lps)
}

/// Linear-time KMP search for a `?`-free literal segment `needle` in `haystack`.
fn find_literal_kmp<E>(
    needle: &[char],
    haystack: &[char],
    charge: &mut impl FnMut(usize) -> Result<(), E>,
) -> Result<Option<usize>, E> {
    if needle.is_empty() {
        return Ok(Some(0));
    }
    if needle.len() > haystack.len() {
        return Ok(None);
    }
    let lps = kmp_lps(needle, charge)?;
    let mut hi = 0usize;
    let mut ni = 0usize;
    while hi < haystack.len() {
        charge(1)?;
        if haystack.get(hi) == needle.get(ni) {
            hi += 1;
            ni += 1;
            if ni == needle.len() {
                return Ok(Some(hi - ni));
            }
        } else if ni > 0 {
            ni = lps.get(ni - 1).copied().unwrap_or(0);
        } else {
            hi += 1;
        }
    }
    Ok(None)
}

/// Earliest occurrence of a `?`-containing segment `seg` inside `haystack`.
/// Streams KMP over the longest literal run inside `seg` (never backtracking
/// `hi`) and caches the last mismatch position so adversarial patterns such as
/// `a?a?...?b` against `aaaa...a` reject each candidate in O(1), with every
/// comparison metered via `charge`.
fn find_wildcard_segment<E>(
    seg: &[char],
    haystack: &[char],
    charge: &mut impl FnMut(usize) -> Result<(), E>,
) -> Result<Option<usize>, E> {
    if seg.len() > haystack.len() {
        return Ok(None);
    }
    let (mut best_start, mut best_len) = (0usize, 0usize);
    let mut run_start = 0usize;
    for (idx, &c) in seg.iter().enumerate() {
        if c == '?' {
            if idx - run_start > best_len {
                best_start = run_start;
                best_len = idx - run_start;
            }
            run_start = idx + 1;
        }
    }
    if seg.len() - run_start > best_len {
        best_start = run_start;
        best_len = seg.len() - run_start;
    }
    if best_len == 0 {
        charge(seg.len())?;
        return Ok(Some(0));
    }

    let mut probe = seg.iter().rposition(|&c| c != '?').unwrap_or(best_start);

    if best_len >= 2
        && let Some(anchor) = seg.get(best_start..best_start + best_len)
    {
        let tail_after_anchor = seg.len() - (best_start + best_len);
        let search_end = haystack.len() - tail_after_anchor;
        let lps = kmp_lps(anchor, charge)?;
        let mut hi = best_start;
        let mut ni = 0usize;
        while hi < search_end {
            charge(1)?;
            if haystack.get(hi) == anchor.get(ni) {
                hi += 1;
                ni += 1;
                if ni == anchor.len() {
                    let cand = (hi - ni) - best_start;
                    if let Some(window) = haystack.get(cand..cand + seg.len()) {
                        charge(1)?;
                        let probe_ok = seg.get(probe).copied() == Some('?')
                            || seg.get(probe) == window.get(probe);
                        if probe_ok {
                            let mut matched = true;
                            for (j, (&pc, &tc)) in seg.iter().zip(window.iter()).enumerate() {
                                if j >= best_start && j < best_start + best_len {
                                    continue;
                                }
                                charge(1)?;
                                if pc != '?' && pc != tc {
                                    probe = j;
                                    matched = false;
                                    break;
                                }
                            }
                            if matched {
                                return Ok(Some(cand));
                            }
                        }
                    }
                    ni = lps.get(ni - 1).copied().unwrap_or(0);
                }
            } else if ni > 0 {
                ni = lps.get(ni - 1).copied().unwrap_or(0);
            } else {
                hi += 1;
            }
        }
        return Ok(None);
    }

    let max_cand = haystack.len() - seg.len();
    for cand in 0..=max_cand {
        let Some(window) = haystack.get(cand..cand + seg.len()) else {
            break;
        };
        charge(1)?;
        if seg.get(probe).copied() != Some('?') && seg.get(probe) != window.get(probe) {
            continue;
        }
        let mut matched = true;
        for (j, (&pc, &tc)) in seg.iter().zip(window.iter()).enumerate() {
            charge(1)?;
            if pc != '?' && pc != tc {
                probe = j;
                matched = false;
                break;
            }
        }
        if matched {
            return Ok(Some(cand));
        }
    }
    Ok(None)
}

fn glob_with<E>(
    pattern: &str,
    text: &str,
    mut charge: impl FnMut(usize) -> Result<(), E>,
) -> Result<bool, E> {
    // Lowercasing may expand Unicode text. Charge before either allocation.
    charge(
        pattern
            .len()
            .saturating_add(text.len())
            .saturating_mul(3)
            .saturating_add(1),
    )?;
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();

    let Some(first_star) = p.iter().position(|&c| c == '*') else {
        return chars_match_fixed(&p, &t, &mut charge);
    };
    let last_star = p.iter().rposition(|&c| c == '*').unwrap_or(first_star);
    let prefix = p.get(..first_star).unwrap_or(&[]);
    let suffix = p.get(last_star + 1..).unwrap_or(&[]);
    if prefix.len().saturating_add(suffix.len()) > t.len() {
        return Ok(false);
    }
    let Some(t_prefix) = t.get(..prefix.len()) else {
        return Ok(false);
    };
    if !chars_match_fixed(prefix, t_prefix, &mut charge)? {
        return Ok(false);
    }
    let suffix_start = t.len() - suffix.len();
    let Some(t_suffix) = t.get(suffix_start..) else {
        return Ok(false);
    };
    if !chars_match_fixed(suffix, t_suffix, &mut charge)? {
        return Ok(false);
    }

    let mut rem = t.get(prefix.len()..suffix_start).unwrap_or(&[]);
    if last_star > first_star
        && let Some(mid) = p.get(first_star + 1..last_star)
    {
        for seg in mid.split(|&c| c == '*').filter(|s| !s.is_empty()) {
            let found = if seg.contains(&'?') {
                find_wildcard_segment(seg, rem, &mut charge)?
            } else {
                find_literal_kmp(seg, rem, &mut charge)?
            };
            let Some(pos) = found else {
                return Ok(false);
            };
            rem = rem.get(pos + seg.len()..).unwrap_or(&[]);
        }
    }
    Ok(true)
}

/// Errors from [`parse_lock`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LockParseError {
    /// Ran out of input where a term was expected.
    #[error("unexpected end of lock expression")]
    UnexpectedEnd,
    /// An operator had no operand (e.g. `a & & b`).
    #[error("empty term in lock expression")]
    EmptyTerm,
    /// A `(` without its `)`.
    #[error("expected ')' in lock expression")]
    UnbalancedParen,
    /// Input remained after a complete expression (usually a stray `)`).
    #[error("unexpected trailing input at byte {0} in lock expression")]
    Trailing(usize),
    /// A `#...` reference that isn't a number, or an empty name.
    #[error("bad object reference {0:?}")]
    BadObjRef(String),
    /// The resolver closure did not recognize a name.
    #[error("unknown name {0:?} in lock expression")]
    UnknownName(String),
    /// `@#123/xyzzy` with an unrecognized lock type.
    #[error("unknown lock type {0:?}")]
    UnknownLockType(String),
    /// A `^` term whose left side isn't `flag`, or with an empty flag name.
    #[error("bad flag lock {0:?} (expected flag^NAME)")]
    BadFlagLock(String),
    /// The key nests deeper than the parser will go — either more nested
    /// `(`/`!` than [`MAX_LOCK_PARSE_FRAMES`] (a stack bound) or a tree
    /// taller than [`MAX_LOCK_EXPR_DEPTH`] (an AST bound). `max` is
    /// whichever limit refused.
    #[error("lock expression nests too deeply (limit {max})")]
    TooDeep {
        /// The limit that was exceeded.
        max: u32,
    },
    /// The key contains more AST nodes than [`MAX_LOCK_NODES`].
    #[error("lock expression is too large (limit {max} nodes)")]
    TooLarge {
        /// The limit that was exceeded.
        max: u32,
    },
    /// The key contains more indirect (`@obj/lock`) references than
    /// [`MAX_LOCK_INDIRECTS`].
    #[error("lock expression has too many indirect references (limit {max})")]
    TooManyIndirects {
        /// The limit that was exceeded.
        max: u32,
    },
    /// An attribute lock glob pattern exceeds [`MAX_LOCK_GLOB_LEN`].
    #[error("attribute lock glob pattern is too long (limit {max} bytes)")]
    GlobTooLong {
        /// The limit that was exceeded.
        max: usize,
    },
}

/// Maximum recursion budget the parser will spend on nested `(` and `!`.
///
/// This is a **stack** bound, and only that. Each nested `(` costs 4 (the
/// `parse_primary` → `parse_or` → `parse_and` → `parse_unary` descent it
/// re-enters) and each `!` costs 1 (`parse_unary`'s self-call), so a key may
/// nest 16 parentheses or 64 negations before [`parse_lock`] refuses with
/// [`LockParseError::TooDeep`]. Without it `("(".repeat(n))` walks the guard
/// page and the process dies of SIGSEGV — not an error a command handler can
/// turn into "I don't understand that key".
///
/// It does **not** bound the produced tree: parentheses are transparent (they
/// yield the inner expression, not a node) and `a & b & c …` is parsed by an
/// O(1)-stack loop, so `#1&#1&…` never touches this counter at all. That is
/// [`MAX_LOCK_EXPR_DEPTH`]'s job.
pub const MAX_LOCK_PARSE_FRAMES: u32 = 64;

/// Maximum height of the [`LockExpr`] tree [`parse_lock`] will build.
///
/// An **AST** bound, complementary to [`MAX_LOCK_PARSE_FRAMES`]: leaves are
/// depth 1, `Not(e)` is `1 + depth(e)` and `And`/`Or` are `1 + max(children)`.
/// The `&`/`|` loops keep this running maximum in O(1) as they left-nest, so
/// the input class that evades a frame counter entirely — a flat operator
/// chain thousands of terms long, which fits inside one 8192-character input
/// line — is refused here instead of being stored as a tree that later
/// recurses in `Display`, in `eval_lock`, in serde, and in `Drop`.
///
/// 48 is sized against serde_json's recursion limit of 128: an `And`/`Or`
/// node costs two JSON levels (`{"And":[…]}`), so a maximal tree serializes
/// about 101 levels deep inside the object/`locks` wrapper, with headroom. No
/// hand-written lock comes close — the deepest key in the tree or the docs is
/// a handful of terms — so a refusal here is a refusal of an attack.
pub const MAX_LOCK_EXPR_DEPTH: u32 = 48;

/// Maximum total AST nodes (leaves + operators) [`parse_lock`] will build.
///
/// Complementary to [`MAX_LOCK_EXPR_DEPTH`]: a balanced tree of depth 8 stays
/// far under depth 48 while containing hundreds of nodes. Capping total AST
/// size bounds memory, serialization, and evaluation work for every parsed key.
pub const MAX_LOCK_NODES: u32 = 128;

/// Maximum indirect (`@obj/lock`) leaves [`parse_lock`] will accept in one
/// lock expression. Bounds indirection fan-out at parse time alongside
/// [`MAX_LOCK_STEPS`] and cycle detection in [`eval_lock`].
pub const MAX_LOCK_INDIRECTS: u32 = 16;

/// Parse TinyMUX-ish lock syntax into a [`LockExpr`].
///
/// Names are **not** resolvable at parse time by this crate (that needs
/// matchit/world context), so the caller supplies `resolve`, which maps a
/// name to an [`ObjId`]. The parser hands it:
/// - the literal string `"me"` for the keyword `me` (the caller decides who
///   that is),
/// - the name with its `*` stripped for `*player` references,
/// - any other bare word that is not a `#dbref`.
///
/// Returning `None` from the resolver fails the parse with
/// [`LockParseError::UnknownName`].
///
/// ```
/// use mush_core::lock::{parse_lock, LockExpr};
/// use mush_core::obj::ObjId;
/// let e = parse_lock("me | flag^WIZARD", |name| {
///     (name == "me").then_some(ObjId(42))
/// })
/// .unwrap();
/// assert_eq!(e.to_string(), "#42 | flag^WIZARD");
/// ```
pub fn parse_lock<R>(src: &str, resolve: R) -> Result<LockExpr, LockParseError>
where
    R: FnMut(&str) -> Option<ObjId>,
{
    let mut p = Parser {
        src,
        pos: 0,
        frames: 0,
        nodes: 0,
        indirects: 0,
        resolve,
    };
    let (expr, depth) = p.parse_or()?;
    p.skip_ws();
    if p.pos < p.src.len() {
        return Err(LockParseError::Trailing(p.pos));
    }
    // The loops in `parse_or`/`parse_and` already refuse at the bound; this
    // recheck is the one that holds even if a future arm forgets to.
    if depth > MAX_LOCK_EXPR_DEPTH {
        return Err(LockParseError::TooDeep {
            max: MAX_LOCK_EXPR_DEPTH,
        });
    }
    Ok(expr)
}

struct Parser<'a, R> {
    src: &'a str,
    pos: usize,
    /// Recursion budget spent, in stack frames — see
    /// [`MAX_LOCK_PARSE_FRAMES`]. Charged by `enter` and returned by `leave`
    /// on **every** path out of the recursive arms, error paths included, so
    /// a failed sub-parse cannot make a later sibling refuse spuriously.
    frames: u32,
    /// Total AST nodes built so far — bounded by [`MAX_LOCK_NODES`].
    nodes: u32,
    /// Total `Indirect` leaves built so far — bounded by [`MAX_LOCK_INDIRECTS`].
    indirects: u32,
    resolve: R,
}

/// The depth of a node built from two sub-results, refusing at the bound.
///
/// Called once per `&`/`|` iteration and once per `!`, which is what keeps
/// the running height O(1) to maintain no matter how long the chain is.
fn combine_depth(a: u32, b: u32) -> Result<u32, LockParseError> {
    let d = a.max(b).saturating_add(1);
    if d > MAX_LOCK_EXPR_DEPTH {
        return Err(LockParseError::TooDeep {
            max: MAX_LOCK_EXPR_DEPTH,
        });
    }
    Ok(d)
}

impl<R: FnMut(&str) -> Option<ObjId>> Parser<'_, R> {
    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn bump(&mut self) {
        if let Some(c) = self.peek() {
            self.pos += c.len_utf8();
        }
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace()) {
            self.bump();
        }
    }

    /// Charge `cost` stack frames, refusing rather than descending past
    /// [`MAX_LOCK_PARSE_FRAMES`]. Nothing is charged when it refuses, so the
    /// counter stays exact.
    fn enter(&mut self, cost: u32) -> Result<(), LockParseError> {
        if self.frames + cost > MAX_LOCK_PARSE_FRAMES {
            return Err(LockParseError::TooDeep {
                max: MAX_LOCK_PARSE_FRAMES,
            });
        }
        self.frames += cost;
        Ok(())
    }

    /// Return frames charged by a matching `enter`.
    fn leave(&mut self, cost: u32) {
        self.frames = self.frames.saturating_sub(cost);
    }

    /// Count one AST node, refusing past [`MAX_LOCK_NODES`].
    fn count_node(&mut self) -> Result<(), LockParseError> {
        let next = self.nodes.saturating_add(1);
        if next > MAX_LOCK_NODES {
            return Err(LockParseError::TooLarge {
                max: MAX_LOCK_NODES,
            });
        }
        self.nodes = next;
        Ok(())
    }

    /// Count one indirect (`@obj/lock`) leaf, refusing past [`MAX_LOCK_INDIRECTS`].
    fn count_indirect(&mut self) -> Result<(), LockParseError> {
        let next = self.indirects.saturating_add(1);
        if next > MAX_LOCK_INDIRECTS {
            return Err(LockParseError::TooManyIndirects {
                max: MAX_LOCK_INDIRECTS,
            });
        }
        self.indirects = next;
        Ok(())
    }

    /// Each parser fn returns its expression **and** that expression's depth,
    /// so the iterative `&`/`|` loops below can hold the tree height without
    /// a second walk (and without any stack of their own).
    fn parse_or(&mut self) -> Result<(LockExpr, u32), LockParseError> {
        let (mut left, mut depth) = self.parse_and()?;
        loop {
            self.skip_ws();
            if self.peek() == Some('|') {
                self.bump();
                let (right, rd) = self.parse_and()?;
                depth = combine_depth(depth, rd)?;
                self.count_node()?;
                left = LockExpr::Or(Box::new(left), Box::new(right));
            } else {
                return Ok((left, depth));
            }
        }
    }

    fn parse_and(&mut self) -> Result<(LockExpr, u32), LockParseError> {
        let (mut left, mut depth) = self.parse_unary()?;
        loop {
            self.skip_ws();
            if self.peek() == Some('&') {
                self.bump();
                let (right, rd) = self.parse_unary()?;
                depth = combine_depth(depth, rd)?;
                self.count_node()?;
                left = LockExpr::And(Box::new(left), Box::new(right));
            } else {
                return Ok((left, depth));
            }
        }
    }

    fn parse_unary(&mut self) -> Result<(LockExpr, u32), LockParseError> {
        self.skip_ws();
        if self.peek() == Some('!') {
            self.bump();
            // One frame: this is a genuine self-call.
            self.enter(1)?;
            let inner = self.parse_unary();
            self.leave(1);
            let (expr, d) = inner?;
            let depth = combine_depth(d, 0)?;
            self.count_node()?;
            Ok((LockExpr::Not(Box::new(expr)), depth))
        } else {
            self.parse_primary()
        }
    }

    fn parse_primary(&mut self) -> Result<(LockExpr, u32), LockParseError> {
        self.skip_ws();
        if self.peek() == Some('(') {
            self.bump();
            // Four frames: re-entering `parse_or` re-enters the whole
            // precedence descent down to `parse_primary`.
            self.enter(4)?;
            let inner = self.parse_or();
            self.leave(4);
            let (expr, depth) = inner?;
            self.skip_ws();
            if self.peek() == Some(')') {
                self.bump();
                // Parentheses are transparent: no node, so no depth either.
                Ok((expr, depth))
            } else {
                Err(LockParseError::UnbalancedParen)
            }
        } else {
            self.parse_term()
        }
    }

    /// A term runs until an operator/paren; spaces inside are allowed
    /// (player names may contain them) and trimmed at the edges.
    fn parse_term(&mut self) -> Result<(LockExpr, u32), LockParseError> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if matches!(c, '&' | '|' | '(' | ')') {
                break;
            }
            self.bump();
        }
        let raw = self.src[start..self.pos].trim();
        if raw.is_empty() {
            return Err(if self.pos >= self.src.len() {
                LockParseError::UnexpectedEnd
            } else {
                LockParseError::EmptyTerm
            });
        }
        // Every leaf is depth 1 and costs 1 AST node.
        let expr = self.classify(raw)?;
        self.count_node()?;
        Ok((expr, 1))
    }

    /// `#123` directly; anything else (with optional `*`) via the resolver.
    fn objref(&mut self, s: &str) -> Result<ObjId, LockParseError> {
        let s = s.trim();
        if let Some(num) = s.strip_prefix('#') {
            return num
                .parse::<u32>()
                .map(ObjId)
                .map_err(|_| LockParseError::BadObjRef(s.to_string()));
        }
        let name = s.strip_prefix('*').unwrap_or(s).trim();
        if name.is_empty() {
            return Err(LockParseError::BadObjRef(s.to_string()));
        }
        (self.resolve)(name).ok_or_else(|| LockParseError::UnknownName(name.to_string()))
    }

    fn classify(&mut self, raw: &str) -> Result<LockExpr, LockParseError> {
        if raw.eq_ignore_ascii_case("#true") {
            return Ok(LockExpr::Const(true));
        }
        if raw.eq_ignore_ascii_case("#false") {
            return Ok(LockExpr::Const(false));
        }
        if let Some(rest) = raw.strip_prefix('+') {
            return Ok(LockExpr::Carry(self.objref(rest)?));
        }
        if let Some(rest) = raw.strip_prefix('$') {
            return Ok(LockExpr::OwnedBy(self.objref(rest)?));
        }
        if let Some(rest) = raw.strip_prefix('@') {
            let (obj_part, lt) = match rest.split_once('/') {
                Some((o, t)) => (o, t.trim().parse::<LockType>()?),
                None => (rest, LockType::Basic),
            };
            let id = self.objref(obj_part)?;
            self.count_indirect()?;
            return Ok(LockExpr::Indirect(id, lt));
        }
        if let Some((left, right)) = raw.split_once('^') {
            let name = right.trim();
            if !left.trim().eq_ignore_ascii_case("flag") || name.is_empty() {
                return Err(LockParseError::BadFlagLock(raw.to_string()));
            }
            return Ok(LockExpr::HasFlag(SmolStr::new(name)));
        }
        if let Some((name, glob)) = raw.rsplit_once(':') {
            let name = name.trim();
            if name.is_empty() {
                return Err(LockParseError::EmptyTerm);
            }
            let glob = glob.trim();
            if glob.len() > MAX_LOCK_GLOB_LEN {
                return Err(LockParseError::GlobTooLong {
                    max: MAX_LOCK_GLOB_LEN,
                });
            }
            return Ok(LockExpr::AttrMatch {
                name: UncasedName::from(name),
                glob: glob.to_string(),
            });
        }
        Ok(LockExpr::Is(self.objref(raw)?))
    }
}

impl LockExpr {
    /// A left-nested `&` chain `depth` expression nodes tall: the shape
    /// `@channel/perm <chan>=join=#1&#1&…` builds, and the one input class a
    /// frame counter cannot see. **Test support only.** It is the single
    /// compound-node builder outside `parse_lock`, kept here so the
    /// construction ratchet (`tests/lock_construction_ratchet.rs`) stays exact:
    /// tests that plant over-deep fixtures call this instead of naming the
    /// variants themselves. It is deliberately unbounded, because those tests
    /// exist to exceed the bound; production code must never call it.
    #[doc(hidden)]
    #[must_use]
    pub fn and_chain(depth: u32) -> Self {
        let mut expr = Self::Const(true);
        for _ in 1..depth {
            expr = Self::And(Box::new(expr), Box::new(Self::Const(true)));
        }
        expr
    }

    /// Height of this tree: leaves are 1, `Not` is `1 + child`, `And`/`Or`
    /// are `1 + max(children)`.
    ///
    /// Walked with an explicit worklist rather than by recursion, because the
    /// whole point of the measurement is to be safe on a tree that did *not*
    /// come from [`parse_lock`] — one deserialized from Redis, or built by a
    /// module — where nothing has yet established that it is under
    /// [`MAX_LOCK_EXPR_DEPTH`].
    ///
    /// ```
    /// use mush_core::lock::parse_lock;
    /// use mush_core::obj::ObjId;
    /// let e = parse_lock("#1 & !(#2 | #3)", |_| Some(ObjId(1))).unwrap();
    /// assert_eq!(e.depth(), 4);
    /// ```
    #[must_use]
    pub fn depth(&self) -> u32 {
        /// One step of the walk: visit a node, or fold `n` finished children.
        enum Step<'a> {
            Visit(&'a LockExpr),
            Fold(u8),
        }
        let mut work = vec![Step::Visit(self)];
        let mut done: Vec<u32> = Vec::new();
        while let Some(step) = work.pop() {
            match step {
                Step::Visit(e) => match e {
                    LockExpr::Not(a) => {
                        work.push(Step::Fold(1));
                        work.push(Step::Visit(a));
                    }
                    LockExpr::And(a, b) | LockExpr::Or(a, b) => {
                        work.push(Step::Fold(2));
                        work.push(Step::Visit(a));
                        work.push(Step::Visit(b));
                    }
                    _ => done.push(1),
                },
                Step::Fold(n) => {
                    let mut max = 0u32;
                    for _ in 0..n {
                        max = max.max(done.pop().unwrap_or(0));
                    }
                    done.push(max.saturating_add(1));
                }
            }
        }
        done.pop().unwrap_or(1)
    }

    /// Truncate this tree to at most `max` levels, failing closed: every
    /// compound node at or below level `max` (the root is level 1) is replaced
    /// by `#FALSE`, and the number of replacements is returned.
    ///
    /// This is the repair the decode path needs. `depth()` only *measures* an
    /// unvetted tree; a store that hands back something taller than
    /// [`MAX_LOCK_EXPR_DEPTH`] has to do something with it, and the choice
    /// this crate makes is to keep the object loadable and make the
    /// over-deep part unpassable rather than to fail the whole load.
    ///
    /// Walked with an explicit worklist, for the same reason [`depth`] is: the
    /// tree being repaired is by definition one nothing has vetted.
    ///
    /// Note what "fail closed" means and what it does not: the *subtree* is
    /// replaced by `#FALSE`, not the expression. `!` above the cut still
    /// inverts it, exactly as `!#FALSE` does anywhere else. Truncation changes
    /// the meaning of the key — that is unavoidable — it just refuses to
    /// invent a passing leaf.
    ///
    /// ```
    /// use mush_core::lock::parse_lock;
    /// use mush_core::obj::ObjId;
    /// let mut e = parse_lock("#1 & (#2 | #3)", |_| Some(ObjId(1))).unwrap();
    /// assert_eq!(e.depth(), 3);
    /// assert_eq!(e.clamp_depth(2), 1);
    /// assert_eq!(e.to_string(), "#1 & #FALSE");
    /// ```
    ///
    /// [`depth`]: LockExpr::depth
    pub fn clamp_depth(&mut self, max: u32) -> u32 {
        let mut clamped = 0u32;
        let mut work: Vec<(&mut LockExpr, u32)> = vec![(self, 1)];
        while let Some((node, level)) = work.pop() {
            if compound(node) && level >= max {
                // Dropping the old subtree here is safe however tall it is:
                // `Drop for LockExpr` dismantles iteratively.
                *node = LockExpr::Const(false);
                clamped = clamped.saturating_add(1);
                continue;
            }
            match node {
                LockExpr::Not(a) => work.push((a.as_mut(), level.saturating_add(1))),
                LockExpr::And(a, b) | LockExpr::Or(a, b) => {
                    work.push((a.as_mut(), level.saturating_add(1)));
                    work.push((b.as_mut(), level.saturating_add(1)));
                }
                _ => {}
            }
        }
        clamped
    }

    /// Binding strength for the printer: `|` = 1, `&` = 2, `!` = 3, atoms = 4.
    fn prec(&self) -> u8 {
        match self {
            LockExpr::Or(..) => 1,
            LockExpr::And(..) => 2,
            LockExpr::Not(..) => 3,
            _ => 4,
        }
    }

    /// Precedence-aware printing; parenthesizes whenever this node binds
    /// looser than its context requires, and forces parens on right-nested
    /// same-precedence children so the (left-associative) parser rebuilds
    /// the identical tree.
    ///
    /// `budget` is the structural fence (`MAX_LOCK_STRUCTURAL_DEPTH`): the
    /// printer recurses, and `Display` is reached from log lines and from
    /// `@examine` on trees the parser never saw, so past the budget it writes
    /// `#FALSE` and stops rather than descending. The budget sits far above
    /// anything serde can decode, so this never fires on a real key and the
    /// `parse(print(e)) == e` invariant is untouched.
    fn fmt_prec(&self, f: &mut fmt::Formatter<'_>, min: u8, budget: u32) -> fmt::Result {
        if budget == 0 {
            return f.write_str("#FALSE");
        }
        let budget = budget - 1;
        let paren = self.prec() < min;
        if paren {
            f.write_str("(")?;
        }
        match self {
            LockExpr::Const(true) => f.write_str("#TRUE")?,
            LockExpr::Const(false) => f.write_str("#FALSE")?,
            LockExpr::Is(id) => write!(f, "{id}")?,
            LockExpr::Carry(id) => write!(f, "+{id}")?,
            LockExpr::OwnedBy(id) => write!(f, "${id}")?,
            LockExpr::HasFlag(n) => write!(f, "flag^{n}")?,
            LockExpr::AttrMatch { name, glob } => write!(f, "{name}:{glob}")?,
            LockExpr::Indirect(id, lt) => write!(f, "@{id}/{lt}")?,
            LockExpr::Not(e) => {
                f.write_str("!")?;
                e.fmt_prec(f, 3, budget)?;
            }
            LockExpr::And(a, b) => {
                a.fmt_prec(f, 2, budget)?;
                f.write_str(" & ")?;
                b.fmt_prec(f, 3, budget)?;
            }
            LockExpr::Or(a, b) => {
                a.fmt_prec(f, 1, budget)?;
                f.write_str(" | ")?;
                b.fmt_prec(f, 2, budget)?;
            }
        }
        if paren {
            f.write_str(")")?;
        }
        Ok(())
    }
}

impl fmt::Display for LockExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_prec(f, 0, MAX_LOCK_STRUCTURAL_DEPTH)
    }
}

/// Dismantle a tree iteratively.
///
/// The compiler's own drop glue is recursive — freeing `Not(Not(Not(…)))`
/// re-enters itself once per level — so the last stack overflow left in this module is the one
/// that happens *after* a deep tree is already in RAM, where no parse bound
/// can help: overwrite an over-deep lock, or let its object fall out of the
/// world, and the free walks the guard page and kills the world task. This
/// impl replaces each child with `#FALSE` and pushes the child onto a heap
/// worklist instead.
///
/// The nodes this loop drops have had their compound children taken already,
/// so their own `drop` hits the `deep` guard below and returns without
/// allocating: the recursion is two frames deep, not `n`.
///
/// The cost of the impl is that `LockExpr`'s variants can never be
/// destructured by move (`let LockExpr::Not(inner) = e else { … }`); nothing
/// in the workspace does, and `mem::replace` on a `&mut` is the substitute.
impl Drop for LockExpr {
    fn drop(&mut self) {
        /// Move `node`'s compound children onto `work`, leaving `#FALSE`s.
        fn dismantle(node: &mut LockExpr, work: &mut Vec<LockExpr>) {
            let take = |child: &mut Box<LockExpr>, work: &mut Vec<LockExpr>| {
                if compound(child) {
                    work.push(std::mem::replace(child.as_mut(), LockExpr::Const(false)));
                }
            };
            match node {
                LockExpr::Not(a) => take(a, work),
                LockExpr::And(a, b) | LockExpr::Or(a, b) => {
                    take(a, work);
                    take(b, work);
                }
                _ => {}
            }
        }

        let deep = match &*self {
            LockExpr::Not(a) => compound(a),
            LockExpr::And(a, b) | LockExpr::Or(a, b) => compound(a) || compound(b),
            _ => false,
        };
        if !deep {
            return;
        }
        let mut work: Vec<LockExpr> = Vec::new();
        dismantle(self, &mut work);
        while let Some(mut node) = work.pop() {
            dismantle(&mut node, &mut work);
            // `node` drops here with only `#FALSE` children: shallow.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attr::Attr;
    use crate::obj::{GOD, LIMBO, ObjKind};
    use crate::object_read::PlainRead;

    /// Resolver used by parser-only tests: me → #2, Nick → #2, Bob → #7.
    fn fake_resolver(name: &str) -> Option<ObjId> {
        match name {
            "me" | "Nick" => Some(ObjId(2)),
            "Bob" => Some(ObjId(7)),
            _ => None,
        }
    }

    fn round_trips(src: &str) -> LockExpr {
        let e = parse_lock(src, fake_resolver).unwrap();
        let printed = e.to_string();
        let again = parse_lock(&printed, fake_resolver)
            .unwrap_or_else(|err| panic!("reparse of {printed:?} failed: {err}"));
        assert_eq!(
            again, e,
            "round trip changed AST for {src:?} -> {printed:?}"
        );
        e
    }

    #[test]
    fn parse_simple_terms() {
        assert_eq!(
            parse_lock("#5", fake_resolver).unwrap(),
            LockExpr::Is(ObjId(5))
        );
        assert_eq!(
            parse_lock("me", fake_resolver).unwrap(),
            LockExpr::Is(ObjId(2))
        );
        assert_eq!(
            parse_lock("*Nick", fake_resolver).unwrap(),
            LockExpr::Is(ObjId(2))
        );
        assert_eq!(
            parse_lock("Bob", fake_resolver).unwrap(),
            LockExpr::Is(ObjId(7))
        );
        assert_eq!(
            parse_lock("+#5", fake_resolver).unwrap(),
            LockExpr::Carry(ObjId(5))
        );
        assert_eq!(
            parse_lock("+*Bob", fake_resolver).unwrap(),
            LockExpr::Carry(ObjId(7))
        );
        assert_eq!(
            parse_lock("$#7", fake_resolver).unwrap(),
            LockExpr::OwnedBy(ObjId(7))
        );
        assert_eq!(
            parse_lock("#TRUE", fake_resolver).unwrap(),
            LockExpr::Const(true)
        );
        assert_eq!(
            parse_lock("#false", fake_resolver).unwrap(),
            LockExpr::Const(false)
        );
        assert_eq!(
            parse_lock("flag^WIZARD", fake_resolver).unwrap(),
            LockExpr::HasFlag(SmolStr::new("WIZARD"))
        );
        assert_eq!(
            parse_lock("@#3/enter", fake_resolver).unwrap(),
            LockExpr::Indirect(ObjId(3), LockType::Enter)
        );
        // Indirect without an explicit type defaults to Basic.
        assert_eq!(
            parse_lock("@#3", fake_resolver).unwrap(),
            LockExpr::Indirect(ObjId(3), LockType::Basic)
        );
        assert_eq!(
            parse_lock("sex:m*le", fake_resolver).unwrap(),
            LockExpr::AttrMatch {
                name: UncasedName::from("sex"),
                glob: "m*le".into()
            }
        );
        // Namespaced attr: split at the LAST colon.
        assert_eq!(
            parse_lock("wod:clan:V*", fake_resolver).unwrap(),
            LockExpr::AttrMatch {
                name: UncasedName::from("wod:clan"),
                glob: "V*".into()
            }
        );
    }

    #[test]
    fn parse_precedence_not_over_and_over_or() {
        // a | b & c  ==  a | (b & c)
        let e = parse_lock("#1 | #2 & #3", fake_resolver).unwrap();
        assert_eq!(
            e,
            LockExpr::Or(
                Box::new(LockExpr::Is(ObjId(1))),
                Box::new(LockExpr::And(
                    Box::new(LockExpr::Is(ObjId(2))),
                    Box::new(LockExpr::Is(ObjId(3)))
                ))
            )
        );
        // !a & b  ==  (!a) & b
        let e = parse_lock("!#1 & #2", fake_resolver).unwrap();
        assert_eq!(
            e,
            LockExpr::And(
                Box::new(LockExpr::Not(Box::new(LockExpr::Is(ObjId(1))))),
                Box::new(LockExpr::Is(ObjId(2)))
            )
        );
        // Parens override.
        let e = parse_lock("(#1 | #2) & #3", fake_resolver).unwrap();
        assert_eq!(
            e,
            LockExpr::And(
                Box::new(LockExpr::Or(
                    Box::new(LockExpr::Is(ObjId(1))),
                    Box::new(LockExpr::Is(ObjId(2)))
                )),
                Box::new(LockExpr::Is(ObjId(3)))
            )
        );
    }

    #[test]
    fn parse_errors() {
        assert!(matches!(
            parse_lock("(#1 | #2", fake_resolver),
            Err(LockParseError::UnbalancedParen)
        ));
        assert!(matches!(
            parse_lock("#1 &", fake_resolver),
            Err(LockParseError::UnexpectedEnd)
        ));
        assert!(matches!(
            parse_lock("#1 | #2)", fake_resolver),
            Err(LockParseError::Trailing(_))
        ));
        // Spaces are allowed INSIDE a term (player names), so "#1 #2" is one
        // bad object reference, not two terms.
        assert!(matches!(
            parse_lock("#1 #2", fake_resolver),
            Err(LockParseError::BadObjRef(_))
        ));
        assert!(matches!(
            parse_lock("nobody", fake_resolver),
            Err(LockParseError::UnknownName(n)) if n == "nobody"
        ));
        assert!(matches!(
            parse_lock("@#3/xyzzy", fake_resolver),
            Err(LockParseError::UnknownLockType(_))
        ));
        assert!(matches!(
            parse_lock("power^BOOT", fake_resolver),
            Err(LockParseError::BadFlagLock(_))
        ));
        assert!(matches!(
            parse_lock("#12x", fake_resolver),
            Err(LockParseError::BadObjRef(_))
        ));
        assert!(matches!(
            parse_lock("", fake_resolver),
            Err(LockParseError::UnexpectedEnd)
        ));
    }

    #[test]
    fn print_round_trip_compound() {
        let e = round_trips("me & (+#5 | $#7) & !flag^WIZARD & sex:m* & @#3/enter");
        assert_eq!(
            e.to_string(),
            "#2 & (+#5 | $#7) & !flag^WIZARD & sex:m* & @#3/enter"
        );
        round_trips("!(#2 | #3) & flag^DARK");
        round_trips("#TRUE | #FALSE");
        round_trips("!!#4");
        round_trips("wod:clan:Ven* | @#0/zone");
        round_trips("+#1 & ($#2 | !(+#3 & #4))");
    }

    #[test]
    fn print_round_trip_right_nested_trees() {
        // Right-nested same-op trees must print with parens to survive the
        // left-associative parser.
        let e = LockExpr::Or(
            Box::new(LockExpr::Is(ObjId(1))),
            Box::new(LockExpr::Or(
                Box::new(LockExpr::Is(ObjId(2))),
                Box::new(LockExpr::Is(ObjId(3))),
            )),
        );
        assert_eq!(e.to_string(), "#1 | (#2 | #3)");
        assert_eq!(parse_lock(&e.to_string(), fake_resolver).unwrap(), e);

        let e = LockExpr::And(
            Box::new(LockExpr::And(
                Box::new(LockExpr::Is(ObjId(1))),
                Box::new(LockExpr::Is(ObjId(2))),
            )),
            Box::new(LockExpr::Is(ObjId(3))),
        );
        // Left-nested prints flat and reparses identically.
        assert_eq!(e.to_string(), "#1 & #2 & #3");
        assert_eq!(parse_lock(&e.to_string(), fake_resolver).unwrap(), e);
    }

    #[test]
    fn glob_matcher() {
        assert!(glob_match("", ""));
        assert!(glob_match("*", ""));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("a?c", "abc"));
        assert!(!glob_match("a?c", "ac"));
        assert!(glob_match("a*c", "abbbbc"));
        assert!(glob_match("a*c", "ac"));
        assert!(!glob_match("a*c", "ab"));
        assert!(glob_match("*end", "the end"));
        assert!(glob_match("start*", "start of it"));
        assert!(glob_match("m*l?", "MALE"));
        assert!(!glob_match("no", "yes"));
        assert!(glob_match("a*b*c", "azzbzzc"));
        assert!(!glob_match("a*b*c", "azzczzb"));
    }

    /// Build a world for eval tests:
    /// #2 Nick (player, in Limbo, marker wod:kindred, sex=male),
    /// #3 sword (thing, carried by Nick),
    /// #4 door (thing with Enter lock = Is(Nick), Basic lock = self-indirect).
    fn eval_world() -> (World, ObjId, ObjId, ObjId) {
        let mut w = World::new_minimal();
        let nick = w.create(ObjKind::Player, "Nick", GOD, Some(LIMBO)).unwrap();
        let sword = w.create(ObjKind::Thing, "sword", GOD, Some(nick)).unwrap();
        let door = w.create(ObjKind::Thing, "door", GOD, Some(LIMBO)).unwrap();
        w.get_mut(nick)
            .unwrap()
            .markers
            .insert(SmolStr::new("wod:kindred"));
        w.set_attr(nick, "sex", Attr::text("male", GOD)).unwrap();
        w.get_mut(door)
            .unwrap()
            .locks
            .insert(LockType::Enter, LockExpr::Is(nick));
        w.get_mut(door)
            .unwrap()
            .locks
            .insert(LockType::Basic, LockExpr::Indirect(door, LockType::Basic));
        (w, nick, sword, door)
    }

    #[test]
    fn eval_truth_table() {
        let (w, nick, sword, door) = eval_world();
        let d = MAX_LOCK_DEPTH;

        // Const / Is
        assert!(eval_lock(&w, &LockExpr::Const(true), nick, d));
        assert!(!eval_lock(&w, &LockExpr::Const(false), nick, d));
        assert!(eval_lock(&w, &LockExpr::Is(nick), nick, d));
        assert!(!eval_lock(&w, &LockExpr::Is(nick), GOD, d));

        // Carry: Nick holds the sword, God does not.
        assert!(eval_lock(&w, &LockExpr::Carry(sword), nick, d));
        assert!(!eval_lock(&w, &LockExpr::Carry(sword), GOD, d));

        // OwnedBy: players own themselves.
        assert!(eval_lock(&w, &LockExpr::OwnedBy(nick), nick, d));
        assert!(!eval_lock(&w, &LockExpr::OwnedBy(GOD), nick, d));
        // The sword was created by (and is owned by) God.
        assert!(eval_lock(&w, &LockExpr::OwnedBy(GOD), sword, d));

        // HasFlag: core flag (God is WIZARD in new_minimal), marker flag,
        // case-insensitive both ways.
        assert!(eval_lock(&w, &LockExpr::HasFlag("WIZARD".into()), GOD, d));
        assert!(eval_lock(&w, &LockExpr::HasFlag("wizard".into()), GOD, d));
        assert!(!eval_lock(&w, &LockExpr::HasFlag("WIZARD".into()), nick, d));
        assert!(eval_lock(
            &w,
            &LockExpr::HasFlag("wod:kindred".into()),
            nick,
            d
        ));
        assert!(eval_lock(
            &w,
            &LockExpr::HasFlag("WOD:KINDRED".into()),
            nick,
            d
        ));
        assert!(!eval_lock(
            &w,
            &LockExpr::HasFlag("wod:kindred".into()),
            GOD,
            d
        ));
        assert!(!eval_lock(&w, &LockExpr::HasFlag("nosuch".into()), nick, d));

        // AttrMatch: glob over Nick's sex attribute.
        let sex_male = LockExpr::AttrMatch {
            name: "sex".into(),
            glob: "m*le".into(),
        };
        let sex_f = LockExpr::AttrMatch {
            name: "SEX".into(),
            glob: "f*".into(),
        };
        assert!(eval_lock(&w, &sex_male, nick, d));
        assert!(!eval_lock(&w, &sex_f, nick, d));
        assert!(!eval_lock(&w, &sex_male, GOD, d)); // no attr at all

        // Indirect: door's Enter lock admits Nick only.
        let via_door = LockExpr::Indirect(door, LockType::Enter);
        assert!(eval_lock(&w, &via_door, nick, d));
        assert!(!eval_lock(&w, &via_door, GOD, d));
        // Unset lock type on an existing object = unlocked.
        assert!(eval_lock(
            &w,
            &LockExpr::Indirect(door, LockType::Use),
            GOD,
            d
        ));
        // Missing object fails.
        assert!(!eval_lock(
            &w,
            &LockExpr::Indirect(ObjId(999), LockType::Basic),
            nick,
            d
        ));

        // Nesting.
        let complex = LockExpr::And(
            Box::new(LockExpr::Or(
                Box::new(LockExpr::HasFlag("WIZARD".into())),
                Box::new(LockExpr::Carry(sword)),
            )),
            Box::new(LockExpr::Not(Box::new(LockExpr::Is(GOD)))),
        );
        assert!(eval_lock(&w, &complex, nick, d)); // carries sword, not God
        assert!(!eval_lock(&w, &complex, GOD, d)); // wizard but IS God
    }

    #[test]
    fn eval_indirect_cycle_terminates_false() {
        let (w, nick, _sword, door) = eval_world();
        // door's Basic lock points at itself: must terminate, and fail closed.
        let e = LockExpr::Indirect(door, LockType::Basic);
        assert!(!eval_lock(&w, &e, nick, MAX_LOCK_DEPTH));
        // Depth 0 fails immediately even on a healthy indirect.
        assert!(!eval_lock(
            &w,
            &LockExpr::Indirect(door, LockType::Enter),
            nick,
            0
        ));
    }

    #[test]
    fn eval_attr_match_sees_inherited_attrs() {
        let (mut w, nick, _sword, _door) = eval_world();
        let proto = w.create(ObjKind::Thing, "proto", GOD, Some(LIMBO)).unwrap();
        w.set_attr(proto, "species", Attr::text("vampire", GOD))
            .unwrap();
        w.set_parent(nick, Some(proto)).unwrap();
        let e = LockExpr::AttrMatch {
            name: "species".into(),
            glob: "vamp*".into(),
        };
        assert!(eval_lock(&w, &e, nick, MAX_LOCK_DEPTH));
    }

    #[test]
    fn locktype_display_from_str_round_trip() {
        for lt in [
            LockType::Basic,
            LockType::Enter,
            LockType::Use,
            LockType::Page,
            LockType::Teleport,
            LockType::Speech,
            LockType::Zone,
            LockType::Drop,
            LockType::Give,
            LockType::Mail,
        ] {
            assert_eq!(lt.to_string().parse::<LockType>().unwrap(), lt);
            assert_eq!(
                lt.to_string().to_uppercase().parse::<LockType>().unwrap(),
                lt
            );
        }
    }
    // ---------------------------------------------------------------
    // Depth bounds (WS01-S1). The three input classes that reach the
    // parser's two guards live in their own test binaries
    // (`tests/lock_parse_depth_*.rs`) because a regression there is a
    // stack overflow, which kills the process rather than failing an
    // assert. What is checked here is the *edge*: the tallest tree the
    // parser will still build, and the first one it will not.

    /// `#1 & #1 & …` with `n` operators — depth `n + 1`, left-nested.
    fn and_chain(n: u32) -> String {
        let mut s = String::from("#1");
        for _ in 0..n {
            s.push_str(" & #1");
        }
        s
    }

    /// The `|` twin of [`and_chain`].
    fn or_chain(n: u32) -> String {
        let mut s = String::from("#1");
        for _ in 0..n {
            s.push_str(" | #1");
        }
        s
    }

    /// `!!!…#1` with `n` negations — depth `n + 1`.
    fn bang_chain(n: u32) -> String {
        let mut s = "!".repeat(n as usize);
        s.push_str("#1");
        s
    }

    #[test]
    fn chains_at_exactly_max_depth_parse_and_round_trip() {
        let max = MAX_LOCK_EXPR_DEPTH;
        for src in [and_chain(max - 1), or_chain(max - 1), bang_chain(max - 1)] {
            let e = round_trips(&src);
            assert_eq!(e.depth(), max, "expected a depth-{max} tree from {src:?}");
        }
    }

    #[test]
    fn chains_one_past_max_depth_are_refused() {
        let max = MAX_LOCK_EXPR_DEPTH;
        for src in [and_chain(max), or_chain(max), bang_chain(max)] {
            assert_eq!(
                parse_lock(&src, fake_resolver),
                Err(LockParseError::TooDeep { max }),
                "expected TooDeep for {src:?}"
            );
        }
    }

    #[test]
    fn depth_is_exact_for_each_chain_shape() {
        for n in [0, 1, 2, 7, MAX_LOCK_EXPR_DEPTH - 1] {
            for src in [and_chain(n), or_chain(n), bang_chain(n)] {
                let e = parse_lock(&src, fake_resolver).unwrap();
                assert_eq!(e.depth(), n + 1, "wrong depth for {src:?}");
            }
        }
        // Mixed shapes: depth is 1 + max(children), not a node count.
        let e = parse_lock("#1 & !(#2 | #3)", fake_resolver).unwrap();
        assert_eq!(e.depth(), 4);
        let e = parse_lock("(#1)", fake_resolver).unwrap();
        assert_eq!(e.depth(), 1, "parentheses build no node");
    }

    #[test]
    fn nested_parens_are_bounded_by_the_frame_guard() {
        // Parentheses cost 4 frames each and build no node, so it is the
        // frame counter — not the depth counter — that stops them.
        let ok = MAX_LOCK_PARSE_FRAMES as usize / 4;
        let src = format!("{}#1{}", "(".repeat(ok), ")".repeat(ok));
        assert_eq!(
            parse_lock(&src, fake_resolver).unwrap(),
            LockExpr::Is(ObjId(1))
        );
        let src = format!("{}#1{}", "(".repeat(ok + 1), ")".repeat(ok + 1));
        assert_eq!(
            parse_lock(&src, fake_resolver),
            Err(LockParseError::TooDeep {
                max: MAX_LOCK_PARSE_FRAMES
            })
        );
    }

    #[test]
    fn frames_are_returned_so_siblings_do_not_inherit_them() {
        // Sixteen sibling paren groups each spend a quarter of the budget
        // in turn; only a failure to decrement on the way out would make
        // the later ones refuse.
        let src = std::iter::repeat_n("(#1)", 16)
            .collect::<Vec<_>>()
            .join(" & ");
        let e = parse_lock(&src, fake_resolver).unwrap();
        assert_eq!(e.depth(), 16);
        // Same for the error path: a failed sub-parse inside parens must
        // not leak its frames into the next attempt.
        for _ in 0..8 {
            assert!(parse_lock("((#1)", fake_resolver).is_err());
        }
        assert!(parse_lock("((((#1))))", fake_resolver).is_ok());
    }

    #[test]
    fn depth_walks_a_tree_the_parser_would_have_refused() {
        // `depth()` is used on trees that did not come from the parser
        // (deserialized, module-built): it must not recurse. Dropping the
        // tree afterwards does not recurse either (WS01-S2); the taller
        // shapes live in `tests/lock_depth_*.rs`, on a small stack, where
        // an overflow names itself.
        let mut e = LockExpr::Is(ObjId(1));
        for _ in 0..2_000 {
            e = LockExpr::And(Box::new(e), Box::new(LockExpr::Const(true)));
        }
        assert_eq!(e.depth(), 2_001);
    }

    // ---------------------------------------------------------------
    // Structural budget (WS01-S2). The fence is 1024 levels, twenty times
    // the parser's own bound, precisely so it never fires on a key anyone
    // can write: these are the tallest legal shapes, and they must
    // evaluate and print exactly as they did before the budget existed.
    // The trees that *do* hit the budget are in `tests/lock_depth_eval.rs`.

    #[test]
    fn the_tallest_legal_chains_evaluate_and_round_trip_unchanged() {
        let (w, nick, _sword, _door) = eval_world();
        let me = nick.to_string();
        // 47 terms / 47 levels — one under MAX_LOCK_EXPR_DEPTH.
        let ands = std::iter::repeat_n(me.as_str(), 47)
            .collect::<Vec<_>>()
            .join(" & ");
        let bangs = format!("{}{me}", "!".repeat(46));

        let e = round_trips(&ands);
        assert_eq!(e.depth(), 47);
        assert!(
            eval_lock(&w, &e, nick, MAX_LOCK_DEPTH),
            "a 47-term & chain of `is me` passes for me"
        );
        let e = round_trips(&bangs);
        assert_eq!(e.depth(), 47);
        assert!(
            eval_lock(&w, &e, nick, MAX_LOCK_DEPTH),
            "46 negations is an even number of them"
        );
        // One negation fewer: the answer flips, so the budget is not
        // quietly swallowing the whole spine.
        let e = round_trips(&format!("{}{me}", "!".repeat(45)));
        assert!(!eval_lock(&w, &e, nick, MAX_LOCK_DEPTH));
    }

    #[test]
    fn an_indirect_chain_still_spends_the_indirect_budget_only() {
        // Five objects, each of whose basic lock is the next one's. The
        // structural budget is reseeded at every hop; what stops the walk
        // is MAX_LOCK_DEPTH, and five hops are well inside it.
        let mut w = World::new_minimal();
        let nick = w.create(ObjKind::Player, "Nick", GOD, Some(LIMBO)).unwrap();
        let chain: Vec<ObjId> = (0..5)
            .map(|i| {
                w.create(ObjKind::Thing, &format!("link{i}"), GOD, Some(LIMBO))
                    .unwrap()
            })
            .collect();
        for pair in chain.windows(2) {
            let (here, next) = (pair[0], pair[1]);
            w.get_mut(here)
                .unwrap()
                .locks
                .insert(LockType::Basic, LockExpr::Indirect(next, LockType::Basic));
        }
        // The last link has no basic lock, and an unset lock is unlocked.
        let e = LockExpr::Indirect(chain[0], LockType::Basic);
        assert!(eval_lock(&w, &e, nick, MAX_LOCK_DEPTH));
        // Four hops is one too few to reach the end of a five-link chain.
        assert!(!eval_lock(&w, &e, nick, 4));
        assert!(eval_lock(&w, &e, nick, 5));
    }

    #[test]
    fn parse_lock_caps_total_nodes_and_indirect_leaves() {
        // Exactly MAX_LOCK_INDIRECTS indirect leaves parses and round-trips.
        let at_limit = std::iter::repeat_n("@me/page", MAX_LOCK_INDIRECTS as usize)
            .collect::<Vec<_>>()
            .join(" | ");
        round_trips(&at_limit);

        // One more indirect leaf is refused at parse time.
        let over_indirects = std::iter::repeat_n("@me/page", MAX_LOCK_INDIRECTS as usize + 1)
            .collect::<Vec<_>>()
            .join(" | ");
        assert_eq!(
            parse_lock(&over_indirects, fake_resolver),
            Err(LockParseError::TooManyIndirects {
                max: MAX_LOCK_INDIRECTS,
            })
        );

        // A balanced tree of 64 leaves has 64 + 63 = 127 <= MAX_LOCK_NODES (128)
        // nodes and depth 7 <= MAX_LOCK_EXPR_DEPTH (48): accepted.
        fn balanced(leaves: usize) -> String {
            if leaves <= 1 {
                return "#1".to_string();
            }
            let left = leaves / 2;
            let right = leaves - left;
            format!("({} | {})", balanced(left), balanced(right))
        }
        assert!(parse_lock(&balanced(64), fake_resolver).is_ok());

        // 65 leaves = 65 + 64 = 129 > MAX_LOCK_NODES: refused as TooLarge.
        assert_eq!(
            parse_lock(&balanced(65), fake_resolver),
            Err(LockParseError::TooLarge {
                max: MAX_LOCK_NODES,
            })
        );
    }

    #[test]
    fn wide_self_referential_indirect_lock_terminates_fast_and_false() {
        let (mut w, nick, _sword, _door) = eval_world();
        // Construct a wide self-referential lock `@me/page | @me/page | ...`
        // right at the parser's indirect ceiling, AND a programmatic 64-wide
        // tree that bypasses parse limits (e.g., decoded from legacy storage).
        let src = std::iter::repeat_n("@me/page", MAX_LOCK_INDIRECTS as usize)
            .collect::<Vec<_>>()
            .join(" | ");
        let parsed = parse_lock(&src, |name| (name == "me").then_some(nick)).unwrap();
        w.get_mut(nick)
            .unwrap()
            .locks
            .insert(LockType::Page, parsed.clone());

        let start = std::time::Instant::now();
        assert!(!eval_lock(&w, &parsed, nick, MAX_LOCK_DEPTH));
        assert!(!eval_lock(
            &w,
            &LockExpr::Indirect(nick, LockType::Page),
            nick,
            MAX_LOCK_DEPTH
        ));
        assert!(
            start.elapsed() < std::time::Duration::from_millis(200),
            "wide self-referential indirect lock must not explode exponentially: {:?}",
            start.elapsed()
        );

        // Even an unparsed 64-wide self-referential lock (64^10 without bounds)
        // terminates almost instantly and fails closed.
        let mut wide = LockExpr::Indirect(nick, LockType::Page);
        for _ in 1..64 {
            wide = LockExpr::Or(
                Box::new(wide),
                Box::new(LockExpr::Indirect(nick, LockType::Page)),
            );
        }
        w.get_mut(nick)
            .unwrap()
            .locks
            .insert(LockType::Page, wide.clone());
        let start = std::time::Instant::now();
        assert!(!eval_lock(&w, &wide, nick, MAX_LOCK_DEPTH));
        assert!(
            start.elapsed() < std::time::Duration::from_millis(200),
            "programmatic 64-wide self-referential lock must terminate fast: {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn exponential_indirect_dag_memoizes_and_step_budget_fails_closed_even_under_not() {
        // Non-cyclic 10-level 4-way diamond DAG (4^10 > 1M paths without
        // memoization). With `(ObjId, LockType)` memoization each node evaluates
        // once, so the DAG finishes in microseconds and returns its true value.
        let mut w = World::new_minimal();
        let nick = w.create(ObjKind::Player, "Nick", GOD, Some(LIMBO)).unwrap();
        let nodes: Vec<ObjId> = (0..10)
            .map(|i| {
                w.create(ObjKind::Thing, &format!("dag{i}"), GOD, Some(LIMBO))
                    .unwrap()
            })
            .collect();
        // Tail always fails so every OR branch is explored.
        w.get_mut(nodes[9])
            .unwrap()
            .locks
            .insert(LockType::Basic, LockExpr::Const(false));
        for pair in nodes.windows(2) {
            let (here, next) = (pair[0], pair[1]);
            let mut expr = LockExpr::Indirect(next, LockType::Basic);
            for _ in 1..4 {
                expr = LockExpr::Or(
                    Box::new(expr),
                    Box::new(LockExpr::Indirect(next, LockType::Basic)),
                );
            }
            w.get_mut(here).unwrap().locks.insert(LockType::Basic, expr);
        }
        let root = LockExpr::Indirect(nodes[0], LockType::Basic);
        let negated_root = LockExpr::Not(Box::new(root.clone()));

        let start = std::time::Instant::now();
        assert!(!eval_lock(&w, &root, nick, MAX_LOCK_DEPTH));
        assert!(eval_lock(&w, &negated_root, nick, MAX_LOCK_DEPTH));
        assert!(
            start.elapsed() < std::time::Duration::from_millis(200),
            "diamond DAG must memoize and finish fast: {:?}",
            start.elapsed()
        );

        // Exhausting MAX_LOCK_STEPS on a genuinely oversized AST (bypassing
        // parse_lock) must still poison evaluation and fail closed even under `!`.
        let mut wide = LockExpr::Const(false);
        for _ in 0..MAX_LOCK_STEPS {
            wide = LockExpr::Or(Box::new(wide), Box::new(LockExpr::Const(false)));
        }
        let negated_wide = LockExpr::Not(Box::new(wide.clone()));
        assert!(!eval_lock(&w, &wide, nick, MAX_LOCK_DEPTH));
        assert!(!eval_lock(&w, &negated_wide, nick, MAX_LOCK_DEPTH));
    }

    #[test]
    fn repeated_sibling_indirects_are_not_mistaken_for_a_cycle() {
        let (w, nick, _sword, door) = eval_world();
        // `door`'s Enter lock passes for `nick`. Referencing it twice as
        // siblings is a valid DAG, not a cycle, so both references pass.
        let expr = LockExpr::And(
            Box::new(LockExpr::Indirect(door, LockType::Enter)),
            Box::new(LockExpr::Indirect(door, LockType::Enter)),
        );
        assert!(eval_lock(&w, &expr, nick, MAX_LOCK_DEPTH));
    }

    #[test]
    fn negated_cyclic_indirect_lock_fails_closed() {
        let (mut w, nick, _sword, door) = eval_world();

        // Direct self-cycle wrapped in `!` or `!!` must still fail closed.
        let self_cycle = LockExpr::Indirect(door, LockType::Basic);
        let neg_self = LockExpr::Not(Box::new(self_cycle.clone()));
        let double_neg_self = LockExpr::Not(Box::new(neg_self.clone()));
        assert!(!eval_lock(&w, &neg_self, nick, MAX_LOCK_DEPTH));
        assert!(!eval_lock(&w, &double_neg_self, nick, MAX_LOCK_DEPTH));

        // Mutual 2-node cycle A -> B -> A wrapped in `!` (and inside compound
        // operators under `!`) must fail closed.
        let a = w.create(ObjKind::Thing, "cyc_a", GOD, Some(LIMBO)).unwrap();
        let b = w.create(ObjKind::Thing, "cyc_b", GOD, Some(LIMBO)).unwrap();
        w.get_mut(a)
            .unwrap()
            .locks
            .insert(LockType::Basic, LockExpr::Indirect(b, LockType::Basic));
        w.get_mut(b)
            .unwrap()
            .locks
            .insert(LockType::Basic, LockExpr::Indirect(a, LockType::Basic));

        let neg_mutual = LockExpr::Not(Box::new(LockExpr::Indirect(a, LockType::Basic)));
        assert!(!eval_lock(&w, &neg_mutual, nick, MAX_LOCK_DEPTH));

        // Even when the cycle itself is negated inside one of the cycle members
        // (`A = !@B`, `B = @A`), evaluating `@A` or `!@A` must fail closed.
        w.get_mut(a).unwrap().locks.insert(
            LockType::Basic,
            LockExpr::Not(Box::new(LockExpr::Indirect(b, LockType::Basic))),
        );
        assert!(!eval_lock(
            &w,
            &LockExpr::Indirect(a, LockType::Basic),
            nick,
            MAX_LOCK_DEPTH
        ));
        assert!(!eval_lock(&w, &neg_mutual, nick, MAX_LOCK_DEPTH));

        // Negated compound with a cyclic operand also poisons to false.
        let neg_or = LockExpr::Not(Box::new(LockExpr::Or(
            Box::new(self_cycle.clone()),
            Box::new(LockExpr::Const(false)),
        )));
        let neg_and = LockExpr::Not(Box::new(LockExpr::And(
            Box::new(LockExpr::Const(true)),
            Box::new(self_cycle),
        )));
        assert!(!eval_lock(&w, &neg_or, nick, MAX_LOCK_DEPTH));
        assert!(!eval_lock(&w, &neg_and, nick, MAX_LOCK_DEPTH));

        // Sanity: non-cyclic negated indirect lock still behaves normally.
        let neg_healthy = LockExpr::Not(Box::new(LockExpr::Indirect(door, LockType::Enter)));
        assert!(!eval_lock(&w, &neg_healthy, nick, MAX_LOCK_DEPTH));
        assert!(eval_lock(&w, &neg_healthy, GOD, MAX_LOCK_DEPTH));
    }

    #[test]
    fn negated_deep_indirect_lock_fails_closed() {
        let mut w = World::new_minimal();
        let nick = w.create(ObjKind::Player, "Nick", GOD, Some(LIMBO)).unwrap();
        let len = usize::from(MAX_LOCK_DEPTH) + 2;
        let chain: Vec<ObjId> = (0..len)
            .map(|i| {
                w.create(ObjKind::Thing, &format!("hop{i}"), GOD, Some(LIMBO))
                    .unwrap()
            })
            .collect();
        for pair in chain.windows(2) {
            let (here, next) = (pair[0], pair[1]);
            w.get_mut(here)
                .unwrap()
                .locks
                .insert(LockType::Basic, LockExpr::Indirect(next, LockType::Basic));
        }
        // Tail evaluates to `false`, so a reachable negated chain WOULD pass,
        // whereas hitting `indirect == 0` must poison and return `false`.
        w.get_mut(*chain.last().unwrap())
            .unwrap()
            .locks
            .insert(LockType::Basic, LockExpr::Const(false));

        let deep = LockExpr::Indirect(chain[0], LockType::Basic);
        let neg_deep = LockExpr::Not(Box::new(deep.clone()));
        assert!(!eval_lock(&w, &deep, nick, MAX_LOCK_DEPTH));
        assert!(!eval_lock(&w, &neg_deep, nick, MAX_LOCK_DEPTH));

        // Depth 0 on even a single indirect hop wrapped in `!` fails closed.
        assert!(!eval_lock(&w, &neg_deep, nick, 0));

        // Sanity: within budget (starting near the tail) `!` inverts `#FALSE` -> `true`.
        let shallow_neg = LockExpr::Not(Box::new(LockExpr::Indirect(
            chain[len - 2],
            LockType::Basic,
        )));
        assert!(eval_lock(&w, &shallow_neg, nick, MAX_LOCK_DEPTH));
        assert!(!eval_lock(&w, &shallow_neg, nick, 1));
    }

    #[test]
    fn negated_structural_cutoff_fails_closed() {
        let (w, nick, _sword, _door) = eval_world();

        // Exercise the `structural == 0` guard directly for `Not`, `And`, and
        // `Or` beneath outer negations while keeping `steps` ample, verifying
        // poison (`Ok(None)`) propagates through `!` instead of flipping to true.
        let cases = [
            LockExpr::Not(Box::new(LockExpr::Not(Box::new(LockExpr::Const(false))))),
            LockExpr::Not(Box::new(LockExpr::Not(Box::new(LockExpr::Not(Box::new(
                LockExpr::Const(false),
            )))))),
            LockExpr::Not(Box::new(LockExpr::And(
                Box::new(LockExpr::Const(true)),
                Box::new(LockExpr::Const(false)),
            ))),
            LockExpr::Not(Box::new(LockExpr::Or(
                Box::new(LockExpr::Const(false)),
                Box::new(LockExpr::Const(false)),
            ))),
        ];
        for expr in &cases {
            let mut read = PlainRead::new(&w);
            let mut steps = MAX_LOCK_STEPS;
            let mut active = Vec::new();
            let mut memo = HashMap::new();
            let (res, _) = eval_with(
                &mut read,
                expr,
                nick,
                MAX_LOCK_DEPTH,
                1,
                &mut steps,
                &mut active,
                &mut memo,
            )
            .unwrap();
            assert_eq!(
                res, None,
                "structural cutoff beneath `!` must poison as None for {expr:?}"
            );
        }

        // Deeply nested negated chains (both odd and even depths past budget)
        // must always fail closed via `eval_lock`.
        for depth in [2_000usize, 2_001usize] {
            for base in [LockExpr::Const(false), LockExpr::Const(true)] {
                let mut e = base;
                for _ in 0..depth {
                    e = LockExpr::Not(Box::new(e));
                }
                assert!(
                    !eval_lock(&w, &e, nick, MAX_LOCK_DEPTH),
                    "deeply nested Not chain (depth {depth}) must fail closed"
                );
            }
        }
    }

    #[test]
    fn p2_core_5_linear_glob_and_dag_memoization_prevent_cpu_dos() {
        // 1. parse_lock caps glob pattern length at MAX_LOCK_GLOB_LEN.
        let ok_glob = "a".repeat(MAX_LOCK_GLOB_LEN);
        assert!(parse_lock(&format!("bio:{ok_glob}"), fake_resolver).is_ok());
        let too_long = "a".repeat(MAX_LOCK_GLOB_LEN + 1);
        assert_eq!(
            parse_lock(&format!("bio:{too_long}"), fake_resolver),
            Err(LockParseError::GlobTooLong {
                max: MAX_LOCK_GLOB_LEN,
            })
        );

        // 2. Classic quadratic backtracking patterns complete fast in linear time.
        let big_text = "a".repeat(65_536);
        let hostile_literal = format!("*{}b*", "a".repeat(120));
        let hostile_qmark = format!("*{}b*", "a?".repeat(60));
        let start = std::time::Instant::now();
        assert!(!glob_match(&hostile_literal, &big_text));
        assert!(!glob_match(&hostile_qmark, &big_text));
        assert!(
            start.elapsed() < std::time::Duration::from_millis(200),
            "adversarial glob patterns must not trigger quadratic stalls: {:?}",
            start.elapsed()
        );

        // 3. Diamond indirect DAG across a single object's 10 LockTypes with a
        // 64 KiB JSON attribute leaf evaluates the leaf glob only ONCE and
        // stays fast under `eval_lock`'s metered `BoundedRead`.
        let mut w = World::new_minimal();
        let nick = w.create(ObjKind::Player, "Nick", GOD, Some(LIMBO)).unwrap();
        let big_val = serde_json::json!({ "blob": big_text });
        w.set_attr(nick, "bio", Attr::json(big_val, nick)).unwrap();

        let lock_types = [
            LockType::Basic,
            LockType::Enter,
            LockType::Use,
            LockType::Page,
            LockType::Teleport,
            LockType::Speech,
            LockType::Zone,
            LockType::Drop,
            LockType::Give,
            LockType::Mail,
        ];
        w.get_mut(nick).unwrap().locks.insert(
            lock_types[9],
            LockExpr::AttrMatch {
                name: "bio".into(),
                glob: hostile_literal.clone(),
            },
        );
        for pair in lock_types.windows(2) {
            let (curr, next) = (pair[0], pair[1]);
            let dag_step = LockExpr::Or(
                Box::new(LockExpr::Indirect(nick, next)),
                Box::new(LockExpr::Indirect(nick, next)),
            );
            w.get_mut(nick).unwrap().locks.insert(curr, dag_step);
        }

        struct CountingRead<'w> {
            world: &'w World,
            work: usize,
            nick_reads: usize,
        }
        impl<'w> ObjectRead<'w> for CountingRead<'w> {
            type Error = std::convert::Infallible;
            fn object(&mut self, id: ObjId) -> Result<Option<&'w crate::Obj>, Self::Error> {
                if self.world.get(id).is_some_and(|o| o.name == "Nick") {
                    self.nick_reads += 1;
                }
                Ok(self.world.get(id))
            }
            fn charge_work(&mut self, units: usize) -> Result<(), Self::Error> {
                self.work = self.work.saturating_add(units);
                Ok(())
            }
            fn object_slots(&mut self) -> Result<usize, Self::Error> {
                Ok(self.world.object_slot_count())
            }
        }

        let root = LockExpr::Indirect(nick, lock_types[0]);
        let mut counting = CountingRead {
            world: &w,
            work: 0,
            nick_reads: 0,
        };
        let start = std::time::Instant::now();
        assert!(!eval_lock_with(&mut counting, &root, nick, MAX_LOCK_DEPTH).unwrap());
        assert!(
            start.elapsed() < std::time::Duration::from_millis(200),
            "10-level single-object LockType diamond DAG took too long: {:?}",
            start.elapsed()
        );
        // Without memoization, 2^9 = 512 leaf visits would read `nick` hundreds
        // of times and spend hundreds of millions of work units.
        assert!(
            counting.nick_reads <= 20,
            "memoization must visit each (obj, lock_type) once, saw {} reads",
            counting.nick_reads
        );

        // 4. Exhausting BoundedRead's work budget fails closed even under `!`.
        let mut huge_attr = String::with_capacity(200_000);
        for _ in 0..200_000 {
            huge_attr.push('a');
        }
        w.set_attr(nick, "huge", Attr::text(huge_attr, nick))
            .unwrap();
        let over_budget = LockExpr::AttrMatch {
            name: "huge".into(),
            glob: "*b*".into(),
        };
        assert!(!eval_lock(&w, &over_budget, nick, MAX_LOCK_DEPTH));
        assert!(!eval_lock(
            &w,
            &LockExpr::Not(Box::new(over_budget)),
            nick,
            MAX_LOCK_DEPTH
        ));

        // 5. Programmatic oversized glob fails closed even under `!`.
        let oversized = LockExpr::AttrMatch {
            name: "bio".into(),
            glob: format!("*{}*", "a".repeat(MAX_LOCK_GLOB_LEN)),
        };
        assert!(!eval_lock(&w, &oversized, nick, MAX_LOCK_DEPTH));
        assert!(!eval_lock(
            &w,
            &LockExpr::Not(Box::new(oversized)),
            nick,
            MAX_LOCK_DEPTH
        ));
    }

    #[test]
    fn p2_core_5_glob_matches_dp_reference_on_tricky_cases() {
        fn dp_glob(pattern: &str, text: &str) -> bool {
            let p: Vec<char> = pattern.to_lowercase().chars().collect();
            let t: Vec<char> = text.to_lowercase().chars().collect();
            let mut dp = vec![vec![false; t.len() + 1]; p.len() + 1];
            dp[0][0] = true;
            for i in 1..=p.len() {
                if p[i - 1] == '*' {
                    dp[i][0] = dp[i - 1][0];
                }
            }
            for i in 1..=p.len() {
                for j in 1..=t.len() {
                    dp[i][j] = if p[i - 1] == '*' {
                        dp[i - 1][j] || dp[i][j - 1]
                    } else if p[i - 1] == '?' || p[i - 1] == t[j - 1] {
                        dp[i - 1][j - 1]
                    } else {
                        false
                    };
                }
            }
            dp[p.len()][t.len()]
        }

        let patterns = [
            "",
            "*",
            "**",
            "?",
            "??",
            "*?",
            "?*",
            "*?*",
            "a*a",
            "a**a",
            "*a*b*c*",
            "*a?a*b?b*",
            "*aa?a*b*",
            "*a?b*b?a*",
            "ab*cd*ef",
            "?*a*?b*?",
            "aa?aa",
            "*aa?b*",
            "*a?a?a*",
        ];
        let texts = [
            "",
            "a",
            "aa",
            "aaa",
            "ab",
            "aba",
            "aab",
            "aaba",
            "aaab",
            "ababab",
            "aabbcc",
            "aaabbbccc",
            "xaaaxbbb",
            "aaxaab",
            "abxxcdxxef",
            "axbxbxa",
            "AABAA",
        ];
        for p in patterns {
            for t in texts {
                assert_eq!(
                    glob_match(p, t),
                    dp_glob(p, t),
                    "mismatch for pattern={p:?} text={t:?}"
                );
            }
        }
    }

    #[test]
    fn p2_core_6_restricted_internal_and_module_attrs_fail_closed_even_under_not() {
        let mut w = World::new_minimal();
        let alice = w
            .create(ObjKind::Player, "Alice", GOD, Some(LIMBO))
            .unwrap();
        let bob = w.create(ObjKind::Player, "Bob", GOD, Some(LIMBO)).unwrap();

        // Populate unflagged attributes on Alice (simulating legacy data) while
        // leaving them absent on Bob so presence-oracle resistance is tested too.
        for (name, val) in [
            ("ignore", Attr::json(serde_json::json!([bob.0]), alice)),
            (
                "macros",
                Attr::json(serde_json::json!({"hi": "say secret"}), alice),
            ),
            (
                "sheet:tacsim",
                Attr::json(serde_json::json!({"staff_secret": "mole"}), alice),
            ),
            (
                "agent:knowledge",
                Attr::json(serde_json::json!({"secret": "vault"}), alice),
            ),
            ("tacsim:raider", Attr::text("hidden", GOD)),
            ("script:on_look", Attr::text("return 42;", GOD)),
            ("atlas:node", Attr::text("secret-sector", GOD)),
        ] {
            w.set_attr(alice, name, val).unwrap();
            let pos = LockExpr::AttrMatch {
                name: name.into(),
                glob: "*".into(),
            };
            let neg = LockExpr::Not(Box::new(pos.clone()));
            let dis = LockExpr::Or(Box::new(pos.clone()), Box::new(LockExpr::Const(true)));
            for who in [alice, bob] {
                assert!(
                    !eval_lock(&w, &pos, who, MAX_LOCK_DEPTH),
                    "{name} must not match for {who}"
                );
                assert!(
                    !eval_lock(&w, &neg, who, MAX_LOCK_DEPTH),
                    "!{name} must fail closed for {who} (no presence oracle)"
                );
                assert!(
                    !eval_lock(&w, &dis, who, MAX_LOCK_DEPTH),
                    "{name}|#TRUE must poison for {who}"
                );
            }
        }
    }
}
