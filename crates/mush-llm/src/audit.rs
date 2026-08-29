//! Audit log (design 03 §4.3). Every call — success, refusal, error, budget
//! refusal, or kill-switch — leaves one attributed row.
//!
//! **Persistence seam (M7):** rows live in a bounded in-memory ring and are
//! mirrored to `tracing`. The durable path is an `XADD` to a
//! `mod:{ns}:llm_audit` stream via `ModuleOp::StreamAppend` (owned by the
//! worldgen module / `mushd`); mush-llm keeps no store handle. Full prompt
//! text is deliberately **not** stored (it contains player speech) — only a
//! SHA-256 and a truncated summary.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use mush_core::ObjId;
use sha2::{Digest, Sha256};

/// How a call turned out.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Completed normally.
    Ok,
    /// The model refused.
    Refused,
    /// A provider/transport error.
    Error,
    /// Refused pre-call by a budget ceiling.
    Budget,
    /// Refused pre-call by a kill switch.
    Disabled,
}

impl Outcome {
    /// The wire label.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::Refused => "refused",
            Outcome::Error => "error",
            Outcome::Budget => "budget",
            Outcome::Disabled => "disabled",
        }
    }
}

/// One audit row.
#[derive(Clone, Debug, PartialEq)]
pub struct AuditRecord {
    /// Wall-clock unix seconds.
    pub ts: u64,
    /// Feature route name.
    pub feature: String,
    /// Provider id.
    pub provider: String,
    /// Model id (as routed).
    pub model: String,
    /// Triggering actor (builder/player), if any.
    pub actor: Option<ObjId>,
    /// NPC, if this was an agent turn.
    pub npc: Option<ObjId>,
    /// SHA-256 of the full assembled prompt (hex).
    pub prompt_sha256: String,
    /// First ≤200 chars of the last user turn.
    pub prompt_summary: String,
    /// First ≤200 chars of the response text.
    pub response_summary: String,
    /// Input tokens.
    pub input_tokens: u64,
    /// Output tokens.
    pub output_tokens: u64,
    /// Cost, USD (authoritative or estimated), if known.
    pub cost_usd: Option<f64>,
    /// End-to-end latency, ms.
    pub latency_ms: u64,
    /// Result.
    pub outcome: Outcome,
}

/// The bounded in-memory audit ring plus a `tracing` mirror.
pub struct AuditSink {
    ring: Mutex<VecDeque<AuditRecord>>,
    capacity: usize,
}

impl std::fmt::Debug for AuditSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let g = self.ring.lock().expect("audit lock");
        write!(f, "AuditSink({}/{} rows)", g.len(), self.capacity)
    }
}

impl Default for AuditSink {
    fn default() -> AuditSink {
        AuditSink::new(1024)
    }
}

impl AuditSink {
    /// A sink retaining the most recent `capacity` rows.
    #[must_use]
    pub fn new(capacity: usize) -> AuditSink {
        AuditSink {
            ring: Mutex::new(VecDeque::with_capacity(capacity.min(1024))),
            capacity: capacity.max(1),
        }
    }

    /// Record one row (and emit a scrubbed `tracing` event — never the prompt
    /// text, never a key).
    pub fn record(&self, rec: AuditRecord) {
        tracing::info!(
            target: "mush_llm::audit",
            feature = %rec.feature,
            provider = %rec.provider,
            model = %rec.model,
            actor = ?rec.actor,
            npc = ?rec.npc,
            input_tokens = rec.input_tokens,
            output_tokens = rec.output_tokens,
            cost_usd = ?rec.cost_usd,
            latency_ms = rec.latency_ms,
            outcome = rec.outcome.as_str(),
            prompt_sha256 = %rec.prompt_sha256,
            "llm call"
        );
        let mut g = self.ring.lock().expect("audit lock");
        if g.len() == self.capacity {
            g.pop_front();
        }
        g.push_back(rec);
    }

    /// The most recent `n` rows, newest last.
    #[must_use]
    pub fn recent(&self, n: usize) -> Vec<AuditRecord> {
        let g = self.ring.lock().expect("audit lock");
        let skip = g.len().saturating_sub(n);
        g.iter().skip(skip).cloned().collect()
    }

    /// Total rows currently retained.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ring.lock().expect("audit lock").len()
    }

    /// Whether the ring is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ring.lock().expect("audit lock").is_empty()
    }
}

/// Hex SHA-256 of a prompt (stable, non-reversible attribution key).
#[must_use]
pub fn prompt_sha256(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    let digest = h.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Truncate to ≤200 chars on a char boundary, appending `…` when clipped.
#[must_use]
pub fn summarize(s: &str) -> String {
    let s = s.trim();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i >= 200 {
            out.push('…');
            break;
        }
        out.push(c);
    }
    out
}

/// Current wall-clock unix seconds.
#[must_use]
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_truncates_on_char_boundary() {
        let long = "é".repeat(500);
        let s = summarize(&long);
        assert_eq!(s.chars().count(), 201); // 200 + the ellipsis
        assert!(s.ends_with('…'));
        assert_eq!(summarize("  short  "), "short");
    }

    #[test]
    fn sha256_is_stable_and_hex() {
        let a = prompt_sha256("hello world");
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(a, prompt_sha256("hello world"));
        assert_ne!(a, prompt_sha256("hello world!"));
    }

    #[test]
    fn ring_bounds_capacity() {
        let sink = AuditSink::new(2);
        for i in 0..5 {
            sink.record(AuditRecord {
                ts: i,
                feature: "f".into(),
                provider: "p".into(),
                model: "m".into(),
                actor: None,
                npc: None,
                prompt_sha256: String::new(),
                prompt_summary: String::new(),
                response_summary: String::new(),
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: None,
                latency_ms: 0,
                outcome: Outcome::Ok,
            });
        }
        assert_eq!(sink.len(), 2);
        let recent = sink.recent(10);
        assert_eq!(recent.first().unwrap().ts, 3);
        assert_eq!(recent.last().unwrap().ts, 4);
    }
}
