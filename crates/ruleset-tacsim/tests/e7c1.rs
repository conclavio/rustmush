//! **E7 cut 1 acceptance** (10b §3.1) — the three lenses, in order.
//!
//! - **P (pack identity)** — *can two byte streams produce one digest, or one stream
//!   two?* Everything rests on [`ruleset_tacsim::pack_digest`], so it is tested first
//!   and hardest: what is hashed, what is deliberately not, and what makes a pack
//!   digest different from every other digest over the same bytes.
//! - **S (seed idempotency)** — *can any interleaving or crash duplicate a seeded
//!   object or lose the latch?* The objects and the latch ride one `WriteBatch`, the
//!   world is a single writer, and the latch survives its own document's corruption
//!   because it is derived from the stored **key**.
//! - **B (boot fail-closed)** — *does any malformed pack degrade silently instead of
//!   refusing?* A corrupt stored document is skipped **loudly and the world boots**; a
//!   malformed pack **file** refuses the boot naming the file. Those are different
//!   answers on purpose; the file half lives in `mush-server`'s `config_ext` tests,
//!   because only the host can fail a boot.
//!
//! Everything runs through the real `Engine` via `TestWorld`, on the module alone —
//! `ruleset-tacsim` depends on `{mush-core, mush-sim}` and nothing else, so there is
//! nothing else to compose.

use std::collections::{BTreeMap, HashMap};

use mush_core::{Audience, CanonicalId, GOD, ModuleBootData, ModuleWrite, ObjId};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use ruleset_tacsim::{
    MAX_PACK_ENTRIES, MAX_REQUIRES, MAX_SEED_OBJECTS, PACK_DIGEST_DOMAIN, PackError, PackSource,
    RulesPack, TacsimConfig, TacsimRuleset, TacsimStore, check_requires, fixture, load_pack,
    pack_digest, store,
};
use smol_str::SmolStr;

// ---- rig -------------------------------------------------------------------

fn cid(s: &str) -> CanonicalId {
    CanonicalId::new(s).expect("a well-formed canonical id")
}

fn boot_with(cfg: TacsimConfig) -> TestWorld {
    TestWorld::with_modules(vec![Box::new(TacsimRuleset::with_config(cfg))])
}

/// The corridor fixture, live.
fn boot() -> TestWorld {
    boot_with(fixture::config())
}

fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n")
}

fn has_error(events: &[(Audience, OutputEvent)]) -> bool {
    events.iter().any(|(_, e)| {
        matches!(e, OutputEvent::Sys(rt) if rt.spans().iter().any(|sp| sp.tag.as_deref() == Some("error")))
    })
}

fn ok(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let ev = tw.run(actor, line);
    assert!(!has_error(&ev), "`{line}` was refused: {}", texts(&ev));
    texts(&ev)
}

fn err(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let ev = tw.run(actor, line);
    assert!(
        has_error(&ev),
        "`{line}` should have been refused, got: {}",
        texts(&ev)
    );
    texts(&ev)
}

fn snap(tw: &TestWorld) -> &TacsimStore {
    tw.world()
        .module_state::<TacsimStore>()
        .expect("the tacsim slot always exists")
}

type Kv = BTreeMap<SmolStr, Vec<(SmolStr, serde_json::Value)>>;

/// Every module's persisted documents, folded from the ops written since the world was
/// built.
fn fold_all(tw: &mut TestWorld) -> Kv {
    let mut kv: Kv = BTreeMap::new();
    for op in tw.take_module_ops() {
        let entry = kv.entry(op.ns.clone()).or_default();
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                entry.retain(|(k, _)| *k != key);
                entry.push((key, value));
            }
            ModuleWrite::Del { key } => entry.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    kv
}

fn reboot(tw: &mut TestWorld, kv: &Kv) {
    let boot: HashMap<SmolStr, ModuleBootData> = kv
        .iter()
        .map(|(ns, entries)| {
            (
                ns.clone(),
                ModuleBootData {
                    kv: entries.clone(),
                    streams: HashMap::new(),
                },
            )
        })
        .collect();
    tw.modules_loaded(boot);
}

fn patch(kv: &mut Kv, key: &str, f: impl Fn(&mut serde_json::Value)) {
    let entry = kv
        .get_mut(&SmolStr::new(ruleset_tacsim::NS))
        .expect("the namespace wrote");
    let mut seen = false;
    for (k, v) in entry.iter_mut() {
        if k.as_str() == key {
            f(v);
            seen = true;
        }
    }
    assert!(seen, "no such persisted key to patch: {key}");
}

/// A pack body with its declared digest filled in for it, so a test can perturb
/// exactly one thing at a time.
fn signed(body: &str) -> String {
    let text = body.replace("DIGEST", &"0".repeat(64));
    let parsed: RulesPack = serde_json::from_str(&text).expect("the wire parses");
    let digest = parsed.computed_digest().expect("digests").to_hex();
    body.replace("DIGEST", &digest)
}

