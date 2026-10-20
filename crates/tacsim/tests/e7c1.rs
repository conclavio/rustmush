//! **E7 cut 1 acceptance** (10b §3.1) — the three lenses, in order.
//!
//! - **P (pack identity)** — *can two byte streams produce one digest, or one stream
//!   two?* Everything rests on [`tacsim::pack_digest`], so it is tested first
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
//! `tacsim` depends on `{mush-core, mush-sim}` and nothing else, so there is
//! nothing else to compose.

use std::collections::{BTreeMap, HashMap};

use mush_core::{Audience, CanonicalId, GOD, ModuleBootData, ModuleWrite, ObjId};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;
use tacsim::{
    MAX_CONFIGURED_PACKS, MAX_LEG_SECS, MAX_PACK_ENTRIES, MAX_PINS, MAX_RECIPE_LINES,
    MAX_RECIPE_UNITS, MAX_REQUIRES, MAX_ROUTE_PROPELLANT, MAX_SEED_OBJECTS, PACK_DIGEST_DOMAIN,
    PACK_PAGE, PackEntry, PackError, PackKind, PackManifest, PackSource, ResolveError, RulesPack,
    TacsimConfig, TacsimRuleset, TacsimStore, check_requires, fixture, load_pack, pack_digest,
    store, verify_sources,
};

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

fn boot_data(kv: &Kv) -> HashMap<SmolStr, ModuleBootData> {
    kv.iter()
        .map(|(ns, entries)| {
            (
                ns.clone(),
                ModuleBootData {
                    kv: entries.clone(),
                    streams: HashMap::new(),
                    ..Default::default()
                },
            )
        })
        .collect()
}

fn reboot(tw: &mut TestWorld, kv: &Kv) {
    tw.modules_loaded(boot_data(kv));
}

/// A reboot that may **refuse**, which `TestWorld::modules_loaded` cannot express: the
/// P1 digest gate is a boot error, and a boot error is the thing being asserted.
fn reboot_result(tw: &mut TestWorld, kv: &Kv) -> Result<(), mush_core::module::ModError> {
    let mut out = mush_core::Outbox::new();
    let mut fx = mush_core::Effects::new();
    tw.engine_mut()
        .modules_loaded(boot_data(kv), &mut out, &mut fx)
}

