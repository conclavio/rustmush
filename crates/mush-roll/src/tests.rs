#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{ModuleBootData, ModuleOp, ModuleWrite, ObjId, mechanics::DefinitionDigest};
use serde_json::{Value, json};

use crate::*;

const AUTHOR: ObjId = ObjId(10);
const OTHER: ObjId = ObjId(11);

fn draft(source: &str) -> WorkingCopy {
    WorkingCopy {
        source: source.into(),
        envelope: Envelope {
            schema: json!({"widgets": []}),
            presets: json!({}),
            profiles: json!({"line": {"min_odds": 30}}),
            rules: json!({"line": []}),
            declared_class: "I".into(),
            mind_abi: "test=1".into(),
        },
    }
}

/// Test-language adapter only. Real Rhai lexical normalization and bounded
/// compilation are tacsim adapter tests; roll never assumes a source grammar.
struct Validator;
impl RevisionAdapter for Validator {
    fn normalize_source(&self, source: &str) -> Result<String, RollError> {
        let mut chars = source.chars().peekable();
        let mut quoted = false;
        let mut escaped = false;
        let mut result = String::new();
        while let Some(ch) = chars.next() {
            if quoted {
                result.push(ch);
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    quoted = false;
                }
            } else if ch == '"' {
                quoted = true;
                result.push(ch);
            } else if ch == '\r' {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                result.push('\n');
            } else {
                result.push(ch);
            }
        }
        Ok(result)
    }
    fn validate(&self, source: &str, _: &Envelope) -> Result<(), RollError> {
        if source.contains("invalid") {
            return Err(invalid("compile:1:1", "invalid test source"));
        }
        Ok(())
    }
}

#[derive(Default)]
struct Sources {
    bytes: BTreeMap<DefinitionDigest, String>,
    keys: BTreeSet<DefinitionDigest>,
}
impl SourceAdapter for Sources {
    type Token = Option<(DefinitionDigest, String)>;
    fn prepare_source(
        &self,
        source: &str,
        abi: &str,
    ) -> Result<PreparedSource<Self::Token>, RollError> {
        let digest = source_digest(source);
        if self.keys.contains(&digest) {
            if self.bytes.get(&digest).map(String::as_str) != Some(source) {
                return Err(invalid("source", "occupied unreadable source"));
            }
            return Ok(PreparedSource {
                digest,
                operations: Vec::new(),
                token: None,
            });
        }
        Ok(PreparedSource {
            digest,
            operations: vec![ModuleOp {
                ns: "source_owner".into(),
                op: ModuleWrite::JsonSet {
                    key: digest.to_hex().into(),
                    value: json!({"sha256": digest, "bytes": source, "abi": abi}),
                },
            }],
            token: Some((digest, source.into())),
        })
    }
    fn read_source(&self, digest: DefinitionDigest) -> Result<&str, RollError> {
        self.bytes
            .get(&digest)
            .map(String::as_str)
            .ok_or_else(|| invalid("source", "missing or unreadable source"))
    }
}
impl Sources {
    fn apply(&mut self, token: <Self as SourceAdapter>::Token) {
        if let Some((digest, source)) = token {
            self.keys.insert(digest);
            self.bytes.insert(digest, source);
        }
    }
}

type Disk = BTreeMap<(String, String), Value>;

fn apply_batch(disk: &mut Disk, operations: impl IntoIterator<Item = ModuleOp>) {
    let mut next = disk.clone();
    for op in operations {
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                next.insert((op.ns.into(), key.into()), value);
            }
            ModuleWrite::Del { key } => {
                next.remove(&(op.ns.into(), key.into()));
            }
            ModuleWrite::StreamAppend { .. } => panic!("no streams"),
        }
    }
    *disk = next;
}

fn load(disk: &Disk) -> RollStore {
    RollStore::load(&ModuleBootData {
        kv: disk
            .iter()
            .filter(|((ns, _), _)| ns == NS)
            .map(|((_, key), value)| (key.as_str().into(), value.clone()))
            .collect(),
        ..Default::default()
    })
    .expect("bounded boot")
}

#[derive(Default)]
struct World {
    store: RollStore,
    sources: Sources,
    disk: Disk,
}
impl World {
    fn commit(&mut self, prepared: PreparedChange) {
        self.store
            .publish_with(prepared, |operations| {
                apply_batch(&mut self.disk, operations.iter().cloned())
            })
            .expect("publish");
    }
    fn create(&mut self, actor: ObjId, name: &str, source: &str) {
        self.commit(
            self.store
                .prepare_create(actor, name, draft(source))
                .expect("create"),
        );
    }
    fn save(
        &mut self,
        actor: ObjId,
        name: &str,
        notes: &str,
        version: Option<&str>,
    ) -> RevisionRef {
        let base = self.store.mind(actor, name).expect("owned mind").base();
        let prepared = self
            .store
            .prepare_save(actor, name, base, notes, version, &Validator, &self.sources)
            .expect("save prepares");
        let (change, source, reference) = prepared.into_parts();
        self.store
            .publish_with(change, |operations| {
                self.sources.apply(source.token);
                apply_batch(
                    &mut self.disk,
                    source
                        .operations
                        .into_iter()
                        .chain(operations.iter().cloned()),
                );
            })
            .expect("publish save");
        reference
    }
    fn edit(&mut self, actor: ObjId, name: &str, source: &str) {
        let base = self.store.mind(actor, name).expect("owned mind").base();
        self.commit(
            self.store
                .prepare_edit(actor, name, base, draft(source))
                .expect("edit"),
        );
    }
    fn reboot(&mut self) {
        self.store = load(&self.disk);
        let mut sources = Sources::default();
        for ((ns, key), value) in &self.disk {
            if ns != "source_owner" {
                continue;
            }
            let digest = DefinitionDigest::from_hex(key).expect("source key");
            sources.keys.insert(digest);
            if let Some(source) = value.get("bytes").and_then(Value::as_str)
                && source_digest(source) == digest
            {
                sources.bytes.insert(digest, source.into());
            }
        }
        self.sources = sources;
        self.store.audit_sources(&Validator, &self.sources).unwrap();
    }
}

