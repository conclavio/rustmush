#![cfg(test)]

//! These test records exercise codecs and owner fences. A structural RatingRef
//! is deliberately not a certified receipt and cannot pass the complete audit.

use mush_core::{
    ClockRef, ClockStamp, CommitToken, Engine, GameModule, ModError, ModuleBootData, ModuleCtx,
    ModuleOp, ModuleWrite, ObjId, Outbox, Registrar, Registry, World, mechanics::DefinitionDigest,
};
use mush_economy::{AccountId, Amount, TransactionId};
use mush_org::{PrincipalId, SystemPrincipalId};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use super::families::CommerceFamilies;
use super::filed::{FiledRevisionContent, FilingPosition, FilingSequence};
use super::rating::{RatingFamilies, RatingRef};
use super::records::*;
use super::roots::*;
use super::{
    CharterId, FiledRevisionId, LicenceId, LineId, RevisionIdentity, TermGeneration, TermWindow,
    WorldIcSeconds,
};
use crate::*;

const ACTOR: ObjId = ObjId(10);
fn token(n: u8) -> CommitToken {
    CommitToken::from_hex(&format!("{n:032x}")).unwrap()
}
fn digest(n: u8) -> DefinitionDigest {
    DefinitionDigest::from_hex(&format!("{n:064x}")).unwrap()
}
fn now(n: i64) -> WorldIcSeconds {
    WorldIcSeconds::from_stamp(&ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at: n,
    })
    .unwrap()
}
fn draft() -> WorkingCopy {
    WorkingCopy {
        source: "fn think() {}".into(),
        envelope: Envelope {
            schema: json!({"sections":[]}),
            presets: json!({}),
            profiles: json!({"Default":{}}),
            rules: json!({}),
            declared_class: "I".into(),
            mind_abi: "test=1".into(),
        },
    }
}

#[test]
fn schema_line_identity_survives_new_executable_or_runtime_but_not_schema_changes() {
    let original = draft();
    let line = schema_line_digest(&original.envelope).unwrap();
    let mut changed = original.clone();
    changed.source = "fn think() { let updated = true; }".into();
    changed.envelope.mind_abi = "test=2".into();
    changed.envelope.declared_class = "II".into();
    changed.envelope.profiles = json!({"Changed":{}});
    changed.envelope.rules = json!({"Changed":[]});
    changed.envelope.presets = json!({"Preset":{}});
    assert_ne!(
        original.envelope.digest(&original.source).unwrap(),
        changed.envelope.digest(&changed.source).unwrap()
    );
    assert_eq!(schema_line_digest(&changed.envelope).unwrap(), line);

    changed.envelope.schema = json!({"sections":[{"name":"Helm","fields":[]}]});
    assert_ne!(schema_line_digest(&changed.envelope).unwrap(), line);
}

