//! Structural persisted-lineage fixtures, never a commercial grant or fork.
//! These owner-only fixtures do not claim a current paid grant or fork permission.
use super::*;
use crate::commerce::{CharterId, FiledRevisionId, LineId, RevisionIdentity, filed::ParentContent};
use crate::model::{DERIVATIVE_FORMAT, DerivativeProvenance};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn ancestry(
    world: &World,
    parent: RevisionRef,
) -> Result<DerivativeProvenance, Box<dyn std::error::Error>> {
    Ok(DerivativeProvenance {
        parent: ParentContent {
            filed: FiledRevisionId::new(parent.key.mind)?,
            revision: RevisionIdentity::capture(
                world.store.revision(parent.key.character, parent.key)?,
            )?,
        },
        charter: CharterId::from_object(ObjId(90))?,
        line: LineId::new(parent.key.mind)?,
    })
}

fn mark(world: &mut World, name: &str, provenance: &DerivativeProvenance) -> TestResult {
    let mind = world.store.mind(AUTHOR, name)?;
    let id = mind.id();
    let key = mind.key();
    for ((ns, candidate), row) in &mut world.disk {
        if ns == NS
            && (candidate == &key || candidate.starts_with(&format!("revision:{}:{id}:", AUTHOR.0)))
        {
            let object = row.as_object_mut().ok_or("record object")?;
            object.insert("format".into(), json!(DERIVATIVE_FORMAT));
            object.insert("provenance".into(), serde_json::to_value(provenance)?);
        }
    }
    world.reboot();
    Ok(())
}

fn fixture() -> Result<(World, RevisionRef, DerivativeProvenance), Box<dyn std::error::Error>> {
    let mut world = World::default();
    world.create(AUTHOR, "parent", "parent source");
    let parent = world.save(AUTHOR, "parent", "", None);
    let provenance = ancestry(&world, parent)?;
    world.create(AUTHOR, "child", "working child");
    mark(&mut world, "child", &provenance)?;
    assert_eq!(
        world.store.mind(AUTHOR, "child")?.provenance.as_ref(),
        Some(&provenance)
    );
    Ok((world, parent, provenance))
}

#[test]
fn owner_lifecycle_preserves_lineage_and_cas_without_content_authority() -> TestResult {
    let (mut world, _, provenance) = fixture()?;
    let stale = world.store.mind(AUTHOR, "child")?.base();
    world.edit(AUTHOR, "child", "replacement source");
    let before = world.disk.clone();
    assert_eq!(
        world
            .store
            .prepare_edit(AUTHOR, "child", stale, draft("stale"))
            .err(),
        Some(RollError::StaleEdit)
    );
    assert_eq!(world.disk, before);
    let first = world.save(AUTHOR, "child", "notes", Some("named"));
    let saved = world.store.revision(AUTHOR, first.key)?.clone();
    assert_eq!(saved.provenance.as_ref(), Some(&provenance));
    assert_eq!(saved.require_original_access(), Err(RollError::Missing));
    saved.verify_source(&world.sources)?; // Frozen immutable custody still validates.

    for (copy, revision) in [("copydraft", None), ("copysaved", Some(first))] {
        let change = world
            .store
            .prepare_copy(AUTHOR, copy, "child", revision, &world.sources)?;
        world.commit(change);
        assert_eq!(
            world.store.mind(AUTHOR, copy)?.provenance.as_ref(),
            Some(&provenance)
        );
    }
    let base = world.store.mind(AUTHOR, "child")?.base();
    world.commit(
        world
            .store
            .prepare_rename_version(AUTHOR, "child", base, "named", "kept")?,
    );
    let base = world.store.mind(AUTHOR, "child")?.base();
    world.commit(
        world
            .store
            .prepare_forget_version(AUTHOR, "child", base, "kept")?,
    );
    let base = world.store.mind(AUTHOR, "child")?.base();
    world.commit(
        world
            .store
            .prepare_rename(AUTHOR, "child", base, "renamed")?,
    );
    assert_eq!(
        world.store.mind(AUTHOR, "renamed")?.provenance.as_ref(),
        Some(&provenance)
    );
    let base = world.store.mind(AUTHOR, "renamed")?.base();
    let before = world.disk.clone();
    assert_eq!(
        world.store.prepare_remove(AUTHOR, "renamed", base).err(),
        Some(RollError::Missing)
    );
    assert_eq!(world.disk, before);
    world.reboot();
    assert_eq!(world.store.revision(AUTHOR, first.key)?, &saved);
    assert_eq!(world.store.mind(AUTHOR, "renamed")?.id(), first.key.mind);
    assert_eq!(
        world.store.mind(AUTHOR, "renamed")?.provenance.as_ref(),
        Some(&provenance)
    );
    Ok(())
}

