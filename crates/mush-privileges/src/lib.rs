//! # mush-privileges
//!
//! Explicit-only privileges (D1, D-D-14; `docs/interface/04-discord-implementation-plan.md`
//! §3.1). A [`mush_core::Privileges`] bit is the opposite of a power in one way that
//! matters: **no role implies it** — `perm::has_privilege` never short-circuits on
//! Wizard or God — so the only way an object holds one is a deliberate grant made
//! here, and the grant, the revocation, every permitted **use** and every refused
//! attempt are appended to a durable, append-only audit ledger.
//!
//! The ledger records *that* something happened to a privilege and never *what was
//! seen*: an entry carries an actor, an action, the privilege's name, the subject and
//! a reason, and [`audit`] refuses any reason or detail shaped like an identity.
//!
//! ## How it hangs off the module API
//!
//! - **State** is one [`PrivilegesState`] slot: the audit [`mush_core::Ledger`]
//!   (open tail in RAM, sealed chunks on disk and in the archive tier) plus the
//!   transient pending archive reads.
//! - **Commands**: `@privilege/grant|revoke|list|audit|archive` (Wizard).
//! - **Public API**: [`audit`] — how other modules (`mush-server`'s `@discord`
//!   family) append to this log, since cross-namespace writes go through the
//!   owning module's API.
//! - **Boot** restores the ledger from `ModuleBootData::ledger`; the archive pages
//!   through the H1 store-read seam like `+chronicle/archive`.

mod commands;
mod data;

use std::fmt;
use std::sync::Arc;

use mush_core::module::{GameModule, ModError, ModuleBootData, ModuleCtx, Registrar};
use mush_core::{Ctx, is_log_key};

pub use data::{
    AuditAction, LOG, MAX_REASON_BYTES, MAX_RECENT_LINES, NS, PendingAuditRead, PrivilegeAction,
    PrivilegeAuditEntry, PrivilegesState, contains_identity, parse_privilege, privilege_name,
};

use data::push_op;

/// The privileges module. Hand one to `Engine::with_modules`.
pub struct PrivilegesModule;

impl PrivilegesModule {
    /// A fresh module instance.
    #[must_use]
    pub fn new() -> PrivilegesModule {
        PrivilegesModule
    }
}

impl Default for PrivilegesModule {
    fn default() -> Self {
        PrivilegesModule::new()
    }
}

impl GameModule for PrivilegesModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.state(PrivilegesState::default());
        reg.store_read_completion(Arc::new(commands::AuditArchiveRead))?;
        commands::register(reg)?;
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        let mut audit = mush_core::Ledger::new();
        audit
            .restore_from_kv(&data.ledger)
            .map_err(|error| ModError::msg(format!("privileges audit restore failed: {error}")))?;
        for (key, _) in &data.kv {
            if !is_log_key(key) {
                tracing::warn!(%key, "privileges: unknown persisted key ignored");
            }
        }
        *ctx.w
            .module_state_mut::<PrivilegesState>()
            .expect("privileges state slot installed at registration") = PrivilegesState {
            pending_reads: Default::default(),
            audit,
        };
        Ok(())
    }
}

/// Why [`audit`] refused to record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrivilegeError {
    /// The module is not loaded in this composition.
    NotLoaded,
    /// The entry would carry an identity, or the ledger refused the append.
    Refused(String),
}