const TWO_ENTRIES: &str = r#"{
  "manifest": { "id": "t.core", "version": 1, "kind": "rules_definition",
                "digest": "DIGEST", "requires": [] },
  "entries": {
    "metals": { "kind": "commodity", "def": { "label": "Metals", "in_use": true } },
    "volatiles": { "kind": "commodity", "def": { "label": "Volatiles" } }
  }
}"#;

// ---- P: the digest -------------------------------------------------------

/// **Identical bytes give one digest, and formatting is not content.**
///
/// A reordered table does not change the digest — the bytes hashed are the *canonical*
/// encoding of the parsed pack, never the file — while a whitespace-only edit **inside
/// a value** does. That pairing is the only self-consistent reading of 10b §3.1's
/// digest list: hashing raw file bytes would make the reordering change the digest too,
/// which the same list forbids.
#[test]
fn e7c1_digest_identical_bytes_agree_and_a_reordered_table_does_not() {
    let text = signed(TWO_ENTRIES);
    let first = load_pack(&text).expect("loads");
    let second = load_pack(&text).expect("loads");
    assert_eq!(first, second, "two loads are byte-identical in memory");
    assert_eq!(
        serde_json::to_string(&first).unwrap(),
        serde_json::to_string(&second).unwrap()
    );

    let reordered = signed(
        r#"{ "entries": {
               "volatiles": { "def": { "label": "Volatiles" }, "kind": "commodity" },
               "metals": { "def": { "in_use": true, "label": "Metals" }, "kind": "commodity" } },
             "manifest": { "version": 1, "digest": "DIGEST", "requires": [],
                           "kind": "rules_definition", "id": "t.core" } }"#,
    );
    assert_ne!(reordered, text, "the two files really are different bytes");
    let other = load_pack(&reordered).expect("loads");
    assert_eq!(
        first.computed_digest().unwrap(),
        other.computed_digest().unwrap(),
        "canonical encoding: reordering is not a content change"
    );

    let spaced =
        load_pack(&signed(&TWO_ENTRIES.replace("\"Metals\"", "\"Metals \""))).expect("loads");
    assert_ne!(
        first.computed_digest().unwrap(),
        spaced.computed_digest().unwrap(),
        "a whitespace edit inside a value IS a content change"
    );
}

/// **The manifest's own `digest` field is not in the hashed bytes.** A self-referential
/// hash is unverifiable, so changing only the declaration must not move the computation.
#[test]
fn e7c1_digest_excludes_the_manifests_own_digest_field() {
    let text = signed(TWO_ENTRIES);
    let good = load_pack(&text).expect("loads");
    let tampered = text.replace(&good.manifest().digest().to_hex(), &"a".repeat(64));
    let parsed: RulesPack = serde_json::from_str(&tampered).expect("the wire still parses");
    assert_eq!(
        parsed.computed_digest().unwrap(),
        good.computed_digest().unwrap(),
        "the declared digest must not feed the hash"
    );
    let canonical = good.canonical_value().unwrap();
    assert!(canonical["manifest"].get("digest").is_none());
    assert!(canonical["manifest"].get("version").is_some());
    assert!(canonical.get("entries").is_some());
}

/// **A declaration computed over non-canonical bytes is refused AT LOAD**, not at first
/// use — the comparison happens before the pack is admitted anywhere.
#[test]
fn e7c1_digest_a_non_canonical_declaration_is_refused_at_load() {
    let text = signed(TWO_ENTRIES);
    let good = load_pack(&text).expect("loads");
    // Exactly what an author gets by hashing the file's own bytes instead.
    let over_raw = mush_core::DefinitionDigest::for_canonical_value(
        PACK_DIGEST_DOMAIN,
        &serde_json::json!(text),
    )
    .expect("hashes");
    let bad = text.replace(&good.manifest().digest().to_hex(), &over_raw.to_hex());
    match load_pack(&bad) {
        Err(PackError::DigestMismatch { declared, computed }) => {
            assert_eq!(declared, over_raw.to_hex());
            assert_eq!(computed, good.manifest().digest().to_hex());
        }
        other => panic!("a non-canonical declaration must be refused at load: {other:?}"),
    }
}

/// **The `tacsim.pack.v1` domain tag separates a pack digest from every other digest
/// over the same bytes**, because `hash_parts` length-prefixes it.
#[test]
fn e7c1_digest_the_domain_tag_separates_a_pack_from_a_definition() {
    let value = load_pack(&signed(TWO_ENTRIES))
        .expect("loads")
        .canonical_value()
        .expect("encodes");
    let ours = pack_digest(&value).expect("hashes");
    let theirs = mush_core::DefinitionDigest::for_canonical_value("tacsim.definition.v1", &value)
        .expect("hashes");
    assert_ne!(ours, theirs, "the domain tag must be inside the hash");
    assert_eq!(
        ours,
        pack_digest(&value).expect("hashes"),
        "and it is stable"
    );
}

