//! The privileges audit data model and its persistence keys (D1, D-D-14).
//!
//! The module owns one append-only log, `mod:privileges:log:audit:{chunk}`,
//! on the chunked [`Ledger`] substrate — idempotent on retry, sealed and
//! archived like every other ledger, paged through the H1 store-read seam.
//! Every entry records **that** something happened to a privilege — a grant,
//! a revocation, a permitted use, a refused attempt, or one of the Discord
//! identity mutations — and **never what was seen**: no entry may carry an
//! identity, and [`contains_identity`] is the gate every reason and detail
//! passes before it is appended.

use std::collections::HashMap;

use mush_core::{Deferred, Ledger, ModuleOp, ModuleWrite, ObjId, Outbox, Privileges};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// The persistence namespace (`mod:privileges:*`).
pub const NS: &str = "privileges";

/// The single audit log id.
pub const LOG: &str = "audit";

/// Longest accepted grant/revoke reason, bytes.
pub const MAX_REASON_BYTES: usize = 240;

/// How many entries the recent (RAM-tail) view prints at most.
pub const MAX_RECENT_LINES: usize = 50;

/// What happened to a privilege.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    /// `@privilege/grant`.
    Grant,
    /// `@privilege/revoke`.
    Revoke,
    /// A permitted use of the privilege (`@discord/show`).
    Use,
    /// An attempt without the privilege.
    Denied,
    /// `@discord/unlink <player>` by staff.
    Unlink,
    /// `@discord/ban <player>`.
    Ban,
    /// `@discord/unban <player>`.
    Unban,
}

impl AuditAction {
    /// The staff-facing label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            AuditAction::Grant => "grant",
            AuditAction::Revoke => "revoke",
            AuditAction::Use => "use",
            AuditAction::Denied => "denied",
            AuditAction::Unlink => "unlink",
            AuditAction::Ban => "ban",
            AuditAction::Unban => "unban",
        }
    }
}

/// One immutable audit entry — the payload stored in the ledger.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PrivilegeAuditEntry {
    /// Wall-clock record time (unix seconds).
    pub at: i64,
    /// Who acted.
    pub actor: ObjId,
    /// What they did.
    pub action: AuditAction,
    /// The privilege concerned, by name (`"DISCORD_IDENTITY"`), when one is.
    #[serde(default)]
    pub privilege: Option<SmolStr>,
    /// The object acted on or looked at, when one is.
    #[serde(default)]
    pub subject: Option<ObjId>,
    /// The staff-entered reason, when the action requires one.
    #[serde(default)]
    pub reason: Option<String>,
    /// Action-specific detail. Never an identity ([`contains_identity`]).
    #[serde(default)]
    pub detail: serde_json::Value,
}

/// What a caller hands [`crate::audit`] to record.
#[derive(Clone, Debug, PartialEq)]
pub struct PrivilegeAction {
    /// What happened.
    pub action: AuditAction,
    /// The privilege concerned, when one is.
    pub privilege: Option<Privileges>,
    /// The object acted on or looked at, when one is.
    pub subject: Option<ObjId>,
    /// The reason, when the action requires one.
    pub reason: Option<String>,
    /// Action-specific detail.
    pub detail: serde_json::Value,
}

/// One archive page a reader is waiting on (H1): the correlation
/// `Ctx::read_store` returned and the chunk range it covers. Transient world
/// RAM — a restart drops it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingAuditRead {
    /// The correlation the completion must carry.
    pub correlation: u64,
    /// When it was requested (world seconds).
    pub requested_at: i64,
    /// The requested page (0 is the newest sealed page).
    pub page: u64,
    /// The chunk range the page covers, inclusive.
    pub chunk_lo: u64,
    /// The chunk range the page covers, inclusive.
    pub chunk_hi: u64,
}

/// The live module state, held in a module state slot on the `World`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PrivilegesState {
    /// In-flight archive reads, one per requesting actor (H1). Transient.
    pub pending_reads: HashMap<ObjId, PendingAuditRead>,
    /// The append-only audit log (only the open tail lives in RAM).
    pub audit: Ledger,
}

impl PrivilegesState {
    /// The audit entries currently in RAM (the open tail), oldest first.
    #[must_use]
    pub fn recent(&self) -> Vec<PrivilegeAuditEntry> {
        decode_entries(self.audit.open_entries(LOG).iter().map(|e| &e.value))
    }
}

/// Decode ledger values into entries, skipping anything that does not parse
/// (a future schema is not a reason to hide the rest of the log).
pub(crate) fn decode_entries<'a>(
    values: impl Iterator<Item = &'a serde_json::Value>,
) -> Vec<PrivilegeAuditEntry> {
    values
        .filter_map(|v| serde_json::from_value::<PrivilegeAuditEntry>(v.clone()).ok())
        .collect()
}

/// The canonical name of one privilege bit (`"DISCORD_IDENTITY"`), or the
/// `|`-joined names of a set.
#[must_use]
pub fn privilege_name(p: Privileges) -> SmolStr {
    let names: Vec<&str> = p.iter_names().map(|(n, _)| n).collect();
    SmolStr::new(names.join("|"))
}

/// Parse a staff-typed privilege name (case-insensitive) into exactly one bit.
#[must_use]
pub fn parse_privilege(s: &str) -> Option<Privileges> {
    let p = Privileges::from_name(&s.trim().to_ascii_uppercase())?;
    (p.bits().count_ones() == 1).then_some(p)
}

/// Whether `text` contains something shaped like a Discord snowflake (a run
/// of 17–20 ASCII digits). The audit log records that an identity was looked
/// at, never the identity itself (D-D-14); a reason or detail that would smuggle
/// one in is refused before it is appended.
#[must_use]
pub fn contains_identity(text: &str) -> bool {
    let mut run = 0usize;
    for c in text.chars() {
        if c.is_ascii_digit() {
            run += 1;
        } else {
            if (17..=20).contains(&run) {
                return true;
            }
            run = 0;
        }
    }
    (17..=20).contains(&run)
}

/// Queue one privileges write on the outbox deferred side-channel (the
/// engine drains it into the per-command `WriteBatch`).
pub(crate) fn push_op(out: &mut Outbox, op: ModuleWrite) {
    out.defer(Deferred::Persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_shapes_are_caught_and_ordinary_numbers_are_not() {
        assert!(contains_identity("user 123456789012345678 did it"));
        assert!(contains_identity("12345678901234567"));
        assert!(contains_identity("x98765432109876543210"));
        assert!(!contains_identity("round 3, dbref #4521, unix 1700000000"));
        assert!(!contains_identity("1234567890123456"));
        assert!(!contains_identity("123456789012345678901"));
        assert!(!contains_identity(""));
    }

    #[test]
    fn privilege_names_round_trip_and_reject_sets_and_unknowns() {
        assert_eq!(
            parse_privilege("discord_identity"),
            Some(Privileges::DISCORD_IDENTITY)
        );
        assert_eq!(
            parse_privilege(" DISCORD_IDENTITY "),
            Some(Privileges::DISCORD_IDENTITY)
        );
        assert_eq!(parse_privilege("WIZARD"), None);
        assert_eq!(parse_privilege(""), None);
        assert_eq!(
            privilege_name(Privileges::DISCORD_IDENTITY).as_str(),
            "DISCORD_IDENTITY"
        );
    }
}