#[test]
fn libraries_are_character_scoped_and_dimension_caps_precede_the_adapter() {
    let mut world = World::default();
    for index in 0..MAX_MINDS {
        world.create(AUTHOR, &format!("mind{index}"), "working");
    }
    assert!(
        matches!(world.store.prepare_create(AUTHOR, "extra", draft("working")), Err(RollError::Invalid { field, .. }) if field == "library")
    );
    assert_eq!(world.store.names(OTHER), Vec::<&str>::new());
    assert!(matches!(
        world.store.mind(OTHER, "mind0"),
        Err(RollError::Missing)
    ));
    world.create(OTHER, "mind0", "other character");
    assert_eq!(
        world.store.mind(OTHER, "mind0").unwrap().draft().source,
        "other character"
    );
    assert!(
        draft(&"x".repeat(MAX_SOURCE_BYTES))
            .validate_bounds()
            .is_ok()
    );
    assert!(
        matches!(draft(&"x".repeat(MAX_SOURCE_BYTES + 1)).validate_bounds(), Err(RollError::Invalid { field, .. }) if field == "source")
    );
}

#[test]
fn maximum_escaped_draft_and_full_history_survive_edit_save_and_hydration() {
    let mut world = World::default();
    world.create(AUTHOR, "capacity", "working");
    for index in 0..MAX_NAMED_VERSIONS {
        // Quotes and backslashes double the encoded size of a legal name.
        let name = format!("{index:04}{}", "\\\"".repeat(30));
        assert_eq!(name.len(), 64);
        world.save(AUTHOR, "capacity", "", Some(&name));
    }
    for _ in 0..KEEP_UNNAMED {
        world.save(AUTHOR, "capacity", "", None);
    }
    let mut copy = draft(&"\u{0001}".repeat(MAX_SOURCE_BYTES));
    copy.envelope.schema = Value::Object(
        (0..8)
            .map(|i| (format!("field{i}"), Value::String(String::new())))
            .collect(),
    );
    let mut remaining = MAX_ENVELOPE_BYTES - serde_json::to_vec(&copy.envelope).unwrap().len();
    for value in copy.envelope.schema.as_object_mut().unwrap().values_mut() {
        let len = remaining.min(8_192);
        *value = Value::String("a".repeat(len));
        remaining -= len;
    }
    assert_eq!(remaining, 0);
    assert_eq!(
        serde_json::to_vec(&copy.envelope).unwrap().len(),
        MAX_ENVELOPE_BYTES
    );
    copy.validate_bounds().expect("maximum owner-legal draft");
    let before = world.disk.clone();
    let base = world.store.mind(AUTHOR, "capacity").unwrap().base();
    let prepared = world
        .store
        .prepare_edit(AUTHOR, "capacity", base, copy.clone())
        .expect("all legal draft bytes fit the bounded owner record");
    assert_eq!(world.disk, before, "preparation does not publish");
    world.commit(prepared);
    let record = world.store.mind(AUTHOR, "capacity").unwrap();
    assert_eq!(record.named.len(), MAX_NAMED_VERSIONS);
    assert_eq!(record.unnamed.len(), KEEP_UNNAMED);
    let bytes = serde_json::to_vec(record).unwrap();
    assert!(
        bytes.len() > 256 * 1024,
        "escaped source, full envelope and history are all present"
    );
    assert!(bytes.len() <= MAX_DOCUMENT_BYTES);
    world.reboot();
    assert_eq!(world.store.mind(AUTHOR, "capacity").unwrap().draft(), &copy);
    let reference = world.save(AUTHOR, "capacity", &"n".repeat(MAX_NOTES_BYTES), None);
    assert_eq!(
        reference.digest,
        copy.envelope.digest(&copy.source).unwrap(),
        "the executable hash uses the same complete bounded bytes"
    );
    world.reboot();
    let loaded = world.store.mind(AUTHOR, "capacity").unwrap();
    assert_eq!(loaded.draft(), &copy);
    assert_eq!(loaded.named.len(), MAX_NAMED_VERSIONS);
    assert_eq!(loaded.unnamed.len(), KEEP_UNNAMED);
    assert_eq!(loaded.latest(), Some(reference.key));
}

