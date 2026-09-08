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

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::attr::{AttrValue, UncasedName};
use crate::flags::Flags;
use crate::obj::ObjId;
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
/// indirection decrements it and an expression at depth 0 fails closed, so
/// `@lock`-cycles terminate as `false`. Callers normally pass
/// [`MAX_LOCK_DEPTH`].
///
/// Semantics of the leaves:
/// - `Carry`: the target object is directly in `who`'s contents.
/// - `OwnedBy`: `who.owner` is the target.
/// - `HasFlag`: matched against core [`Flags`] by (case-folded) name first,
///   then against the object's marker set, case-insensitively.
/// - `AttrMatch`: attribute looked up with parent-chain inheritance
///   ([`World::attr`]); `Text` values match their text, `Json` values match
///   their compact serialization; a missing attribute fails.
/// - `Indirect`: a missing target object fails; a target without that lock
///   type passes (an unset lock is unlocked, as in TinyMUX).
///
/// Structural nesting (`!`, `&`, `|`) is bounded separately by
/// [`MAX_LOCK_STRUCTURAL_DEPTH`]; a tree taller than that fails closed at the
/// bound instead of overflowing the world task's stack.
#[must_use]
pub fn eval_lock(w: &World, expr: &LockExpr, who: ObjId, depth: u8) -> bool {
    eval_with(w, expr, who, depth, MAX_LOCK_STRUCTURAL_DEPTH)
}

/// [`eval_lock`] with both budgets explicit.
///
/// `indirect` is the public `depth`: it counts [`LockExpr::Indirect`] hops and
/// is what [`MAX_LOCK_DEPTH`] sizes. `structural` counts `Not`/`And`/`Or`
/// levels within one object's key and is reseeded on every indirection,
/// because each hop starts a fresh (and separately bounded) tree — the two
/// budgets multiply into the true stack ceiling, which is why the indirect
/// budget stays small.
fn eval_with(w: &World, expr: &LockExpr, who: ObjId, indirect: u8, structural: u32) -> bool {
    match expr {
        LockExpr::Const(b) => *b,
        LockExpr::Is(id) => who == *id,
        LockExpr::Carry(id) => w.contents_of(who).contains(id),
        LockExpr::OwnedBy(id) => w.get(who).is_some_and(|o| o.owner == *id),
        LockExpr::HasFlag(name) => {
            let Some(o) = w.get(who) else { return false };
            if let Some(fl) = Flags::from_name(&name.to_uppercase()) {
                o.flags.contains(fl)
            } else {
                o.markers.iter().any(|m| m.eq_ignore_ascii_case(name))
            }
        }
        LockExpr::AttrMatch { name, glob } => match w.attr(who, name.as_str()) {
            Some(a) => match &a.value {
                AttrValue::Text(s) => glob_match(glob, s),
                AttrValue::Json(v) => glob_match(glob, &v.to_string()),
            },
            None => false,
        },
        LockExpr::Indirect(obj, lt) => {
            if indirect == 0 {
                return false;
            }
            match w.get(*obj) {
                None => false,
                Some(o) => match o.locks.get(lt) {
                    None => true,
                    // A fresh tree: the structural budget starts over, the
                    // indirect one does not.
                    Some(inner) => {
                        eval_with(w, inner, who, indirect - 1, MAX_LOCK_STRUCTURAL_DEPTH)
                    }
                },
            }
        }
        LockExpr::Not(e) => {
            if structural == 0 {
                return false;
            }
            !eval_with(w, e, who, indirect, structural - 1)
        }
        LockExpr::And(a, b) => {
            if structural == 0 {
                return false;
            }
            eval_with(w, a, who, indirect, structural - 1)
                && eval_with(w, b, who, indirect, structural - 1)
        }
        LockExpr::Or(a, b) => {
            if structural == 0 {
                return false;
            }
            eval_with(w, a, who, indirect, structural - 1)
                || eval_with(w, b, who, indirect, structural - 1)
        }
    }
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
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut mark = 0usize;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            // Backtrack: let the last '*' swallow one more character.
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
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
        // Every leaf is depth 1.
        self.classify(raw).map(|e| (e, 1))
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
            return Ok(LockExpr::Indirect(self.objref(obj_part)?, lt));
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
            return Ok(LockExpr::AttrMatch {
                name: UncasedName::from(name),
                glob: glob.trim().to_string(),
            });
        }
        Ok(LockExpr::Is(self.objref(raw)?))
    }
}

impl LockExpr {
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
    /// `budget` is the structural fence ([`MAX_LOCK_STRUCTURAL_DEPTH`]): the
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
}
