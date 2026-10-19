#![cfg(test)]

//! Structural coherence tests only. The full registered Engine fixture proves
//! actual E4 and root authority; these reject individually lawful crossed rows
//! before the publishing builder derives a root or hands off writer custody.

use super::*;
use crate::commerce::rating::{RatingClass, RatingSubject, test_support::complete_rating};
use crate::{Envelope, RevisionKey};
use mush_core::{ClockRef, CommitToken, RequestUid, mechanics::DefinitionDigest};
use mush_economy::{Amount, CatalogRef, TransactionId};
use mush_org::SystemPrincipalId;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn original_filing_rejects_structurally_valid_marked_source() -> TestResult {
    let mut f = fixture(1)?;
    let parent = fixture(2)?;
    f.subject.revision.format = crate::model::DERIVATIVE_FORMAT;
    f.subject.revision.provenance = Some(crate::model::DerivativeProvenance {
        parent: crate::commerce::filed::ParentContent {
            filed: parent.rows.filed.content.id(),
            revision: parent.subject.identity,
        },
        charter: parent.rows.charter.id,
        line: parent.rows.line.id,
    });
    // Executable digest/subject/term/economic rows are otherwise unchanged.
    // Only owner provenance prevents laundering the child as parent=None.
    f.subject.revision.validate()?;
    assert!(f.check(&f.rows).is_err());
    Ok(())
}
fn token(byte: u8) -> CommitToken {
    CommitToken::from_bytes([byte; 16])
}
fn digest(byte: u8) -> DefinitionDigest {
    DefinitionDigest::from_bytes([byte; 32])
}

struct Fixture {
    subject: Subject,
    rating: RatingReceipt,
    rows: InitialRows,
    expected_window: TermWindow,
}
impl Fixture {
    fn check(&self, rows: &InitialRows) -> Result<(), PublicationError> {
        rows.check_subject(
            &self.subject,
            self.rows.operation.commit,
            &self.rows.operation.economic,
            &self.rating,
            self.expected_window,
        )
    }
}
fn fixture(byte: u8) -> Result<Fixture, Box<dyn std::error::Error>> {
    let source = format!("fn think() {{ let value = {byte}; }}");
    let envelope = Envelope {
        schema: json!({"sections":[]}),
        presets: json!({}),
        profiles: json!({"Default":{}}),
        rules: json!({"Default":[]}),
        declared_class: "I".into(),
        mind_abi: "test=1".into(),
    };
    let revision = Revision {
        format: crate::FORMAT,
        provenance: None,
        reference: RevisionRef {
            key: RevisionKey {
                character: ObjId(7),
                mind: 1,
                number: u64::from(byte),
            },
            digest: envelope.digest(&source)?,
        },
        source: crate::source_digest(&source),
        envelope,
        notes: String::new(),
    };
    let subject = Subject {
        parent: None,
        actor: ObjId(7),
        identity: RevisionIdentity::capture(&revision)?,
        revision,
        issued: ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at: 100,
        },
    };
    let schema = schema_line_digest(subject.revision.envelope())?;
    let rating = complete_rating(
        RatingSubject::new(
            subject.identity,
            digest(20),
            schema,
            digest(byte),
            digest(21),
            RatingClass::I,
        ),
        RatingCompatibility::combat_v1(digest(22)),
        digest(23),
        token(30 + byte),
        subject.issued.clone(),
    )?
    .receipt;
    let charter = CharterId::from_object(ObjId(20 + u32::from(byte)))?;
    let line = LineId::new(u64::from(byte))?;
    let filed = FiledRevisionId::new(u64::from(byte))?;
    let licence = LicenceId::new(u64::from(byte))?;
    let term = TermGeneration::new(1)?;
    let commit = token(byte);
    let window = InitialFilingTerms::new(3600, 100)?.window(&subject.issued)?;
    let shell: ShellKey = serde_json::from_value(json!({"requester":subject.actor,
        "request":RequestUid::from_bytes([byte;16])}))?;
    let duplicate = canonical_duplicate_digest(&subject.revision)?;
    let operation = InitialFilingOperation {
        parent: None,
        format: PUBLICATION_FORMAT,
        commit,
        requester: subject.actor,
        shell,
        shell_receipt: token(byte + 10),
        charter,
        line,
        filed,
        licence,
        term,
        revision: subject.identity,
        rating: rating.reference()?,
        economic: FilingEconomicReceipt {
            house: None,
            transaction: TransactionId(u64::from(byte)),
            journal_seq: u64::from(byte),
            title: charter.object(),
            issued_owner: PrincipalId::Object(subject.actor),
            title_revision: 1,
            payer: AccountId(0),
            sink: AccountId(1),
            fee: Amount::from_minor(100),
            issuer: SystemPrincipalId::new("roll.registry").ok_or("issuer")?,
            catalog: CatalogRef {
                ruleset: "roll".into(),
                code: "mind_charter".into(),
            },
        },
    };
    let rows = InitialRows {
        charter: CharterRecord {
            format: PUBLICATION_FORMAT,
            id: charter,
            name: "Original".into(),
            shell_receipt: operation.shell_receipt,
            operation: commit,
            line,
        },
        line: LineRecord {
            format: PUBLICATION_FORMAT,
            id: line,
            charter,
            schema_digest: schema,
            head: filed,
            next_sequence: 2,
            operation: commit,
        },
        filed: FiledRecord {
            format: PUBLICATION_FORMAT,
            content: FiledRevisionContent::capture(
                filed,
                FilingPosition {
                    charter,
                    line,
                    sequence: FilingSequence::new(1)?,
                },
                &subject.revision,
                WorldIcSeconds::from_stamp(&subject.issued)?,
                None,
            )?,
            definition: rating.subject().definition(),
            duplicate,
            rating: rating.reference()?,
            operation: commit,
        },
        status: RevisionStatus {
            format: PUBLICATION_FORMAT,
            filed,
            withdrawn_by: None,
            recalled_by: None,
        },
        licence: OwnerLicence {
            format: PUBLICATION_FORMAT,
            id: licence,
            charter,
            line,
            term,
            operation: commit,
        },
        term: OwnerTerm {
            attestation: None,
            parent: None,
            format: PUBLICATION_FORMAT,
            licence,
            generation: term,
            window,
            revision: RevisionPolicy::Follow { line },
            operation: commit,
        },
        identity: FiledIdentityIndex {
            format: PUBLICATION_FORMAT,
            duplicate,
            revision: subject.identity,
            filed,
        },
        position: FiledPositionIndex {
            format: PUBLICATION_FORMAT,
            line,
            filed,
        },
        operation,
    };
    let f = Fixture {
        subject,
        rating,
        rows,
        expected_window: window,
    };
    f.check(&f.rows)?;
    Ok(f)
}