/// **`MAX_PACK_ENTRIES` refuses, and a stored pack past it is refused rather than
/// truncated.**
#[test]
fn e7c1_digest_max_pack_entries_refuses_and_never_truncates() {
    let entries: Vec<String> = (0..=MAX_PACK_ENTRIES)
        .map(|i| {
            format!("\"c{i}\": {{ \"kind\": \"commodity\", \"def\": {{ \"label\": \"c\" }} }}")
        })
        .collect();
    let text = format!(
        "{{ \"manifest\": {{ \"id\": \"t.big\", \"version\": 1, \"kind\": \"rules_definition\", \
         \"digest\": \"{}\", \"requires\": [] }}, \"entries\": {{ {} }} }}",
        "0".repeat(64),
        entries.join(",")
    );
    match serde_json::from_str::<RulesPack>(&text) {
        Err(error) => assert!(error.to_string().contains("maximum"), "{error}"),
        Ok(pack) => panic!("{} entries were admitted", pack.entries().len()),
    }
    assert!(load_pack(&text).is_err(), "the load path refuses it too");
}

// ---- B: loading ------------------------------------------------------------

/// **An unknown key is an error, never ignored**, at every level of the wire.
#[test]
fn e7c1_load_an_unknown_key_fails_closed() {
    for (from, to) in [
        ("\"entries\"", "\"surprise\": 1, \"entries\""),
        ("\"requires\": []", "\"requires\": [], \"extra\": 2"),
        (
            "\"label\": \"Metals\"",
            "\"label\": \"Metals\", \"grade\": 3",
        ),
    ] {
        let text = TWO_ENTRIES.replacen(from, to, 1);
        let refused = load_pack(&text.replace("DIGEST", &"0".repeat(64)))
            .expect_err("an unknown key must refuse");
        assert!(
            refused.to_string().contains("unknown field"),
            "{from}: {refused}"
        );
    }
}

/// **A missing `requires` and a `requires` version mismatch each fail closed**, with
/// different messages: "you forgot a file" is not "you upgraded underneath me".
#[test]
fn e7c1_load_requires_fails_closed_on_missing_and_on_mismatch() {
    let dependent = load_pack(&signed(
        r#"{ "manifest": { "id": "t.seed", "version": 1, "kind": "campaign_seed",
                           "digest": "DIGEST",
                           "requires": [{ "id": "t.core", "version": 1 }] },
             "entries": { "yard": { "kind": "location",
                                    "def": { "label": "Yard", "role": "yard" } } } }"#,
    ))
    .expect("loads");
    let mut loaded: BTreeMap<(CanonicalId, u32), RulesPack> = BTreeMap::new();
    assert!(matches!(
        check_requires(&dependent, &loaded),
        Err(PackError::MissingRequire { .. })
    ));
    let core_v2 = load_pack(&signed(&TWO_ENTRIES.replacen(
        "\"version\": 1",
        "\"version\": 2",
        1,
    )))
    .expect("loads");
    loaded.insert((cid("t.core"), 2), core_v2);
    match check_requires(&dependent, &loaded) {
        Err(PackError::RequireVersionMismatch {
            wanted, present, ..
        }) => {
            assert_eq!((wanted, present.as_str()), (1, "2"));
        }
        other => panic!("a version mismatch is its own refusal: {other:?}"),
    }
    loaded.insert(
        (cid("t.core"), 1),
        load_pack(&signed(TWO_ENTRIES)).expect("loads"),
    );
    assert!(check_requires(&dependent, &loaded).is_ok());
}

/// **Every wire re-runs its own constructor.** A derived `Deserialize` would walk past
/// every cap and make 10b §7 advisory on load — the exact E5a finding.
#[test]
fn e7c1_load_every_wire_re_runs_its_constructor() {
    let mut many = String::new();
    for i in 0..=MAX_REQUIRES {
        many.push_str(&format!("{{\"id\":\"d{i}\",\"version\":1}},"));
    }
    let many = many.trim_end_matches(',').to_string();
    for (requires, version, needle) in [
        ("", 0, "starts at one"),
        ("{\"id\":\"t.core\",\"version\":1}", 1, "itself"),
        (
            "{\"id\":\"d0\",\"version\":1},{\"id\":\"d0\",\"version\":2}",
            1,
            "more than once",
        ),
        (many.as_str(), 1, "maximum"),
    ] {
        let text = format!(
            "{{ \"manifest\": {{ \"id\": \"t.core\", \"version\": {version}, \
             \"kind\": \"rules_definition\", \"digest\": \"{}\", \"requires\": [{requires}] }}, \
             \"entries\": {{}} }}",
            "0".repeat(64)
        );
        let refused = serde_json::from_str::<RulesPack>(&text).expect_err(needle);
        assert!(refused.to_string().contains(needle), "{needle}: {refused}");
    }
    let refused = serde_json::from_str::<RulesPack>(&TWO_ENTRIES.replace("DIGEST", "nope"))
        .expect_err("a malformed digest");
    assert!(refused.to_string().contains("unreadable"), "{refused}");

    // The seed latch's own redundancy is an invariant: a stored record whose declared
    // count disagrees with its placement map is refused, not believed.
    let latch = serde_json::json!({
        "pack": "t.seed", "version": 1, "digest": "0".repeat(64),
        "applied_at": { "clock": { "kind": "world_ic", "generation": 1 }, "at": 0 },
        "objects": 9, "placed": { "yard": 5 }
    });
    let refused = serde_json::from_value::<ruleset_tacsim::SeedApplication>(latch)
        .expect_err("a count that disagrees with the map");
    assert!(refused.to_string().contains("declares 9"), "{refused}");
}

