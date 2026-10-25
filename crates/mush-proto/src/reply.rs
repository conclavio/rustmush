//! Per-connection command reply delimiters. These are never character state.
use serde::{Deserialize, Serialize};

/// The gateway retains this row for the physical connection's lifetime.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputBrackets {
    /// One printable line before a typed command's answer.
    pub prefix: Option<String>,
    /// One printable line after the complete answer.
    pub suffix: Option<String>,
}

/// Semantic tag for an opening reply delimiter.
pub const OUTPUT_PREFIX_TAG: &str = "output-prefix";
/// Semantic tag for a closing reply delimiter.
pub const OUTPUT_SUFFIX_TAG: &str = "output-suffix";
