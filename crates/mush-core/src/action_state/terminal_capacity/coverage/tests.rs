//! Core relation/capacity tests use inert typed rows. They grant no native
//! payment, source or host authority; the registered owner integration tests
//! exercise those independent images and the production producer.
use super::*;
use crate::{DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder};
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
fn op(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
fn dg(n: u8) -> DefinitionDigest {
    DefinitionDigest::from_bytes([n; 32])
}
fn reference(o: &str, k: &str, id: &str, payload: Value) -> TestResult<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(
        owner(o)?,
        DefinitionDescriptor::new(DefinitionKind::new(k)?, DefinitionId::new(id)?, 1, payload)?,
    )?)
}
struct Fixture {
    registry: StateFamilyRegistry,
    records: BTreeMap<StateKey, StoredStateRecord>,
    raw: Value,
    quote: Value,
    publication: Value,
    retention: Value,
}
impl Fixture {
    fn view(&self) -> View<'_> {
        View {
            registry: &self.registry,
            records: &self.records,
            mutations: None,
        }
    }
    fn put(&mut self, f: &str, id: String, v: Value) -> TestResult {
        let k = key(f, id)?.resolve(&self.registry)?;
        let d = self
            .registry
            .entries
            .get(&(k.owner.clone(), k.family.clone()))
            .ok_or("descriptor")?
            .descriptor();
        let envelope = json!({"family":{"owner":k.owner,"id":k.family},"key":k.encoded,"schema":{"id":d.value_schema(),"version":1},"value":v});
        self.records.insert(
            k,
            StoredStateRecord {
                value: Box::new(v),
                canonical: CanonicalStateBytes(
                    canonical_json_bytes_with_limit(&envelope, 262144)?.into(),
                ),
            },
        );
        Ok(())
    }
    fn sync(&mut self) -> TestResult {
        let r = Row::parse(self.raw.clone(), op(3))?;
        self.put(POLICY, op(3).to_hex(), self.raw.clone())?;
        self.put(EXPOSURE, op(3).to_hex(), r.exposure()?)?;
        self.put(HULL, format!("{:08x}", r.hull.0), r.head()?)?;
        self.put(RETENTION, op(3).to_hex(), self.retention.clone())?;
        self.put(QUOTE, op(2).to_hex(), self.quote.clone())?;
        self.put(PRODUCT, op(1).to_hex(), self.publication.clone())?;
        self.put(
            COUNTS,
            "registry".into(),
            json!({"format":1,"products":1,"quotes":1,"policies":1}),
        )?;
        Ok(())
    }
    fn reservation(&self) -> TestResult<Reservation> {
        let r = Row::parse(self.raw.clone(), op(3))?;
        Ok(Reservation {
            format: 1,
            funding: op(3),
            effect: r.action.clone(),
            sponsor: r.key(POLICY)?,
            targets: targets(&r)?,
            protocol_fingerprint: Some(r.fingerprint(&self.view())?),
        })
    }
}
fn fixture() -> TestResult<Fixture> {
    let manifest = reference(
        "tacsim",
        "registry_manifest",
        "combat",
        json!({"fixture":1}),
    )?;
    let economy = reference(
        "economy",
        "owner_participant",
        "initial_filing",
        json!({"fixture":1}),
    )?;
    let roll = reference(
        "roll",
        "owner_participant",
        "commercial_roots",
        json!({"format":1,"protocol":"commercial_retention_v1"}),
    )?;
    let protocol = json!({"format":1,"manifest":manifest,"participants":[economy,roll]});
    let p = protocol::Captured::parse(&protocol)?;
    let actions = [p.action(0)?, p.action(1)?, p.action(2)?];
    let mut builder = StateFamilyRegistryBuilder::new();
    for (o, f, ceiling) in FAMILIES.iter().chain(&PREREQUISITES) {
        builder.register(
            owner(o)?,
            protocol::descriptor(o, f, *ceiling)?,
            |k: &String| Ok(k.clone()),
            |v: &Value| Ok(v.clone()),
            |_: &Value| Ok(()),
            |v: &Value| Ok(v.clone()),
        )?;
    }
    builder.delegate(owner("roll")?, owner("tacsim")?)?;
    builder.hull_coverage_capacity(
        &owner("tacsim")?,
        manifest,
        [economy, roll],
        actions.clone(),
    )?;
    let owners = [owner("tacsim")?, owner("economy")?, owner("roll")?]
        .into_iter()
        .collect();
    let readiness = crate::mechanics::MechanicalOwnerReadiness::for_owners(&owners);
    let registry = builder.freeze_for_owners_with_readiness(&owners, readiness)?;
    let schema = reference("tacsim", "unit", "fixture", json!({}))?;
    let zone = reference("tacsim", "zone", "fixture", json!({}))?;
    let publication = json!({"format":1,"operation":op(1),"action":actions[0],"protocol":protocol,"publisher":7,"authorization":{},"product":{},"posted_at_utc":1});
    let risk = json!({"ic_open":op(4),"seal_digest":dg(1),"admission_digest":dg(2),"departure_digest":dg(3),"hull":99,"hull_title_revision":1,"hull_revision":0,"hull_schema":schema,"zone":zone,"qualified":null});
    let qt = json!({"format":1,"buyer":7,"insurer":0,"scope":0,"treasury":0,"payer":1,"instrument":"credits","adjudicator":8,"insured_value":100000,"base_premium":10000,"discount":0,"premium":10000,"liability":75000,"duration_seconds":3600,"claim_delay_seconds":60,"quoted_at_utc":1,"expires_at_utc":100});
    let quote = json!({"format":1,"request":dg(7),"operation":op(2),"action":actions[0],"protocol":protocol,"publication":op(1),"publication_digest":digest("tacsim.coverage.publication.v1",&publication,98304)?,"product_digest":dg(9),"risk":risk,"terms":qt});
    let mut basis = wire::without(&risk, &["qualified"])?;
    let o = basis.as_object_mut().ok_or("basis")?;
    for (n, v) in [
        ("format", json!(1)),
        ("product", json!(op(1))),
        ("product_digest", quote["publication_digest"].clone()),
        ("quote", json!(op(2))),
        (
            "quote_digest",
            json!(digest("tacsim.coverage.quote.v1", &quote, 16384)?),
        ),
        ("qualified_role", Value::Null),
    ] {
        o.insert(n.into(), v);
    }
    let stamp = |at| {
        json!(ClockStamp {
            clock: crate::ClockRef::WorldIc { generation: 1 },
            at
        })
    };
    let mut terms = wire::without(&qt, &["duration_seconds"])?;
    terms
        .as_object_mut()
        .ok_or("terms")?
        .insert("starts_at".into(), stamp(50));
    terms
        .as_object_mut()
        .ok_or("terms")?
        .insert("ends_at".into(), stamp(3650));
    let consent = json!({"format":1,"operation":op(3),"effect":actions[0],"accepted_at_utc":2,"basis":basis,"terms":terms});
    let binding = json!({"transaction":0,"journal_seq":1,"plan_hash":vec![1u8;32],"hold":{"id":0},"title":{"fixture":true}});
    let history = json!({"intent":{"consent":consent,"contract":0,"hold":0,"title":{"fixture":true}},"binding":binding,"terminal":null});
    let raw = json!({"format":1,"request":dg(10),"operation":op(3),"action":actions[0],"protocol":protocol,"sequence":1,"previous":null,"quote":op(2),"quote_digest":basis["quote_digest"],"history":history});
    let retention = json!({"format":1,"consent":consent,"revision":{},"rating":{},"claim_digest":dg(8),"contract":0,"hold":0,"binding_digest":digest("roll.coverage.native_binding.v1",&binding,65536)?,"history_digest":digest("economy.coverage.history.v1",&history,65536)?,"terminal":null});
    let mut f = Fixture {
        registry,
        records: BTreeMap::new(),
        raw,
        quote,
        publication,
        retention,
    };
    f.sync()?;
    Ok(f)
}
#[test]
fn fixed_targets_use_known_policy_keys_and_exact_complete_budget() -> TestResult {
    let f = fixture()?;
    let r = f.reservation()?;
    verify(Phase::Funding, &r, &f.view())?;
    assert_eq!(r.targets.len(), 6);
    assert_eq!(r.targets.iter().map(|t| t.ceiling).sum::<usize>(), 131072);
    for t in r
        .targets
        .iter()
        .filter(|t| matches!(t.mode, SlotMode::Create))
    {
        assert_eq!(t.key.encoded, op(3).to_hex());
    }
    Ok(())
}
#[test]
fn cold_missing_capacity_or_any_counterpart_refuses() -> TestResult {
    let mut f = fixture()?;
    let saved = Arc::new(encode(f.reservation()?)?);
    let reservations = BTreeMap::from([(saved.id(), saved)]);
    audit_boot(&f.view(), &reservations)?;
    assert!(audit_boot(&f.view(), &BTreeMap::new()).is_err());
    for family in [POLICY, EXPOSURE, HULL, RETENTION, QUOTE, PRODUCT, COUNTS] {
        let k = f
            .records
            .keys()
            .find(|k| k.family.as_str() == family)
            .ok_or("key")?
            .clone();
        let value = f.records.remove(&k).ok_or("record")?;
        assert!(
            audit_boot(&f.view(), &reservations).is_err(),
            "missing counterpart"
        );
        f.records.insert(k, value);
    }
    Ok(())
}
#[test]
fn quote_product_digest_cannot_replace_full_publication_identity() -> TestResult {
    let mut f = fixture()?;
    f.raw["history"]["intent"]["consent"]["basis"]["product_digest"] =
        f.quote["product_digest"].clone();
    let r = Row::parse(f.raw.clone(), op(3))?;
    assert!(r.sources(&f.view()).is_err());
    Ok(())
}
#[test]
fn required_null_fields_and_closed_shapes_cannot_be_omitted() -> TestResult {
    let f = fixture()?;
    for name in ["previous", "history"] {
        let mut v = f.raw.clone();
        v.as_object_mut().ok_or("raw")?.remove(name);
        assert!(Row::parse(v, op(3)).is_err());
    }
    let mut v = f.raw.clone();
    v["history"]
        .as_object_mut()
        .ok_or("history")?
        .remove("terminal");
    assert!(Row::parse(v, op(3)).is_err());
    let mut v = f.raw.clone();
    v.as_object_mut()
        .ok_or("raw")?
        .insert("authority".into(), json!(true));
    assert!(Row::parse(v, op(3)).is_err());
    Ok(())
}
#[test]
fn complete_future_retention_bound_refuses_before_body_limit() -> TestResult {
    let mut f = fixture()?;
    let r = Row::parse(f.raw.clone(), op(3))?;
    r.future_room(&f.view())?;
    f.retention["revision"] = json!("x".repeat(11500));
    assert!(canonical_json_bytes_with_limit(&f.retention, 16384).is_ok());
    f.put(RETENTION, op(3).to_hex(), f.retention.clone())?;
    assert!(r.future_room(&f.view()).is_err());
    Ok(())
}
#[test]
fn forged_exposure_and_orphan_terminal_fail_cold_inverse() -> TestResult {
    let mut f = fixture()?;
    let saved = Arc::new(encode(f.reservation()?)?);
    let reservations = BTreeMap::from([(saved.id(), saved)]);
    let r = Row::parse(f.raw.clone(), op(3))?;
    let mut exposure = r.exposure()?;
    exposure["liability"] = json!(1);
    f.put(EXPOSURE, op(3).to_hex(), exposure)?;
    assert!(audit_boot(&f.view(), &reservations).is_err());
    f.sync()?;
    f.put(RECEIPT, op(9).to_hex(), json!({"format":1}))?;
    assert!(audit_boot(&f.view(), &reservations).is_err());
    Ok(())
}
#[test]
fn exact_images_are_required_and_registration_cannot_swap_actions() -> TestResult {
    let f = fixture()?;
    let p = f.registry.hull_coverage.as_ref().ok_or("protocol")?;
    let id = RegisteredTxnIdentity {
        action: p.admission_action().clone(),
        ordinal: 0,
        commit: op(3),
        actor: ObjId(7),
        principal: PrincipalId::Object { id: ObjId(7) },
    };
    assert!(
        p.required_images(&id)
            .ok_or("requirement")?
            .check(&crate::participant::OwnerImages::default())
            .is_err()
    );
    let captured = field(&f.raw, "protocol")?;
    let manifest = parsed(field(captured, "manifest")?)?;
    let participants = parsed(field(captured, "participants")?)?;
    let cap = protocol::Captured::parse(captured)?;
    assert!(
        Protocol::new(
            &owner("tacsim")?,
            manifest,
            participants,
            [cap.action(1)?, cap.action(0)?, cap.action(2)?]
        )
        .is_err()
    );
    Ok(())
}
#[test]
fn native_zero_account_contract_and_tactics_revision_domains_remain_valid() -> TestResult {
    let f = fixture()?;
    let r = Row::parse(f.raw.clone(), op(3))?;
    r.paired(&f.view())?;
    assert_eq!(r.terms()?["treasury"], json!(0));
    assert_eq!(r.intent()?["contract"], json!(0));
    assert_eq!(r.basis()?["hull_revision"], json!(0));
    Ok(())
}