#[test]
fn abi_only_revision_keeps_duplicate_key_but_source_schema_and_program_changes_do_not() {
    let (_, _, _, original) = saved();
    let source = draft().source;
    let duplicate = canonical_duplicate_digest(&original).unwrap();
    let revise = |mut revision: Revision, source: &str| {
        revision.source = source_digest(source);
        revision.reference.key.number += 1;
        revision.reference.digest = revision.envelope.digest(source).unwrap();
        revision.validate().unwrap();
        revision
    };
    let mut abi = original.clone();
    abi.envelope.mind_abi = "test=2".into();
    let abi = revise(abi, &source);
    assert_ne!(abi.reference().digest, original.reference().digest);
    assert_eq!(canonical_duplicate_digest(&abi).unwrap(), duplicate);
    let changed_source = revise(original.clone(), "fn think() { let changed = true; }");
    assert_ne!(
        canonical_duplicate_digest(&changed_source).unwrap(),
        duplicate
    );
    let mut schema = original.clone();
    schema.envelope.schema = json!({"sections":[{"name":"Helm","fields":[]}]});
    assert_ne!(
        canonical_duplicate_digest(&revise(schema, &source)).unwrap(),
        duplicate
    );
    let mut program = original.clone();
    program.envelope.rules = json!({"Default":[]});
    assert_ne!(
        canonical_duplicate_digest(&revise(program, &source)).unwrap(),
        duplicate
    );

    let first = FiledIdentityIndex {
        format: PUBLICATION_FORMAT,
        duplicate,
        revision: RevisionIdentity::capture(&original).unwrap(),
        filed: FiledRevisionId::new(1).unwrap(),
    };
    let second = FiledIdentityIndex {
        format: PUBLICATION_FORMAT,
        duplicate: canonical_duplicate_digest(&abi).unwrap(),
        revision: RevisionIdentity::capture(&abi).unwrap(),
        filed: FiledRevisionId::new(2).unwrap(),
    };
    assert_eq!(first.key(), second.key());
    let (mut engine, families) = engine();
    let mut txn = engine.action_transaction(&families.identities).unwrap();
    txn.insert(&families.identities, &first.key(), first.clone())
        .unwrap();
    let (_, install) = txn.prepare().unwrap().into_parts();
    engine.install_prepared_action(install).unwrap();
    let mut txn = engine.action_transaction(&families.identities).unwrap();
    assert!(
        txn.insert(&families.identities, &second.key(), second)
            .is_err(),
        "ABI change cannot bypass the actual unique typed key"
    );
    drop(txn);
    assert_eq!(
        engine
            .action_state()
            .get(&families.identities, &first.key())
            .unwrap(),
        Some(&first)
    );
    let mut wire = serde_json::to_value(&first).unwrap();
    wire.as_object_mut().unwrap().remove("duplicate");
    assert!(serde_json::from_value::<FiledIdentityIndex>(wire).is_err());
}
struct Language;
impl RevisionAdapter for Language {
    fn normalize_source(&self, source: &str) -> Result<String, RollError> {
        Ok(source.into())
    }
    fn validate(&self, _: &str, _: &Envelope) -> Result<(), RollError> {
        Ok(())
    }
}
#[derive(Default)]
pub(super) struct Sources(BTreeMap<DefinitionDigest, String>);
impl SourceAdapter for Sources {
    type Token = (DefinitionDigest, String);
    fn prepare_source(
        &self,
        source: &str,
        _: &str,
    ) -> Result<PreparedSource<Self::Token>, RollError> {
        let digest = source_digest(source);
        Ok(PreparedSource {
            digest,
            operations: Vec::new(),
            token: (digest, source.into()),
        })
    }
    fn read_source(&self, digest: DefinitionDigest) -> Result<&str, RollError> {
        self.0
            .get(&digest)
            .map(String::as_str)
            .ok_or(RollError::Missing)
    }
}
type Disk = BTreeMap<String, Value>;
fn write(disk: &mut Disk, ops: &[ModuleOp]) {
    for op in ops {
        assert_eq!(op.ns, NS);
        match &op.op {
            ModuleWrite::JsonSet { key, value } => {
                disk.insert(key.to_string(), value.clone());
            }
            ModuleWrite::Del { key } => {
                disk.remove(key.as_str());
            }
            ModuleWrite::StreamAppend { .. } => panic!("no Roll streams"),
        }
    }
}
fn boot(disk: &Disk) -> ModuleBootData {
    ModuleBootData {
        kv: disk
            .iter()
            .map(|(key, value)| (key.as_str().into(), value.clone()))
            .collect(),
        ..Default::default()
    }
}
pub(super) fn saved() -> (RollStore, Sources, Disk, Revision) {
    let mut store = RollStore::default();
    let mut sources = Sources::default();
    let mut disk = Disk::new();
    let created = store.prepare_create(ACTOR, "Draft", draft()).unwrap();
    store
        .publish_with(created, |ops| write(&mut disk, ops))
        .unwrap();
    let base = store.mind(ACTOR, "Draft").unwrap().base();
    let saved = store
        .prepare_save(ACTOR, "Draft", base, "", None, &Language, &sources)
        .unwrap();
    let (change, source, reference) = saved.into_parts();
    sources.0.insert(source.token.0, source.token.1);
    store
        .publish_with(change, |ops| write(&mut disk, ops))
        .unwrap();
    let revision = store.revision(ACTOR, reference.key).unwrap().clone();
    let mut store = RollStore::load_commerce(&boot(&disk)).unwrap();
    store.audit_sources(&Language, &sources).unwrap();
    (store, sources, disk, revision)
}

fn rows(revision: &Revision) -> (FiledRecord, OwnerTerm, InitialFilingOperation) {
    let charter = CharterId::new(20).unwrap();
    let line = LineId::new(1).unwrap();
    let filed = FiledRevisionId::new(1).unwrap();
    let licence = LicenceId::new(1).unwrap();
    let term = TermGeneration::new(1).unwrap();
    let identity = RevisionIdentity::capture(revision).unwrap();
    let rating: RatingRef =
        serde_json::from_value(json!({"job":CommitToken::from_bytes([2;16]),"id":digest(2),"schema":1,"receipt_digest":digest(3)}))
            .unwrap();
    let content = FiledRevisionContent::capture(
        filed,
        FilingPosition {
            charter,
            line,
            sequence: FilingSequence::new(1).unwrap(),
        },
        revision,
        now(100),
        None,
    )
    .unwrap();
    let file = FiledRecord {
        format: PUBLICATION_FORMAT,
        content,
        definition: digest(4),
        duplicate: canonical_duplicate_digest(revision).unwrap(),
        rating: rating.clone(),
        operation: token(2),
    };
    let term_row = OwnerTerm {
        attestation: None,
        parent: None,
        format: PUBLICATION_FORMAT,
        licence,
        generation: term,
        window: TermWindow::new(now(100), now(100), now(200), now(210)).unwrap(),
        revision: RevisionPolicy::Pin {
            filed,
            revision: identity,
        },
        operation: token(2),
    };
    let operation = InitialFilingOperation {
        parent: None,
        format: PUBLICATION_FORMAT,
        commit: token(2),
        requester: ACTOR,
        shell: serde_json::from_value(json!({
            "requester":ACTOR,
            "request":mush_core::action::RequestUid::from_bytes([1;16]),
        }))
        .unwrap(),
        shell_receipt: token(1),
        charter,
        line,
        filed,
        licence,
        term,
        revision: identity,
        rating,
        economic: FilingEconomicReceipt {
            house: None,
            transaction: TransactionId(1),
            journal_seq: 1,
            title: charter.object(),
            issued_owner: PrincipalId::Object(ACTOR),
            title_revision: 1,
            payer: AccountId(1),
            sink: AccountId(2),
            fee: Amount::ZERO,
            issuer: SystemPrincipalId::new("test.registry").unwrap(),
            catalog: mush_economy::CatalogRef {
                ruleset: "roll".into(),
                code: "mind_charter".into(),
            },
        },
    };
    (file, term_row, operation)
}