#[test]
fn invalid_working_text_is_editable_but_stale_edits_and_invalid_saves_change_nothing() {
    let mut world = World::default();
    world.create(AUTHOR, "scout", "valid");
    let base = world.store.mind(AUTHOR, "scout").unwrap().base();
    world.edit(AUTHOR, "scout", "invalid working text");
    assert!(matches!(
        world
            .store
            .prepare_edit(AUTHOR, "scout", base, draft("valid")),
        Err(RollError::StaleEdit)
    ));
    let after = world.disk.clone();
    let base = world.store.mind(AUTHOR, "scout").unwrap().base();
    assert!(
        world
            .store
            .prepare_save(AUTHOR, "scout", base, "", None, &Validator, &world.sources)
            .is_err()
    );
    assert_eq!(world.disk, after);
    assert!(
        world
            .store
            .mind(AUTHOR, "scout")
            .unwrap()
            .latest()
            .is_none()
    );
    world.reboot();
    assert_eq!(
        world.store.mind(AUTHOR, "scout").unwrap().draft().source,
        "invalid working text"
    );
}

#[test]
fn lexical_normalization_preserves_literal_bytes_and_notes_do_not_change_identity() {
    let mut world = World::default();
    let text = "let text = \"first\r\nsecond\";\r\nthink();\r\n";
    let normalized = "let text = \"first\r\nsecond\";\nthink();\n";
    world.create(AUTHOR, "scout", text);
    let one = world.save(AUTHOR, "scout", "note one", None);
    assert_eq!(
        world.store.mind(AUTHOR, "scout").unwrap().draft().source,
        normalized
    );
    let two = world.save(AUTHOR, "scout", "different note", Some("Version A"));
    assert_ne!(one.key, two.key);
    assert_eq!(one.digest, two.digest);
    assert_eq!(
        world
            .store
            .resolve_digest(AUTHOR, "scout", &one.digest.to_hex())
            .unwrap(),
        two
    );
    assert_eq!(
        world
            .store
            .resolve_digest(AUTHOR, "scout", &one.digest.to_hex()[..8])
            .unwrap(),
        two
    );
    let revision = world.store.revision(AUTHOR, one.key).unwrap();
    assert_eq!(
        world.sources.read_source(revision.source()).unwrap(),
        normalized
    );
    assert_eq!(revision.notes(), "note one");
    assert_eq!(normalized.matches('\n').count(), text.matches('\n').count());
    let doc = &world.disk[&(NS.into(), one.key.key())];
    assert!(doc.get("bytes").is_none() && doc.get("source").unwrap().is_string());
    assert!(doc["envelope"]["profiles"].is_object());
    world.reboot();
    assert!(world.store.quarantine().is_empty());
    assert_eq!(
        world.store.revision(AUTHOR, one.key).unwrap().notes(),
        "note one"
    );
}

#[test]
fn every_executable_envelope_field_changes_the_revision_digest() {
    let original = draft("source").envelope;
    let baseline = original.digest("source").unwrap();
    let mut cases = Vec::new();
    let mut next = original.clone();
    next.schema = json!({"widgets": ["toggle"]});
    cases.push(next);
    let mut next = original.clone();
    next.presets = json!({"careful": {}});
    cases.push(next);
    let mut next = original.clone();
    next.profiles = json!({"line": {"min_odds": 60}});
    cases.push(next);
    let mut next = original.clone();
    next.rules = json!({"line": ["fire when ready"]});
    cases.push(next);
    let mut next = original.clone();
    next.declared_class = "II".into();
    cases.push(next);
    let mut next = original.clone();
    next.mind_abi = "test=2".into();
    cases.push(next);
    for envelope in cases {
        assert_ne!(envelope.digest("source").unwrap(), baseline);
    }
    assert_ne!(original.digest("different source").unwrap(), baseline);
}

#[test]
fn prepared_operations_are_inert_and_stale_publication_never_applies_the_other_owner() {
    let mut world = World::default();
    world.create(AUTHOR, "scout", "one");
    let base = world.store.mind(AUTHOR, "scout").unwrap().base();
    let prepared = world
        .store
        .prepare_save(AUTHOR, "scout", base, "", None, &Validator, &world.sources)
        .unwrap();
    let rebooted = load(&world.disk);
    assert!(rebooted.mind(AUTHOR, "scout").unwrap().latest().is_none());
    assert!(world.sources.bytes.is_empty());
    world.edit(AUTHOR, "scout", "two");
    let mut applied = false;
    assert_eq!(
        world
            .store
            .publish_with(prepared.into_parts().0, |_| applied = true),
        Err(RollError::StaleOperation)
    );
    assert!(!applied);
    assert!(world.sources.bytes.is_empty());
}

#[test]
fn source_only_crash_is_an_orphan_and_revision_without_source_is_quarantined() {
    let mut world = World::default();
    world.create(AUTHOR, "scout", "source");
    let base = world.store.mind(AUTHOR, "scout").unwrap().base();
    let prepared = world
        .store
        .prepare_save(AUTHOR, "scout", base, "", None, &Validator, &world.sources)
        .unwrap();
    let mut source_only = world.disk.clone();
    apply_batch(&mut source_only, prepared.source().operations.clone());
    assert!(
        load(&source_only)
            .mind(AUTHOR, "scout")
            .unwrap()
            .latest()
            .is_none()
    );
    let mut revision_only = world.disk.clone();
    apply_batch(
        &mut revision_only,
        prepared.change().operations().iter().cloned(),
    );
    let mut broken = load(&revision_only);
    broken
        .audit_sources(&Validator, &Sources::default())
        .unwrap();
    assert!(!broken.quarantine().is_empty());
    assert!(matches!(
        broken.prepare_prune(1),
        Err(RollError::Quarantined(_))
    ));
    let saved = world.save(AUTHOR, "scout", "", None);
    world.reboot();
    assert!(world.store.quarantine().is_empty());
    assert_eq!(
        world.store.revision(AUTHOR, saved.key).unwrap().reference(),
        saved
    );
}