/// **Q6A's rules/seed distinction is structural**, and a route may only name places its
/// own pack declares — resolving across packs would make a route's meaning depend on
/// load order.
#[test]
fn e7c1_load_an_entry_on_the_wrong_side_of_the_kind_refuses() {
    let location_in_rules = TWO_ENTRIES.replacen(
        "\"volatiles\": { \"kind\": \"commodity\", \"def\": { \"label\": \"Volatiles\" } }",
        "\"yard\": { \"kind\": \"location\", \"def\": { \"label\": \"Y\", \"role\": \"yard\" } }",
        1,
    );
    let refused = load_pack(&location_in_rules.replace("DIGEST", &"0".repeat(64)))
        .expect_err("a location in a rules pack");
    assert!(refused.to_string().contains("may not declare"), "{refused}");

    let dangling = r#"{ "manifest": { "id": "t.seed", "version": 1, "kind": "campaign_seed",
                        "digest": "DIGEST", "requires": [] },
          "entries": {
            "here": { "kind": "location", "def": { "label": "H", "role": "yard" } },
            "r": { "kind": "route", "def": { "from": "here", "to": "elsewhere",
                    "legs": [{ "to": "elsewhere", "base_s": 5 }],
                    "drive_class": "t", "propellant": 0 } } } }"#;
    let refused = load_pack(&dangling.replace("DIGEST", &"0".repeat(64)))
        .expect_err("a route naming an undeclared place");
    assert!(
        refused.to_string().contains("does not declare"),
        "{refused}"
    );
}

/// **A corrupt stored `tacsim:*` document is skipped loudly and the world boots** — the
/// opposite answer to a malformed pack *file*, which refuses the boot naming it
/// (`mush-server`'s `config_ext` tests). Both answers are asserted, on purpose.
///
/// And the direction of each loss is checked, not just its survival: a corrupt **seed
/// latch still bars a re-seed**, because it is the stored *key* that latches.
#[test]
fn e7c1_boot_a_corrupt_stored_document_is_skipped_loudly_and_the_world_boots() {
    let mut tw = boot();
    ok(&mut tw, GOD, "@tacsim/seed tacsim.corridor commit");
    let mut kv = fold_all(&mut tw);

    // Corrupt the ledger row, the latch and a pin, all three.
    ok(&mut tw, GOD, "@tacsim/pin alloy_smelt=tacsim.core@1");
    let extra = fold_all(&mut tw);
    for (k, v) in extra.get(&SmolStr::new(ruleset_tacsim::NS)).unwrap() {
        let ns = kv.entry(SmolStr::new(ruleset_tacsim::NS)).or_default();
        ns.retain(|(key, _)| key != k);
        ns.push((k.clone(), v.clone()));
    }
    patch(&mut kv, "tacsim:pack:tacsim.core@1", |v| {
        *v = serde_json::json!({ "id": "tacsim.core", "version": 1, "rubbish": true });
    });
    patch(&mut kv, "tacsim:seed:tacsim.corridor@1", |v| {
        *v = serde_json::json!("not a record at all");
    });
    patch(&mut kv, "tacsim:pin:alloy_smelt", |v| {
        *v = serde_json::json!({ "subject": "somebody_else", "pack": "tacsim.core",
                                 "version": 1, "pinned_at": null });
    });

    let mut fresh = boot();
    reboot(&mut fresh, &kv);
    // The world booted. Boot **repairs** the ledger row it could not read, from the
    // loaded pack, which is a write — drain it, so the re-seed assertion below is about
    // the seed and nothing else.
    let _ = fresh.take_module_ops();
    let store = snap(&fresh);
    // The bodies are gone...
    assert!(store.seed(&cid("tacsim.corridor"), 1).is_none());
    assert!(store.pin(&cid("alloy_smelt")).is_none());
    // ...but the KEY still latches: a re-seed is refused, not repeated.
    assert!(store.is_applied(&cid("tacsim.corridor"), 1));
    assert_eq!(store.applied_version(&cid("tacsim.corridor")), Some(1));
    let mut fresh = fresh;
    let out = ok(&mut fresh, GOD, "@tacsim/seed tacsim.corridor commit");
    assert!(out.contains("already been applied"), "{out}");
    assert!(
        fresh.take_module_ops().is_empty(),
        "a corrupt latch must still write nothing on re-seed"
    );
}

// ---- S: the seed latch -----------------------------------------------------