#[test]
fn house_attribution_is_required_only_in_the_scoped_operation_format()
-> Result<(), Box<dyn std::error::Error>> {
    let (_, _, _, revision) = saved();
    let (_, _, personal) = rows(&revision);
    let old = serde_json::to_value(&personal)?;
    assert!(old.get("economic").and_then(|v| v.get("house")).is_none());
    let roundtrip: InitialFilingOperation = serde_json::from_value(old.clone())?;
    assert_eq!(serde_json::to_value(&roundtrip)?, old);
    let mut null_attribution = old.clone();
    null_attribution
        .get_mut("economic")
        .and_then(Value::as_object_mut)
        .ok_or("economic")?
        .insert("house".into(), Value::Null);
    assert!(serde_json::from_value::<InitialFilingOperation>(null_attribution).is_err());
    let mut house = personal.clone();
    house.format = 2;
    house.economic.issued_owner = PrincipalId::Organization(mush_org::OrgId(7));
    house.economic.house = Some(mush_economy::registry_filing::HouseFilingAttribution::new(
        ACTOR,
        mush_org::OrgId(7),
        mush_org::OrgScopeId(9),
    )?);
    let encoded = serde_json::to_value(&house)?;
    let decoded: InitialFilingOperation = serde_json::from_value(encoded.clone())?;
    assert_eq!(decoded, house);
    assert_eq!(
        decoded.audit_subject()?.house(),
        house.economic.house.as_ref()
    );
    for mode in 0..5 {
        let mut crossed = encoded.clone();
        match mode {
            0 => {
                *crossed.get_mut("format").ok_or("format")? = json!(1);
            }
            1 => {
                crossed
                    .get_mut("economic")
                    .and_then(Value::as_object_mut)
                    .ok_or("economic")?
                    .remove("house");
            }
            2 => {
                *crossed
                    .get_mut("economic")
                    .and_then(|v| v.get_mut("house"))
                    .and_then(|v| v.get_mut("curator"))
                    .ok_or("curator")? = json!(ACTOR.0 + 1);
            }
            3 => {
                *crossed
                    .get_mut("economic")
                    .and_then(|v| v.get_mut("house"))
                    .and_then(|v| v.get_mut("organization"))
                    .ok_or("organization")? = json!(8);
            }
            _ => {
                *crossed
                    .get_mut("economic")
                    .and_then(|v| v.get_mut("house"))
                    .and_then(|v| v.get_mut("format"))
                    .ok_or("attribution format")? = json!(2);
            }
        }
        assert!(
            serde_json::from_value::<InitialFilingOperation>(crossed).is_err(),
            "mode {mode}"
        );
    }
    let mut missing = old;
    *missing.get_mut("format").ok_or("format")? = json!(2);
    assert!(serde_json::from_value::<InitialFilingOperation>(missing).is_err());
    Ok(())
}

/// Isolate the generation/bind unit contract. This does not claim a healthy
/// deployment or exercise the complete typed/rating/economic audit below.
pub(super) fn open_gate_for_owner_unit_test(store: &mut RollStore) {
    let (generation, epoch) = store.commerce_audit_base().unwrap();
    store.finish_commerce_audit(generation, epoch).unwrap();
}

#[test]
fn commerce_starts_closed_even_with_zero_roots_and_source_audit_does_not_open_it() {
    let mut store = RollStore::load_commerce(&ModuleBootData::default()).unwrap();
    assert!(store.prepare_prune(1).is_err());
    store.audit_sources(&Language, &Sources::default()).unwrap();
    assert!(store.prepare_prune(1).is_err());
    assert!(store.commerce_ready().is_err());
    assert!(
        RollStore::default().prepare_prune(1).is_ok(),
        "M6 mode unchanged"
    );
}

#[test]
fn initialization_requires_clean_closed_source_audit_and_no_existing_commercial_roots() {
    let mut empty = RollStore::load_commerce(&ModuleBootData::default()).unwrap();
    assert!(empty.commerce_initialization_base().is_err());
    empty.audit_sources(&Language, &Sources::default()).unwrap();
    assert_eq!(
        empty.commerce_initialization_base().unwrap(),
        empty.commerce_audit_base().unwrap()
    );
    assert!(
        empty.commerce_ready().is_err(),
        "emptiness never opens readiness"
    );
    assert!(RollStore::default().commerce_initialization_base().is_err());
    empty
        .quarantine_collection("tacsim", "unreadable-retained-data")
        .unwrap();
    assert!(empty.commerce_initialization_base().is_err());

    let (mut populated, _, _, revision) = saved();
    // Ordinary M6 drafts/revisions are permitted; only private commercial
    // retention prevents an otherwise explicit first installation.
    assert!(populated.commerce_initialization_base().is_ok());
    open_gate_for_owner_unit_test(&mut populated);
    assert!(populated.commerce_initialization_base().is_err());
    let (file, term, operation) = rows(&revision);
    let roots = InitialRoots::from_records(&file, &term, &operation).unwrap();
    let prepared = populated.prepare_commercial_roots(&roots).unwrap();
    populated.bind_commercial_roots(prepared).unwrap().apply();
    populated.close_commerce_for_audit().unwrap();
    assert!(populated.commerce_audit_base().is_ok());
    assert!(populated.commerce_initialization_base().is_err());
}