impl Fixture {
    fn close(
        &mut self,
        cause: &str,
        generation: u64,
        at: i64,
        close_generation: u64,
        closed: i64,
        destroyed: bool,
    ) -> TestResult {
        let stamp = |generation, at| {
            json!(ClockStamp {
                clock: crate::ClockRef::WorldIc { generation },
                at
            })
        };
        let p = protocol::Captured::parse(&self.raw["protocol"])?;
        let action = p.action(if cause == "destroyed" { 1 } else { 2 })?;
        let zone = self.raw["history"]["intent"]["consent"]["basis"]["zone"].clone();
        let reason = if cause == "destroyed" {
            json!({"kind":cause,"loss":op(5),"loss_digest":dg(11),"occurred_at":stamp(close_generation,closed),"zone":zone,"transition":2})
        } else {
            json!({"kind":cause,"close":op(5),"close_digest":dg(11),"transition":2})
        };
        let decision = json!({"format":1,"operation":op(6),"action":action,"actor":if cause=="cancelled"{7}else{8},"at_ic":stamp(generation,at),"posted_at_utc":3,"reason":reason});
        let native = json!({"decision":decision,"before_title":self.raw["history"]["binding"]["title"],"transaction":1,"journal_seq":2,"plan_hash":vec![2u8;32],"hold":{"id":0},"title":{"fixture":true}});
        self.raw["history"]["terminal"] = native.clone();
        self.retention["terminal"] = decision;
        self.retention["history_digest"] = json!(digest(
            "economy.coverage.history.v1",
            &self.raw["history"],
            65536
        )?);
        self.sync()?;
        let r = Row::parse(self.raw.clone(), op(3))?;
        let terminal = json!({"format":1,"request":dg(12),"operation":op(6),"action":action,"protocol":self.raw["protocol"],"policy":op(3),"policy_digest":r.policy_digest()?,"native_history":r.history_digest()?,"close":{"open":op(4),"close":op(5),"digest":dg(11),"transition":2,"at_ic":stamp(close_generation,closed),"zone":zone,"kind":if destroyed{json!({"kind":"destroyed"})}else{json!({"kind":"returned_to_base","base":42})}}});
        self.put(TERMINAL, op(3).to_hex(), terminal)?;
        self.put(RECEIPT,op(3).to_hex(),json!({"format":1,"operation":op(6),"policy":op(3),"history":r.history_digest()?,"transaction":1,"journal_seq":2,"state":r.state()?}))?;
        Ok(())
    }
}
#[test]
fn delayed_covered_loss_uses_original_window_and_releases_exact_reservation() -> TestResult {
    let mut f = fixture()?;
    let reservation = f.reservation()?;
    f.close("destroyed", 1, 4000, 1, 100, true)?;
    verify(Phase::Terminal, &reservation, &f.view())?;
    audit_boot(&f.view(), &BTreeMap::new())?;
    let saved = Arc::new(encode(reservation)?);
    assert!(audit_boot(&f.view(), &BTreeMap::from([(saved.id(), saved)])).is_err());
    let mut invalid = fixture()?;
    invalid.close("destroyed", 1, 4000, 1, 3650, true)?;
    assert!(
        Row::parse(invalid.raw.clone(), op(3))?
            .paired(&invalid.view())
            .is_err()
    );
    Ok(())
}
#[test]
fn cross_clock_clean_release_never_erases_an_original_covered_loss() -> TestResult {
    let mut f = fixture()?;
    let reservation = f.reservation()?;
    f.close("cancelled", 2, 10, 2, 9, false)?;
    verify(Phase::Terminal, &reservation, &f.view())?;
    let mut invalid = fixture()?;
    invalid.close("clean_close", 2, 10, 1, 100, true)?;
    assert!(
        Row::parse(invalid.raw.clone(), op(3))?
            .paired(&invalid.view())
            .is_err()
    );
    let mut expiry = fixture()?;
    expiry.close("expired_clean", 2, 4000, 2, 3999, false)?;
    assert!(
        Row::parse(expiry.raw.clone(), op(3))?
            .paired(&expiry.view())
            .is_err()
    );
    Ok(())
}
