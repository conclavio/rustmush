//! **The memory** (17b cut M2.1; doc 17 §8 "Memory"; D-UM-30 as amended —
//! printable ASCII since 2026-10-04 after review, finding 6).
//!
//! A mind's memory is a bounded canonical map living for the sortie: short
//! keys to small values — an INT, a bool, unit, a printable-ASCII string or
//! an array of INTs. The sim task holds its **bytes** and never a
//! `rhai::Map`; a think decodes them, binds the map as `this`, and encodes
//! what the script left (`crate::Sandbox::think`).
//!
//! This file is **the ONE canonical codec** (17b's settled question (i)):
//! [`encode`] and [`decode`] are to be the only writer and reader of memory
//! bytes in the workspace once 17b cut M2.2 makes them the arena's own (until
//! then `crates/tacsim/src/arena/mind.rs` keeps its `encode_memory` and
//! `decode_memory`, whose bytes these match), so the bytes a Rhai mind's
//! memory hashes as are exactly the bytes the native stock mind's do. The first half is rhai-free ([`Value`], [`Memory`], [`Limits`],
//! [`MemError`], [`check`], [`encode`], [`decode`]); the second converts to
//! and from Rhai's map ([`MemFault`], [`from_map`], [`to_map`]).
//!
//! # The grammar
//!
//! ```text
//! memory := "{" [ pair { "," pair } ] "}"
//! pair   := string ":" value
//! value  := int | "true" | "false" | "null" | string | ints
//! ints   := "[" [ int { "," int } ] "]"
//! int    := [ "-" ] ( "0" | digit1-9 { digit } )      an i64; never "-0"
//! string := '"' { char | '\"' | '\\' } '"'
//! char   := a byte in 0x20..=0x7E other than '"' and '\'
//! ```
//!
//! Keys strictly ascending in byte order (the `BTreeMap`'s own), no
//! whitespace anywhere, no float, no nesting, no other escape. Under the
//! printable-ASCII rule [`check`] enforces, a quote and a backslash are the
//! whole of `serde_json`'s compact escaping, so **these bytes are what
//! `serde_json::to_vec` writes for a `BTreeMap<_, serde_json::Value>` of the
//! same shapes** (`Unit` as `null`, `Ints` as an array of numbers); 17b cut
//! M2.2 pins that in tacsim against the arena's present `encode_memory`.

use std::collections::BTreeMap;
use std::fmt;

use rhai::{Array, Dynamic, ImmutableString, Map};

/// **One memory value** (D-UM-30).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// An INT.
    Int(i64),
    /// A bool.
    Bool(bool),
    /// Unit, `()` in a script: encoded `null`.
    Unit,
    /// A string of printable ASCII.
    Str(String),
    /// An array of INTs.
    Ints(Vec<i64>),
}

/// **A mind's memory, decoded** (D-UM-30): sorted by key, so one map has one
/// encoding.
pub type Memory = BTreeMap<String, Value>;

/// **A class's memory limits** (doc 17 §9's class table; D-UM-30).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The most keys.
    pub keys: usize,
    /// The longest key, bytes (characters, under the ASCII rule).
    pub key_chars: usize,
    /// The longest string value, bytes.
    pub string_bytes: usize,
    /// The longest INT array.
    pub array_len: usize,
}

/// **Why a memory was refused** (D-UM-30): the shape check's answer, the
/// `memory` fault's cause (D-UM-31).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemError {
    /// More keys than the class holds: how many there were.
    Count(usize),
    /// A key that is empty, too long, or not printable ASCII.
    Key(String),
    /// The value under this key: a string too long or not printable ASCII,
    /// an array too long, or (from [`from_map`]) a value of no memory shape.
    Value(String),
}

impl fmt::Display for MemError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemError::Count(count) => write!(f, "memory holds {count} keys, past its class's cap"),
            MemError::Key(key) => write!(
                f,
                "memory key {key:?} is empty, too long or not printable ASCII"
            ),
            MemError::Value(key) => write!(
                f,
                "memory key {key:?} holds a value too long or of no memory shape"
            ),
        }
    }
}

/// Whether every byte of `text` is printable ASCII, `0x20..=0x7E`.
fn printable(text: &str) -> bool {
    text.bytes().all(|byte| (0x20..=0x7E).contains(&byte))
}