#[test]
fn every_typed_obligation_derives_an_exact_root_and_all_missing_is_refused() {
    let (store, _, _, revision) = saved();
    let (file, term, operation) = rows(&revision);
    let request = InitialRoots::from_records(&file, &term, &operation).unwrap();
    assert_eq!(request.roots().len(), 3);
    assert!(store.commercial_roots().next().is_none());
    for root in request.roots() {
        assert!(store.check_root_subject(root).is_ok());
        assert!(
            store.check_required_root(root).is_err(),
            "absence is not an empty obligation list"
        );
    }
    let mut follow = term.clone();
    follow.revision = RevisionPolicy::Follow {
        line: operation.line,
    };
    assert_eq!(
        InitialRoots::from_records(&file, &follow, &operation)
            .unwrap()
            .roots()
            .len(),
        2
    );
    let mut other = operation.clone();
    other.licence = LicenceId::new(2).unwrap();
    assert!(InitialRoots::from_records(&file, &term, &other).is_err());
}

#[test]
fn filing_operation_requires_the_exact_requesters_reservation_key() {
    let (_, _, _, revision) = saved();
    let (_, _, mut operation) = rows(&revision);
    operation.validate().unwrap();
    let mut wire = serde_json::to_value(&operation).unwrap();
    wire.as_object_mut().unwrap().remove("shell");
    assert!(serde_json::from_value::<InitialFilingOperation>(wire).is_err());
    operation.shell = serde_json::from_value(json!({
        "requester":ObjId(ACTOR.0 + 1),
        "request":mush_core::action::RequestUid::from_bytes([1;16]),
    }))
    .unwrap();
    assert_eq!(operation.validate(), Err(PublicationError::Invalid));
}

#[test]
fn initial_filing_retains_exact_catalog_and_personal_title_history() {
    let (_, _, _, revision) = saved();
    let (_, _, operation) = rows(&revision);
    let subject = operation.audit_subject().unwrap();
    let mut first_journal_entry = operation.clone();
    first_journal_entry.economic.journal_seq = 0;
    first_journal_entry.validate().unwrap();
    assert_ne!(first_journal_entry.audit_subject().unwrap(), subject);
    let mut crossed = operation.clone();
    crossed.economic.catalog.code = "other_product".into();
    assert_ne!(crossed.audit_subject().unwrap(), subject);
    let mut wire = serde_json::to_value(&operation).unwrap();
    let catalog = wire
        .get_mut("economic")
        .unwrap()
        .get_mut("catalog")
        .unwrap()
        .as_object_mut()
        .unwrap();
    catalog.insert("authority".into(), json!(true));
    assert!(serde_json::from_value::<InitialFilingOperation>(wire).is_err());
    crossed = operation.clone();
    crossed.economic.issued_owner = PrincipalId::Object(ObjId(ACTOR.0 + 1));
    assert_eq!(crossed.validate(), Err(PublicationError::Invalid));
    crossed = operation.clone();
    crossed.economic.title_revision = 2;
    assert_eq!(crossed.validate(), Err(PublicationError::Invalid));
    crossed = operation;
    crossed.economic.payer = crossed.economic.sink;
    assert_eq!(crossed.validate(), Err(PublicationError::Invalid));
}

#[test]
fn commercial_bind_drop_is_inert_and_apply_installs_the_exact_prepared_usage() {
    let (mut store, _, mut disk, revision) = saved();
    open_gate_for_owner_unit_test(&mut store);
    let (file, term, op) = rows(&revision);
    let request = InitialRoots::from_records(&file, &term, &op).unwrap();
    let before = store.current_usage().unwrap();
    let prepared = store.prepare_commercial_roots(&request).unwrap();
    let expected = prepared.usage().unwrap();
    assert_eq!(expected.before(), before);
    assert_eq!(expected.after().records(), before.records() + 3);
    drop(store.bind_commercial_roots(prepared).unwrap());
    assert_eq!(store.current_usage(), Some(before));
    assert!(store.commercial_roots().next().is_none());
    let prepared = store.prepare_commercial_roots(&request).unwrap();
    write(&mut disk, prepared.operations());
    store.bind_commercial_roots(prepared).unwrap().apply();
    assert_eq!(store.current_usage(), Some(expected.after()));
    assert_eq!(
        expected.after().document_bytes(),
        disk.values()
            .map(|v| serde_json::to_vec(v).unwrap().len() as u64)
            .sum::<u64>()
    );
    assert_eq!(
        expected.after().key_bytes(),
        disk.keys().map(|key| key.len() as u64).sum::<u64>()
    );
    for root in request.roots() {
        assert!(store.check_required_root(root).is_ok());
    }
    assert!(
        store.prepare_commercial_roots(&request).is_err(),
        "permanent obligation cannot be replaced"
    );
    assert!(
        !RollStore::load(&boot(&disk))
            .unwrap()
            .quarantine()
            .is_empty(),
        "undeclared M6 composition refuses commercial rows"
    );
}