fn patch(kv: &mut Kv, key: &str, f: impl Fn(&mut serde_json::Value)) {
    let entry = kv
        .get_mut(&SmolStr::new(tacsim::NS))
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
    // S7 (D-P-1): a pack's digest is the content layer's, over the authored bytes.
    let text = body.replace("DIGEST", &"0".repeat(64));
    let parsed = mush_content::parse_pack(&text).expect("the wire parses");
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
    // S7: the hash is over the authored bytes' canonical form, the content layer's.
    let parsed = mush_content::parse_pack(&tampered).expect("the wire still parses");
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
    let refused = serde_json::from_value::<tacsim::SeedApplication>(latch)
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
    for (k, v) in extra.get(&SmolStr::new(tacsim::NS)).unwrap() {
        let ns = kv.entry(SmolStr::new(tacsim::NS)).or_default();
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

/// **`MAX_SEED_OBJECTS` refuses before the first object is created** — and it is the
/// **seed-object** cap that fires, not the entry cap standing in for it.
///
/// Rewritten by the independent verify (P5): the pack is now `MAX_SEED_OBJECTS + 1`
/// locations *inside* `MAX_PACK_ENTRIES`, and the assertion is the exact variant, so
/// the two guards can never again be confused. `vf_p5_…` below pins the sibling.
#[test]
fn e7c1_seed_max_seed_objects_refuses_before_the_first_object() {
    // A seed pack instantiates a *subset* of its entries, so the seed-object cap has to
    // sit below the entry cap or it can never be the guard that fires (P5).
    const _: () = assert!(MAX_SEED_OBJECTS < MAX_PACK_ENTRIES);
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
    let wire: tacsim::pack::RulesPackWire =
        serde_json::from_str(&text).expect("the entry table is inside MAX_PACK_ENTRIES");
    match RulesPack::try_from(wire) {
        Err(PackError::TooManySeedObjects { actual, max }) => {
            assert_eq!((actual, max), (MAX_SEED_OBJECTS + 1, MAX_SEED_OBJECTS));
        }
        other => panic!("the SEED-OBJECT cap must be what refuses: {other:?}"),
    }
    assert!(load_pack(&text).is_err(), "the load path refuses it too");
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
        Err(tacsim::ResolveError::Retired { version, how, .. }) => {
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
        Err(tacsim::ResolveError::Retired { version, how, .. }) => {
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
    // E7 cut 9 added THREE `PackEntry::Formation` rules definitions — a lance, a
    // star and a wing — so the core pack carries 14 entries and its digest moved
    // (re-derived, never hand-edited: `load_pack` recomputes and compares, so the
    // move is its own test).
    assert_eq!(core.pack.entries().len(), 14);
    let corridor = snap(&tw)
        .pack(&cid(fixture::CORRIDOR_PACK), 1)
        .expect("loaded")
        .clone();
    // E7 cut 3 added one `PackEntry::Deposit` at `ceres_extraction`; E7 cut 4
    // added one `PackEntry::Facility` at `phobos_yard`; S7 (T17) added the three
    // bodies of the corridor's sky.
    assert_eq!(corridor.pack.entries().len(), 9);
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
    // S7 (T17): the corridor crosses from Ceres to Mars, so the sky pinned it.
    assert!(
        route.distance_km().is_some_and(|km| km > 0),
        "the ephemeris pins a distance between two bodies' places"
    );
    assert_eq!(
        route.bodies().map(|(a, b)| (a.as_str(), b.as_str())),
        Some(("ceres", "mars"))
    );
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
        .get(&SmolStr::new(tacsim::NS))
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

// ---- E7 cut 1 independent verify fold ----
//
// One test per confirmed finding of the independent, probe-confirming adversarial
// verify (10b §4 step (4)). The lens ids are the verify's own, so the four cross-lens
// duplicates land once each: P1 covers S1, P2 covers S2, B4 covers P4, B5 covers S6.
// P9 and S5 are unit tests in `seed.rs` and `store.rs`, where a bare `Ctx` and a bare
// `ModuleWrite` are in scope; B5 and B6 are in `mush-server`'s `config_ext` and
// `mush-config`'s `config`, where the host and the section live.

/// A rules pack of authored entry lines, with `DIGEST` still to be filled in — so a
/// body that is *meant* to be refused can be handed to [`load_pack`] without [`signed`]
/// having to parse it first.
fn rules_pack(id: &str, version: u32, entries: &[String]) -> String {
    format!(
        "{{ \"manifest\": {{ \"id\": \"{id}\", \"version\": {version}, \
         \"kind\": \"rules_definition\", \"digest\": \"DIGEST\", \"requires\": [] }}, \
         \"entries\": {{ {} }} }}",
        entries.join(",")
    )
}

/// A body whose declared digest is zeros: the refusal under test happens at parse, long
/// before the digest is consulted.
fn unsigned(body: &str) -> String {
    body.replace("DIGEST", &"0".repeat(64))
}

/// The **exact** [`PackError`] a body is refused with.
///
/// `load_pack` funnels every constructor refusal through `serde`'s `Error::custom` and
/// so answers `PackError::Parse`; going through the wire's own `TryFrom` keeps the
/// variant, which is what a bound's regression test needs to assert.
fn refusal(body: &str) -> PackError {
    let text = unsigned(body);
    assert!(
        load_pack(&text).is_err(),
        "the load path must refuse it too"
    );
    let wire: tacsim::pack::RulesPackWire =
        serde_json::from_str(&text).expect("the wire itself parses");
    RulesPack::try_from(wire).expect_err("the constructor must refuse it")
}

/// One commodity entry line.
fn commodity(id: &str) -> String {
    format!("\"{id}\": {{ \"kind\": \"commodity\", \"def\": {{ \"label\": \"C\" }} }}")
}

/// A one-location campaign-seed pack, signed.
fn seed_pack(id: &str) -> String {
    signed(&format!(
        "{{ \"manifest\": {{ \"id\": \"{id}\", \"version\": 1, \"kind\": \"campaign_seed\", \
         \"digest\": \"DIGEST\", \"requires\": [] }}, \"entries\": {{ \"yard\": {{ \
         \"kind\": \"location\", \"def\": {{ \"label\": \"Y\", \"role\": \"yard\" }} }} }} }}"
    ))
}

fn sources(bodies: Vec<String>) -> Vec<PackSource> {
    bodies
        .into_iter()
        .enumerate()
        .map(|(i, bytes)| PackSource {
            name: format!("<pack {i}>"),
            bytes,
        })
        .collect()
}

/// **P1 (covers S1) — a published version re-signed in place refuses the boot, naming
/// the file and both digests, and `@tacsim/pack verify` says MISMATCH rather than ok.**
///
/// The ledger row's digest was written and read by nothing: `boot_reconcile` skipped an
/// already-published version on a bare key test, so an operator who edited a pack in
/// place under the same `(id, version)` and re-signed it had it adopted at the next boot
/// with no refusal, no warning and no ledger correction — invariant (i)'s exact mistake,
/// and with a seeded world it moved a published corridor's transit times underneath a
/// hauling contract while the latch barred the only re-seed that could reconcile it.
/// `@tacsim/pack verify` could not say otherwise either: it compared the loaded pack
/// with its own manifest, which is the equality `load_pack` proved at boot, so it could
/// structurally only print `ok`.
#[test]
fn vf_p1_a_re_published_version_refuses_the_boot_and_verify_reports_the_mismatch() {
    let mut tw = boot();
    ok(&mut tw, GOD, "@tacsim/seed tacsim.corridor commit");
    ok(&mut tw, GOD, "@tacsim/pin metals=tacsim.core@1");
    let kv = fold_all(&mut tw);
    let published = snap(&tw)
        .pack(&cid("tacsim.core"), 1)
        .expect("loaded")
        .pack
        .manifest()
        .digest();

    // The SAME (id, version), edited in place and honestly re-signed.
    let edited = signed(
        &fixture::CORE_JSON
            .replacen(&published.to_hex(), "DIGEST", 1)
            .replacen("\"Metals\"", "\"Metals \"", 1),
    );
    let re_signed: RulesPack = serde_json::from_str(&edited).expect("the edit parses");
    let content = re_signed.manifest().digest();
    assert_ne!(content, published, "the edit really is other bytes");

    // Silence is only correct when the digests AGREE: the unedited file reboots on the
    // same ledger writing nothing at all.
    let mut same = boot();
    let _ = same.take_module_ops();
    reboot(&mut same, &kv);
    assert!(
        same.take_module_ops().is_empty(),
        "an unchanged pack re-writes no ledger row"
    );

    let mut edited_world = boot_with(TacsimConfig {
        packs: vec![PackSource {
            name: "<edited tacsim.core>".to_string(),
            bytes: edited,
        }],
        ..fixture::config()
    });
    let refused =
        reboot_result(&mut edited_world, &kv).expect_err("a re-published version must refuse");
    let refused = refused.to_string();
    for needle in [
        "<edited tacsim.core>",
        "tacsim.core@1",
        &published.to_hex(),
        &content.to_hex(),
    ] {
        assert!(refused.contains(needle), "{needle} missing from: {refused}");
    }

    // And from the other side of the same fact: the verb can now report it.
    let out = ok(&mut edited_world, GOD, "@tacsim/pack verify tacsim.core@1");
    assert!(out.contains("MISMATCH"), "{out}");
    assert!(
        out.contains(&published.to_hex()) && out.contains(&content.to_hex()),
        "{out}"
    );
}

/// **P2 (covers S2) — retiring one seeded pack refuses that pack's subjects and no
/// others, and names the pack that actually declares each.**
///
/// The retired-seed check looped over the whole `applied` index and admitted
/// `published_keys` as an independent trigger, which is a fact about the *pack* and not
/// about the *subject*: dropping one already-applied campaign-seed file from
/// `seed_paths` — an ordinary operator action, since the seed has run and the latch bars
/// a re-run — made `resolve` refuse for **every** subject in **every** still-loaded
/// rules pack, misattributing each to the unrelated seed pack. That is a total denial of
/// the store's headline query, it appeared only after a reboot, and it survived every
/// boot thereafter.
#[test]
fn vf_p2_a_retired_seed_refuses_only_the_subjects_it_declares() {
    let mut tw = boot();
    ok(&mut tw, GOD, "@tacsim/seed tacsim.corridor commit");
    let kv = fold_all(&mut tw);

    // The operator drops the applied seed file; `tacsim.core@1` still loads.
    let mut dropped = boot_with(TacsimConfig {
        seeds: Vec::new(),
        ..fixture::config()
    });
    reboot(&mut dropped, &kv);
    assert_eq!(
        snap(&dropped)
            .resolve(&cid("metals"))
            .expect("a commodity tacsim.core alone declares"),
        (cid("tacsim.core"), 1),
        "an unrelated pack's subject must not be poisoned by a retired seed"
    );
    assert_eq!(
        snap(&dropped)
            .resolve(&cid(fixture::RECIPE))
            .expect("resolves"),
        (cid("tacsim.core"), 1)
    );
    match snap(&dropped).resolve(&cid("ceres_extraction")) {
        Err(ResolveError::Retired {
            pack, version, how, ..
        }) => assert_eq!(
            (pack.as_str(), version, how),
            ("tacsim.corridor", 1, "seeded")
        ),
        other => panic!("the retired seed's OWN subject must fail closed: {other:?}"),
    }
    // And a subject nothing ever declared is still `Unknown`, not `Retired`.
    assert!(matches!(
        snap(&dropped).resolve(&cid("nothing_declares_this")),
        Err(ResolveError::Unknown { .. })
    ));
}

/// **P3 — the pack wire bounds magnitudes, not only counts, and the bounds evaluate.**
///
/// `PackEntry::check` bounded lengths and counts and no magnitude at all, so a recipe
/// consuming zero metals and producing `u64::MAX` structural alloys loaded, and so did a
/// route with a 136-year leg and free propellant. Those are the authored values cut 2's
/// inventory, cut 4's work orders and cut 6's journeys compute quantity, cost and
/// duration from, and cut 1's stated handoff ships the numbers now with the arithmetic
/// arriving later.
#[test]
fn vf_p3_magnitudes_are_bounded_on_the_wire_and_the_bounds_still_evaluate() {
    let recipe = |input: u64, output: u64| {
        rules_pack(
            "t.rec",
            1,
            &[
                commodity("metals"),
                commodity("structural_alloys"),
                format!(
                    "\"smelt\": {{ \"kind\": \"recipe\", \"def\": {{ \"label\": \"S\", \
                     \"inputs\": [{{ \"commodity\": \"metals\", \"units\": {input} }}], \
                     \"outputs\": [{{ \"commodity\": \"structural_alloys\", \
                     \"units\": {output} }}] }} }}"
                ),
            ],
        )
    };
    assert!(matches!(
        refusal(&recipe(0, 1)),
        PackError::ZeroRecipeUnits { side, .. } if side == "input"
    ));
    assert!(matches!(
        refusal(&recipe(4, 0)),
        PackError::ZeroRecipeUnits { side, .. } if side == "output"
    ));
    match refusal(&recipe(4, MAX_RECIPE_UNITS + 1)) {
        PackError::RecipeUnitsTooLarge {
            commodity,
            actual,
            max,
        } => assert_eq!(
            (commodity.as_str(), actual, max),
            ("structural_alloys", MAX_RECIPE_UNITS + 1, MAX_RECIPE_UNITS)
        ),
        other => panic!("a u64::MAX output must be refused on load: {other:?}"),
    }
    assert!(load_pack(&unsigned(&recipe(4, u64::MAX))).is_err());

    let route = |base_s: u64, propellant: u64| {
        format!(
            "{{ \"manifest\": {{ \"id\": \"t.r\", \"version\": 1, \"kind\": \"campaign_seed\", \
             \"digest\": \"DIGEST\", \"requires\": [] }}, \"entries\": {{ \
             \"a\": {{ \"kind\": \"location\", \"def\": {{ \"label\": \"A\", \"role\": \"y\" }} }}, \
             \"b\": {{ \"kind\": \"location\", \"def\": {{ \"label\": \"B\", \"role\": \"y\" }} }}, \
             \"r\": {{ \"kind\": \"route\", \"def\": {{ \"from\": \"a\", \"to\": \"b\", \
             \"legs\": [{{ \"to\": \"b\", \"base_s\": {base_s} }}], \"drive_class\": \"d\", \
             \"propellant\": {propellant} }} }} }} }}"
        )
    };
    match refusal(&route(u64::from(MAX_LEG_SECS) + 1, 0)) {
        PackError::LegTooLong { actual, max } => {
            assert_eq!((actual, max), (MAX_LEG_SECS + 1, MAX_LEG_SECS));
        }
        other => panic!("a 136-year leg must be refused on load: {other:?}"),
    }
    match refusal(&route(60, MAX_ROUTE_PROPELLANT + 1)) {
        PackError::PropellantTooLarge { actual, max } => {
            assert_eq!(
                (actual, max),
                (MAX_ROUTE_PROPELLANT + 1, MAX_ROUTE_PROPELLANT)
            );
        }
        other => panic!("an unbounded propellant plan must be refused: {other:?}"),
    }
    // Zero propellant is legal, and deliberately so: a ballistic transfer needs none.
    assert!(load_pack(&signed(&route(60, 0))).is_ok());

    // The bounds are chosen so the extremes still evaluate. A full recipe first.
    let lines: Vec<String> = (0..MAX_RECIPE_LINES)
        .map(|i| format!("{{ \"commodity\": \"c{i}\", \"units\": {MAX_RECIPE_UNITS} }}"))
        .collect();
    let mut entries: Vec<String> = (0..MAX_RECIPE_LINES)
        .map(|i| commodity(&format!("c{i}")))
        .collect();
    entries.push(format!(
        "\"big\": {{ \"kind\": \"recipe\", \"def\": {{ \"label\": \"B\", \"inputs\": [{}], \
         \"outputs\": [{}] }} }}",
        lines.join(","),
        lines.join(",")
    ));
    let big =
        load_pack(&signed(&rules_pack("t.max", 1, &entries))).expect("the extreme recipe loads");
    let Some(PackEntry::Recipe(def)) = big.entry(&cid("big")) else {
        panic!("the recipe is there")
    };
    let total = def
        .inputs
        .iter()
        .try_fold(0_i64, |acc, line| {
            i64::try_from(line.units)
                .ok()
                .and_then(|u| acc.checked_add(u))
        })
        .expect("a MAX_RECIPE_LINES x MAX_RECIPE_UNITS recipe cannot overflow i64");
    assert_eq!(
        total,
        i64::try_from(MAX_RECIPE_LINES).unwrap() * i64::try_from(MAX_RECIPE_UNITS).unwrap()
    );

    // Then a full route, materialized and evaluated through `route_duration`.
    let places: Vec<String> = (0..=mush_sim::MAX_LEGS)
        .map(|i| {
            format!(
                "\"p{i:03}\": {{ \"kind\": \"location\", \"def\": {{ \"label\": \"P\", \
                 \"role\": \"y\" }} }}"
            )
        })
        .collect();
    let legs: Vec<String> = (1..=mush_sim::MAX_LEGS)
        .map(|i| format!("{{ \"to\": \"p{i:03}\", \"base_s\": {MAX_LEG_SECS} }}"))
        .collect();
    let long = signed(&format!(
        "{{ \"manifest\": {{ \"id\": \"t.long\", \"version\": 1, \"kind\": \"campaign_seed\", \
         \"digest\": \"DIGEST\", \"requires\": [] }}, \"entries\": {{ {}, \
         \"far\": {{ \"kind\": \"route\", \"def\": {{ \"from\": \"p000\", \
         \"to\": \"p{:03}\", \"legs\": [{}], \"drive_class\": \"d\", \"propellant\": 0 }} }} }} }}",
        places.join(","),
        mush_sim::MAX_LEGS,
        legs.join(",")
    ));
    let mut tw = boot_with(TacsimConfig {
        enabled: true,
        packs: Vec::new(),
        seeds: sources(vec![long]),
        ..TacsimConfig::default()
    });
    ok(&mut tw, GOD, "@tacsim/seed t.long commit");
    let route = snap(&tw).route(&cid("far")).expect("materialized").clone();
    let drive = store::nominal_drive(route.drive_class()).expect("a nominal drive");
    let unladen = mush_sim::route_duration(&route, &drive, 0).expect("no overflow unladen");
    assert_eq!(
        unladen,
        i64::from(MAX_LEG_SECS) * i64::try_from(mush_sim::MAX_LEGS).unwrap()
    );
    let laden = mush_sim::route_duration(&route, &drive, drive.payload_capacity())
        .expect("no overflow at full payload");
    assert_eq!(laden, unladen * 2);
}

/// **P5 — the entry cap and the seed-object cap are separate, reachable refusals.**
///
/// `MAX_SEED_OBJECTS` was 2 048 against `MAX_PACK_ENTRIES` 512, so the entry cap always
/// fired first: `PackError::TooManySeedObjects`, `SeedError::TooManyObjects` and the
/// latch wire's own re-check were all dead, and the shipped test asserted loosely enough
/// that the wrong guard satisfied it.
#[test]
fn vf_p5_the_entry_cap_and_the_seed_object_cap_are_separate_refusals() {
    let manifest = |kind| {
        PackManifest::new(
            cid("t.cap"),
            1,
            kind,
            mush_core::DefinitionDigest::from_bytes([0_u8; 32]),
            Vec::new(),
        )
        .expect("a manifest")
    };
    let mut too_many_entries: BTreeMap<CanonicalId, PackEntry> = BTreeMap::new();
    for i in 0..=MAX_PACK_ENTRIES {
        too_many_entries.insert(
            cid(&format!("c{i:04}")),
            PackEntry::Commodity(tacsim::CommodityDef {
                label: "C".to_string(),
                in_use: false,
            }),
        );
    }
    match RulesPack::new(manifest(PackKind::RulesDefinition), too_many_entries) {
        Err(PackError::TooManyEntries { actual, max }) => {
            assert_eq!((actual, max), (MAX_PACK_ENTRIES + 1, MAX_PACK_ENTRIES));
        }
        other => panic!("the ENTRY cap is its own refusal: {other:?}"),
    }
    let mut too_many_objects: BTreeMap<CanonicalId, PackEntry> = BTreeMap::new();
    for i in 0..=MAX_SEED_OBJECTS {
        too_many_objects.insert(
            cid(&format!("p{i:04}")),
            PackEntry::Location(tacsim::LocationDef {
                label: "P".to_string(),
                role: cid("yard"),
                body: None,
                lat_mdeg: None,
                lon_mdeg: None,
            }),
        );
    }
    match RulesPack::new(manifest(PackKind::CampaignSeed), too_many_objects) {
        Err(PackError::TooManySeedObjects { actual, max }) => {
            assert_eq!((actual, max), (MAX_SEED_OBJECTS + 1, MAX_SEED_OBJECTS));
        }
        other => panic!("the SEED-OBJECT cap is its own refusal: {other:?}"),
    }
}

/// **P6 — a duplicate entry id refuses, and the two single-entry variants stay
/// distinct.**
///
/// `serde_json` resolves a repeated map key by last-write-wins with no error, unlike the
/// `deny_unknown_fields` struct fields around it, and the digest is taken over the
/// *parsed* map — so a self-consistent file whose authored bytes said one thing and
/// whose loaded semantics said another passed every gate, and its digest was identical
/// to the survivor-only file's. What runs is what is hashed, so this was a hole in the
/// audit story rather than in the gate; but a reviewer, a diff or a CI check reading
/// the file could not tell which entry the world actually got.
#[test]
fn vf_p6_a_duplicate_entry_id_refuses_and_the_two_variants_stay_distinct() {
    let body = |entries: &str| {
        format!(
            "{{ \"manifest\": {{ \"id\": \"t.dup\", \"version\": 1, \
             \"kind\": \"rules_definition\", \"digest\": \"{}\", \"requires\": [] }}, \
             \"entries\": {{ {entries} }} }}",
            "0".repeat(64)
        )
    };
    let metals = "\"metals\": { \"kind\": \"commodity\", \"def\": { \"label\": \"Metals\", \
                  \"in_use\": true } }";
    let slag = "\"metals\": { \"kind\": \"commodity\", \"def\": { \"label\": \"Slag\", \
                \"in_use\": false } }";
    let refused = load_pack(&body(&format!("{metals}, {slag}"))).expect_err("a repeated id");
    assert!(
        refused.to_string().contains("metals") && refused.to_string().contains("more than once"),
        "{refused}"
    );
    // Each half on its own is a legal pack, and the two are not the same pack.
    let first = serde_json::from_str::<RulesPack>(&body(metals)).expect("the first variant");
    let second = serde_json::from_str::<RulesPack>(&body(slag)).expect("the second variant");
    assert_ne!(
        first.computed_digest().unwrap(),
        second.computed_digest().unwrap(),
        "the collapse used to hide one of these behind the other's digest"
    );
}

/// **P7 — the corridor fixture is flyable carrying its own declared propellant.**
///
/// `tacsim_corridor.json` declares `propellant: 1200` while the one `DriveProfile` cut 1
/// ships had `payload_capacity: 1_000`, so evaluating the fixture's one route **with its
/// own propellant load** was `RouteDefError::Overloaded`. Every shipped test evaluated at
/// `payload = 0`, where capacity cancels, so nothing noticed that the two numbers in the
/// same fixture contradicted each other — and cut 6 inherits them as the corridor's
/// tuning baseline. Whether propellant counts against payload is cut 6's decision;
/// `nominal_drive`'s doc-comment now states the ruling it will read.
#[test]
fn vf_p7_the_corridor_is_flyable_carrying_its_own_declared_propellant() {
    let mut tw = boot();
    ok(&mut tw, GOD, "@tacsim/seed tacsim.corridor commit");
    let route = snap(&tw)
        .route(&cid(fixture::ROUTE))
        .expect("materialized")
        .clone();
    assert_eq!(route.propellant(), 1_200);
    let drive = store::nominal_drive(route.drive_class()).expect("a nominal drive");
    assert!(
        route.propellant() <= drive.payload_capacity(),
        "the fixture's own two numbers must be mutually flyable"
    );
    let laden = mush_sim::route_duration(&route, &drive, route.propellant())
        .expect("the corridor flies carrying its own propellant");
    let empty = mush_sim::route_duration(&route, &drive, 0).expect("and empty");
    assert!(laden > empty, "a laden hull is slower, not faster");
}

/// **P8 — a `requires` cycle refuses naming both packs, and a declared dependency
/// digest is compared.**
///
/// `PackRequirement` was a name-and-number edge with no digest, which is what let P1's
/// in-place edit propagate invisibly through a dependent pack whose own digest never
/// changed; and `PackManifest::new` refused only self-dependency and duplication, so a
/// mutual cycle loaded and `verify_sources` returned both packs. The cycle is inert in
/// cut 1 — nothing orders packs — but the acyclicity a later cut's load ordering needs
/// is now guaranteed at boot rather than assumed.
#[test]
fn vf_p8_a_requires_cycle_and_a_requires_digest_mismatch_both_refuse() {
    let mutual = |id: &str, other: &str| {
        signed(&format!(
            "{{ \"manifest\": {{ \"id\": \"{id}\", \"version\": 1, \
             \"kind\": \"rules_definition\", \"digest\": \"DIGEST\", \
             \"requires\": [{{ \"id\": \"{other}\", \"version\": 1 }}] }}, \
             \"entries\": {{ {} }} }}",
            commodity("metals")
        ))
    };
    let cfg = TacsimConfig {
        enabled: true,
        packs: sources(vec![mutual("t.a", "t.b"), mutual("t.b", "t.a")]),
        ..TacsimConfig::default()
    };
    let refused = verify_sources(&cfg).expect_err("a mutual requires cycle");
    assert!(refused.contains("cycle"), "{refused}");
    assert!(
        refused.contains("t.a@1") && refused.contains("t.b@1"),
        "the cycle must be named: {refused}"
    );

    // A dependency bound to a content, satisfied and unsatisfied.
    let core = signed(TWO_ENTRIES);
    let core_digest = load_pack(&core)
        .expect("loads")
        .manifest()
        .digest()
        .to_hex();
    let dependent = |digest: &str| {
        signed(&format!(
            "{{ \"manifest\": {{ \"id\": \"t.dep\", \"version\": 1, \
             \"kind\": \"rules_definition\", \"digest\": \"DIGEST\", \
             \"requires\": [{{ \"id\": \"t.core\", \"version\": 1, \"digest\": \"{digest}\" }}] }}, \
             \"entries\": {{ {} }} }}",
            commodity("ore")
        ))
    };
    let good = TacsimConfig {
        enabled: true,
        packs: sources(vec![core.clone(), dependent(&core_digest)]),
        ..TacsimConfig::default()
    };
    assert_eq!(
        verify_sources(&good)
            .expect("a matching digest loads")
            .len(),
        2
    );
    let bad = TacsimConfig {
        enabled: true,
        packs: sources(vec![core, dependent(&"b".repeat(64))]),
        ..TacsimConfig::default()
    };
    let refused = verify_sources(&bad).expect_err("a dependency at other content");
    assert!(
        refused.contains(&core_digest) && refused.contains(&"b".repeat(64)),
        "{refused}"
    );
    assert!(
        refused.contains("<pack 1>"),
        "the file must be named: {refused}"
    );
}

/// **S3 — a placed object destroyed since the seed is paged and its route is skipped,
/// never materialized over a tombstone.**
///
/// `reindex_routes` resolved endpoints purely against the latch's `placed` map and had
/// no `World` at all, so its only failure mode was a symbol *missing from the map*; boot
/// walked only the loaded packs and touched placement not at all. So the question
/// "an object the latch names is gone" answered **neither** page nor re-create: it was
/// silent, and `store.route(...)` went on handing out endpoints for objects `world.get()`
/// answers `None` for. Cut 6 plans journeys from exactly this projection.
#[test]
fn vf_s3_a_destroyed_placed_object_skips_its_route_and_pages_the_pack() {
    let mut tw = boot();
    ok(&mut tw, GOD, "@tacsim/seed tacsim.corridor commit");
    let phobos = snap(&tw)
        .seeded_objects()
        .find(|(id, _)| id.as_str() == "phobos_yard")
        .map(|(_, obj)| obj)
        .expect("the yard was placed");
    assert!(snap(&tw).route(&cid(fixture::ROUTE)).is_some());
    let kv = fold_all(&mut tw);

    tw.world_mut()
        .destroy(phobos)
        .expect("the yard is destroyed");
    let swept = tw.world_mut().sweep();
    assert!(swept.contains(&phobos));
    assert!(tw.world().get(phobos).is_none());
    reboot(&mut tw, &kv);

    assert!(
        snap(&tw).route(&cid(fixture::ROUTE)).is_none(),
        "a RouteDef over a tombstone is worse than no RouteDef"
    );
    assert_eq!(
        snap(&tw)
            .missing_placements()
            .map(|(p, v, n)| (p.to_string(), v, n))
            .collect::<Vec<_>>(),
        vec![("tacsim.corridor".to_string(), 1, 1)],
        "boot must have paged this pack, level 2, and only this pack"
    );
    // The two survivors are still addressable; only the dangling one is withheld.
    assert_eq!(snap(&tw).seeded_objects().count(), 2);
    assert!(
        snap(&tw).is_applied(&cid("tacsim.corridor"), 1),
        "and the latch still bars a re-seed"
    );
}

/// **S4 — two seeded packs declaring one route id are both addressable, and neither
/// shadows the other.**
///
/// `routes` was keyed by the route entry's **pack-local** id alone while `RulesPack::new`
/// enforces only that a route names places *its own* pack declares, and `reindex_routes`
/// finished with a last-insert-wins loop — so a second campaign-seed pack whose id merely
/// sorts later silently replaced the corridor's canonical `ceres_phobos`, reporting one
/// route where two were declared. Deterministic across reboots, so not a replay
/// divergence, but a fail-open shadowing of the very definition T1 rests on.
#[test]
fn vf_s4_two_packs_declaring_one_route_id_are_both_addressable() {
    let rival = signed(
        r#"{ "manifest": { "id": "zz.rival", "version": 1, "kind": "campaign_seed",
                           "digest": "DIGEST", "requires": [] },
             "entries": {
               "r_from": { "kind": "location", "def": { "label": "F", "role": "yard" } },
               "r_mid":  { "kind": "location", "def": { "label": "M", "role": "yard" } },
               "r_to":   { "kind": "location", "def": { "label": "T", "role": "yard" } },
               "ceres_phobos": { "kind": "route", "def": { "from": "r_from", "to": "r_to",
                   "legs": [{ "to": "r_mid", "base_s": 7 }, { "to": "r_to", "base_s": 9 }],
                   "drive_class": "fusion_torch", "propellant": 0 } } } }"#,
    );
    let mut cfg = fixture::config();
    cfg.seeds.push(PackSource {
        name: "<zz.rival>".to_string(),
        bytes: rival,
    });
    let mut tw = boot_with(cfg);
    ok(&mut tw, GOD, "@tacsim/seed tacsim.corridor commit");
    ok(&mut tw, GOD, "@tacsim/seed zz.rival commit");
    let kv = fold_all(&mut tw);
    reboot(&mut tw, &kv);

    let store = snap(&tw);
    assert_eq!(
        store.routes().count(),
        2,
        "two declared routes, two projections"
    );
    let corridor = store
        .route_in(&cid("tacsim.corridor"), 1, &cid(fixture::ROUTE))
        .expect("the corridor's own route");
    assert_eq!(
        corridor
            .legs()
            .iter()
            .map(mush_sim::LegDef::base_s)
            .collect::<Vec<_>>(),
        vec![86_400, 172_800],
        "the later-sorting pack must not have replaced it"
    );
    let rival = store
        .route_in(&cid("zz.rival"), 1, &cid(fixture::ROUTE))
        .expect("the rival's route is addressable too");
    assert_eq!(
        rival
            .legs()
            .iter()
            .map(mush_sim::LegDef::base_s)
            .collect::<Vec<_>>(),
        vec![7, 9]
    );
    // Asking by id alone goes through `resolve`, which is exact and per-subject.
    assert_eq!(
        store
            .route(&cid(fixture::ROUTE))
            .map(|r| r.legs()[0].base_s()),
        Some(86_400)
    );
}

/// **S7 — `commit` gates its confirmation on the durable ack; `dry` does not.**
///
/// The module's one irreversible command carried no `gate_on_durable_ack`, alone among
/// the tree's irreversible staff commands (`mush-housing`, `mush-jobs`, `mush-mail`,
/// `mush-economy` and `ruleset-wod`'s economy all do). The consequence was only that a
/// dropped durable write gave the staffer the generic diagnostic rather than the
/// command-specific warning — the objects and the latch are lost together, so re-running
/// is correct — but consistency with the tree's pattern is the point.
#[test]
fn vf_s7_commit_gates_on_the_durable_ack_and_dry_does_not() {
    let mut tw = boot();
    let _ = tw.engine_mut().take_durable_gate();
    ok(&mut tw, GOD, "@tacsim/seed tacsim.corridor dry");
    assert_eq!(
        tw.engine_mut().take_durable_gate(),
        None,
        "`dry` writes nothing, so it gates nothing"
    );
    ok(&mut tw, GOD, "@tacsim/seed tacsim.corridor commit");
    assert_eq!(
        tw.engine_mut().take_durable_gate(),
        Some((GOD, false)),
        "the one irreversible command waits on its receipt"
    );
}

/// **B1 — `@tacsim/pack verify` is staff-only and paged.**
///
/// It was gated by `require_enabled` alone and looped every loaded pack with no page
/// bound at all, calling `computed_digest()` — a canonical `to_value` over the whole
/// entry table, then a canonical JSON encode, then SHA-256 — for each. At
/// `MAX_CONFIGURED_PACKS` × `MAX_PACK_BYTES` that is 32 MiB of hashing per invocation,
/// synchronously on the world's single writer task, from **any player**, at command rate
/// with no throttle, emitting one `Outbox` message per pack with no cap. `pack_list`
/// beside it was already capped at `PACK_PAGE`.
#[test]
fn vf_b1_verify_is_staff_only_and_paged() {
    let packs: Vec<String> = (0..MAX_CONFIGURED_PACKS)
        .map(|i| {
            signed(&rules_pack(
                &format!("t.p{i:03}"),
                1,
                &[commodity("metals")],
            ))
        })
        .collect();
    let mut tw = boot_with(TacsimConfig {
        enabled: true,
        packs: sources(packs),
        seeds: Vec::new(),
        ..TacsimConfig::default()
    });
    assert_eq!(snap(&tw).packs().count(), MAX_CONFIGURED_PACKS);
    let ordinary = tw.spawn_player("Rigger");

    let refused = tw.run(ordinary, "@tacsim/pack verify");
    assert_eq!(refused.len(), 1, "a non-staff invocation hashes nothing");
    assert!(has_error(&refused));
    assert!(texts(&refused).contains("Permission denied."));

    let paged = tw.run(GOD, "@tacsim/pack verify");
    assert!(
        paged.len() <= PACK_PAGE + 1,
        "the no-argument form pages: {} rows",
        paged.len()
    );
    assert!(
        texts(&paged).contains(&format!("more than {PACK_PAGE} packs are loaded")),
        "and says so: {}",
        texts(&paged)
    );
    // Naming one is still exact, and still one row.
    let one = ok(&mut tw, GOD, "@tacsim/pack verify t.p063@1");
    assert_eq!(one.lines().count(), 1);
    assert!(one.contains("t.p063@1 ok"), "{one}");
}

/// **B2 — `@tacsim/pack list` is not a pack-existence oracle.**
///
/// The page marker was pushed on records **visited**, ahead of `may_view`, so a server
/// loading 33 campaign-seed packs and no rules pack answered a non-staff viewer *"more
/// than 32 packs are loaded"* where an empty server answered *"No packs are loaded."*;
/// and because the visit budget is a fixed `PACK_PAGE`, a viewer who got the marker
/// could subtract the rendered rows from it and learn exactly how many packs inside that
/// window were hidden.
#[test]
fn vf_b2_pack_list_is_not_a_pack_existence_oracle() {
    // (a) A window entirely of packs this viewer may not see reads as an empty server.
    let mut hidden = boot_with(TacsimConfig {
        enabled: true,
        packs: Vec::new(),
        seeds: sources((0..40).map(|i| seed_pack(&format!("s.p{i:03}"))).collect()),
        ..TacsimConfig::default()
    });
    let mut empty = boot_with(TacsimConfig {
        enabled: true,
        ..TacsimConfig::default()
    });
    let viewer = hidden.spawn_player("Rigger");
    let other = empty.spawn_player("Rigger");
    let hidden_text = texts(&hidden.run(viewer, "@tacsim/pack list"));
    let empty_text = texts(&empty.run(other, "@tacsim/pack list"));
    assert_eq!(
        hidden_text, empty_text,
        "40 staff-only packs must be indistinguishable from none"
    );
    assert!(hidden_text.contains("No packs are loaded."));

    // (b) One hidden pack sorting first inside the window renders exactly what a server
    //     carrying only the visible ones renders — the row count leaks nothing.
    let visible: Vec<String> = (0..PACK_PAGE)
        .map(|i| {
            signed(&rules_pack(
                &format!("t.core{i:03}"),
                1,
                &[commodity("metals")],
            ))
        })
        .collect();
    let mut with_hidden = boot_with(TacsimConfig {
        enabled: true,
        packs: sources(visible.clone()),
        seeds: sources(vec![seed_pack("a.seed000")]),
        ..TacsimConfig::default()
    });
    let mut without = visible;
    without.pop(); // the last rules pack falls outside the visit window either way
    let mut no_hidden = boot_with(TacsimConfig {
        enabled: true,
        packs: sources(without),
        seeds: Vec::new(),
        ..TacsimConfig::default()
    });
    let a = with_hidden.spawn_player("Rigger");
    let b = no_hidden.spawn_player("Rigger");
    let with_text = texts(&with_hidden.run(a, "@tacsim/pack list"));
    let without_text = texts(&no_hidden.run(b, "@tacsim/pack list"));
    assert_eq!(
        with_text, without_text,
        "the rendered rows must not say how many packs were hidden"
    );
    assert!(
        !with_text.contains("more than"),
        "and no marker betrays the exhausted visit budget: {with_text}"
    );
}

/// **B3 — `@tacsim/pack show` pages its entries and names what it did not render.**
///
/// The entry loop was unbounded: three header rows plus one message per entry, so one
/// `@tacsim/pack show <public rules pack>` from any player emitted 515 output events at
/// `MAX_PACK_ENTRIES`. `mush-core`'s `Outbox` carries no per-command message cap to fall
/// back on, and every other listing surface on this tree pages.
#[test]
fn vf_b3_pack_show_pages_its_entries_and_names_the_remainder() {
    let entries: Vec<String> = (0..MAX_PACK_ENTRIES)
        .map(|i| commodity(&format!("c{i:03}")))
        .collect();
    let mut tw = boot_with(TacsimConfig {
        enabled: true,
        packs: sources(vec![signed(&rules_pack("t.big", 1, &entries))]),
        seeds: Vec::new(),
        ..TacsimConfig::default()
    });
    let ordinary = tw.spawn_player("Rigger");
    let page = tw.run(ordinary, "@tacsim/pack show t.big");
    assert!(
        page.len() <= PACK_PAGE + 4,
        "three header rows, one page of entries and one trailing row: {} events",
        page.len()
    );
    let text = texts(&page);
    assert!(text.contains("  c000 (commodity)"), "{text}");
    assert!(text.contains("  c031 (commodity)"), "{text}");
    assert!(!text.contains("  c032 (commodity)"), "{text}");
    assert!(
        text.contains(&format!(
            "({} more entries; @tacsim/pack show t.big@1 c032)",
            MAX_PACK_ENTRIES - PACK_PAGE
        )),
        "the trailing row must name the remainder and how to continue: {text}"
    );
    // And the cursor it prints continues the listing.
    let next = tw.run(ordinary, "@tacsim/pack show t.big@1 c032");
    let next_text = texts(&next);
    assert!(next.len() <= PACK_PAGE + 4);
    assert!(next_text.contains("  c032 (commodity)"), "{next_text}");
    assert!(!next_text.contains("  c031 (commodity)"), "{next_text}");
}

/// **B4 (covers P4) — a pin needs a declared subject, the family is capped, and it can
/// be read and drained.**
///
/// `pin_cmd` validated only that the *target* version was loaded; the **subject** was
/// any `CanonicalId` and was checked against nothing, while `resolve` gives a pin
/// absolute precedence ahead of the `candidates` scan — so a typo'd subject became a
/// durable pin answering `Ok` for a definition no pack declares, contradicting the
/// store's own "a pin wins and is exact" and handing cut 2 onward a resolvable id with
/// nothing behind it. And each accepted pin inserted into an **unbounded** map with no
/// cap, no `ModuleWrite::Del` anywhere in the crate and no verb to see it, so the family
/// grew monotonically, was reloaded in full at every boot, and was invisible from the
/// game.
#[test]
fn vf_b4_a_pin_needs_a_declared_subject_and_the_family_is_capped_and_drainable() {
    let mut tw = boot();
    let refused = err(&mut tw, GOD, "@tacsim/pin no_such_subject=tacsim.core@1");
    assert!(refused.contains("No such pack."), "{refused}");
    assert!(snap(&tw).pin(&cid("no_such_subject")).is_none());
    assert!(matches!(
        snap(&tw).resolve(&cid("no_such_subject")),
        Err(ResolveError::Unknown { .. })
    ));

    // Read and drain: the family has a verb and a `Del` path.
    let empty = ok(&mut tw, GOD, "@tacsim/pin list");
    assert!(empty.contains("No subjects are pinned."), "{empty}");
    ok(&mut tw, GOD, "@tacsim/pin metals=tacsim.core@1");
    let listed = ok(&mut tw, GOD, "@tacsim/pin list");
    assert!(listed.contains("metals -> tacsim.core@1"), "{listed}");
    let _ = tw.take_module_ops();
    let cleared = ok(&mut tw, GOD, "@tacsim/pin metals=");
    assert!(cleared.contains("no longer pinned"), "{cleared}");
    assert!(snap(&tw).pin(&cid("metals")).is_none());
    assert!(
        tw.take_module_ops()
            .iter()
            .any(|op| matches!(op.op, ModuleWrite::Del { .. })),
        "unpinning deletes the document rather than leaving a tombstone row"
    );
    err(&mut tw, GOD, "@tacsim/pin metals=");

    // And the cap refuses past `MAX_PINS`, writing nothing.
    let entries: Vec<String> = (0..=MAX_PINS)
        .map(|i| commodity(&format!("s{i:03}")))
        .collect();
    let mut capped = boot_with(TacsimConfig {
        enabled: true,
        packs: sources(vec![signed(&rules_pack("t.many", 1, &entries))]),
        seeds: Vec::new(),
        ..TacsimConfig::default()
    });
    for i in 0..MAX_PINS {
        ok(&mut capped, GOD, &format!("@tacsim/pin s{i:03}=t.many@1"));
    }
    let _ = capped.take_module_ops();
    let refused = err(
        &mut capped,
        GOD,
        &format!("@tacsim/pin s{MAX_PINS:03}=t.many@1"),
    );
    assert!(
        refused.contains(&MAX_PINS.to_string()),
        "the cap must be named: {refused}"
    );
    assert!(
        capped.take_module_ops().is_empty(),
        "a refused pin writes no module op"
    );
    assert_eq!(snap(&capped).pins().count(), MAX_PINS);
}

/// **B8 — a route in a rules pack is refused by the kind rule, not by the endpoint
/// symptom.**
///
/// `RulesPack::new`'s `# Errors` block said *"a location in a rules pack or a route in a
/// seed pack"*, which is exactly backwards against its own `misplaced` match and against
/// the shipped fixture — a route inside a `campaign_seed` manifest is what
/// `tacsim_corridor.json` is. The real constraint is the other way round: a route's
/// endpoints must be places its own pack declares and a rules pack may declare none, so
/// a route there was unauthorable and surfaced as `UnknownRouteEndpoint` rather than the
/// message that names the rule.
#[test]
fn vf_b8_a_route_in_a_rules_pack_is_the_kind_rule_not_the_endpoint_symptom() {
    let text = format!(
        "{{ \"manifest\": {{ \"id\": \"t.rules\", \"version\": 1, \
         \"kind\": \"rules_definition\", \"digest\": \"{}\", \"requires\": [] }}, \
         \"entries\": {{ \"r\": {{ \"kind\": \"route\", \"def\": {{ \"from\": \"a\", \
         \"to\": \"b\", \"legs\": [{{ \"to\": \"b\", \"base_s\": 5 }}], \
         \"drive_class\": \"d\", \"propellant\": 0 }} }} }} }}",
        "0".repeat(64)
    );
    let refused = load_pack(&text).expect_err("a route in a rules pack");
    let refused = refused.to_string();
    assert!(
        refused.contains("is a route, which a rules pack may not declare"),
        "the rule, not the symptom: {refused}"
    );
    assert!(
        !refused.contains("names the place"),
        "and not the endpoint symptom: {refused}"
    );
    // The other half of the rule is the shipped fixture: a seed pack's route loads.
    let corridor = load_pack(fixture::CORRIDOR_JSON).expect("the corridor still loads");
    assert!(matches!(
        corridor.entry(&cid(fixture::ROUTE)),
        Some(PackEntry::Route(_))
    ));
}