/// **`dry` writes nothing and lists every planned object; `commit` applies once; a
/// second `commit` is a HIT writing nothing.**
#[test]
fn e7c1_seed_dry_writes_nothing_and_commit_applies_exactly_once() {
    let mut tw = boot();
    // Boot itself publishes the two ledger rows; drain them so the assertions below
    // are about the seed and nothing else.
    let _ = tw.take_module_ops();

    let out = ok(&mut tw, GOD, "@tacsim/seed tacsim.corridor dry");
    assert!(out.contains("would create 3 object(s)"), "{out}");
    for place in fixture::PLACES {
        assert!(out.contains(place), "dry must list {place}: {out}");
    }
    assert!(
        tw.take_module_ops().is_empty(),
        "`dry` must write nothing at all"
    );
    assert!(snap(&tw).seed(&cid("tacsim.corridor"), 1).is_none());

    let out = ok(&mut tw, GOD, "@tacsim/seed tacsim.corridor commit");
    assert!(out.contains("3 object(s) created"), "{out}");
    let record = snap(&tw)
        .seed(&cid("tacsim.corridor"), 1)
        .expect("the latch")
        .clone();
    assert_eq!(record.objects(), 3);
    assert_eq!(record.placed().len(), 3);
    let wrote = tw.take_module_ops();
    assert!(!wrote.is_empty());

    let out = ok(&mut tw, GOD, "@tacsim/seed tacsim.corridor commit");
    assert!(
        out.contains("already been applied"),
        "the second is a HIT: {out}"
    );
    assert!(
        tw.take_module_ops().is_empty(),
        "a second commit must write nothing"
    );
    assert_eq!(
        snap(&tw).seed(&cid("tacsim.corridor"), 1).unwrap(),
        &record,
        "and must not touch the latch"
    );
}

/// **Two `commit`s in one world serialize and the loser is a HIT, not a partial write.**
///
/// "Two processes, one store" is not a reachable shape: the world is a single writer,
/// so this is the race that exists, and it is run from two different actors to make the
/// point that the serialization is the world's rather than the caller's.
#[test]
fn e7c1_seed_two_commits_in_one_world_serialize_and_the_loser_is_a_hit() {
    let mut tw = boot();
    let second_staff = tw.spawn_player("Deckard");
    // Give the second actor staff, so the HIT cannot be a permission artefact.
    tw.world_mut()
        .get_mut(second_staff)
        .expect("the player exists")
        .flags
        .insert(mush_core::Flags::ROYALTY);
    let _ = tw.take_module_ops();

    ok(&mut tw, GOD, "@tacsim/seed tacsim.corridor commit");
    let first = snap(&tw).seed(&cid("tacsim.corridor"), 1).unwrap().clone();
    let _ = tw.take_module_ops();

    let out = ok(&mut tw, second_staff, "@tacsim/seed tacsim.corridor commit");
    assert!(out.contains("already been applied"), "{out}");
    assert!(tw.take_module_ops().is_empty());
    assert_eq!(snap(&tw).seed(&cid("tacsim.corridor"), 1).unwrap(), &first);
    // Exactly one applied set: three objects, not six.
    assert_eq!(snap(&tw).seeded_objects().count(), 3);
}

/// **A different version of the same pack is refused, naming the applied one.**
#[test]
fn e7c1_seed_a_different_version_is_refused_naming_the_applied_one() {
    let mut tw = boot();
    ok(&mut tw, GOD, "@tacsim/seed tacsim.corridor commit");
    let kv = fold_all(&mut tw);

    // Reboot a world that loads only v2 of the corridor while the store carries the
    // v1 latch — the exact "you upgraded underneath me" shape.
    let v2 = fixture::CORRIDOR_JSON.replacen("\"version\": 1", "\"version\": 2", 1);
    let v2: RulesPack = serde_json::from_str(&v2).expect("parses");
    let v2 = serde_json::to_string(&v2).expect("re-encodes").replacen(
        &v2.manifest().digest().to_hex(),
        &v2.computed_digest().unwrap().to_hex(),
        1,
    );
    let cfg = TacsimConfig {
        seeds: vec![PackSource {
            name: "<v2>".to_string(),
            bytes: v2,
        }],
        ..fixture::config()
    };
    let mut tw = boot_with(cfg);
    reboot(&mut tw, &kv);
    // Boot publishes a ledger row for the newly loaded v2; drain it.
    let _ = tw.take_module_ops();
    let refused = err(&mut tw, GOD, "@tacsim/seed tacsim.corridor commit");
    assert!(
        refused.contains("tacsim.corridor@1 has already been applied"),
        "the applied version must be named: {refused}"
    );
    assert!(refused.contains("@2"), "and the one refused: {refused}");
    assert!(tw.take_module_ops().is_empty());
}

