#![cfg(test)]

//! Doc 46 CT.0: the Registry revocation's committed snapshot
//! (`tests/ct0/registry_revocation.snap`). Declared in `notice.rs` so the
//! player's notice and refusal text are reachable.
//!
//! A revocation has one end: the permanent cause row and the revoked-term status
//! keyed by licence and generation. The walk writes it for each accepted term
//! shape the row has: an owner's Follow term and a paid Pin term.
//! Not reached: the provider's publication (it needs the term's owner history and
//! an open commerce), so the rows are the provider's own types built here.
use super::super::authority::{Authorization, Source};
use super::super::records::{AcceptedTerm, Reason, RevocationOperation};
use super::*;
use crate::commerce::paid::{PaidForm, PaidTermRecord, PaidTerms};
use crate::commerce::rating::RatingClass;
use crate::commerce::records::{OwnerTerm, RevisionPolicy};
use crate::commerce::tests::ct0_rig::{self as rig, TestResult};
use crate::commerce::{
    CharterId, FiledRevisionId, LineId, RevisionIdentity, TermWindow, WorldIcSeconds,
};
use mush_core::{ClockRef, ClockStamp, DefinitionDigest};
use mush_org::{OrgId, OrgScopeId, SystemPrincipalId};
use mush_testkit::snapshot::Snapshot;

fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
fn hash(n: u8) -> DefinitionDigest {
    DefinitionDigest::from_bytes([n; 32])
}
fn identity() -> TestResult<RevisionIdentity> {
    Ok(serde_json::from_value(json!({"format":1,
        "revision":{"key":{"character":1,"mind":1,"number":1},"digest":hash(1)},
        "source":hash(2)}))?)
}
fn window() -> TestResult<TermWindow> {
    Ok(TermWindow::new(
        WorldIcSeconds(10),
        WorldIcSeconds(10),
        WorldIcSeconds(110),
        WorldIcSeconds(130),
    )?)
}
fn policy() -> TestResult<RegistryRevocationPolicy> {
    Ok(RegistryRevocationPolicy::new(
        SystemPrincipalId::new("roll.registry").ok_or("issuer")?,
        OrgId(4),
        OrgScopeId(6),
        vec![ObjId(7), ObjId(8)],
    )?)
}

fn operation(term: AcceptedTerm) -> TestResult<RevocationOperation> {
    let policy = policy()?;
    let row = RevocationOperation {
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
        term,
        charter: CharterId::new(9)?,
        line: LineId::new(2)?,
        birth_operation: token(1),
        initial_operation: token(1),
        initial_issuer: policy.issuer.clone(),
        filed: FiledRevisionId::new(3)?,
        revision: identity()?,
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
fn ct0_registry_revocation_snapshot() -> TestResult {
    let (_engine, f) = rig::families()?;
    let mut s = Snapshot::new();
    s.section("registry revocation: identity strings");
    s.line("capability", REGISTRY_REVOCATION_CAPABILITY);
    s.line("command", COMMAND);
    rig::family(&mut s, &f.registry_revocations);
    rig::family(&mut s, &f.revoked_terms);

    s.section("registry revocation: protocol");
    let policy = policy()?;
    for (n, d) in policy.definitions()?.iter().enumerate() {
        rig::descriptor(&mut s, &format!("definition {n}"), crate::NS, d)?;
    }
    s.wire("action", &policy.action()?)?;

    let owner = AcceptedTerm::Owner {
        term: Box::new(OwnerTerm {
            format: 1,
            licence: LicenceId::new(9)?,
            generation: TermGeneration::new(1)?,
            window: window()?,
            revision: RevisionPolicy::Follow {
                line: LineId::new(2)?,
            },
            operation: token(2),
            parent: None,
            attestation: None,
        }),
    };
    let paid = AcceptedTerm::Paid {
        term: Box::new(PaidTermRecord::original(
            LicenceId::new(9)?,
            TermGeneration::new(1)?,
            window()?,
            PaidTerms::original_personal_form(RatingClass::I, PaidForm::Sealed {}, false)?,
            RevisionPolicy::Pin {
                filed: FiledRevisionId::new(3)?,
                revision: identity()?,
            },
            token(2),
        )?),
    };
    for (title, term) in [("owner term revoked", owner), ("paid term revoked", paid)] {
        s.section(&format!("registry revocation: {title}"));
        let row = operation(term)?;
        let bytes = rig::document(&mut s, "operation", &f.registry_revocations, &row)?;
        rig::read_back(&bytes, &row)?;
        s.wire("cause digest", &row.digest()?)?;
        s.wire("accepted term digest", &row.term.digest()?)?;
        s.line("status key", &row.term.status_key());
        let root = row.root()?;
        s.line("cause root", &root.anchor.record());
        s.wire("cause root identity", &root.identity)?;
        let status = row.status()?;
        status.matches(&row)?;
        let bytes = rig::document(&mut s, "revoked term", &f.revoked_terms, &status)?;
        rig::read_back(&bytes, &status)?;
        let notice = Notice {
            actor: ObjId(5),
            licence: status.licence,
            generation: status.generation,
            grace_end: status.window.grace_end(),
            operation: row.operation,
        };
        s.text("notice", &notice.text());
        s.codec(&format!("RevocationOperation ({title})"), &row, |j| {
            j["format"] = 2.into()
        })?;
        s.codec(&format!("RevokedTerm ({title})"), &status, |j| {
            j["format"] = 2.into()
        })?;
    }
    s.section("registry revocation: refusal");
    s.text("unavailable", &unavailable().to_string());
    s.codec_shape("RegistryRevocationPolicy", &policy)?;
    s.check(
        env!("CARGO_MANIFEST_DIR"),
        "tests/ct0/registry_revocation.snap",
    )?;
    Ok(())
}