/// **Whether a memory fits a class** (D-UM-30 as amended): the count first
/// (more than `limits.keys` is [`MemError::Count`]), then each pair in key
/// order — a key empty, longer than `limits.key_chars` bytes or holding a
/// byte outside `0x20..=0x7E` is [`MemError::Key`]; a string longer than
/// `limits.string_bytes` bytes or holding a byte outside `0x20..=0x7E`, or an
/// array longer than `limits.array_len`, is [`MemError::Value`].
///
/// # Errors
///
/// The first refusal in that order.
pub fn check(memory: &Memory, limits: &Limits) -> Result<(), MemError> {
    if memory.len() > limits.keys {
        return Err(MemError::Count(memory.len()));
    }
    for (key, value) in memory {
        if key.is_empty() || key.len() > limits.key_chars || !printable(key) {
            return Err(MemError::Key(key.clone()));
        }
        let fits = match value {
            Value::Int(_) | Value::Bool(_) | Value::Unit => true,
            Value::Str(text) => text.len() <= limits.string_bytes && printable(text),
            Value::Ints(items) => items.len() <= limits.array_len,
        };
        if !fits {
            return Err(MemError::Value(key.clone()));
        }
    }
    Ok(())
}

/// Write `text` as a JSON string: a quote, the text with `"` written `\"` and
/// `\` written `\\`, a quote.
fn put_string(out: &mut String, text: &str) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            other => out.push(other),
        }
    }
    out.push('"');
}

/// **A memory's canonical bytes** (D-UM-30; the grammar in this file's
/// head): `{`, each pair in key order as `"key":value` joined by `,`, `}` —
/// an INT as its decimal, `true`/`false`, unit as `null`, a string quoted
/// with only `"` and `\` escaped, an INT array as `[` decimals joined by `,`
/// `]`; the empty map `{}`. What `serde_json::to_vec` writes for the same
/// shapes, for every memory [`check`] accepts.
#[must_use]
pub fn encode(memory: &Memory) -> Vec<u8> {
    use fmt::Write as _;
    let mut out = String::from("{");
    for (index, (key, value)) in memory.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        put_string(&mut out, key);
        out.push(':');
        match value {
            Value::Int(n) => {
                let _ = write!(out, "{n}");
            }
            Value::Bool(true) => out.push_str("true"),
            Value::Bool(false) => out.push_str("false"),
            Value::Unit => out.push_str("null"),
            Value::Str(text) => put_string(&mut out, text),
            Value::Ints(items) => {
                out.push('[');
                for (at, n) in items.iter().enumerate() {
                    if at > 0 {
                        out.push(',');
                    }
                    let _ = write!(out, "{n}");
                }
                out.push(']');
            }
        }
    }
    out.push('}');
    out.into_bytes()
}

/// A cursor over the bytes [`decode`] reads.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    /// The byte under the cursor, if any.
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    /// Step over `byte`, or answer `None`.
    fn eat(&mut self, byte: u8) -> Option<()> {
        (self.peek()? == byte).then(|| self.at += 1)
    }

    /// Step over the literal `word`, or answer `None`.
    fn word(&mut self, word: &[u8]) -> Option<()> {
        let end = self.at.checked_add(word.len())?;
        (self.bytes.get(self.at..end)? == word).then(|| self.at = end)
    }

    /// A string: printable ASCII with only `\"` and `\\` escaped.
    fn string(&mut self) -> Option<String> {
        self.eat(b'"')?;
        let mut text = String::new();
        loop {
            match self.peek()? {
                b'"' => {
                    self.at += 1;
                    return Some(text);
                }
                b'\\' => {
                    self.at += 1;
                    let escaped = self.peek()?;
                    if escaped != b'"' && escaped != b'\\' {
                        return None;
                    }
                    text.push(char::from(escaped));
                    self.at += 1;
                }
                byte @ 0x20..=0x7E => {
                    text.push(char::from(byte));
                    self.at += 1;
                }
                _ => return None,
            }
        }
    }

    /// An INT: `-`? then `0` or a digit 1–9 and more digits, fitting an
    /// `i64`; never `-0`.
    fn int(&mut self) -> Option<i64> {
        let start = self.at;
        let negative = self.eat(b'-').is_some();
        let first = self.peek()?;
        if !first.is_ascii_digit() {
            return None;
        }
        self.at += 1;
        if first == b'0' {
            if negative || self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                return None;
            }
        } else {
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.at += 1;
            }
        }
        let text = std::str::from_utf8(self.bytes.get(start..self.at)?).ok()?;
        text.parse().ok()
    }

    /// A value of the grammar.
    fn value(&mut self) -> Option<Value> {
        match self.peek()? {
            b't' => self.word(b"true").map(|()| Value::Bool(true)),
            b'f' => self.word(b"false").map(|()| Value::Bool(false)),
            b'n' => self.word(b"null").map(|()| Value::Unit),
            b'"' => self.string().map(Value::Str),
            b'[' => {
                self.at += 1;
                let mut items = Vec::new();
                if self.eat(b']').is_some() {
                    return Some(Value::Ints(items));
                }
                loop {
                    items.push(self.int()?);
                    if self.eat(b']').is_some() {
                        return Some(Value::Ints(items));
                    }
                    self.eat(b',')?;
                }
            }
            _ => self.int().map(Value::Int),
        }
    }
}