/// **The objects and the latch are one `WriteBatch`, so a reboot sees both or
/// neither**, and the fixture is reproducible from an empty store.
#[test]
fn e7c1_seed_the_objects_and_the_latch_survive_a_reboot_together() {
    let mut tw = boot();
    ok(&mut tw, GOD, "@tacsim/seed tacsim.corridor commit");
    let before: BTreeMap<String, ObjId> = snap(&tw)
        .seeded_objects()
        .map(|(id, obj)| (id.to_string(), obj))
        .collect();
    assert_eq!(before.len(), 3);
    let kv = fold_all(&mut tw);
    reboot(&mut tw, &kv);
    let after: BTreeMap<String, ObjId> = snap(&tw)
        .seeded_objects()
        .map(|(id, obj)| (id.to_string(), obj))
        .collect();
    assert_eq!(
        before, after,
        "the latch names the same objects after a reboot"
    );
    for obj in after.values() {
        assert!(
            tw.world().get(*obj).is_some(),
            "and the objects are still there"
        );
    }
    // Reproducible from an empty store: a fresh world seeded from the same pack makes
    // the same pack-local ids and the same route.
    let mut fresh = boot();
    ok(&mut fresh, GOD, "@tacsim/seed tacsim.corridor commit");
    let names: Vec<String> = snap(&fresh)
        .seeded_objects()
        .map(|(id, _)| id.to_string())
        .collect();
    assert_eq!(names, before.keys().cloned().collect::<Vec<_>>());
}

/// **`MAX_SEED_OBJECTS` refuses before the first object is created.**
#[test]
fn e7c1_seed_max_seed_objects_refuses_before_the_first_object() {
    let entries: Vec<String> = (0..=MAX_SEED_OBJECTS)
        .map(|i| {
            format!("\"p{i}\": {{ \"kind\": \"location\", \"def\": {{ \"label\": \"P\", \"role\": \"yard\" }} }}")
        })
        .collect();
    let text = format!(
        "{{ \"manifest\": {{ \"id\": \"t.big\", \"version\": 1, \"kind\": \"campaign_seed\", \
         \"digest\": \"{}\", \"requires\": [] }}, \"entries\": {{ {} }} }}",
        "0".repeat(64),
        entries.join(",")
    );
    // It never even becomes a pack: `MAX_SEED_OBJECTS` is re-checked by the wire, so a
    // pack this size is refused at load, long before `@tacsim/seed` could plan it.
    let refused = serde_json::from_str::<RulesPack>(&text).expect_err("past MAX_SEED_OBJECTS");
    assert!(
        refused.to_string().contains("maximum") || refused.to_string().contains("instantiate"),
        "{refused}"
    );
}

/// **A seed onto a store already carrying another pack's objects applies only its own
/// and says nothing about theirs.**
#[test]
fn e7c1_seed_applies_only_its_own_and_names_nothing_of_anothers() {
    let other = signed(
        r#"{ "manifest": { "id": "t.other", "version": 1, "kind": "campaign_seed",
                           "digest": "DIGEST", "requires": [] },
             "entries": { "depot": { "kind": "location",
                                     "def": { "label": "Some Depot", "role": "yard" } } } }"#,
    );
    let mut cfg = fixture::config();
    cfg.seeds.push(PackSource {
        name: "<t.other>".to_string(),
        bytes: other,
    });
    let mut tw = boot_with(cfg);
    ok(&mut tw, GOD, "@tacsim/seed t.other commit");
    let out = ok(&mut tw, GOD, "@tacsim/seed tacsim.corridor commit");
    assert!(out.contains("3 object(s)"), "{out}");
    assert!(!out.contains("depot") && !out.contains("Depot"), "{out}");
    // Two independent latches, four objects, and neither pack's latch mentions the
    // other's.
    assert_eq!(snap(&tw).seeded_objects().count(), 4);
    let corridor = snap(&tw).seed(&cid("tacsim.corridor"), 1).unwrap();
    assert_eq!(corridor.placed().len(), 3);
    assert!(corridor.placed().get(&cid("depot")).is_none());
    assert_eq!(snap(&tw).applied_version(&cid("t.other")), Some(1));
}

// ---- pinning ---------------------------------------------------------------