#[test]
fn individually_valid_crossed_revision_shell_rating_economic_and_root_rows_refuse() -> TestResult {
    let a = fixture(1)?;
    let b = fixture(2)?;
    let mut mixed = a.rows.clone();
    mixed.filed = b.rows.filed.clone();
    mixed.filed.validate()?;
    assert_eq!(a.check(&mixed), Err(PublicationError::Mismatch));
    mixed = a.rows.clone();
    mixed.charter = b.rows.charter.clone();
    mixed.charter.validate()?;
    assert_eq!(a.check(&mixed), Err(PublicationError::Mismatch));
    mixed = a.rows.clone();
    mixed.operation.economic = b.rows.operation.economic.clone();
    mixed.operation.charter = b.rows.operation.charter;
    mixed.operation.validate()?;
    assert_eq!(a.check(&mixed), Err(PublicationError::Mismatch));
    mixed = a.rows.clone();
    mixed.filed.rating = b.rating.reference()?;
    mixed.operation.rating = mixed.filed.rating.clone();
    mixed.filed.validate()?;
    mixed.operation.validate()?;
    assert_eq!(a.check(&mixed), Err(PublicationError::Mismatch));
    mixed = a.rows.clone();
    mixed.identity = b.rows.identity.clone();
    mixed.identity.validate()?;
    assert_eq!(a.check(&mixed), Err(PublicationError::Mismatch));
    Ok(())
}

#[test]
fn lawful_but_uncaptured_definition_dates_and_window_cannot_change_the_bundle() -> TestResult {
    let a = fixture(1)?;
    let mut mixed = a.rows.clone();
    mixed.filed.definition = digest(99);
    mixed.filed.validate()?;
    assert_eq!(a.check(&mixed), Err(PublicationError::Mismatch));
    mixed = a.rows.clone();
    mixed.term.window = InitialFilingTerms::new(7200, 100)?.window(&a.subject.issued)?;
    mixed.term.validate()?;
    assert_eq!(a.check(&mixed), Err(PublicationError::Mismatch));
    mixed = a.rows.clone();
    let later = ClockStamp {
        at: 101,
        ..a.subject.issued.clone()
    };
    mixed.filed.content = FiledRevisionContent::capture(
        a.rows.filed.content.id(),
        a.rows.filed.content.position(),
        &a.subject.revision,
        WorldIcSeconds::from_stamp(&later)?,
        None,
    )?;
    mixed.filed.validate()?;
    assert_eq!(a.check(&mixed), Err(PublicationError::Mismatch));
    Ok(())
}