#[test]
fn closing_or_reauditing_invalidates_prepared_roots_and_collection() {
    let (mut store, sources, _, revision) = saved();
    open_gate_for_owner_unit_test(&mut store);
    let (file, term, op) = rows(&revision);
    let roots = InitialRoots::from_records(&file, &term, &op).unwrap();
    let prepared = store.prepare_commercial_roots(&roots).unwrap();
    let prune = store.prepare_prune(1).unwrap();
    store.close_commerce_for_audit().unwrap();
    assert!(store.bind_commercial_roots(prepared).is_err());
    assert!(store.check_prepared(&prune.change).is_err());
    open_gate_for_owner_unit_test(&mut store);
    let prepared = store.prepare_commercial_roots(&roots).unwrap();
    store.audit_sources(&Language, &sources).unwrap();
    assert!(store.bind_commercial_roots(prepared).is_err());
    assert!(store.prepare_prune(1).is_err());
}

#[test]
fn exact_commercial_codecs_reject_wrong_owner_target_shape_and_noncanonical_anchors() {
    let (mut store, _, _, revision) = saved();
    open_gate_for_owner_unit_test(&mut store);
    let root = RequiredRoot {
        anchor: RootAnchor::Filed(FiledRevisionId::new(1).unwrap()),
        identity: RevisionIdentity::capture(&revision).unwrap(),
    };
    assert!(
        store
            .prepare_reference("roll", root.anchor.key(), vec![root.target()])
            .is_err()
    );
    let mut external = root.anchor.key();
    external.owner = "tacsim".into();
    assert!(
        store
            .prepare_reference("tacsim", external, vec![root.target()])
            .is_err()
    );
    for bad in [
        "v1/filed/1",
        "v1/filed/0000000000000000",
        "v1/filed/000000000000000A",
        "v1/operation/1",
        "v1/filed/0000000000000001/extra",
        "v1/rating/00000000000000000000000000000001",
    ] {
        assert!(RootAnchor::parse(bad).is_err(), "{bad}");
    }
    let request =
        InitialRoots::from_records(&rows(&revision).0, &rows(&revision).1, &rows(&revision).2)
            .unwrap();
    let prepared = store.prepare_commercial_roots(&request).unwrap();
    use mush_core::authoritative::{AuthoritativeKey, ReplayWrite};
    use mush_core::mechanics::OwnerId;
    use mush_core::participant::OwnerParticipant;
    let adapter = super::participant::RootParticipant;
    let first = prepared.operations().first().unwrap();
    let ModuleWrite::JsonSet { key, value } = &first.op else {
        panic!("root write");
    };
    let wrap = |value| ReplayWrite::ModuleJsonSet {
        namespace: OwnerId::parse_canonical(NS).unwrap(),
        key: AuthoritativeKey::parse_canonical(key).unwrap(),
        value,
    };
    assert!(adapter.accepts_write(&wrap(value.clone())));
    let mut malformed = value.clone();
    malformed
        .as_object_mut()
        .unwrap()
        .insert("authority".into(), json!(true));
    assert!(!adapter.accepts_write(&wrap(malformed)));
    let mut wrong_count = value.clone();
    wrong_count
        .get_mut("revisions")
        .unwrap()
        .as_array_mut()
        .unwrap()
        .clear();
    assert!(!adapter.accepts_write(&wrap(wrong_count)));
    assert!(!adapter.accepts_write(&ReplayWrite::ModuleDelete {
        namespace: OwnerId::parse_canonical(NS).unwrap(),
        key: AuthoritativeKey::parse_canonical(key).unwrap()
    }));
}

#[test]
fn rating_root_binds_admission_and_measured_identity() {
    let id = super::rating::RatingId::from_hex(&digest(2).to_string()).unwrap();
    let first = RootAnchor::Rating { job: token(1), id };
    let second = RootAnchor::Rating { job: token(2), id };
    assert_ne!(first.key(), second.key());
    assert_eq!(RootAnchor::parse(&first.record()).unwrap(), first);
    assert_eq!(RootAnchor::parse(&second.record()).unwrap(), second);
    assert!(RootAnchor::parse(&format!("v1/rating/{id}")).is_err());
    assert!(RootAnchor::parse(&format!("{}/extra", first.record())).is_err());
}

fn rating_job(revision: &Revision, job: CommitToken) -> super::rating::RatingJob {
    use super::rating::{RatingClass, RatingCompatibility, RatingJob, RatingSubject};
    RatingJob::new(
        job,
        RatingSubject::new(
            RevisionIdentity::capture(revision).unwrap(),
            revision.reference().digest,
            schema_line_digest(&revision.envelope).unwrap(),
            digest(2),
            digest(3),
            RatingClass::I,
        ),
        RatingCompatibility::combat_v1(digest(4)),
        digest(5),
    )
}

fn apply_rating_roots(
    store: &mut RollStore,
    disk: &mut Disk,
    roots: &super::rating_roots::RatingRoots,
) {
    let prepared = store.prepare_rating_roots(roots).unwrap();
    write(disk, prepared.operations());
    store.bind_commercial_roots(prepared).unwrap().apply();
}