#[test]
fn follow_resolves_once_and_pins_keep_the_exact_revision_after_edits() {
    let mut world = World::default();
    world.create(AUTHOR, "scout", "one");
    let first = world.save(AUTHOR, "scout", "", None);
    world.commit(
        world
            .store
            .prepare_select(
                AUTHOR,
                Selection::Follow {
                    mind: first.key.mind,
                },
            )
            .unwrap(),
    );
    let sealed = world
        .store
        .resolve_selection(AUTHOR, &world.sources)
        .unwrap();
    world.edit(AUTHOR, "scout", "two");
    let second = world.save(AUTHOR, "scout", "", None);
    assert_eq!(sealed.reference(), first);
    assert_eq!(
        world
            .store
            .resolve_selection(AUTHOR, &world.sources)
            .unwrap()
            .reference(),
        second
    );
    world.commit(
        world
            .store
            .prepare_select(AUTHOR, Selection::Pin { revision: first })
            .unwrap(),
    );
    world.reboot();
    assert_eq!(
        world
            .store
            .resolve_selection(AUTHOR, &world.sources)
            .unwrap()
            .reference(),
        first
    );
    assert!(matches!(
        world.store.revision(OTHER, first.key),
        Err(RollError::Missing)
    ));
}

#[test]
fn pruning_keeps_named_last_fifty_and_each_explicit_owner_reference_after_reboot() {
    let mut world = World::default();
    world.create(AUTHOR, "scout", "revision 1");
    let named = world.save(AUTHOR, "scout", "named", Some(&"n".repeat(64)));
    let mut revisions = vec![named];
    for number in 2..=57 {
        world.edit(AUTHOR, "scout", &format!("revision {number}"));
        revisions.push(world.save(AUTHOR, "scout", "", None));
    }
    for (index, kind) in [
        ReferenceKind::MatchRecord,
        ReferenceKind::Tape,
        ReferenceKind::ProvingRun,
        ReferenceKind::OpenSeal,
        ReferenceKind::Listing,
    ]
    .into_iter()
    .enumerate()
    {
        world.commit(
            world
                .store
                .prepare_reference(
                    "game",
                    ReferenceKey {
                        owner: "game".into(),
                        kind,
                        record: format!("kept-{index}"),
                    },
                    vec![revisions[index + 1]],
                )
                .unwrap(),
        );
    }
    world.reboot();
    assert!(world.store.quarantine().is_empty());
    let prune = world.store.prepare_prune(MAX_GC_BATCH).unwrap();
    assert_eq!(prune.removed, vec![revisions[6]]); // r7; r2..r6 have external owners.
    world.commit(prune.change);
    world.reboot();
    assert!(world.store.revision(AUTHOR, revisions[6].key).is_err());
    for revision in revisions.iter().take(6) {
        assert!(world.store.revision(AUTHOR, revision.key).is_ok());
    }
    let base = world.store.mind(AUTHOR, "scout").unwrap().base();
    world.commit(world.store.prepare_remove(AUTHOR, "scout", base).unwrap());
    let prune = world.store.prepare_prune(MAX_GC_BATCH).unwrap();
    assert!(
        prune.removed.is_empty(),
        "removing the label preserves the last fifty roots"
    );
    world.commit(prune.change);
    world.reboot();
    assert!(world.store.names(AUTHOR).is_empty());
    for revision in revisions.iter().take(6) {
        assert!(world.store.revision(AUTHOR, revision.key).is_ok());
    }
    for revision in revisions.iter().skip(7) {
        assert!(world.store.revision(AUTHOR, revision.key).is_ok());
    }
    world.create(AUTHOR, "scout", "new identity");
    assert_ne!(
        world.store.mind(AUTHOR, "scout").unwrap().id(),
        named.key.mind
    );
}