/// **A subject pinned to `v1` is unaffected by publishing `v2`; both versions load
/// concurrently; an unpinned consumer of a retired version fails closed.**
#[test]
fn e7c1_pin_a_pinned_subject_ignores_a_newer_version_and_a_retired_one_fails_closed() {
    let v1 = signed(TWO_ENTRIES);
    let v2 = signed(
        &TWO_ENTRIES
            .replacen("\"version\": 1", "\"version\": 2", 1)
            .replace("\"Volatiles\"", "\"Volatile compounds\""),
    );
    let cfg = TacsimConfig {
        enabled: true,
        packs: vec![
            PackSource {
                name: "<v1>".to_string(),
                bytes: v1.clone(),
            },
            PackSource {
                name: "<v2>".to_string(),
                bytes: v2.clone(),
            },
        ],
        ..TacsimConfig::default()
    };
    let mut tw = boot_with(cfg.clone());
    // Both versions load concurrently.
    assert_eq!(
        snap(&tw).versions_of(&cid("t.core")).collect::<Vec<_>>(),
        vec![1, 2]
    );
    // Unpinned, an unseeded subject resolves to the newest loaded version.
    assert_eq!(
        snap(&tw).resolve(&cid("volatiles")).expect("resolves"),
        (cid("t.core"), 2)
    );
    // Pinned, it stays at v1 no matter what is published.
    ok(&mut tw, GOD, "@tacsim/pin volatiles=t.core@1");
    assert_eq!(
        snap(&tw).resolve(&cid("volatiles")).expect("resolves"),
        (cid("t.core"), 1)
    );
    let kv = fold_all(&mut tw);

    // Retire v1 — the operator drops it from `pack_paths` — and the pin fails **closed**,
    // naming the version, rather than silently upgrading to v2.
    let only_v2 = TacsimConfig {
        packs: vec![PackSource {
            name: "<v2>".to_string(),
            bytes: v2,
        }],
        ..cfg.clone()
    };
    let mut retired = boot_with(only_v2);
    reboot(&mut retired, &kv);
    match snap(&retired).resolve(&cid("volatiles")) {
        Err(ruleset_tacsim::ResolveError::Retired { version, how, .. }) => {
            assert_eq!((version, how), (1, "pinned"));
        }
        other => panic!("a pin to a retired version must fail closed: {other:?}"),
    }
    // And an *unpinned* consumer of a retired **seeded** version fails closed too.
    let mut seeded = boot();
    ok(&mut seeded, GOD, "@tacsim/seed tacsim.corridor commit");
    let seeded_kv = fold_all(&mut seeded);
    let corridor_v2 = {
        let text = fixture::CORRIDOR_JSON.replacen("\"version\": 1", "\"version\": 2", 1);
        let parsed: RulesPack = serde_json::from_str(&text).expect("parses");
        text.replacen(
            &parsed.manifest().digest().to_hex(),
            &parsed.computed_digest().unwrap().to_hex(),
            1,
        )
    };
    let mut moved = boot_with(TacsimConfig {
        seeds: vec![PackSource {
            name: "<corridor v2>".to_string(),
            bytes: corridor_v2,
        }],
        ..fixture::config()
    });
    reboot(&mut moved, &seeded_kv);
    match snap(&moved).resolve(&cid("ceres_extraction")) {
        Err(ruleset_tacsim::ResolveError::Retired { version, how, .. }) => {
            assert_eq!((version, how), (1, "seeded"));
        }
        other => panic!("an unpinned consumer of a retired version must fail closed: {other:?}"),
    }
}

/// A pin names one exact version, and a pin to a version this server does not load is
/// refused at the command rather than stored as a pin to nothing.
#[test]
fn e7c1_pin_requires_an_exact_loaded_version() {
    let mut tw = boot();
    err(&mut tw, GOD, "@tacsim/pin alloy_smelt=tacsim.core");
    err(&mut tw, GOD, "@tacsim/pin alloy_smelt=tacsim.core@7");
    let ordinary = tw.spawn_player("Rigger");
    err(&mut tw, ordinary, "@tacsim/pin alloy_smelt=tacsim.core@1");
    ok(&mut tw, GOD, "@tacsim/pin alloy_smelt=tacsim.core@1");
    assert_eq!(
        snap(&tw).pin(&cid("alloy_smelt")).map(|p| p.version()),
        Some(1)
    );
}

// ---- the fixture, the routes and disclosure --------------------------------

/// **The corridor fixture: three places, two commodities in use, one recipe and one
/// route exist after `commit` — and nowhere else.**
#[test]
fn e7c1_fixture_the_corridor_exists_after_commit_and_nowhere_else() {
    let mut tw = boot();
    // The pack's *definitions* are there from boot; the world objects are not.
    let core = snap(&tw)
        .pack(&cid("tacsim.core"), 1)
        .expect("loaded")
        .clone();
    for family in fixture::COMMODITY_FAMILIES {
        assert!(
            core.pack.entry(&cid(family)).is_some(),
            "the pack must declare {family}"
        );
    }
    assert!(core.pack.entry(&cid(fixture::RECIPE)).is_some());
    assert_eq!(core.pack.entries().len(), 11);
    let corridor = snap(&tw)
        .pack(&cid(fixture::CORRIDOR_PACK), 1)
        .expect("loaded")
        .clone();
    assert_eq!(corridor.pack.entries().len(), 4);
    assert!(
        core.pack.entries().len() + corridor.pack.entries().len() <= fixture::MAX_FIXTURE_ENTRIES
    );
    assert_eq!(
        snap(&tw).seeded_objects().count(),
        0,
        "nothing before commit"
    );
    assert!(snap(&tw).route(&cid(fixture::ROUTE)).is_none());

    ok(&mut tw, GOD, "@tacsim/seed tacsim.corridor commit");
    assert_eq!(snap(&tw).seeded_objects().count(), 3);
    let route = snap(&tw)
        .route(&cid(fixture::ROUTE))
        .expect("materialized")
        .clone();
    let places: BTreeMap<String, ObjId> = snap(&tw)
        .seeded_objects()
        .map(|(id, obj)| (id.to_string(), obj))
        .collect();
    assert_eq!(
        route.endpoints(),
        (places["ceres_extraction"], places["phobos_yard"])
    );
    assert_eq!(route.legs().len(), 2);
    assert_eq!(route.legs()[0].to(), places["transfer_node"]);
    assert!(route.alignment_band().is_none(), "cut 6 binds the band");
    // The pure evaluator agrees with itself over the fixture's own drive class.
    let drive = store::nominal_drive(route.drive_class()).expect("a nominal drive");
    let a = mush_sim::route_duration(&route, &drive, 0).expect("evaluates");
    let b = mush_sim::route_duration(&route, &drive, 0).expect("evaluates");
    assert_eq!((a, b), (86_400 + 172_800, 86_400 + 172_800));

    // And **no quantity of anything** exists (10b §9 q4: there is no interim).
    let encoded = serde_json::to_string(&core.pack).expect("encodes");
    assert!(
        !encoded.contains("\"quantity\""),
        "cut 1 stores no quantities"
    );
}