#[test]
fn rating_admission_progress_and_completion_exchange_only_the_pending_root() {
    use super::rating::{RatingJobState, test_support::complete_rating};
    use super::rating_roots::{RatingRoots, required};
    let (mut store, _, mut disk, revision) = saved();
    open_gate_for_owner_unit_test(&mut store);
    let queued = rating_job(&revision, token(7));
    let admission = RatingRoots::admission(&queued, token(7)).unwrap();
    assert!(RatingRoots::admission(&queued, token(8)).is_err());
    apply_rating_roots(&mut store, &mut disk, &admission);
    assert_eq!(store.commercial_roots().count(), 2);
    for root in required(&queued).unwrap() {
        store.check_required_root(&root).unwrap();
    }
    assert!(store.prepare_rating_roots(&admission).is_err());
    let running = queued
        .transition(RatingJobState::Running { lease_sequence: 1 })
        .unwrap();
    let progress = RatingRoots::transition(&queued, &running, None, token(8)).unwrap();
    let retained = disk.clone();
    let usage = store.current_usage().unwrap();
    let grant = store.prepare_rating_roots(&progress).unwrap();
    assert_eq!(grant.operations().len(), 2);
    assert_eq!(grant.usage().unwrap().before, usage);
    assert_eq!(grant.usage().unwrap().after, usage);
    for operation in grant.operations() {
        let ModuleWrite::JsonSet { key, value } = &operation.op else {
            panic!("progress only re-publishes retained roots");
        };
        assert_eq!(retained.get(key.as_str()), Some(value));
    }
    assert_eq!(store.current_usage().unwrap(), usage);
    assert_eq!(disk, retained, "preparation changes no durable image");
    write(&mut disk, grant.operations());
    store.bind_commercial_roots(grant).unwrap().apply();
    assert_eq!(disk, retained);
    assert_eq!(store.current_usage().unwrap(), usage);

    let settled = RatingRoots::settled_progress(&running, token(9)).unwrap();
    let stale = store.prepare_rating_roots(&settled).unwrap();
    assert_eq!(stale.operations().len(), 2);
    apply_rating_roots(&mut store, &mut disk, &settled);
    assert!(store.check_commercial_prepared(&stale).is_err());
    assert_eq!(disk, retained);
    assert_eq!(store.current_usage().unwrap(), usage);
    let complete = complete_rating(
        queued.subject().clone(),
        queued.compatibility().clone(),
        digest(5),
        queued.job(),
        ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at: 100,
        },
    )
    .unwrap();
    let finish =
        RatingRoots::transition(&running, &complete.job, Some(&complete.receipt), token(9))
            .unwrap();
    assert_eq!(
        store
            .prepare_rating_roots(&finish)
            .unwrap()
            .operations()
            .len(),
        3
    );
    apply_rating_roots(&mut store, &mut disk, &finish);
    assert_eq!(store.commercial_roots().count(), 2);
    assert!(!disk.contains_key(&RootAnchor::RatingPending(queued.job()).key().key()));
    for root in required(&complete.job).unwrap() {
        store.check_required_root(&root).unwrap();
    }
    assert!(RatingRoots::transition(&complete.job, &running, None, token(10)).is_err());
    assert!(
        store.prepare_rating_roots(&finish).is_err(),
        "a completion cannot overwrite roots"
    );
}

#[test]
fn rating_progress_requires_both_exact_retained_roots_before_any_write() {
    use super::rating::RatingJobState;
    use super::rating_roots::RatingRoots;
    let (mut store, sources, mut disk, revision) = saved();
    open_gate_for_owner_unit_test(&mut store);
    let queued = rating_job(&revision, token(7));
    apply_rating_roots(
        &mut store,
        &mut disk,
        &RatingRoots::admission(&queued, token(7)).unwrap(),
    );
    let running = queued
        .transition(RatingJobState::Running { lease_sequence: 1 })
        .unwrap();
    for missing in [
        RootAnchor::RatingJob(queued.job()),
        RootAnchor::RatingPending(queued.job()),
    ] {
        let mut broken = disk.clone();
        broken.remove(&missing.key().key());
        let mut loaded = RollStore::load_commerce(&boot(&broken)).unwrap();
        loaded.audit_sources(&Language, &sources).unwrap();
        open_gate_for_owner_unit_test(&mut loaded);
        let usage = loaded.current_usage();
        for progress in [
            RatingRoots::transition(&queued, &running, None, token(8)).unwrap(),
            RatingRoots::settled_progress(&running, token(9)).unwrap(),
        ] {
            assert!(loaded.prepare_rating_roots(&progress).is_err());
            assert_eq!(loaded.current_usage(), usage);
            assert_eq!(loaded.commercial_roots().count(), 1);
        }
    }
}

#[test]
fn rating_abort_preserves_job_evidence_and_missing_before_root_refuses_without_mutation() {
    use super::rating::{RatingFailure, RatingJobState};
    use super::rating_roots::{RatingRoots, required};
    for terminal in [
        RatingJobState::Failed {
            reason: RatingFailure::Conformance,
        },
        RatingJobState::Cancelled {},
        RatingJobState::Interrupted {},
    ] {
        let (mut store, sources, mut disk, revision) = saved();
        open_gate_for_owner_unit_test(&mut store);
        let job = rating_job(&revision, token(7));
        apply_rating_roots(
            &mut store,
            &mut disk,
            &RatingRoots::admission(&job, token(7)).unwrap(),
        );
        let ended = job.transition(terminal).unwrap();
        let abort = RatingRoots::transition(&job, &ended, None, token(8)).unwrap();
        let mut broken = disk.clone();
        broken.remove(&RootAnchor::RatingPending(job.job()).key().key());
        let mut loaded = RollStore::load_commerce(&boot(&broken)).unwrap();
        loaded.audit_sources(&Language, &sources).unwrap();
        open_gate_for_owner_unit_test(&mut loaded);
        let usage = loaded.current_usage();
        assert!(loaded.prepare_rating_roots(&abort).is_err());
        assert_eq!(loaded.current_usage(), usage);
        assert_eq!(loaded.commercial_roots().count(), 1);
        apply_rating_roots(&mut store, &mut disk, &abort);
        assert_eq!(store.commercial_roots().count(), 1);
        let retained = required(&ended).unwrap();
        assert_eq!(retained.len(), 1);
        let permanent = retained.first().unwrap();
        store.check_required_root(permanent).unwrap();
        assert!(matches!(permanent.anchor, RootAnchor::RatingJob(_)));
        assert!(RatingRoots::transition(&ended, &job, None, token(9)).is_err());
    }
}