#[test]
fn explicit_prune_is_character_scoped_bounded_and_rechecks_external_roots() {
    let mut world = World::default();
    world.create(AUTHOR, "mine", "owned 1");
    let mut mine = vec![world.save(AUTHOR, "mine", "", None)];
    for number in 2..=60 {
        world.edit(AUTHOR, "mine", &format!("owned {number}"));
        mine.push(world.save(AUTHOR, "mine", "", None));
    }
    // The other character independently owns the same source as our r2.
    // Even an unrooted foreign revision prevents its source being a candidate.
    world.create(OTHER, "theirs", "owned 2");
    let mut theirs = vec![world.save(OTHER, "theirs", "", None)];
    for number in 2..=60 {
        world.edit(OTHER, "theirs", &format!("other {number}"));
        theirs.push(world.save(OTHER, "theirs", "", None));
    }
    let key = ReferenceKey {
        owner: "game".into(),
        kind: ReferenceKind::OpenSeal,
        record: "active".into(),
    };
    world.commit(
        world
            .store
            .prepare_reference("game", key.clone(), vec![mine[0]])
            .unwrap(),
    );
    assert!(world.store.prepare_prune_owned(AUTHOR, 0).is_err());
    assert!(
        world
            .store
            .prepare_prune_owned(AUTHOR, MAX_GC_BATCH + 1)
            .is_err()
    );
    let stale = world.store.prepare_prune_owned(AUTHOR, 3).unwrap();
    assert_eq!(stale.removed, mine[1..4]);
    assert!(!stale.source_candidates.contains(&source_digest("owned 2")));
    world.commit(
        world
            .store
            .prepare_reference("game", key, vec![mine[0], mine[2]])
            .unwrap(),
    );
    let before = world.disk.clone();
    assert!(matches!(
        world
            .store
            .publish_with(stale.change, |_| panic!("stale prune writes")),
        Err(RollError::StaleOperation)
    ));
    assert_eq!(world.disk, before);
    let sources_before = world.sources.bytes.clone();
    let prepared = world.store.prepare_prune_owned(AUTHOR, 3).unwrap();
    assert_eq!(prepared.removed, vec![mine[1], mine[3], mine[4]]);
    assert_eq!(prepared.change.operations().len(), 3);
    world.commit(prepared.change);
    world.reboot();
    assert!(world.store.quarantine().is_empty());
    assert_eq!(
        world.sources.bytes, sources_before,
        "revision pruning never deletes source blobs"
    );
    for revision in theirs {
        assert_eq!(
            world
                .store
                .revision(OTHER, revision.key)
                .unwrap()
                .reference(),
            revision
        );
    }
    assert!(world.store.revision(AUTHOR, mine[1].key).is_err());
    assert!(world.store.revision(AUTHOR, mine[2].key).is_ok());
    for revision in mine.iter().skip(10) {
        assert!(
            world.store.revision(AUTHOR, revision.key).is_ok(),
            "last fifty remain rooted"
        );
    }
    // A foreign unreadable reference-bearing document must still block this
    // author's collection: it may name one of this author's revisions.
    world.disk.insert(
        (NS.into(), format!("library:{}", OTHER.0)),
        json!({"bad": true}),
    );
    world.reboot();
    assert!(matches!(
        world.store.prepare_prune_owned(AUTHOR, 1),
        Err(RollError::Quarantined(_))
    ));
}

#[test]
fn unreadable_reference_or_unknown_record_blocks_collection_without_deleting_it() {
    let mut world = World::default();
    world.create(AUTHOR, "scout", "source");
    let revision = world.save(AUTHOR, "scout", "", None);
    let key = ReferenceKey {
        owner: "game".into(),
        kind: ReferenceKind::OpenSeal,
        record: "one".into(),
    };
    world.commit(
        world
            .store
            .prepare_reference("game", key.clone(), vec![revision])
            .unwrap(),
    );
    world.disk.get_mut(&(NS.into(), key.key())).unwrap()["unrecognized"] = json!(true);
    world.disk.insert(
        (NS.into(), "future:owner".into()),
        json!({"revision": revision}),
    );
    let before = world.disk.clone();
    world.reboot();
    assert_eq!(world.store.quarantine().len(), 2);
    assert!(matches!(
        world.store.prepare_prune(1),
        Err(RollError::Quarantined(_))
    ));
    assert_eq!(world.disk, before);
    assert!(matches!(
        world.store.prepare_reference("game", key, vec![revision]),
        Err(RollError::Quarantined(_))
    ));
}

#[test]
fn corrupt_existing_source_is_never_repaired_by_a_save_or_reboot() {
    let mut world = World::default();
    world.create(AUTHOR, "scout", "source");
    let revision = world.save(AUTHOR, "scout", "", None);
    let hash = world.store.revision(AUTHOR, revision.key).unwrap().source();
    world
        .disk
        .get_mut(&("source_owner".into(), hash.to_hex()))
        .unwrap()["bytes"] = json!("tampered");
    let before = world.disk.clone();
    world.reboot();
    assert!(world.store.quarantine().contains_key(&revision.key.key()));
    assert!(world.sources.prepare_source("source", "test=1").is_err());
    assert_eq!(world.disk, before);
    assert!(matches!(
        world.store.prepare_prune(1),
        Err(RollError::Quarantined(_))
    ));
}

#[test]
fn strict_records_and_metadata_fail_with_the_field_named() {
    let mut bad = draft("source");
    bad.envelope.rules = json!({"missing": []});
    assert!(
        matches!(bad.validate_bounds(), Err(RollError::Invalid { field, .. }) if field == "rules")
    );
    bad = draft("source");
    bad.envelope.rules = json!({"line": [0, 0, 0, 0, 0, 0, 0]});
    assert!(bad.validate_bounds().is_err());
    bad = draft("source");
    bad.envelope.profiles = json!({"line": {"float": 1.2}});
    assert!(bad.validate_bounds().is_err());
    let mut world = World::default();
    world.create(AUTHOR, "scout", "source");
    world
        .disk
        .get_mut(&(NS.into(), "mind:10:1".into()))
        .unwrap()["edit"] = json!(-1);
    world.reboot();
    assert!(world.store.quarantine().contains_key("mind:10:1"));
    assert!(world.store.mind(AUTHOR, "scout").is_err());
}