/// **`[tacsim].enabled = false` creates nothing, evaluates nothing and shows nothing** —
/// but the verbs still register, so the layer refuses out loud rather than vanishing.
#[test]
fn e7c1_config_a_disabled_layer_creates_nothing_and_shows_nothing() {
    let mut tw = boot_with(TacsimConfig::default());
    assert!(!snap(&tw).enabled());
    assert_eq!(snap(&tw).packs().count(), 0);
    for line in [
        "@tacsim/pack list",
        "@tacsim/pack show tacsim.core",
        "@tacsim/seed tacsim.corridor dry",
        "@tacsim/seed tacsim.corridor commit",
        "@tacsim/pin alloy_smelt=tacsim.core@1",
    ] {
        let refused = err(&mut tw, GOD, line);
        assert!(refused.contains("not enabled"), "{line}: {refused}");
    }
    assert!(
        tw.take_module_ops().is_empty(),
        "a disabled layer writes nothing"
    );
    assert_eq!(snap(&tw).seeded_objects().count(), 0);
}

/// **No pack-existence oracle.** `@tacsim/pack show` on an unknown id and on a known
/// campaign-seed pack a non-staff viewer may not read answer **byte-identically**, and
/// every refusal is one message.
#[test]
fn e7c1_disclosure_an_unknown_and_an_unreadable_pack_answer_identically() {
    let mut tw = boot();
    let ordinary = tw.spawn_player("Rigger");
    let unknown = tw.run(ordinary, "@tacsim/pack show tacsim.nothing");
    let hidden = tw.run(ordinary, "@tacsim/pack show tacsim.corridor");
    assert_eq!(
        texts(&unknown),
        texts(&hidden),
        "a seed pack must not be distinguishable from one that does not exist"
    );
    assert_eq!(unknown.len(), 1, "one message, through the hiding renderer");
    assert_eq!(hidden.len(), 1);
    assert!(texts(&unknown).contains("No such pack."));
    // The same for `verify`, which would otherwise be the oracle `show` is not.
    assert_eq!(
        texts(&tw.run(ordinary, "@tacsim/pack verify tacsim.nothing")),
        texts(&tw.run(ordinary, "@tacsim/pack verify tacsim.corridor"))
    );
    // A rules pack IS public — that is the whole distinction.
    let public = ok(&mut tw, ordinary, "@tacsim/pack show tacsim.core");
    assert!(public.contains("tacsim.core@1"), "{public}");
    // And staff see the seed pack.
    let staff = ok(&mut tw, GOD, "@tacsim/pack show tacsim.corridor");
    assert!(staff.contains("ceres_phobos"), "{staff}");
    // Seeding and pinning are staff-only, and refuse with one message.
    assert_eq!(
        tw.run(ordinary, "@tacsim/seed tacsim.corridor dry").len(),
        1
    );
    assert_eq!(
        tw.run(ordinary, "@tacsim/pin alloy_smelt=tacsim.core@1")
            .len(),
        1
    );
}

/// `@tacsim/pack verify` re-verifies the **loaded bytes** and re-reads nothing.
#[test]
fn e7c1_verify_re_checks_the_loaded_bytes_and_reads_no_file() {
    let mut tw = boot();
    let _ = tw.take_module_ops();
    let out = ok(&mut tw, GOD, "@tacsim/pack verify");
    assert!(out.contains("tacsim.core@1 ok"), "{out}");
    assert!(out.contains("tacsim.corridor@1 ok"), "{out}");
    assert!(
        tw.take_module_ops().is_empty(),
        "verify is a read: it writes nothing"
    );
    let one = ok(&mut tw, GOD, "@tacsim/pack verify tacsim.core@1");
    assert!(
        one.contains("tacsim.core@1 ok") && !one.contains("corridor"),
        "{one}"
    );
}

/// The published-version ledger is written at boot and is what makes retirement
/// detectable later; a reboot does not re-write a row it already has.
#[test]
fn e7c1_boot_publishes_a_ledger_row_once_per_version() {
    let mut tw = boot();
    assert_eq!(snap(&tw).published().count(), 2);
    let kv = fold_all(&mut tw);
    let rows = kv
        .get(&SmolStr::new(ruleset_tacsim::NS))
        .expect("the namespace wrote")
        .iter()
        .filter(|(k, _)| k.starts_with("tacsim:pack:"))
        .count();
    assert_eq!(rows, 2);
    reboot(&mut tw, &kv);
    assert_eq!(snap(&tw).published().count(), 2);
    assert!(
        tw.take_module_ops().is_empty(),
        "a second boot re-writes no ledger row"
    );
}