#[test]
fn rating_root_write_family_allows_only_exact_pending_deletion() {
    use mush_core::authoritative::{AuthoritativeKey, ReplayWrite};
    use mush_core::mechanics::OwnerId;
    use mush_core::participant::OwnerParticipant;
    let adapter = super::participant::RootParticipant;
    let delete = |namespace: &str, key: String| ReplayWrite::ModuleDelete {
        namespace: OwnerId::parse_canonical(namespace).unwrap(),
        key: AuthoritativeKey::parse_canonical(&key).unwrap(),
    };
    let pending = RootAnchor::RatingPending(token(7));
    assert_eq!(RootAnchor::parse(&pending.record()).unwrap(), pending);
    assert!(adapter.accepts_write(&delete(NS, pending.key().key())));
    assert!(!adapter.accepts_write(&delete("tacsim", pending.key().key())));
    assert!(!adapter.accepts_write(&delete(NS, format!("{}/extra", pending.key().key()))));
    assert!(!adapter.accepts_write(&delete(NS, RootAnchor::RatingJob(token(7)).key().key())));
    let id = super::rating::RatingId::from_hex(&digest(2).to_string()).unwrap();
    assert!(!adapter.accepts_write(&delete(
        NS,
        RootAnchor::Rating { job: token(7), id }.key().key()
    )));
}

#[test]
fn ordinary_roll_record_capacity_cannot_consume_an_admitted_ratings_terminal_root() {
    use super::rating::{RatingJobState, test_support::complete_rating};
    use super::rating_roots::RatingRoots;
    let (mut store, sources, mut disk, revision) = saved();
    open_gate_for_owner_unit_test(&mut store);
    let job = rating_job(&revision, token(7));
    apply_rating_roots(
        &mut store,
        &mut disk,
        &RatingRoots::admission(&job, token(7)).unwrap(),
    );
    for n in disk.len()..MAX_RECORDS {
        let row = ReferenceRecord {
            format: FORMAT,
            key: ReferenceKey {
                owner: "tacsim".into(),
                kind: ReferenceKind::Selection,
                record: format!("fixture/{n}"),
            },
            revisions: vec![revision.reference()],
        };
        disk.insert(row.key.key(), serde_json::to_value(row).unwrap());
    }
    let mut loaded = RollStore::load_commerce(&boot(&disk)).unwrap();
    loaded.audit_sources(&Language, &sources).unwrap();
    open_gate_for_owner_unit_test(&mut loaded);
    assert!(
        loaded
            .prepare_reference(
                "tacsim",
                ReferenceKey {
                    owner: "tacsim".into(),
                    kind: ReferenceKind::Selection,
                    record: "one-too-many".into(),
                },
                vec![revision.reference()]
            )
            .is_err()
    );
    let running = job
        .transition(RatingJobState::Running { lease_sequence: 1 })
        .unwrap();
    for progress in [
        RatingRoots::transition(&job, &running, None, token(8)).unwrap(),
        RatingRoots::settled_progress(&running, token(9)).unwrap(),
    ] {
        let before = loaded.current_usage().unwrap();
        let prepared = loaded.prepare_rating_roots(&progress).unwrap();
        assert_eq!(prepared.operations().len(), 2);
        assert_eq!(prepared.usage().unwrap().before, before);
        assert_eq!(prepared.usage().unwrap().after, before);
        loaded.bind_commercial_roots(prepared).unwrap().apply();
        assert_eq!(loaded.current_usage().unwrap(), before);
    }
    let complete = complete_rating(
        job.subject().clone(),
        job.compatibility().clone(),
        digest(5),
        job.job(),
        ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at: 100,
        },
    )
    .unwrap();
    let finish =
        RatingRoots::transition(&running, &complete.job, Some(&complete.receipt), token(9))
            .unwrap();
    let prepared = loaded.prepare_rating_roots(&finish).unwrap();
    assert_eq!(prepared.usage().unwrap().after.records, MAX_RECORDS);
    loaded.bind_commercial_roots(prepared).unwrap().apply();
    assert_eq!(loaded.current_usage().unwrap().records, MAX_RECORDS);
    for root in super::rating_roots::required(&complete.job).unwrap() {
        loaded.check_required_root(&root).unwrap();
    }
}