impl fmt::Display for PrivilegeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PrivilegeError::NotLoaded => f.write_str("the privileges module is not loaded"),
            PrivilegeError::Refused(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for PrivilegeError {}

/// Append one entry to the privileges audit ledger, queuing the persist op on
/// the caller's batch. Call it **at authorization** — before the act it records
/// is queued off-loop — so the record means "an authorized request was made";
/// a later `Err` from the handler truncates the deferred write and the RAM tail
/// is the only thing ahead, which the next boot re-derives.
///
/// # Errors
///
/// [`PrivilegeError::NotLoaded`] if the module is absent;
/// [`PrivilegeError::Refused`] if the reason or detail contains an identity
/// (D-D-14) or the ledger refuses the append.
pub fn audit(ctx: &mut Ctx<'_>, action: PrivilegeAction) -> Result<(), PrivilegeError> {
    if action.reason.as_deref().is_some_and(contains_identity) {
        return Err(PrivilegeError::Refused(
            "an audit reason may not contain an identifier".into(),
        ));
    }
    let detail_text = serde_json::to_string(&action.detail)
        .map_err(|e| PrivilegeError::Refused(format!("audit detail does not serialize: {e}")))?;
    if contains_identity(&detail_text) {
        return Err(PrivilegeError::Refused(
            "audit detail may not contain an identifier".into(),
        ));
    }
    let entry = PrivilegeAuditEntry {
        at: ctx.w.now(),
        actor: ctx.actor,
        action: action.action,
        privilege: action.privilege.map(privilege_name),
        subject: action.subject,
        reason: action.reason,
        detail: action.detail,
    };
    let value = serde_json::to_value(&entry).expect("audit entry serializes");
    let prepared = ctx
        .w
        .module_state::<PrivilegesState>()
        .ok_or(PrivilegeError::NotLoaded)?
        .audit
        .prepare_append(LOG, value)
        .map_err(|error| PrivilegeError::Refused(format!("audit append refused: {error}")))?;
    let op = {
        let st = ctx
            .w
            .module_state_mut::<PrivilegesState>()
            .ok_or(PrivilegeError::NotLoaded)?;
        st.audit
            .apply_prepared(prepared)
            .map_err(|error| {
                PrivilegeError::Refused(format!("audit append became stale: {error}"))
            })?
            .1
    };
    push_op(ctx.out, op);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{Flags, ObjId, Privileges};
    use mush_testkit::TestWorld;

    fn world() -> (TestWorld, ObjId, ObjId) {
        let mut tw = TestWorld::with_modules(vec![Box::new(PrivilegesModule::new())]);
        let wiz = tw.spawn_player("Wiz");
        tw.world_mut().set_flag(wiz, Flags::WIZARD, true).unwrap();
        let nick = tw.spawn_player("Nick");
        tw.connect(wiz);
        tw.connect(nick);
        (tw, wiz, nick)
    }

    fn holds(tw: &TestWorld, who: ObjId) -> bool {
        tw.world()
            .get(who)
            .is_some_and(|o| o.privileges.contains(Privileges::DISCORD_IDENTITY))
    }

    #[test]
    fn grant_sets_the_bit_audits_atomically_and_tells_both_parties() {
        let (mut tw, wiz, nick) = world();
        assert!(!holds(&tw, nick));
        let out = tw.run(
            wiz,
            "@privilege/grant Nick=discord_identity=on-call rotation",
        );
        tw.assert_saw(&out, wiz, "Granted DISCORD_IDENTITY to Nick");
        tw.assert_saw(&out, nick, "granted you the DISCORD_IDENTITY privilege");
        assert!(holds(&tw, nick));
        // The bit and the audit entry land in the same batch.
        let batch = tw.drain_write_batch();
        assert!(batch.appended_to(NS, LOG), "the audit entry is queued");
        batch.asserts_atomic_with_ledger(nick, NS, LOG);
        let recent = tw
            .world()
            .module_state::<PrivilegesState>()
            .unwrap()
            .recent();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].action, AuditAction::Grant);
        assert_eq!(recent[0].actor, wiz);
        assert_eq!(recent[0].subject, Some(nick));
        assert_eq!(recent[0].privilege.as_deref(), Some("DISCORD_IDENTITY"));
        assert_eq!(recent[0].reason.as_deref(), Some("on-call rotation"));
    }

    #[test]
    fn grant_requires_wizard_a_reason_and_a_known_privilege() {
        let (mut tw, wiz, nick) = world();
        // A mortal cannot grant (min_role), even to themself.
        let out = tw.run(nick, "@privilege/grant Nick=discord_identity=please");
        tw.assert_saw(&out, nick, "Permission denied");
        assert!(!holds(&tw, nick));
        // A reason is required.
        let out = tw.run(wiz, "@privilege/grant Nick=discord_identity");
        tw.assert_saw(&out, wiz, "A reason is required");
        assert!(!holds(&tw, nick));
        // An unknown privilege is refused with the known list.
        let out = tw.run(wiz, "@privilege/grant Nick=wizard=because");
        tw.assert_saw(&out, wiz, "Unknown privilege");
        // A reason shaped like an identity is refused (D-D-14).
        let out = tw.run(
            wiz,
            "@privilege/grant Nick=discord_identity=user 123456789012345678 asked",
        );
        tw.assert_saw(&out, wiz, "may not contain an identifier");
        assert!(!holds(&tw, nick));
        assert!(
            tw.world()
                .module_state::<PrivilegesState>()
                .unwrap()
                .recent()
                .is_empty(),
            "nothing refused is recorded"
        );
    }

    #[test]
    fn revoke_clears_the_bit_and_refuses_when_not_held() {
        let (mut tw, wiz, nick) = world();
        let out = tw.run(wiz, "@privilege/revoke Nick=discord_identity=never had it");
        tw.assert_saw(&out, wiz, "does not hold DISCORD_IDENTITY");
        tw.run(wiz, "@privilege/grant Nick=discord_identity=rotation");
        assert!(holds(&tw, nick));
        let out = tw.run(wiz, "@privilege/revoke Nick=discord_identity=rotation over");
        tw.assert_saw(&out, wiz, "Revoked DISCORD_IDENTITY from Nick");
        tw.assert_saw(&out, nick, "revoked your DISCORD_IDENTITY privilege");
        assert!(!holds(&tw, nick));
        let recent = tw
            .world()
            .module_state::<PrivilegesState>()
            .unwrap()
            .recent();
        assert_eq!(
            recent.iter().map(|e| e.action).collect::<Vec<_>>(),
            vec![AuditAction::Grant, AuditAction::Revoke]
        );
    }

    #[test]
    fn list_and_audit_views_and_examine_show_the_grant() {
        let (mut tw, wiz, _nick) = world();
        let out = tw.run(wiz, "@privilege/list");
        tw.assert_saw(&out, wiz, "No one holds a privilege");
        tw.run(wiz, "@privilege/grant Nick=discord_identity=rotation");
        let out = tw.run(wiz, "@privilege");
        tw.assert_saw(&out, wiz, "Nick: DISCORD_IDENTITY");
        let out = tw.run(wiz, "@privilege/list Nick");
        tw.assert_saw(&out, wiz, "Nick: DISCORD_IDENTITY");
        let out = tw.run(wiz, "@privilege/audit");
        tw.assert_saw(&out, wiz, "Wiz grant DISCORD_IDENTITY -> Nick (rotation)");
        let out = tw.run(wiz, "@privilege/audit Nick");
        tw.assert_saw(&out, wiz, "grant DISCORD_IDENTITY -> Nick");
        let out = tw.run(wiz, "@privilege/audit Wiz");
        tw.assert_saw(&out, wiz, "grant DISCORD_IDENTITY -> Nick");
        // Nothing is archived yet.
        let out = tw.run(wiz, "@privilege/archive");
        tw.assert_saw(&out, wiz, "Nothing of the privilege audit is archived yet");
        // The grant is not private: examine shows it.
        let out = tw.run(wiz, "examine Nick");
        tw.assert_saw(&out, wiz, "Privileges: DISCORD_IDENTITY");
    }

    #[test]
    fn the_identity_gate_sees_numbers_and_strings_in_detail() {
        // `audit` serializes `detail` before checking it, so a numeric
        // snowflake is caught exactly like a textual one (D-D-14); ordinary
        // detail passes.
        let numeric = serde_json::json!({ "user": 123_456_789_012_345_678_u64 });
        assert!(contains_identity(&serde_json::to_string(&numeric).unwrap()));
        let textual = serde_json::json!({ "note": "id 123456789012345678" });
        assert!(contains_identity(&serde_json::to_string(&textual).unwrap()));
        let fine =
            serde_json::json!({ "outcome": "shown", "dm_reachable": true, "at": 1_700_000_000 });
        assert!(!contains_identity(&serde_json::to_string(&fine).unwrap()));
    }
}