#[test]
fn owner_mismatch_cannot_release_another_owners_retention_record() {
    let mut world = World::default();
    world.create(AUTHOR, "scout", "source");
    let revision = world.save(AUTHOR, "scout", "", None);
    let key = ReferenceKey {
        owner: "game".into(),
        kind: ReferenceKind::OpenSeal,
        record: "one".into(),
    };
    assert!(
        world
            .store
            .prepare_reference("other", key.clone(), vec![revision])
            .is_err()
    );
    world.commit(
        world
            .store
            .prepare_reference("game", key.clone(), vec![revision])
            .unwrap(),
    );
    assert!(
        world
            .store
            .prepare_release_reference("other", &key)
            .is_err()
    );
    world.commit(world.store.prepare_release_reference("game", &key).unwrap());
}

#[test]
fn a_valid_head_cannot_bypass_quarantine_of_its_retained_history() {
    for corrupt in [false, true] {
        let mut world = World::default();
        world.create(AUTHOR, "scout", "older source");
        let older = world.save(AUTHOR, "scout", "", None);
        world.edit(AUTHOR, "scout", "current source");
        let head = world.save(AUTHOR, "scout", "", None);
        world.commit(
            world
                .store
                .prepare_select(
                    AUTHOR,
                    Selection::Follow {
                        mind: head.key.mind,
                    },
                )
                .unwrap(),
        );
        let key = (NS.into(), older.key.key());
        if corrupt {
            world.disk.insert(key, json!({"unreadable": true}));
        } else {
            world.disk.remove(&key);
        }
        world.reboot();

        // The newest immutable revision and the library record themselves are
        // readable. The active mind is quarantined because its older retained
        // revision is absent/unreadable; neither name nor numeric-id access
        // may resolve that active library selection for sealing.
        assert_eq!(
            world.store.revision(AUTHOR, head.key).unwrap().reference(),
            head
        );
        assert!(!world.store.quarantine().contains_key("library:10"));
        assert!(world.store.quarantine().contains_key("mind:10:1"));
        assert!(matches!(
            world.store.mind(AUTHOR, "scout"),
            Err(RollError::Missing)
        ));
        assert!(matches!(
            world.store.owned_mind(AUTHOR, head.key.mind),
            Err(RollError::Missing)
        ));
        assert!(matches!(
            world.store.prepare_select(
                AUTHOR,
                Selection::Follow {
                    mind: head.key.mind
                }
            ),
            Err(RollError::Missing)
        ));
        assert!(matches!(
            world
                .store
                .prepare_select(AUTHOR, Selection::Pin { revision: head }),
            Err(RollError::Missing)
        ));
        assert!(matches!(
            world.store.resolve_selection(AUTHOR, &world.sources),
            Err(RollError::Missing)
        ));
        assert!(matches!(
            world.store.prepare_prune(1),
            Err(RollError::Quarantined(_))
        ));
    }
}

#[test]
fn external_collection_quarantine_invalidates_prune_and_preserves_known_roots() {
    let mut world = World::default();
    world.create(AUTHOR, "scout", "source 0");
    for number in 0..=KEEP_UNNAMED {
        world.edit(AUTHOR, "scout", &format!("source {number}"));
        world.save(AUTHOR, "scout", "", None);
    }
    let prune = world.store.prepare_prune(1).unwrap();
    let owned_prune = world.store.prepare_prune_owned(AUTHOR, 1).unwrap();
    assert_eq!(prune.removed.len(), 1);
    assert_eq!(prune.removed, owned_prune.removed);
    let oldest = prune.removed[0];
    let disk = world.disk.clone();
    let sources = world.sources.bytes.clone();
    world
        .store
        .quarantine_collection("tacsim", "retained-tape-unreadable")
        .unwrap();
    assert_eq!(
        world.disk, disk,
        "the derived guard has no persistence record"
    );
    for change in [prune.change, owned_prune.change] {
        assert_eq!(
            world.store.check_prepared(&change),
            Err(RollError::StaleOperation)
        );
        let mut published = false;
        assert_eq!(
            world.store.publish_with(change, |_| published = true),
            Err(RollError::StaleOperation)
        );
        assert!(
            !published,
            "neither deletion nor another owner's callback runs"
        );
    }
    for result in [
        world.store.prepare_prune(1),
        world.store.prepare_prune_owned(AUTHOR, 1),
        world.store.prepare_prune_owned(OTHER, 1),
    ] {
        assert!(matches!(
            result,
            Err(RollError::Quarantined(reason)) if reason == "external retention recovery"
        ));
    }
    assert_eq!(
        world
            .store
            .revision(AUTHOR, oldest.key)
            .unwrap()
            .reference(),
        oldest
    );
    assert_eq!(world.sources.bytes, sources);

    // Recovery can still preserve a trustworthy old root while an unrelated
    // unreadable retained record keeps collection globally disabled.
    let key = ReferenceKey {
        owner: "tacsim".into(),
        kind: ReferenceKind::Tape,
        record: "known-kept-tape".into(),
    };
    world.commit(
        world
            .store
            .prepare_reference("tacsim", key.clone(), vec![oldest])
            .unwrap(),
    );
    world.edit(AUTHOR, "scout", "authoring remains available");
    assert!(world.store.prepare_prune(1).is_err());
    assert_eq!(
        world.store.reference_keys("tacsim", ReferenceKind::Tape),
        vec![key.clone()]
    );

    world.reboot();
    assert!(
        world.store.quarantine().is_empty(),
        "recovery must derive boot-local guards again"
    );
    assert_eq!(
        world.store.reference_keys("tacsim", ReferenceKind::Tape),
        vec![key]
    );
    assert!(world.store.prepare_prune(1).unwrap().removed.is_empty());
    world
        .store
        .quarantine_collection("tacsim", "retained-tape-unreadable")
        .unwrap();
    assert!(world.store.prepare_prune(1).is_err());
}

