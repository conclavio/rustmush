//! Error types for parsing and evaluation limits.

/// A dice expression failed to parse.
///
/// Carries the `winnow` diagnostic rendered to a string; the grammar is small
/// enough that a positional message is more useful than a typed error tree.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid dice expression: {message}")]
pub struct DiceParseError {
    pub message: String,
}

impl DiceParseError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// A parsed expression exceeded a [`crate::RollLimits`] DoS guard.
///
/// Returned by [`crate::DiceExpr::check_limits`] so a command handler can reject
/// a hostile expression (`999999d1000`) with a clear message *before* rolling.
/// [`crate::DiceExpr::eval`] itself never returns this — it defends by capping —
/// but callers that want strict rejection check first.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RollLimitError {
    #[error("too many dice: expression rolls {requested} (limit {limit})")]
    TooManyDice { requested: u64, limit: u32 },
    #[error("die has too many sides: d{requested} (limit {limit})")]
    TooManySides { requested: u32, limit: u32 },
    #[error("expression too complex: {nodes} terms (limit {limit})")]
    TooComplex { nodes: u32, limit: u32 },
    #[error("a die cannot have zero sides (dN with N >= 1)")]
    ZeroSides,
}