#[test]
fn marked_child_retains_exact_parent_after_original_history_is_gone() -> TestResult {
    let (mut world, parent, _) = fixture()?;
    // Advance the parent's bounded history until the referenced revision would
    // ordinarily be collectible. No new public retention root is supplied.
    for index in 0..=KEEP_UNNAMED {
        world.edit(AUTHOR, "parent", &format!("parent {index}"));
        world.save(AUTHOR, "parent", "", None);
    }
    let prune = world.store.prepare_prune(MAX_GC_BATCH)?;
    assert!(!prune.removed.contains(&parent));
    let child = world.save(AUTHOR, "child", "", None);
    let base = world.store.mind(AUTHOR, "child")?.base();
    assert_eq!(
        world.store.prepare_remove(AUTHOR, "child", base).err(),
        Some(RollError::Missing)
    );
    // Retain the separate historical-revision collection premise using the old
    // structurally valid persisted removed-Mind shape. This is not permission
    // to publish a removal and does not pretend to have a typed fork selector.
    let mind = world.store.mind(AUTHOR, "child")?.clone();
    world.disk.remove(&(NS.into(), mind.key()));
    let library = world
        .disk
        .get_mut(&(NS.into(), format!("library:{}", AUTHOR.0)))
        .ok_or("library")?;
    library
        .get_mut("names")
        .and_then(Value::as_object_mut)
        .ok_or("names")?
        .remove("child");
    let retained = ReferenceRecord {
        format: FORMAT,
        key: ReferenceKey {
            owner: NS.into(),
            kind: ReferenceKind::UnnamedHistory,
            record: format!("{}/{}", AUTHOR.0, mind.id()),
        },
        revisions: vec![child],
    };
    world.disk.insert(
        (NS.into(), retained.key.key()),
        serde_json::to_value(retained)?,
    );
    world.reboot();
    world
        .store
        .revision(AUTHOR, child.key)?
        .verify_source(&world.sources)?;
    assert!(
        !world
            .store
            .prepare_prune(MAX_GC_BATCH)?
            .removed
            .contains(&parent)
    );
    Ok(())
}

#[test]
fn record_version_requires_provenance_and_client_content_cannot_carry_it() -> TestResult {
    let (world, _, provenance) = fixture()?;
    let child = world.store.mind(AUTHOR, "child")?;
    let pristine = serde_json::to_value(child)?;
    for malformed in [
        Value::Null,
        json!({}),
        json!({"parent":provenance.parent,"charter":provenance.charter,"line":provenance.line,"rights":true}),
    ] {
        let mut row = pristine.clone();
        row.as_object_mut()
            .ok_or("object")?
            .insert("provenance".into(), malformed);
        assert!(
            serde_json::from_value::<Mind>(row)
                .map_err(|_| RollError::Missing)
                .and_then(|m| m.validate())
                .is_err()
        );
    }
    let mut absent = pristine.clone();
    absent.as_object_mut().ok_or("object")?.remove("provenance");
    assert!(serde_json::from_value::<Mind>(absent)?.validate().is_err());
    let mut wrong_version = pristine;
    wrong_version
        .as_object_mut()
        .ok_or("object")?
        .insert("format".into(), json!(FORMAT));
    assert!(
        serde_json::from_value::<Mind>(wrong_version)?
            .validate()
            .is_err()
    );

    let copy = serde_json::to_value(child.draft())?;
    assert!(copy.get("provenance").is_none());
    let mut injected = copy;
    injected
        .as_object_mut()
        .ok_or("object")?
        .insert("provenance".into(), serde_json::to_value(&provenance)?);
    assert!(serde_json::from_value::<WorkingCopy>(injected).is_err());
    let original = world.store.mind(AUTHOR, "parent")?;
    let bytes = serde_json::to_vec(original)?;
    assert!(
        serde_json::from_slice::<Value>(&bytes)?
            .get("provenance")
            .is_none()
    );
    assert_eq!(
        serde_json::to_vec(&serde_json::from_slice::<Mind>(&bytes)?)?,
        bytes
    );
    original.require_original_access()?;
    Ok(())
}