#[test]
fn external_collection_guards_are_exact_bounded_additive_and_idempotent() {
    let mut store = RollStore::default();
    for owner in [
        "",
        "Tacsim",
        " tacsim",
        "tacsim:other",
        "roll",
        &"x".repeat(257),
    ] {
        assert!(
            store
                .quarantine_collection(owner, "retained-tape-unreadable")
                .is_err()
        );
    }
    for reason in [
        "",
        "PrivateSource",
        "record/1",
        "line\nbreak",
        &"x".repeat(65),
    ] {
        assert!(store.quarantine_collection("tacsim", reason).is_err());
    }
    assert!(store.quarantine().is_empty());
    assert!(store.prepare_prune(1).is_ok());

    store
        .quarantine_collection("tacsim", "retained-tape-unreadable")
        .unwrap();
    let create = store
        .prepare_create(AUTHOR, "scout", draft("valid"))
        .unwrap();
    store
        .quarantine_collection("tacsim", "later-partial-recovery")
        .unwrap();
    store.publish_with(create, |_| {}).unwrap();
    assert_eq!(
        store
            .quarantine()
            .get("collection:tacsim")
            .map(String::as_str),
        Some("retained-tape-unreadable")
    );
    for owner in 1..64 {
        store
            .quarantine_collection(&format!("owner{owner}"), "retained-data-unreadable")
            .unwrap();
    }
    assert_eq!(store.quarantine().len(), 64);
    assert_eq!(
        store.quarantine_collection("onemore", "retained-data-unreadable"),
        Err(RollError::Exhausted("collection guard"))
    );
    assert!(store.quarantine_collection("tacsim", "").is_err());
    assert_eq!(
        store.quarantine().len(),
        64,
        "no owner operation clears or overwrites another guard"
    );
    assert!(
        matches!(store.prepare_prune(1), Err(RollError::Quarantined(reason)) if reason == "external retention recovery")
    );
}

#[test]
fn source_audit_invalidates_a_prepared_prune_before_it_can_delete_anything() {
    let mut world = World::default();
    world.create(AUTHOR, "scout", "source 0");
    for number in 0..=KEEP_UNNAMED {
        world.edit(AUTHOR, "scout", &format!("source {number}"));
        world.save(AUTHOR, "scout", "", None);
    }
    let prune = world.store.prepare_prune(1).unwrap();
    assert_eq!(prune.removed.len(), 1);
    let oldest = prune.removed[0];
    let source = world.store.revision(AUTHOR, oldest.key).unwrap().source();
    world.sources.bytes.remove(&source);
    world
        .store
        .audit_sources(&Validator, &world.sources)
        .unwrap();
    let mut applied = false;
    assert_eq!(
        world.store.publish_with(prune.change, |_| applied = true),
        Err(RollError::StaleOperation)
    );
    assert!(!applied);
    assert!(world.disk.contains_key(&(NS.into(), oldest.key.key())));
    assert!(matches!(
        world.store.prepare_prune(1),
        Err(RollError::Quarantined(_))
    ));
}

#[test]
fn distinct_digest_prefixes_are_ambiguous_but_complete_digests_resolve() {
    let mut world = World::default();
    world.create(AUTHOR, "scout", "one");
    let first = world.save(AUTHOR, "scout", "", None);
    world.edit(AUTHOR, "scout", "two");
    let second = world.save(AUTHOR, "scout", "", None);
    // Synthetic hash-prefix collision tests lookup itself. Source audits are
    // deliberately separate and would reject these forged executable hashes.
    let one = format!("01234567{}", "0".repeat(56));
    let two = format!("01234567{}", "1".repeat(56));
    world.disk.get_mut(&(NS.into(), first.key.key())).unwrap()["reference"]["digest"] = json!(one);
    world.disk.get_mut(&(NS.into(), second.key.key())).unwrap()["reference"]["digest"] = json!(two);
    let store = load(&world.disk);
    assert_eq!(
        store.resolve_digest(AUTHOR, "scout", "01234567"),
        Err(RollError::Ambiguous)
    );
    assert_eq!(
        store.resolve_digest(AUTHOR, "scout", &two).unwrap().key,
        second.key
    );
}