/// **A memory read back off its bytes**: the exact inverse of [`encode`] over
/// the grammar in this file's head and nothing wider — no whitespace, no
/// float, no nesting, no escape but `\"` and `\\`, keys strictly ascending,
/// nothing after the closing brace. `{}` is the empty map. `None` for any
/// byte string [`encode`] could not have written (the caller's rule for
/// that: [`crate::Sandbox::think`] starts over from the empty map).
#[must_use]
pub fn decode(bytes: &[u8]) -> Option<Memory> {
    let mut reader = Reader { bytes, at: 0 };
    let mut memory = Memory::new();
    reader.eat(b'{')?;
    if reader.eat(b'}').is_none() {
        loop {
            let key = reader.string()?;
            if memory
                .last_key_value()
                .is_some_and(|(last, _)| last.as_str() >= key.as_str())
            {
                return None;
            }
            reader.eat(b':')?;
            let value = reader.value()?;
            memory.insert(key, value);
            if reader.eat(b'}').is_some() {
                break;
            }
            reader.eat(b',')?;
        }
    }
    (reader.at == bytes.len()).then_some(memory)
}

// ---------------------------------------------------------------------------
// The rhai-facing half
// ---------------------------------------------------------------------------

/// **Why a script's `this` is not a memory** (D-UM-28, D-UM-30, D-UM-31): a
/// FLOAT is a `type` fault; every other wrong shape is a `memory` fault.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemFault {
    /// A FLOAT somewhere under this key (D-UM-28: memory values are INT
    /// only).
    Float(String),
    /// A shape [`check`] or the conversion refused.
    Shape(MemError),
}

impl fmt::Display for MemFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemFault::Float(key) => write!(f, "memory key {key:?} holds a FLOAT"),
            MemFault::Shape(error) => error.fmt(f),
        }
    }
}

/// Whether `value` is or holds a FLOAT, at any depth.
fn holds_float(value: &Dynamic) -> bool {
    if value.is_float() {
        return true;
    }
    if let Some(items) = value.read_lock::<Array>() {
        return items.iter().any(holds_float);
    }
    if let Some(map) = value.read_lock::<Map>() {
        return map.values().any(holds_float);
    }
    false
}

/// One script value as a memory value, or `None` when it has no memory shape.
fn value_of(value: &Dynamic) -> Option<Value> {
    if value.is_unit() {
        return Some(Value::Unit);
    }
    if let Ok(n) = value.as_int() {
        return Some(Value::Int(n));
    }
    if let Ok(flag) = value.as_bool() {
        return Some(Value::Bool(flag));
    }
    if value.is_string() {
        return value
            .clone()
            .into_immutable_string()
            .ok()
            .map(|text| Value::Str(text.to_string()));
    }
    if let Some(items) = value.read_lock::<Array>() {
        return items
            .iter()
            .map(|item| item.as_int().ok())
            .collect::<Option<Vec<i64>>>()
            .map(Value::Ints);
    }
    None
}

/// **A script's map as a memory** (D-UM-28, D-UM-30): in key order, a value
/// that is or holds a FLOAT is [`MemFault::Float`]; an INT, a bool, unit, a
/// string or an array of INTs converts; anything else (a nested map, an
/// array holding a string, a custom type) is
/// `MemFault::Shape(MemError::Value(key))`; then the whole is [`check`]ed
/// against `limits`.
///
/// # Errors
///
/// The first refusal in that order.
pub fn from_map(map: &Map, limits: &Limits) -> Result<Memory, MemFault> {
    let mut memory = Memory::new();
    for (key, value) in map {
        if holds_float(value) {
            return Err(MemFault::Float(key.to_string()));
        }
        let converted =
            value_of(value).ok_or_else(|| MemFault::Shape(MemError::Value(key.to_string())))?;
        memory.insert(key.to_string(), converted);
    }
    check(&memory, limits).map_err(MemFault::Shape)?;
    Ok(memory)
}

/// **A memory as a script's map**: the value bound as `this` at a think's
/// start.
#[must_use]
pub fn to_map(memory: &Memory) -> Map {
    memory
        .iter()
        .map(|(key, value)| {
            let dynamic = match value {
                Value::Int(n) => Dynamic::from_int(*n),
                Value::Bool(flag) => Dynamic::from_bool(*flag),
                Value::Unit => Dynamic::UNIT,
                Value::Str(text) => Dynamic::from(ImmutableString::from(text.as_str())),
                Value::Ints(items) => {
                    Dynamic::from_array(items.iter().map(|n| Dynamic::from_int(*n)).collect())
                }
            };
            (key.as_str().into(), dynamic)
        })
        .collect()
}
