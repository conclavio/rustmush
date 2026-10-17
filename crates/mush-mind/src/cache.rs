//! **The AST cache** (17b cut M2.1; doc 17 §7 "Parameters", §13 "Records,
//! the seal and replay"; D-UM-25, D-UM-27).
//!
//! A mind is compiled once per `(source digest, MIND_ABI)` and shared by
//! every hull that runs it: the card is handed to each think as an argument,
//! never folded into the AST, so 300 hulls on 300 differently tuned cards
//! compile once (D-UM-27). Compiling is paid at the seal, in the world, never
//! in the sim task (doc 17 §7: about 270 µs with the seal).
//!
//! The digest is the caller's (the seal's `source_sha256`, doc 17 §13): this
//! crate hashes nothing and trusts that one digest names one source.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, PoisonError};

use rhai::AST;

use crate::engine::Sandbox;

/// Maximum strong entries in each sandbox's execution cache. Active runners
/// retain their own immutable Arc after eviction; checks use disposable sandboxes.
pub const MAX_CACHED_MINDS: usize = 256;

type Key = (Digest, &'static str);

/// Bounded LRU retention, separate from a runner's execution lifetime.
#[derive(Default)]
pub(crate) struct Cache {
    entries: BTreeMap<Key, Arc<Compiled>>,
    order: VecDeque<Key>,
}
impl Cache {
    fn get(&mut self, key: &Key) -> Option<Arc<Compiled>> {
        let hit = Arc::clone(self.entries.get(key)?);
        self.order.retain(|old| old != key);
        self.order.push_back(*key);
        Some(hit)
    }
    fn insert(&mut self, key: Key, value: Arc<Compiled>) -> Arc<Compiled> {
        // Another compiler may have filled this miss while we parsed.
        if let Some(hit) = self.get(&key) {
            return hit;
        }
        while self.entries.len() >= MAX_CACHED_MINDS {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            } else {
                // Defensive invariant recovery stays bounded and preserves all
                // runner-owned Arcs. No partial AST is ever returned.
                self.entries.clear();
            }
        }
        self.order.push_back(key);
        self.entries.insert(key, Arc::clone(&value));
        value
    }
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

/// **A mind source's digest** (doc 17 §13): the 32 bytes of its SHA-256,
/// computed by the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Digest(pub [u8; 32]);

/// **A compiled mind** (D-UM-27): the AST, with only its functions kept, and
/// the key it is cached under.
#[derive(Debug)]
pub struct Compiled {
    /// The functions of the source; its top-level statements cleared.
    pub ast: AST,
    /// The source's digest.
    pub digest: Digest,
    /// The `MIND_ABI` it was compiled under (`rhai=1.26.0;api=N;stock=M`).
    pub abi: &'static str,
}

/// **Why a source did not compile** (D-UM-25: a typo is a seal-time error).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompileError {
    /// Rhai refused it — a syntax error, a disabled symbol, a variable no
    /// scope declares (strict variables), a limit the parser enforces (the
    /// expression depths, the function count) — with Rhai's own words.
    Parse(String),
    /// It defines no `fn think(view, card)`: a script function named `think`
    /// of exactly two parameters.
    NoThink,
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompileError::Parse(text) => write!(f, "the mind does not compile: {text}"),
            CompileError::NoThink => f.write_str("the mind defines no fn think of two parameters"),
        }
    }
}

impl std::error::Error for CompileError {}

impl Sandbox {
    /// **Compile a mind, or answer the one already compiled** (D-UM-27):
    /// keyed `(digest, abi)`. A hit answers the stored [`Arc`]; a miss
    /// compiles `source` on the sandbox's engine, requires a script function
    /// `think` of two parameters, **clears the top-level statements** and
    /// stores the result in the bounded least-recently-used cache. Eviction
    /// releases only cache ownership; active runners retain their compiled AST.
    ///
    /// Clearing the statements is the rule, not a saving: the think IS the
    /// function. A top-level statement would never run anyway
    /// ([`Sandbox::think`] calls `think` without evaluating the AST), and a
    /// top-level `const` is invisible inside a function body, which the
    /// parser reads with a fresh state that sees no top-level name — so a
    /// script cannot hand its `think` a value through the top level, and
    /// nothing per-think is ever a script constant (17b's settled question
    /// (ii): the game's per-think facts are zero-argument native functions).
    ///
    /// # Errors
    ///
    /// [`CompileError::Parse`] with Rhai's words; [`CompileError::NoThink`].
    pub fn compile(
        &self,
        source: &str,
        digest: Digest,
        abi: &'static str,
    ) -> Result<Arc<Compiled>, CompileError> {
        let key = (digest, abi);
        if let Some(hit) = self
            .asts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
        {
            return Ok(hit);
        }
        let mut ast = self
            .engine
            .compile(source)
            .map_err(|error| CompileError::Parse(error.to_string()))?;
        if !ast
            .iter_functions()
            .any(|function| function.name == "think" && function.params.len() == 2)
        {
            return Err(CompileError::NoThink);
        }
        ast.clear_statements();
        let compiled = Arc::new(Compiled { ast, digest, abi });
        let mut asts = self.asts.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(asts.insert(key, compiled))
    }

    /// Actual parser location after a failed compile. No execution or cache entry.
    #[must_use]
    pub fn compile_error_position(&self, source: &str) -> Option<(usize, usize)> {
        let error = self.engine.compile(source).err()?;
        let position = error.position();
        position.line().zip(position.position())
    }

    /// **How many compiled minds the cache holds.**
    #[must_use]
    pub fn cached(&self) -> usize {
        self.asts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

#[cfg(test)]
mod diagnostic_position_tests {
    use super::*;
    #[test]
    fn parser_position_is_actual_and_does_not_populate_execution_cache() {
        let sandbox = Sandbox::new(&crate::EngineLimits::default(), |_| {});
        let source = "fn think(view, card) {\n    let x = ;\n}\n";
        let (line, column) = sandbox
            .compile_error_position(source)
            .expect("actual parse location");
        assert_eq!(line, 2);
        assert!(column > 0);
        assert_eq!(sandbox.cached(), 0);
        assert_eq!(
            sandbox.compile_error_position("fn think(view, card) {}"),
            None
        );
        assert_eq!(sandbox.cached(), 0);
    }
}
