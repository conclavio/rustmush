#![cfg(test)]

//! Doc 46 CT.0: the Registry recall's committed snapshot
//! (`tests/ct0/registry_recall.snap`). Declared in `notice.rs` so the player's
//! notice and refusal text are reachable.
//!
//! A recall has one end: the permanent cause row, written once, with the filed
//! revision's status. The walk writes it for a revision its author had not
//! withdrawn and for one already withdrawn (the only other shape the row has).
//! Not reached: the provider's publication (it needs a filed revision, a rating
//! and an open commerce), so the rows are the provider's own types built here.
use super::super::authority::{Authorization, Source};
use super::super::records::{Reason, RecallOperation};
use super::*;
use crate::commerce::tests::ct0_rig::{self as rig, TestResult};
use mush_core::{ClockRef, ClockStamp};
use mush_org::{OrgId, OrgScopeId, SystemPrincipalId};
use mush_testkit::snapshot::Snapshot;

use mush_testkit::fixtures::digest as hash;
use mush_testkit::fixtures::token;

fn policy() -> TestResult<RegistryRecallPolicy> {
    Ok(RegistryRecallPolicy::new(
        SystemPrincipalId::new("roll.registry").ok_or("issuer")?,
        OrgId(4),
        OrgScopeId(6),
        vec![ObjId(7), ObjId(8)],
    )?)
}

fn operation(withdrawn_by: Option<CommitToken>) -> TestResult<RecallOperation> {
    let policy = policy()?;
    let row = RecallOperation {
        format: 1,
        operation: token(20),
        action: policy.action()?,
        actor: ObjId(7),
        authorization: Authorization {
            at: 100,
            organization_revision: 2,
            scope_revision: 3,
            sources: vec![
                Source {
                    tag: "delegation".into(),
                    revision: 4,
                },
                Source {
                    tag: "grant".into(),
                    revision: 5,
                },
            ],
        },
        filed: FiledRevisionId::new(3)?,
        revision: serde_json::from_value(json!({"format":1,
            "revision":{"key":{"character":1,"mind":1,"number":1},"digest":hash(1)},
            "source":hash(2)}))?,
        rating: serde_json::from_value(
            json!({"job":token(30),"id":hash(31),"schema":1,"receipt_digest":hash(32)}),
        )?,
        initial_operation: token(1),
        initial_issuer: policy.issuer.clone(),
        withdrawn_by,
        reason: Reason::RegistryReview,
        at_ic: ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at: 100,
        },
        authority: policy,
    };
    row.validate()?;
    Ok(row)
}

#[test]
fn ct0_registry_recall_snapshot() -> TestResult {
    let (_engine, f) = rig::families()?;
    let mut s = Snapshot::new();
    s.section("registry recall: identity strings");
    s.line("capability", REGISTRY_RECALL_CAPABILITY);
    s.line("command", COMMAND);
    rig::family(&mut s, &f.registry_recalls);
    rig::family(&mut s, &f.status);

    s.section("registry recall: protocol");
    let policy = policy()?;
    for (n, d) in policy.definitions()?.iter().enumerate() {
        rig::descriptor(&mut s, &format!("definition {n}"), crate::NS, d)?;
    }
    s.wire("action", &policy.action()?)?;

    for (title, withdrawn) in [
        ("recalled", None),
        ("recalled after withdrawal", Some(token(15))),
    ] {
        s.section(&format!("registry recall: {title}"));
        let row = operation(withdrawn)?;
        let bytes = rig::document(&mut s, "operation", &f.registry_recalls, &row)?;
        rig::read_back(&bytes, &row)?;
        s.wire("cause digest", &row.digest()?)?;
        let root = row.root()?;
        s.line("cause root", &root.anchor.record());
        s.wire("cause root identity", &root.identity)?;
        let status = row.status();
        let bytes = rig::document(&mut s, "status", &f.status, &status)?;
        rig::read_back(&bytes, &status)?;
        let notice = Notice {
            actor: ObjId(5),
            licence: LicenceId::new(9)?,
            filed: row.filed,
            operation: row.operation,
        };
        s.text("notice", &notice.text());
        s.codec(&format!("RecallOperation ({title})"), &row, |j| {
            j["format"] = 2.into()
        })?;
    }
    s.section("registry recall: refusal");
    s.text("unavailable", &unavailable().to_string());
    s.codec_shape("RegistryRecallPolicy", &policy)?;
    s.codec_shape("Authorization", &operation(None)?.authorization)?;
    s.check(env!("CARGO_MANIFEST_DIR"), "tests/ct0/registry_recall.snap")?;
    Ok(())
}