#[test]
fn missing_crossed_cyclic_and_overdepth_sources_quarantine_without_repair() -> TestResult {
    let (mut world, parent, mut provenance) = fixture()?;
    let child = world.save(AUTHOR, "child", "", None);
    provenance.parent.revision =
        RevisionIdentity::capture(world.store.revision(AUTHOR, child.key)?)?;
    mark(&mut world, "child", &provenance)?;
    assert!(world.store.mind(AUTHOR, "child").is_err(), "self cycle");

    let (mut world, _, _) = fixture()?;
    let child = world.save(AUTHOR, "child", "", None);
    let key = (NS.to_owned(), child.key.key());
    let row = world
        .disk
        .get_mut(&key)
        .ok_or("revision row")?
        .as_object_mut()
        .ok_or("object")?;
    row.insert("format".into(), json!(FORMAT));
    row.remove("provenance");
    let bytes = world.disk.clone();
    world.reboot();
    assert!(
        world.store.mind(AUTHOR, "child").is_err(),
        "saved lineage cannot be stripped independently"
    );
    assert_eq!(world.disk, bytes);

    let (mut world, _, _) = fixture()?;
    world.disk.remove(&(NS.to_owned(), parent.key.key()));
    world.reboot();
    assert!(
        world.store.mind(AUTHOR, "child").is_err(),
        "missing exact parent"
    );

    let mut world = World::default();
    world.create(AUTHOR, "root", "root");
    let mut parent = world.save(AUTHOR, "root", "", None);
    for depth in 1..=crate::commerce::filed::MAX_PARENT_DEPTH + 1 {
        let name = format!("depth{depth}");
        let provenance = ancestry(&world, parent)?;
        world.create(AUTHOR, &name, &name);
        mark(&mut world, &name, &provenance)?;
        if depth > crate::commerce::filed::MAX_PARENT_DEPTH {
            assert!(world.store.mind(AUTHOR, &name).is_err());
        } else {
            parent = world.save(AUTHOR, &name, "", None);
        }
    }
    Ok(())
}

#[test]
fn fork_participant_write_codec_accepts_only_canonical_fresh_attributed_images() -> TestResult {
    use crate::commerce::participant::RootParticipant;
    use mush_core::authoritative::{AuthoritativeKey, ReplayWrite};
    use mush_core::mechanics::OwnerId;
    use mush_core::participant::OwnerParticipant;
    let (world, _, _) = fixture()?;
    // Structural owner fixture only: actual grant/ACK authority is exercised
    // by the Tacsim producer integration. This pins the widened write filter.
    let copy = world
        .store
        .prepare_copy(AUTHOR, "first", "child", None, &world.sources)?;
    let mut images = 0;
    for op in copy.operations() {
        let ModuleWrite::JsonSet { key, value } = &op.op else {
            return Err("create image".into());
        };
        let make = |namespace: &str,
                    key: &str,
                    value: Value|
         -> Result<ReplayWrite, Box<dyn std::error::Error>> {
            Ok(ReplayWrite::ModuleJsonSet {
                namespace: OwnerId::new(namespace)?,
                key: AuthoritativeKey::parse_canonical(key)?,
                value,
            })
        };
        assert!(RootParticipant.accepts_write(&make(NS, key, value.clone())?));
        images += 1;
        assert!(!RootParticipant.accepts_write(&make("tacsim", key, value.clone())?));
        assert!(!RootParticipant.accepts_write(&make(NS, "source:arbitrary", value.clone())?));
        let mut extra = value.clone();
        extra
            .as_object_mut()
            .ok_or("image")?
            .insert("authority".into(), json!(true));
        assert!(!RootParticipant.accepts_write(&make(NS, key, extra)?));
        if key.starts_with("mind:") {
            for (field, bad) in [
                ("edit", json!(1)),
                ("next_revision", json!(2)),
                ("provenance", Value::Null),
            ] {
                let mut changed = value.clone();
                *changed.get_mut(field).ok_or("mind field")? = bad;
                assert!(!RootParticipant.accepts_write(&make(NS, key, changed)?));
            }
            // The shared retention participant now admits the exact Mind
            // deletion shape for its closed Retirement request. This structural
            // filter does not grant that request or prove its paired cause.
            assert!(RootParticipant.accepts_write(&ReplayWrite::ModuleDelete {
                namespace: OwnerId::new(NS)?,
                key: AuthoritativeKey::parse_canonical(key)?
            }));
            assert!(!RootParticipant.accepts_write(&ReplayWrite::ModuleDelete {
                namespace: OwnerId::new("tacsim")?,
                key: AuthoritativeKey::parse_canonical(key)?
            }));
            for bad_key in [
                "mind:1:0",
                "mind:01:2",
                "mind:1:02",
                "mind:1:2:3",
                "mind:1:invalid",
                "library:1",
                "revision:1:1:1",
                "source:arbitrary",
            ] {
                assert!(!RootParticipant.accepts_write(&ReplayWrite::ModuleDelete {
                    namespace: OwnerId::new(NS)?,
                    key: AuthoritativeKey::parse_canonical(bad_key)?
                }));
            }
        }
    }
    assert_eq!(
        images, 2,
        "the actual owner emits only its library and first draft"
    );
    Ok(())
}