struct FamiliesProbe {
    handles: Arc<Mutex<Option<CommerceFamilies>>>,
    ratings: Option<RatingFamilies>,
    shells: Option<mush_core::reserved_shell::ShellReservations>,
    scan_status_only: bool,
}
impl GameModule for FamiliesProbe {
    fn name(&self) -> &'static str {
        NS
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        *self.handles.lock().unwrap() = Some(CommerceFamilies::register(reg)?);
        self.ratings = Some(RatingFamilies::register(reg)?);
        use mush_core::mechanics::{DefinitionDescriptor, DefinitionId, DefinitionKind};
        let action = DefinitionDescriptor::new(
            DefinitionKind::new("action").unwrap(),
            DefinitionId::new(mush_core::reserved_shell::RESERVE_ACTION).unwrap(),
            1,
            json!({"format":1,"purpose":"roll.initial_filing","object":"inert"}),
        )
        .unwrap();
        let outcome = DefinitionDescriptor::new(
            DefinitionKind::new("outcome").unwrap(),
            DefinitionId::new(mush_core::reserved_shell::RESERVE_OUTCOME).unwrap(),
            1,
            json!({"format":1}),
        )
        .unwrap();
        for definition in [&action, &outcome] {
            let expected = vec![definition.clone()];
            reg.definition_validator(
                definition.kind().clone(),
                move |ctx: &mush_core::mechanics::DefinitionValidationContext<'_>| {
                    super::module::validate_definitions(NS, &expected, ctx)
                },
            )?;
        }
        let action = reg.definition(action)?;
        let outcome = reg.definition(outcome)?;
        self.shells = Some(mush_core::reserved_shell::ShellReservations::register(
            reg, action, outcome,
        )?);
        let mut store = RollStore::load_commerce(&ModuleBootData::default()).unwrap();
        store.audit_sources(&Language, &Sources::default()).unwrap();
        reg.state(store);
        Ok(())
    }
    fn finalize_owner_composition(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        let handles = self.handles.lock().unwrap();
        if self.scan_status_only {
            return super::audit::check_status_keys_for_test(ctx, handles.as_ref().unwrap())
                .map_err(|e| ModError::msg(e.to_string()));
        }
        super::audit::audit_and_open(
            ctx,
            handles.as_ref().unwrap(),
            self.ratings.as_ref().unwrap(),
            self.shells.as_ref().unwrap(),
            false,
            false,
            false,
            None,
        )
        .map_err(|e| ModError::msg(e.to_string()))
    }
}
fn engine() -> (Engine, CommerceFamilies) {
    engine_mode(false)
}
fn engine_mode(scan_status_only: bool) -> (Engine, CommerceFamilies) {
    let handles = Arc::new(Mutex::new(None));
    let engine = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(FamiliesProbe {
            handles: handles.clone(),
            ratings: None,
            shells: None,
            scan_status_only,
        })],
    )
    .unwrap();
    let families = handles.lock().unwrap().clone().unwrap();
    (engine, families)
}

#[test]
fn real_boot_audit_refuses_absent_marker_and_does_not_open_collection() {
    let (mut engine, _) = engine();
    let result =
        engine.all_modules_loaded(&mut Outbox::new(), &mut mush_core::engine::Effects::new());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("commercial record is missing")
    );
    assert!(
        engine
            .world
            .module_state::<RollStore>()
            .unwrap()
            .prepare_prune(1)
            .is_err()
    );
}

#[test]
fn real_typed_family_hydration_refuses_unknown_fields_missing_nullable_facts_and_wrong_keys() {
    let (mut first, f) = engine();
    let row = RevisionStatus {
        format: PUBLICATION_FORMAT,
        filed: FiledRevisionId::new(1).unwrap(),
        withdrawn_by: None,
        recalled_by: None,
    };
    let mut txn = first.action_transaction(&f.status).unwrap();
    txn.insert(&f.status, &row.key(), row).unwrap();
    let (ops, install) = txn.prepare().unwrap().into_parts();
    first.install_prepared_action(install).unwrap();
    let op = ops.first().unwrap();
    let ModuleWrite::JsonSet { key, value } = &op.op else {
        panic!("typed row");
    };
    for which in ["unknown", "omitted", "wrong_key", "bad_version"] {
        let mut changed = value.clone();
        let body = changed.get_mut("value").unwrap().as_object_mut().unwrap();
        match which {
            "unknown" => {
                body.insert("holder".into(), json!(99));
            }
            "omitted" => {
                body.remove("withdrawn_by");
            }
            "wrong_key" => {
                body.insert("filed".into(), json!("2"));
            }
            "bad_version" => {
                body.insert("format".into(), json!(2));
            }
            _ => unreachable!(),
        }
        let (mut restarted, _) = engine_mode(which == "wrong_key");
        // Wrong body/key linkage is detected by complete boot audit, while the
        // other malformed envelopes must fail before any typed row installs.
        let result = restarted.hydrate_action_state([mush_core::ActionStateBootRecord::new(
            NS,
            key.clone(),
            changed,
        )]);
        if which == "wrong_key" {
            assert!(result.is_ok());
            let refusal = restarted
                .all_modules_loaded(&mut Outbox::new(), &mut mush_core::engine::Effects::new())
                .unwrap_err();
            assert!(
                refusal.to_string().contains("commercial records disagree"),
                "{refusal}"
            );
        } else {
            assert!(result.is_err(), "{which}");
            assert!(restarted.action_state().is_empty());
        }
    }
}

#[path = "publication_tests/dependent.rs"]
mod dependent;
