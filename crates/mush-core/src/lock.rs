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
#[must_use]
pub fn eval_lock(w: &World, expr: &LockExpr, who: ObjId, depth: u8) -> bool {
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
            if depth == 0 {
                return false;
            }
            match w.get(*obj) {
                None => false,
                Some(o) => match o.locks.get(lt) {
                    None => true,
                    Some(inner) => eval_lock(w, inner, who, depth - 1),
                },
            }
        }
        LockExpr::Not(e) => !eval_lock(w, e, who, depth),
        LockExpr::And(a, b) => eval_lock(w, a, who, depth) && eval_lock(w, b, who, depth),
        LockExpr::Or(a, b) => eval_lock(w, a, who, depth) || eval_lock(w, b, who, depth),
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
}

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
        resolve,
    };
    let expr = p.parse_or()?;
    p.skip_ws();
    if p.pos < p.src.len() {
        return Err(LockParseError::Trailing(p.pos));
    }
    Ok(expr)
}

struct Parser<'a, R> {
    src: &'a str,
    pos: usize,
    resolve: R,
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

    fn parse_or(&mut self) -> Result<LockExpr, LockParseError> {
        let mut left = self.parse_and()?;
        loop {
            self.skip_ws();
            if self.peek() == Some('|') {
                self.bump();
                let right = self.parse_and()?;
                left = LockExpr::Or(Box::new(left), Box::new(right));
            } else {
                return Ok(left);
            }
        }
    }

    fn parse_and(&mut self) -> Result<LockExpr, LockParseError> {
        let mut left = self.parse_unary()?;
        loop {
            self.skip_ws();
            if self.peek() == Some('&') {
                self.bump();
                let right = self.parse_unary()?;
                left = LockExpr::And(Box::new(left), Box::new(right));
            } else {
                return Ok(left);
            }
        }
    }

    fn parse_unary(&mut self) -> Result<LockExpr, LockParseError> {
        self.skip_ws();
        if self.peek() == Some('!') {
            self.bump();
            Ok(LockExpr::Not(Box::new(self.parse_unary()?)))
        } else {
            self.parse_primary()
        }
    }

    fn parse_primary(&mut self) -> Result<LockExpr, LockParseError> {
        self.skip_ws();
        if self.peek() == Some('(') {
            self.bump();
            let expr = self.parse_or()?;
            self.skip_ws();
            if self.peek() == Some(')') {
                self.bump();
                Ok(expr)
            } else {
                Err(LockParseError::UnbalancedParen)
            }
        } else {
            self.parse_term()
        }
    }

    /// A term runs until an operator/paren; spaces inside are allowed
    /// (player names may contain them) and trimmed at the edges.
    fn parse_term(&mut self) -> Result<LockExpr, LockParseError> {
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
        self.classify(raw)
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
    fn fmt_prec(&self, f: &mut fmt::Formatter<'_>, min: u8) -> fmt::Result {
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
                e.fmt_prec(f, 3)?;
            }
            LockExpr::And(a, b) => {
                a.fmt_prec(f, 2)?;
                f.write_str(" & ")?;
                b.fmt_prec(f, 3)?;
            }
            LockExpr::Or(a, b) => {
                a.fmt_prec(f, 1)?;
                f.write_str(" | ")?;
                b.fmt_prec(f, 2)?;
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
        self.fmt_prec(f, 0)
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
}