#[test]
fn renamed_labels_do_not_change_revisions_and_deleted_names_cannot_reuse_ids() {
    let mut world = World::default();
    world.create(AUTHOR, "scout", "source");
    let revision = world.save(AUTHOR, "scout", "notes", Some("first"));
    let base = world.store.mind(AUTHOR, "scout").unwrap().base();
    world.commit(
        world
            .store
            .prepare_rename(AUTHOR, "scout", base, "renamed")
            .unwrap(),
    );
    let base = world.store.mind(AUTHOR, "renamed").unwrap().base();
    world.commit(
        world
            .store
            .prepare_rename_version(AUTHOR, "renamed", base, "first", "stable")
            .unwrap(),
    );
    world.reboot();
    assert_eq!(
        world
            .store
            .revision(AUTHOR, revision.key)
            .unwrap()
            .reference(),
        revision
    );
    let base = world.store.mind(AUTHOR, "renamed").unwrap().base();
    world.commit(
        world
            .store
            .prepare_forget_version(AUTHOR, "renamed", base, "stable")
            .unwrap(),
    );
    let base = world.store.mind(AUTHOR, "renamed").unwrap().base();
    world.commit(world.store.prepare_remove(AUTHOR, "renamed", base).unwrap());
    world
        .disk
        .get_mut(&(NS.into(), "library:10".into()))
        .unwrap()["next_mind"] = json!(1);
    world.reboot();
    assert!(world.store.quarantine().contains_key("library:10"));
    assert!(matches!(
        world.store.prepare_create(AUTHOR, "new", draft("source")),
        Err(RollError::Quarantined(_))
    ));
}

#[test]
fn selection_reference_keys_are_owner_scoped_and_survive_until_explicit_release() {
    let mut world = World::default();
    world.create(AUTHOR, "scout", "source");
    let revision = world.save(AUTHOR, "scout", "", None);
    let own = ReferenceKey {
        owner: "game".into(),
        kind: ReferenceKind::Selection,
        record: "slot-1".into(),
    };
    let other = ReferenceKey {
        owner: "other".into(),
        kind: ReferenceKind::Selection,
        record: "slot-1".into(),
    };
    let seal = ReferenceKey {
        owner: "game".into(),
        kind: ReferenceKind::OpenSeal,
        record: "sortie-1".into(),
    };
    for key in [&own, &other, &seal] {
        world.commit(
            world
                .store
                .prepare_reference(&key.owner, key.clone(), vec![revision])
                .unwrap(),
        );
    }
    world.reboot();
    assert_eq!(ReferenceKind::Selection.as_str(), "selection");
    assert_eq!(
        world.store.reference_keys("game", ReferenceKind::Selection),
        vec![own.clone()]
    );
    assert!(
        world
            .store
            .reference_keys("missing", ReferenceKind::Selection)
            .is_empty()
    );
    assert!(
        world
            .store
            .prepare_release_reference("other", &own)
            .is_err()
    );
    world.commit(world.store.prepare_release_reference("game", &own).unwrap());
    world.reboot();
    assert!(
        world
            .store
            .reference_keys("game", ReferenceKind::Selection)
            .is_empty()
    );
    assert_eq!(
        world
            .store
            .reference_keys("other", ReferenceKind::Selection),
        vec![other]
    );
    assert_eq!(
        world.store.reference_keys("game", ReferenceKind::OpenSeal),
        vec![seal]
    );
}

#[test]
fn retention_batch_is_atomic_bounded_and_rejects_cross_arm_duplicate_keys() {
    let mut world = World::default();
    world.create(AUTHOR, "scout", "source");
    let revision = world.save(AUTHOR, "scout", "", None);
    let selection = ReferenceKey {
        owner: "game".into(),
        kind: ReferenceKind::Selection,
        record: "slot-1".into(),
    };
    let seal = ReferenceKey {
        owner: "game".into(),
        kind: ReferenceKind::OpenSeal,
        record: "sortie-1".into(),
    };
    let record = ReferenceKey {
        owner: "game".into(),
        kind: ReferenceKind::MatchRecord,
        record: "match-1".into(),
    };
    world.commit(
        world
            .store
            .prepare_reference("game", selection.clone(), vec![revision])
            .unwrap(),
    );
    assert!(
        world
            .store
            .prepare_reference_batch(
                "game",
                vec![(selection.clone(), vec![revision])],
                vec![selection.clone()]
            )
            .is_err()
    );
    assert!(
        world
            .store
            .prepare_reference_batch(
                "game",
                vec![(seal.clone(), vec![revision])],
                vec![seal.clone()]
            )
            .is_err()
    );
    assert!(
        world
            .store
            .reference_keys("game", ReferenceKind::OpenSeal)
            .is_empty()
    );
    assert_eq!(
        world.store.reference_keys("game", ReferenceKind::Selection),
        vec![selection.clone()]
    );
    let excessive = (0..=MAX_REFERENCE_CHANGES)
        .map(|index| {
            (
                ReferenceKey {
                    owner: "game".into(),
                    kind: ReferenceKind::OpenSeal,
                    record: format!("large-{index}"),
                },
                vec![revision],
            )
        })
        .collect();
    assert!(
        world
            .store
            .prepare_reference_batch("game", excessive, vec![])
            .is_err()
    );
    assert!(
        world
            .store
            .prepare_reference_batch("game", vec![], vec![])
            .is_err()
    );
    world.commit(
        world
            .store
            .prepare_reference_batch(
                "game",
                vec![
                    (seal.clone(), vec![revision]),
                    (record.clone(), vec![revision]),
                ],
                vec![selection],
            )
            .unwrap(),
    );
    world.reboot();
    assert!(
        world
            .store
            .reference_keys("game", ReferenceKind::Selection)
            .is_empty()
    );
    assert_eq!(
        world.store.reference_keys("game", ReferenceKind::OpenSeal),
        vec![seal]
    );
    assert_eq!(
        world
            .store
            .reference_keys("game", ReferenceKind::MatchRecord),
        vec![record]
    );
}
