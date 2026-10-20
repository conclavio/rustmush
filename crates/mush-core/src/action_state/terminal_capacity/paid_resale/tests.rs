#![cfg(test)]

//! Actual coordinator installation/recovery, with inert business codecs.
//! Native journal values are structural fixtures, never Economy authority.
use super::*;

const OFFER: &str = "market_mind_paid_resale_offers";
const ROLL: &str = "roll_mind_paid_resales";
const MARKET: &str = "market_mind_paid_resales";
const HEAD: &str = "roll_mind_paid_resale_heads";
const HOLDER: &str = "roll_mind_paid_holders";
const TERMINAL: &str = "market_mind_paid_resale_receipts";
const SANCTION: &str = "roll_registry_term_revocations";
const STATUS: &str = "roll_registry_revoked_terms";
const LK: &str = "licence/0000000000000001";
const TK: &str = "licence/0000000000000001/term/0000000000000001";

fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
fn clock(at: i64) -> Value {
    json!({"clock":{"kind":"world_ic","generation":1},"at":at})
}
fn hash(domain: &str, v: &Value) -> Value {
    json!(digest(domain, v).expect("bounded fixture"))
}
fn action_payloads() -> [(&'static str, Value); 5] {
    [
        (
            "offer_paid_mind_resale",
            json!({"format":1,"protocol":"personal_paid_resale_v1","authority":"current_personal_paid_holder","named_buyer":true,"source_free":true,"existing_term":"unchanged","no_automatic_acceptance":true}),
        ),
        (
            "accept_paid_mind_resale",
            json!({"format":1,"protocol":"personal_paid_resale_v1","authority":"named_buyer","claims":"exact_zero","payment":"immediate_exact_gross_split","holder":"same_ack","term":"unchanged"}),
        ),
        (
            "fund_paid_mind_resale",
            json!({"format":1,"protocol":"personal_paid_resale_v1","authority":"named_buyer","claims":"positive","hold":"single_real_gross_escrow","holder":"unchanged","future_terminal_reserved":true}),
        ),
        (
            "settle_paid_mind_resale",
            json!({"format":1,"protocol":"personal_paid_resale_v1","authority":"registered_private_service_intent","trigger":"last_paid_claim_close_or_same_clock_deadline","old_claims":"retained","term":"unchanged"}),
        ),
        (
            "refund_paid_mind_resale",
            json!({"format":1,"protocol":"personal_paid_resale_v1","authority":"registered_private_service_intent","causes":["registry_revoked","clock_changed","recipient_unavailable","monetary_binding_changed"],"source_rights":"not_required"}),
        ),
    ]
}
fn authority() -> Value {
    json!({"format":1,"issuer":"roll.registry","organization":1,"scope":1,"operators":[1]})
}
pub(super) fn register_registry(
    definitions: &mut DefinitionRegistryBuilder,
    retention: &DefinitionRef,
) -> Result<DefinitionRef, Box<dyn std::error::Error>> {
    let owner = OwnerId::new("roll")?;
    let authority = definitions.register(
        owner.clone(),
        DefinitionDescriptor::new(
            DefinitionKind::new("registry_authority")?,
            DefinitionId::new("mind_revocation")?,
            1,
            authority(),
        )?,
    )?;
    Ok(definitions.register(owner,DefinitionDescriptor::new(DefinitionKind::new("action")?,DefinitionId::new("registry_term_revocation")?,1,json!({"format":1,"protocol":"registry_exact_term_revocation_v1","reason":"registry_review","scope":"one_accepted_term_generation","duration":"original_grace_end","later_terms":"fresh_admission","shared_rating":"unchanged"}))?.with_references(vec![authority,retention.clone()]))?)
}
pub(super) fn register_actions(
    definitions: &mut DefinitionRegistryBuilder,
    retention: &DefinitionRef,
) -> Result<[DefinitionRef; 5], Box<dyn std::error::Error>> {
    let mut actions = Vec::new();
    for (id, payload) in action_payloads() {
        actions.push(
            definitions.register(
                OwnerId::new("market")?,
                DefinitionDescriptor::new(
                    DefinitionKind::new("action")?,
                    DefinitionId::new(id)?,
                    1,
                    payload,
                )?
                .with_references(vec![retention.clone()]),
            )?,
        );
    }
    Ok(actions.try_into().map_err(|_| "five actions")?)
}
pub(super) fn register_families(
    builder: &mut StateFamilyRegistryBuilder,
    handles: &mut BTreeMap<String, Handle>,
    compound: bool,
) -> TestResult {
    for (owner, family, max) in [
        ("market", OFFER, 16384),
        ("roll", ROLL, 40960),
        ("market", MARKET, 40960),
        ("roll", HEAD, 4096),
        ("roll", HOLDER, 32768),
        ("market", TERMINAL, 8192),
    ]
    .into_iter()
    .chain(
        compound
            .then_some(())
            .into_iter()
            .flat_map(|_| [("roll", SANCTION, 32768), ("roll", STATUS, 4096)]),
    ) {
        let schema = |suffix: &str| CanonicalId::new(format!("{family}.{suffix}.v1"));
        let (decode, validate, key) = if owner == "roll" {
            ("strict_decode", "invariants", 192)
        } else {
            ("decode", "validate", 32)
        };
        let descriptor = StateFamilyDescriptor::new(
            StateFamilyId::new(family)?,
            schema("key")?,
            schema("value")?,
            1,
            schema(decode)?,
            schema(validate)?,
            schema("canonical")?,
        )?
        .with_limits(key, max)?;
        let owner = OwnerId::new(owner)?;
        let h = builder.register(
            owner.clone(),
            descriptor,
            |k: &String| Ok(k.clone()),
            |v: &Value| Ok(v.clone()),
            |_: &Value| Ok(()),
            |v: &Value| Ok(v.clone()),
        )?;
        if owner.as_str() == "roll" && family != SANCTION && family != STATUS {
            builder.delegate_family(owner.clone(), &h, OwnerId::new("market")?)?;
        }
        if compound && owner.as_str() == "market" && family != OFFER {
            builder.delegate_family(owner, &h, OwnerId::new("roll")?)?;
        }
        handles.insert(family.into(), h);
    }
    Ok(())
}
fn fixture() -> Result<Fixture, Box<dyn std::error::Error>> {
    fixture_with_registry(false)
}
fn fixture_with_registry(compound: bool) -> Result<Fixture, Box<dyn std::error::Error>> {
    let mut f = fixture_product_resale_registry(0, 0, 1, "sealed", true, compound)?;
    let prepared = fund(&mut f, false, true)?;
    install(&mut f, prepared)?;
    let prepared = finish(&mut f, true, false)?;
    install(&mut f, prepared)?;
    reboot(&f, None)?;
    Ok(f)
}
fn read(f: &Fixture, family: &str, key: &str) -> Value {
    f.store
        .get(&f.handles[family], &key.to_owned())
        .expect("fixture read")
        .expect("fixture row")
        .clone()
}
fn actions(f: &Fixture) -> [DefinitionRef; 5] {
    f.resale_actions.clone().expect("configured resale")
}
fn invocation(actor: u32, commit: CommitToken) -> Result<ActionInvocation, ActionMetadataError> {
    invocation_owner("market", actor, commit)
}
fn invocation_owner(
    owner: &str,
    actor: u32,
    commit: CommitToken,
) -> Result<ActionInvocation, ActionMetadataError> {
    let request = RequestUid::from_bytes(*commit.as_bytes());
    let principal = PrincipalId::Object { id: ObjId(actor) };
    ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor: ObjId(actor),
        offered_action: ActionOfferId(CanonicalHash::from_bytes([72; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters: json!({}),
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: OwnerId::new(owner).expect("owner"),
                namespace: CanonicalId::new("resale").expect("namespace"),
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })
}
fn offer_row(f: &Fixture, id: CommitToken, buyer: u32) -> Value {
    let pointer = read(f, LICENCE, LK);
    let birth_operation: CommitToken =
        serde_json::from_value(pointer["operation"].clone()).expect("birth token");
    let (route, allocation) = [
        ("legacy", "roll_mind_paid_purchase"),
        ("entitlement", "roll_mind_entitlement_purchase"),
        ("house", "roll_mind_house_purchase"),
    ]
    .into_iter()
    .find_map(|(route, family)| {
        f.handles
            .get(family)
            .and_then(|h| {
                f.store
                    .get(h, &birth_operation.to_hex())
                    .expect("birth read")
                    .cloned()
            })
            .map(|a| (route, a))
    })
    .expect("actual granted birth");
    let generation = pointer["term"]
        .as_str()
        .expect("term generation")
        .parse::<u64>()
        .expect("generation");
    let term = read(f, TERM, &format!("{LK}/term/{generation:016x}"));
    let birth = json!({"route":route,"operation":birth_operation,"digest":hash("roll.paid_holder.birth.v1",&allocation)});
    let ownership = pointer.get("ownership").cloned().unwrap_or_else(
        || json!({"generation":1,"operation":birth_operation,"digest":birth["digest"]}),
    );
    let economics = if route == "legacy" && generation == 1 {
        None
    } else {
        Some(read(
            f,
            if route == "house" {
                "roll_mind_house_term_economics"
            } else {
                "roll_mind_paid_term_economics"
            },
            &format!("{LK}/term/{generation:016x}"),
        ))
    };
    let economics_digest = economics
        .as_ref()
        .map(|e| {
            hash(
                if route == "house" {
                    house::economics_domain(e).expect("house economics")
                } else {
                    entitlement::economics_domain(e).expect("personal economics")
                },
                e,
            )
        })
        .unwrap_or_else(|| json!(DefinitionDigest::from_bytes([42; 32])));
    let chain = if route == "house" || (route == "legacy" && generation == 1) {
        hash(
            if route == "house" {
                "roll.paid_resale.house_royalty_basis.v1"
            } else {
                "roll.paid_resale.legacy_royalty_basis.v1"
            },
            &json!({"format":1,"birth":birth,"parents":[]}),
        )
    } else {
        economics.as_ref().expect("economics")["grant"]["royalty_chain"]["digest"].clone()
    };
    let issued = if generation == 1 {
        read(
            f,
            match route {
                "legacy" => "market_mind_purchase_receipt",
                "house" => "market_mind_house_receipt",
                _ => "market_mind_entitlement_receipt",
            },
            &birth_operation.to_hex(),
        )["issued_at_ic"]
            .clone()
    } else {
        let operation: CommitToken =
            serde_json::from_value(term["operation"].clone()).expect("term token");
        read(
            f,
            if route == "house" {
                "roll_house_mind_renewal_operation"
            } else {
                "roll_mind_renewal_operation"
            },
            &operation.to_hex(),
        )["state"]["terminal"]["issued_at_ic"]
            .clone()
    };
    let binding = json!({"licence":"1","charter":pointer["charter"],"line":pointer["line"],"birth":birth,"ownership":ownership,"term":term,"term_clock":issued["clock"],"economics_digest":economics_digest,"royalty_chain_digest":chain});
    let basis = json!({"licence":1,"charter":pointer["charter"].as_str().expect("charter").parse::<u32>().expect("charter"),"line":pointer["line"].as_str().expect("line").parse::<u64>().expect("line"),"birth":birth,"holder":ownership,"term":{"generation":generation,"operation":term["operation"],"digest":hash("roll.paid_resale.term.v1",&term),"clock":binding["term_clock"],"window":term["window"],"terms_digest":hash("roll.paid_resale.terms.v1",&term["terms"]),"policy_digest":hash("roll.paid_resale.policy.v1",&term["revision"])},"economics_digest":binding["economics_digest"],"royalty_chain_digest":binding["royalty_chain_digest"]});
    let action = actions(f)[0].clone();
    let at = 240.max(issued["at"].as_i64().expect("actual issued") + 1);
    let terms = json!({"format":1,"offer":id,"offer_action":action,"basis":basis,"seller":pointer["holder"],"buyer":buyer,"instrument":"credits","price":100,"registry":{"permille":0,"recipient":null},"venue":null,"parents":[],"remainder":{"account":10,"owner":{"Object":pointer["holder"]}},"offered_at_ic":clock(at),"offered_at_utc":1000,"expires_at_utc":2000,"settle_seconds":259200});
    json!({"format":1,"operation":id,"action":action,"binding":binding,"terms":terms})
}
fn publish_offer(
    f: &mut Fixture,
    id: CommitToken,
    buyer: u32,
) -> Result<Value, Box<dyn std::error::Error>> {
    let row = offer_row(f, id, buyer);
    publish_offer_row(f, id, row)
}
fn publish_offer_row(
    f: &mut Fixture,
    id: CommitToken,
    row: Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let actor = row["terms"]["seller"].as_u64().ok_or("seller")? as u32;
    let action = actions(f)[0].clone();
    let provider = f.providers.get_exact(&action)?;
    let inv = invocation(actor, id)?;
    let mut txn = f
        .store
        .transaction_for_provider(provider.registration_ordinal())?;
    txn.bind_registered_invocation(provider, &inv, id);
    txn.insert(&f.handles[OFFER], &id.to_hex(), row.clone())?;
    let prepared = txn.prepare()?;
    install(f, prepared)?;
    Ok(row)
}
fn acceptance(f: &Fixture, offer: &Value, id: CommitToken, pending: bool) -> Value {
    let action = actions(f)[if pending { 2 } else { 1 }].clone();
    let previous = f
        .store
        .get(&f.handles[HEAD], &LK.to_owned())
        .expect("head read")
        .map(|h| h["current"].clone())
        .unwrap_or(Value::Null);
    let sequence = previous["sequence"].as_u64().unwrap_or(0) + 1;
    let claims = json!({"next_transition":5,"open_paid":if pending{2}else{0}});
    let consent = json!({"format":1,"operation":id,"effect":action,"offer_digest":hash("market.paid_resale.offer.v1",offer),"terms":offer["terms"],"payer":20,"mode":if pending{"pending"}else{"immediate"},"accepted_at_ic":clock(offer["terms"]["offered_at_ic"]["at"].as_i64().expect("offered")+1),"accepted_at_utc":1001,"deadline":clock(offer["binding"]["term"]["window"]["term_end"].as_i64().expect("term end").min(offer["terms"]["offered_at_ic"]["at"].as_i64().expect("offered")+1+259200)),"claims":claims});
    let hold = if pending {
        json!({"id":30,"account":20,"amount":100,"status":"Held","placed_by":{"Object":offer["terms"]["buyer"]},"beneficiary":{"System":"economy.contract"},"purpose":{"Escrow":{"contract":40}},"holder":{"namespace":"market.paid_resale.v1","key":format!("{}/fund",id.to_hex())},"expires":null,"revision":1,"last_journal_seq":50})
    } else {
        Value::Null
    };
    let economic = json!({"intent":{"consent":consent,"issuer":"fixture.resale","contract":40,"hold":if pending{json!(30)}else{Value::Null}},"acceptance":{"transaction":60,"journal_seq":50,"plan_hash":vec![1u8;32],"hold":hold},"terminal":null});
    let mut row = json!({"format":1,"operation":id,"action":action,"sequence":sequence,"previous":previous,"offer":offer["operation"],"offer_digest":consent["offer_digest"],"binding":offer["binding"],"claim_observation":{"generation":1,"next_transition":5,"open_paid":claims["open_paid"]},"economic":economic,"state":{"phase":"funded"}});
    if !pending {
        settled(&mut row, offer, id, &action);
    }
    row
}
fn holder(row: &Value, offer: &Value, terminal: CommitToken, action: &DefinitionRef) -> Value {
    let b = &row["binding"];
    let t = &b["term"];
    json!({"format":1,"licence":b["licence"],"generation":b["ownership"]["generation"].as_u64().expect("generation")+1,"holder":offer["terms"]["buyer"],"birth":b["birth"],"predecessor":b["ownership"],"operation":terminal,"action":action,"acceptance":row["operation"],"offer":row["offer"],"offer_digest":row["offer_digest"],"term":{"generation":t["generation"],"operation":t["operation"],"digest":hash("roll.paid_resale.term.v1",t)},"accepted_at_ic":row["economic"]["intent"]["consent"]["accepted_at_ic"],"economic_digest":hash("economy.paid_resale.history.v1",&row["economic"])})
}
fn settled(row: &mut Value, offer: &Value, terminal: CommitToken, action: &DefinitionRef) {
    let h = holder(row, offer, terminal, action);
    row["state"] = json!({"phase":"settled","terminal":terminal,"ownership":{"generation":h["generation"],"operation":terminal,"digest":hash("roll.paid_holder.transfer.v1",&h)}});
}
fn terminal_row(
    f: &Fixture,
    prior: &Value,
    offer: &Value,
    commit: CommitToken,
    refund: bool,
) -> Value {
    let mut row = prior.clone();
    let action = actions(f)[if refund { 4 } else { 3 }].clone();
    let reason = if refund {
        json!({"kind":"recipient_unavailable"})
    } else {
        json!({"kind":"deadline_reached","next_transition":5,"open_paid":2})
    };
    let decision = json!({"format":1,"operation":commit,"action":action,"actor":1,"at_ic":clock(if refund {prior["economic"]["intent"]["consent"]["accepted_at_ic"]["at"].as_i64().expect("accepted")+1}else{(prior["economic"]["intent"]["consent"]["accepted_at_ic"]["at"].as_i64().expect("accepted")+1).max(prior["economic"]["intent"]["consent"]["deadline"]["at"].as_i64().expect("deadline"))}),"posted_at_utc":1002,"reason":reason});
    let mut hold = prior["economic"]["acceptance"]["hold"].clone();
    hold["revision"] = json!(2);
    hold["last_journal_seq"] = json!(51);
    hold["status"] = if refund {
        json!("Released")
    } else {
        json!({"Consumed":{"transaction":61,"amount":100}})
    };
    row["economic"]["terminal"] = json!({"decision":decision,"transaction":61,"journal_seq":51,"plan_hash":vec![2u8;32],"hold":hold});
    if refund {
        row["state"] = json!({"phase":"refunded","terminal":commit,"reason":reason});
    } else {
        settled(&mut row, offer, commit, &action);
    }
    row
}
fn publish(
    f: &mut Fixture,
    row: Value,
    action_index: usize,
    commit: CommitToken,
    omit: Option<&str>,
    capacity: bool,
) -> Result<PreparedTxn, ActionStateError> {
    publish_extra(
        f,
        row,
        action_index,
        commit,
        omit,
        capacity,
        None,
        Vec::new(),
        None,
    )
}
fn publish_extra(
    f: &mut Fixture,
    row: Value,
    action_index: usize,
    commit: CommitToken,
    omit: Option<&str>,
    capacity: bool,
    actor_override: Option<u32>,
    extra: Vec<(&str, String, Value)>,
    limits: Option<ActionTxnLimits>,
) -> Result<PreparedTxn, ActionStateError> {
    let id: CommitToken = serde_json::from_value(row["operation"].clone()).expect("token");
    let offer_id: CommitToken = serde_json::from_value(row["offer"].clone()).expect("offer token");
    let offer = read(f, OFFER, &offer_id.to_hex());
    let action = if action_index == 5 {
        f.resale_registry.clone().expect("Registry configured")
    } else {
        actions(f)[action_index].clone()
    };
    let actor = if action_index <= 2 {
        offer["terms"]["buyer"].as_u64().expect("buyer") as u32
    } else {
        1
    };
    let inv = invocation_owner(
        action.key().owner.as_str(),
        actor_override.unwrap_or(actor),
        commit,
    )
    .expect("invocation");
    let provider = f.providers.get_exact(&action).expect("provider");
    let mut images = vec![
        (ROLL, id.to_hex(), row.clone()),
        (
            MARKET,
            id.to_hex(),
            json!({"format":1,"operation":id,"action":row["action"],"licence":"1","sequence":row["sequence"],"offer":row["offer"],"offer_digest":row["offer_digest"],"roll_digest":hash("roll.paid_resale.record.v1",&row),"state":row["state"]}),
        ),
        (
            HEAD,
            LK.to_owned(),
            json!({"format":1,"licence":"1","current":{"sequence":row["sequence"],"operation":id,"digest":hash("roll.paid_resale.record.v1",&row)},"state":row["state"]}),
        ),
    ];
    if row["state"]["phase"] != "funded" {
        images.push((TERMINAL,id.to_hex(),json!({"format":1,"operation":row["state"]["terminal"],"acceptance":id,"licence":"1","offer":row["offer"],"offer_digest":row["offer_digest"],"roll_digest":hash("roll.paid_resale.record.v1",&row),"economic_digest":hash("economy.paid_resale.history.v1",&row["economic"]),"outcome":row["state"]["phase"]})));
        if row["state"]["phase"] == "settled" {
            let h = holder(&row, &offer, commit, &action);
            images.push((
                HOLDER,
                format!(
                    "{LK}/{:016x}",
                    h["generation"].as_u64().expect("generation")
                ),
                h,
            ));
            let mut pointer = read(f, LICENCE, LK);
            pointer["format"] = json!(2);
            pointer["holder"] = offer["terms"]["buyer"].clone();
            pointer["ownership"] = row["state"]["ownership"].clone();
            images.push((LICENCE, LK.to_owned(), pointer));
        }
    }
    images.extend(extra);
    let mut txn = f
        .store
        .transaction_for_provider(provider.registration_ordinal())?;
    txn.bind_registered_invocation(provider, &inv, commit);
    if let Some(limits) = limits {
        txn.set_limits(limits)?;
    }
    let base = txn.read_base();
    for (family, key, value) in images {
        if omit == Some(family) {
            continue;
        }
        if txn.get(&f.handles[family], &key)?.is_some() {
            txn.replace(&f.handles[family], &key, value)?;
        } else {
            txn.insert(&f.handles[family], &key, value)?;
        }
    }
    if capacity {
        let mut ctx = crate::resolution::PrepareCtx::new(inv.idempotency(), commit, None)
            .with_registered_transaction(&base, provider.registration_ordinal());
        if action_index == 2 {
            ctx.reserve_paid_resale_completion(&mut txn)?;
        } else if action_index >= 3 {
            ctx.close_paid_resale_completion(&mut txn, id)?;
        }
    }
    txn.prepare()
}

#[test]
fn immediate_actual_install_preserves_lapsed_term_and_birth_with_exact_holder_chain() -> TestResult
{
    let mut f = fixture()?;
    let original = read(&f, LICENCE, LK);
    let term = read(&f, TERM, TK);
    let birth = read(&f, ALLOCATION, &op().to_hex());
    // Acceptance occurs at241, after this unchanged term's grace_end230.
    let offer = publish_offer(&mut f, token(10), 8)?;
    let row = acceptance(&f, &offer, token(11), false);
    let p = publish(&mut f, row, 1, token(11), None, true)?;
    install(&mut f, p)?;
    let current = read(&f, LICENCE, LK);
    assert_eq!(current["holder"], 8);
    assert_eq!(current["term"], original["term"]);
    assert_eq!(current["operation"], original["operation"]);
    assert!(
        read(&f, TERM, TK) == term && read(&f, ALLOCATION, &op().to_hex()) == birth,
        "immutable birth and term changed"
    );
    assert!(f.store.capacity.is_empty());
    reboot(&f, None)?;
    let offer = publish_offer(&mut f, token(20), 9)?;
    let row = acceptance(&f, &offer, token(21), false);
    let p = publish(&mut f, row, 1, token(21), None, true)?;
    install(&mut f, p)?;
    assert_eq!(read(&f, LICENCE, LK)["ownership"]["generation"], 3);
    reboot(&f, None)?;
    Ok(())
}
#[test]
fn funded_six_targets_then_deadline_settlement_consumes_exact_reservation() -> TestResult {
    let mut f = fixture()?;
    let before = read(&f, LICENCE, LK);
    let offer = publish_offer(&mut f, token(10), 8)?;
    let row = acceptance(&f, &offer, token(11), true);
    let p = publish(&mut f, row.clone(), 2, token(11), None, true)?;
    install(&mut f, p)?;
    assert!(read(&f, LICENCE, LK) == before, "Fund changed holder");
    let reserved = f
        .store
        .capacity
        .records
        .values()
        .next()
        .ok_or("reservation")?;
    assert_eq!(reserved.record.targets.len(), 6);
    assert_eq!(
        reserved
            .record
            .targets
            .iter()
            .map(|t| t.ceiling)
            .sum::<usize>(),
        131072
    );
    assert_eq!(reserved.record.sponsor.owner.as_str(), "market");
    reboot(&f, None)?;
    let closed = terminal_row(&f, &row, &offer, token(12), false);
    let p = publish(&mut f, closed, 3, token(12), None, true)?;
    install(&mut f, p)?;
    assert!(f.store.capacity.is_empty());
    assert_eq!(read(&f, LICENCE, LK)["holder"], 8);
    reboot(&f, None)?;
    Ok(())
}
#[test]
fn refund_then_fresh_offer_same_next_holder_generation_survives_cold_history() -> TestResult {
    let mut f = fixture()?;
    let original = read(&f, LICENCE, LK);
    let offer = publish_offer(&mut f, token(10), 8)?;
    let row = acceptance(&f, &offer, token(11), true);
    let p = publish(&mut f, row.clone(), 2, token(11), None, true)?;
    install(&mut f, p)?;
    let closed = terminal_row(&f, &row, &offer, token(12), true);
    let p = publish(&mut f, closed, 4, token(12), None, true)?;
    install(&mut f, p)?;
    assert!(read(&f, LICENCE, LK) == original, "Refund changed holder");
    reboot(&f, None)?;
    let offer = publish_offer(&mut f, token(20), 9)?;
    let row = acceptance(&f, &offer, token(21), false);
    let p = publish(&mut f, row, 1, token(21), None, true)?;
    install(&mut f, p)?;
    assert_eq!(read(&f, LICENCE, LK)["ownership"]["generation"], 2);
    assert_eq!(read(&f, HEAD, LK)["current"]["sequence"], 2);
    reboot(&f, None)?;
    Ok(())
}
#[test]
fn partial_terminal_missing_marker_and_changed_native_acceptance_never_consume() -> TestResult {
    for missing in [
        Some(ROLL),
        Some(MARKET),
        Some(HEAD),
        Some(HOLDER),
        Some(LICENCE),
        Some(TERMINAL),
        None,
    ] {
        let mut f = fixture()?;
        let offer = publish_offer(&mut f, token(10), 8)?;
        let row = acceptance(&f, &offer, token(11), true);
        let p = publish(&mut f, row.clone(), 2, token(11), None, true)?;
        install(&mut f, p)?;
        let disk = f.disk.clone();
        let closed = terminal_row(&f, &row, &offer, token(12), false);
        assert!(publish(&mut f, closed, 3, token(12), missing, missing.is_some()).is_err());
        assert!(
            f.disk == disk && f.store.capacity.records.len() == 1,
            "failed terminal changed installed custody"
        );
    }
    let mut f = fixture()?;
    let offer = publish_offer(&mut f, token(10), 8)?;
    let row = acceptance(&f, &offer, token(11), true);
    let p = publish(&mut f, row.clone(), 2, token(11), None, true)?;
    install(&mut f, p)?;
    let mut closed = terminal_row(&f, &row, &offer, token(12), false);
    closed["economic"]["intent"]["consent"]["payer"] = json!(99);
    assert!(publish(&mut f, closed, 3, token(12), None, true).is_err());
    assert_eq!(f.store.capacity.records.len(), 1);
    Ok(())
}
#[test]
fn cold_missing_crossed_and_all_missing_transfer_rows_refuse() -> TestResult {
    let mut f = fixture()?;
    let offer = publish_offer(&mut f, token(10), 8)?;
    let row = acceptance(&f, &offer, token(11), true);
    let p = publish(&mut f, row.clone(), 2, token(11), None, true)?;
    install(&mut f, p)?;
    for family in [ROLL, MARKET, HEAD, OFFER, "action-state/capacity/"] {
        assert!(
            reboot(&f, Some(family)).is_err(),
            "missing pending {family}"
        );
    }
    let closed = terminal_row(&f, &row, &offer, token(12), false);
    let p = publish(&mut f, closed, 3, token(12), None, true)?;
    install(&mut f, p)?;
    for family in [ROLL, MARKET, HEAD, HOLDER, TERMINAL, OFFER] {
        assert!(
            reboot(&f, Some(family)).is_err(),
            "missing terminal {family}"
        );
    }
    let disk = f.disk.clone();
    for ((_, key), value) in &mut f.disk {
        if key.contains(HOLDER) {
            value["value"]["holder"] = json!(99);
        }
    }
    assert!(reboot(&f, None).is_err());
    f.disk = disk;
    f.disk.retain(|(_, key), _| {
        ![ROLL, MARKET, HEAD, HOLDER, TERMINAL, OFFER]
            .iter()
            .any(|family| key.contains(family))
    });
    assert!(
        reboot(&f, None).is_err(),
        "format2 pointer cannot survive erased transfer history"
    );
    Ok(())
}

fn compound(
    f: &Fixture,
    prior: &Value,
    offer: &Value,
    id: CommitToken,
) -> (Value, Vec<(&'static str, String, Value)>) {
    let mut row = terminal_row(f, prior, offer, id, true);
    let term = row["binding"]["term"].clone();
    let action = f.resale_registry.clone().expect("exact Registry action");
    let reason = json!({"kind":"registry_revoked","sanction":id,"licence":1,"generation":1,"term_digest":hash("roll.paid_resale.term.v1",&term)});
    row["economic"]["terminal"]["decision"]["action"] = json!(action);
    row["economic"]["terminal"]["decision"]["at_ic"] = clock(152);
    row["economic"]["terminal"]["decision"]["reason"] = reason.clone();
    row["state"]["reason"] = reason;
    let accepted = json!({"kind":"paid","term":term});
    // These closed cause values are coordinator fixture data. The production
    // Roll participant separately proves actual Org authorization and roots.
    let cause = json!({"format":1,"operation":id,"action":action,"authority":authority(),"actor":1,"authorization":{"fixture":"inert"},"term":accepted,"charter":"1","line":"1","birth_operation":op(),"initial_operation":token(1),"initial_issuer":null,"filed":"1","revision":term["revision"],"reason":"registry_review","at_ic":clock(152)});
    let status = json!({"format":1,"licence":"1","generation":"1","operation":id,"term_digest":hash("roll.registry_revocation.accepted_term.v1",&accepted),"window":term["window"]});
    (
        row,
        vec![
            (SANCTION, id.to_hex(), cause),
            (STATUS, TK.to_owned(), status),
        ],
    )
}

#[test]
fn exact_registry_compound_refund_closes_only_original_market_metadata() -> TestResult {
    for omit in [Some(SANCTION), Some(STATUS), None] {
        let mut f = fixture_with_registry(true)?;
        let original = read(&f, LICENCE, LK);
        let mut offer = offer_row(&f, token(10), 8);
        offer["terms"]["offered_at_ic"] = clock(150);
        let offer = publish_offer_row(&mut f, token(10), offer)?;
        let mut row = acceptance(&f, &offer, token(11), true);
        row["economic"]["intent"]["consent"]["accepted_at_ic"] = clock(151);
        let p = publish(&mut f, row.clone(), 2, token(11), None, true)?;
        install(&mut f, p)?;
        let capacity_keys = f
            .disk
            .keys()
            .filter(|(_, k)| k.starts_with("action-state/capacity/"))
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(capacity_keys.len(), 1);
        assert_eq!(capacity_keys[0].0.as_str(), "market");
        let (terminal, extra) = compound(&f, &row, &offer, token(12));
        let result = publish_extra(
            &mut f,
            terminal,
            5,
            token(12),
            omit,
            true,
            None,
            extra,
            None,
        );
        if omit.is_some() {
            assert!(result.is_err());
            assert_eq!(f.store.capacity.records.len(), 1);
            continue;
        }
        install(&mut f, result?)?;
        assert!(f.store.capacity.is_empty() && !f.disk.contains_key(&capacity_keys[0]));
        assert!(
            read(&f, LICENCE, LK) == original,
            "Registry refund changed property"
        );
        reboot(&f, None)?;
        for family in [SANCTION, STATUS] {
            assert!(
                reboot(&f, Some(family)).is_err(),
                "missing exact compound proof"
            );
        }
    }
    Ok(())
}

#[test]
fn actual_actor_marker_and_pending_head_refuse_crossed_admissions() -> TestResult {
    for (actor, marker) in [(Some(99), true), (None, false)] {
        let mut f = fixture()?;
        let offer = publish_offer(&mut f, token(10), 8)?;
        let row = acceptance(&f, &offer, token(11), true);
        let disk = f.disk.clone();
        assert!(
            publish_extra(&mut f, row, 2, token(11), None, marker, actor, vec![], None).is_err()
        );
        assert!(
            f.disk == disk && f.store.capacity.is_empty(),
            "refused admission changed state"
        );
    }
    let mut f = fixture()?;
    let offer = publish_offer(&mut f, token(10), 8)?;
    let row = acceptance(&f, &offer, token(11), true);
    let p = publish(&mut f, row.clone(), 2, token(11), None, true)?;
    install(&mut f, p)?;
    assert!(publish_offer(&mut f, token(20), 9).is_err());
    let closed = terminal_row(&f, &row, &offer, token(12), true);
    assert!(
        publish_extra(
            &mut f,
            closed,
            4,
            token(12),
            None,
            true,
            Some(99),
            vec![],
            None
        )
        .is_err()
    );
    assert_eq!(f.store.capacity.records.len(), 1);
    Ok(())
}

#[test]
fn caught_detached_context_error_stays_poisoned_without_capacity_or_rows() -> TestResult {
    let mut f = fixture()?;
    let action = actions(&f)[2].clone();
    let provider = f.providers.get_exact(&action)?;
    let commit = token(11);
    let inv = invocation(8, commit)?;
    let mut txn = f
        .store
        .transaction_for_provider(provider.registration_ordinal())?;
    txn.bind_registered_invocation(provider, &inv, commit);
    let mut ctx = crate::resolution::PrepareCtx::new(inv.idempotency(), commit, None);
    assert!(ctx.reserve_paid_resale_completion(&mut txn).is_err());
    assert!(txn.prepare().is_err());
    assert!(f.store.capacity.is_empty());
    reboot(&f, None)?;
    Ok(())
}

#[test]
fn full_reservation_charge_has_exact_live_byte_boundary_and_terminal_non_growth() -> TestResult {
    let mut measured = fixture()?;
    let offer = publish_offer(&mut measured, token(10), 8)?;
    let row = acceptance(&measured, &offer, token(11), true);
    let before = measured.store.live_canonical_bytes;
    let p = publish(&mut measured, row, 2, token(11), None, true)?;
    install(&mut measured, p)?;
    let charged = measured
        .store
        .capacity_usage
        .bytes
        .checked_sub(before)
        .ok_or("charge")?;
    assert!(charged >= 131072);
    // Only unrelated aggregate accounting is injected. This does not allocate
    // a 512 MiB fixture or represent real business/native evidence.
    for over in [false, true] {
        let mut f = fixture()?;
        let offer = publish_offer(&mut f, token(10), 8)?;
        let row = acceptance(&f, &offer, token(11), true);
        f.store.live_canonical_bytes = MAX_ACTION_STATE_BOOT_BYTES - charged + usize::from(over);
        let result = publish(&mut f, row.clone(), 2, token(11), None, true);
        if over {
            assert!(
                matches!(result,Err(ActionStateError::LiveBytesExceeded {attempted,..}) if attempted==MAX_ACTION_STATE_BOOT_BYTES+1)
            );
            assert!(f.store.capacity.is_empty());
        } else {
            install(&mut f, result?)?;
            assert_eq!(f.store.capacity_usage.bytes, MAX_ACTION_STATE_BOOT_BYTES);
            let closed = terminal_row(&f, &row, &offer, token(12), false);
            let p = publish(&mut f, closed, 3, token(12), None, true)?;
            install(&mut f, p)?;
            assert!(
                f.store.capacity_usage.bytes <= MAX_ACTION_STATE_BOOT_BYTES
                    && f.store.capacity.is_empty()
            );
        }
    }
    Ok(())
}

#[test]
fn compound_sanction_rows_remain_ordinary_counted_writes() -> TestResult {
    for compound_mode in [false, true] {
        let mut f = fixture_with_registry(true)?;
        let mut offer = offer_row(&f, token(10), 8);
        offer["terms"]["offered_at_ic"] = clock(150);
        let offer = publish_offer_row(&mut f, token(10), offer)?;
        let mut row = acceptance(&f, &offer, token(11), true);
        row["economic"]["intent"]["consent"]["accepted_at_ic"] = clock(151);
        let p = publish(&mut f, row.clone(), 2, token(11), None, true)?;
        install(&mut f, p)?;
        let (terminal, extra) = if compound_mode {
            compound(&f, &row, &offer, token(12))
        } else {
            (terminal_row(&f, &row, &offer, token(12), true), vec![])
        };
        let result = publish_extra(
            &mut f,
            terminal,
            if compound_mode { 5 } else { 4 },
            token(12),
            None,
            true,
            None,
            extra,
            Some(ActionTxnLimits::tightened(5, MAX_ACTION_BUNDLE_BYTES)),
        );
        if compound_mode {
            assert!(result.is_err());
            assert_eq!(f.store.capacity.records.len(), 1);
        } else {
            let p = result?;
            assert_eq!(p.operation_count(), 5);
            install(&mut f, p)?;
        }
    }
    Ok(())
}

#[test]
fn disabled_recipe_cannot_recover_retained_resale_or_format_two_pointer() -> TestResult {
    let mut f = fixture()?;
    let offer = publish_offer(&mut f, token(10), 8)?;
    let row = acceptance(&f, &offer, token(11), false);
    let p = publish(&mut f, row, 1, token(11), None, true)?;
    install(&mut f, p)?;
    let mut registry = f.store.registry.clone();
    registry.paid_resale = None;
    let mut disabled = ActionStateStore::new(registry);
    assert!(
        disabled
            .hydrate(
                f.disk
                    .iter()
                    .map(|((ns, key), value)| ActionStateBootRecord::new(
                        ns.clone(),
                        key.clone(),
                        value.clone()
                    ))
            )
            .is_err()
    );
    Ok(())
}

type Seed = (
    ActionStateStore,
    BTreeMap<String, Handle>,
    BTreeMap<(SmolStr, SmolStr), Value>,
);
fn from_seed((birth, mut handles, disk): Seed) -> Result<Fixture, Box<dyn std::error::Error>> {
    let market = OwnerId::new("market")?;
    let roll = OwnerId::new("roll")?;
    let owners = BTreeSet::from([
        market.clone(),
        roll.clone(),
        OwnerId::new("economy")?,
        OwnerId::new("tacsim")?,
    ]);
    let readiness = MechanicalOwnerReadiness::for_owners(&owners);
    let mut definitions = DefinitionRegistryBuilder::new();
    let economy = definitions.register(
        OwnerId::new("economy")?,
        DefinitionDescriptor::new(
            DefinitionKind::new("owner_participant")?,
            DefinitionId::new("initial_filing")?,
            1,
            json!({"generation":1}),
        )?,
    )?;
    let retention = definitions.register(
        roll.clone(),
        DefinitionDescriptor::new(
            DefinitionKind::new("owner_participant")?,
            DefinitionId::new("commercial_roots")?,
            1,
            json!({"format":1,"protocol":"commercial_retention_v1"}),
        )?,
    )?;
    let resale_actions = register_actions(&mut definitions, &retention)?;
    let mut definitions = definitions.freeze()?;
    definitions.bind_owner_readiness(readiness.clone());
    let mut providers = ActionProviderRegistryBuilder::new();
    for action in &resale_actions {
        providers.register(market.clone(), action.clone(), Arc::new(Provider))?;
    }
    let providers =
        providers.freeze_for_owners_with_readiness(&definitions, &owners, readiness.clone())?;
    let mut builder = StateFamilyRegistryBuilder::new();
    builder.entries = birth.registry.entries.clone();
    for (owner, _) in builder.entries.keys() {
        *builder.owner_counts.entry(owner.clone()).or_default() += 1;
    }
    builder.delegations = birth.registry.delegations.clone();
    builder.family_delegations = birth.registry.family_delegations.clone();
    builder.original_purchase = birth.registry.original_purchase.clone();
    builder.entitlement_purchase = birth.registry.entitlement_purchase.clone();
    builder.house_purchase = birth.registry.house_purchase.clone();
    builder.mind_renewal = birth.registry.mind_renewal.clone();
    builder.house_mind_renewal = birth.registry.house_mind_renewal.clone();
    if !handles.contains_key(OFFER) {
        register_families(&mut builder, &mut handles, false)?;
    }
    builder.paid_resale_capacity(&market, [economy, retention], resale_actions.clone(), None)?;
    let mut store =
        ActionStateStore::new(builder.freeze_for_owners_with_readiness(&owners, readiness)?);
    store.hydrate(disk.iter().map(|((ns, key), value)| {
        ActionStateBootRecord::new(ns.clone(), key.clone(), value.clone())
    }))?;
    // These fixtures only continue resale; old birth/renewal actions are not
    // rebound or replayed through this unused legacy-helper tuple.
    let unused_actions = [
        resale_actions[0].clone(),
        resale_actions[1].clone(),
        resale_actions[2].clone(),
    ];
    Ok(Fixture {
        form: "sealed",
        store,
        handles,
        providers,
        actions: unused_actions,
        resale_actions: Some(resale_actions),
        resale_registry: None,
        invocation: invocation(7, token(99))?,
        disk,
    })
}

#[test]
fn actual_house_entitlement_and_both_renewal_histories_keep_immutable_birth_after_transfer()
-> TestResult {
    let seeds = [
        entitlement::tests::renewal_birth_seed()?,
        house::tests::renewal_birth_seed()?,
        renewal::tests::resale_term_seed(true)?,
        house_renewal::tests::resale_term_seed(true)?,
    ];
    for seed in seeds {
        let mut f = from_seed(seed)?;
        let before = read(&f, LICENCE, LK);
        let disk = f.disk.clone();
        let buyer = if before["holder"] == json!(8) { 9 } else { 8 };
        let offer = publish_offer(&mut f, token(80), buyer)?;
        let row = acceptance(&f, &offer, token(81), false);
        let p = publish(&mut f, row, 1, token(81), None, true)?;
        install(&mut f, p)?;
        let after = read(&f, LICENCE, LK);
        assert_eq!(after["holder"], buyer);
        assert_eq!(after["term"], before["term"]);
        assert_eq!(after["operation"], before["operation"]);
        for ((ns, key), value) in disk {
            if value["family"]["id"] != LICENCE {
                assert!(
                    f.disk.get(&(ns, key)) == Some(&value),
                    "historical acknowledged row changed"
                );
            }
        }
        reboot(&f, None)?;
    }
    Ok(())
}

#[test]
fn actual_open_personal_or_house_renewal_blocks_resale_without_consuming_old_custody() -> TestResult
{
    for seed in [
        renewal::tests::resale_term_seed(false)?,
        house_renewal::tests::resale_term_seed(false)?,
    ] {
        let mut f = from_seed(seed)?;
        let before = f.disk.clone();
        let reserved = f.store.capacity.records.len();
        assert_eq!(reserved, 1);
        assert!(publish_offer(&mut f, token(80), 8).is_err());
        assert!(
            f.disk == before && f.store.capacity.records.len() == reserved,
            "resale changed pending renewal custody"
        );
        reboot(&f, None)?;
    }
    Ok(())
}

#[path = "registration_tests.rs"]
mod registration;

// Actual typed coordinator history; native monetary images remain structural
// fixtures. The Economy owner/genuine paid-holder story tests supply authority.
fn transfer_seed(
    seed: Seed,
    offer_byte: u8,
    buyer: u32,
) -> Result<Seed, Box<dyn std::error::Error>> {
    let mut f = from_seed(seed)?;
    let offer = publish_offer(&mut f, token(offer_byte), buyer)?;
    let id = token(offer_byte + 1);
    let row = acceptance(&f, &offer, id, false);
    let p = publish(&mut f, row, 1, id, None, true)?;
    install(&mut f, p)?;
    reboot(&f, None)?;
    Ok((f.store, f.handles, f.disk))
}
fn closed_seed(house: bool, funded: bool) -> Result<Seed, Box<dyn std::error::Error>> {
    if house {
        house_renewal::tests::closed_holder_seed(funded)
    } else {
        renewal::tests::closed_holder_seed(funded)
    }
}
fn renew_seed(
    house: bool,
    seed: Seed,
    byte: u8,
    previous: Option<CommitToken>,
    terminal: usize,
) -> Result<Seed, Box<dyn std::error::Error>> {
    if house {
        house_renewal::tests::continue_holder_seed(seed, byte, previous, terminal)
    } else {
        renewal::tests::continue_holder_seed(seed, byte, previous, terminal)
    }
}
#[test]
fn transferred_buyers_renew_after_each_seller_refusal_and_keep_exact_ownership() -> TestResult {
    for house in [false, true] {
        for funded in [false, true] {
            let seed = transfer_seed(closed_seed(house, funded)?, 80, 8)?;
            let old_disk = seed.2.clone();
            let seed = renew_seed(house, seed, 100, None, 2)?;
            for ((ns, key), value) in old_disk {
                if ![
                    LICENCE,
                    "roll_mind_renewal_head",
                    "roll_house_mind_renewal_head",
                ]
                .contains(&value["family"]["id"].as_str().unwrap_or(""))
                {
                    assert!(
                        seed.2.get(&(ns, key)) == Some(&value),
                        "old acknowledged history changed"
                    );
                }
            }
            let f = from_seed(seed)?;
            assert_eq!(read(&f, LICENCE, LK)["holder"], 8);
            assert_eq!(read(&f, LICENCE, LK)["term"], "2");
            assert_eq!(read(&f, LICENCE, LK)["ownership"]["generation"], 2);
            reboot(&f, None)?;
        }
    }
    Ok(())
}
#[test]
fn holder_retry_is_exact_and_a_later_transfer_can_renew_an_already_renewed_term() -> TestResult {
    for house in [false, true] {
        let seed = transfer_seed(closed_seed(house, false)?, 80, 8)?;
        let seed = renew_seed(house, seed, 100, None, 4)?;
        let seed = renew_seed(house, seed, 104, Some(token(100)), 3)?;
        let seed = renew_seed(house, seed, 108, Some(token(104)), 2)?;
        // Exercises real renewal state.kind in resale's immutable term join.
        let seed = transfer_seed(seed, 120, 9)?;
        let seed = renew_seed(house, seed, 130, None, 2)?;
        let f = from_seed(seed)?;
        assert_eq!(read(&f, LICENCE, LK)["holder"], 9);
        assert_eq!(read(&f, LICENCE, LK)["term"], "3");
        assert_eq!(read(&f, LICENCE, LK)["ownership"]["generation"], 3);
        reboot(&f, None)?;
    }
    Ok(())
}
#[test]
fn repeated_transfer_before_renewal_preserves_skipped_holder_history() -> TestResult {
    for house in [false, true] {
        let seed = transfer_seed(closed_seed(house, true)?, 80, 8)?;
        let seed = transfer_seed(seed, 90, 9)?;
        let seed = renew_seed(house, seed, 100, None, 2)?;
        let f = from_seed(seed)?;
        assert_eq!(read(&f, LICENCE, LK)["ownership"]["generation"], 3);
        reboot(&f, None)?;
    }
    Ok(())
}
#[test]
fn holder_open_renewal_blocks_resale_and_refuses_missing_or_disabled_holder_proof() -> TestResult {
    for house in [false, true] {
        let seed = transfer_seed(closed_seed(house, false)?, 80, 8)?;
        let seed = renew_seed(house, seed, 100, None, 0)?;
        let mut f = from_seed(seed)?;
        let before = f.disk.clone();
        assert!(publish_offer(&mut f, token(110), 9).is_err());
        assert!(
            f.disk == before,
            "pending renewal changed on refused resale"
        );
        for family in [HOLDER, ROLL, MARKET, TERMINAL, OFFER] {
            let mut disk = f.disk.clone();
            let n = disk.len();
            disk.retain(|_, v| v["family"]["id"] != family);
            assert!(disk.len() < n);
            let mut store = ActionStateStore::new(f.store.registry.clone());
            assert!(
                store
                    .hydrate(
                        disk.into_iter()
                            .map(|((ns, key), value)| ActionStateBootRecord::new(ns, key, value))
                    )
                    .is_err(),
                "missing holder proof admitted"
            );
        }
        let mut registry = f.store.registry.clone();
        registry.paid_resale = None;
        let mut store = ActionStateStore::new(registry);
        assert!(
            store
                .hydrate(
                    f.disk
                        .iter()
                        .map(|((ns, key), value)| ActionStateBootRecord::new(
                            ns.clone(),
                            key.clone(),
                            value.clone()
                        ))
                )
                .is_err()
        );
        reboot(&f, None)?;
    }
    Ok(())
}

#[test]
fn pending_resale_blocks_the_current_transferred_holders_fresh_renewal() -> TestResult {
    for house in [false, true] {
        let seed = transfer_seed(closed_seed(house, false)?, 80, 8)?;
        let mut f = from_seed(seed)?;
        let offer = publish_offer(&mut f, token(90), 9)?;
        let row = acceptance(&f, &offer, token(91), true);
        let p = publish(&mut f, row, 2, token(91), None, true)?;
        install(&mut f, p)?;
        reboot(&f, None)?;
        let seed = (f.store, f.handles, f.disk);
        if house {
            house_renewal::tests::refuse_holder_seed(seed)?;
        } else {
            renewal::tests::refuse_holder_seed(seed)?;
        }
    }
    Ok(())
}

#[test]
fn paid_follow_survives_two_actual_holder_transfers_and_new_renewal_consents() -> TestResult {
    for house in [false, true] {
        let seed = if house {
            house::tests::follow_renewal_birth_seed()?
        } else {
            entitlement::tests::follow_renewal_birth_seed()?
        };
        let seed = transfer_seed(seed, 80, 8)?;
        let seed = renew_seed(house, seed, 100, None, 2)?;
        let seed = transfer_seed(seed, 120, 9)?;
        let seed = renew_seed(house, seed, 130, None, 2)?;
        let f = from_seed(seed)?;
        reboot(&f, None)?;
        let current = read(&f, LICENCE, LK);
        assert_eq!(current["holder"], json!(9));
        assert_eq!(current["ownership"]["generation"], json!(3));
        let term = read(&f, TERM, &format!("{LK}/term/0000000000000003"));
        assert_eq!(term["revision"], json!({"kind":"follow","line":"1"}));
    }
    Ok(())
}
