//! Pure strict-codec/paired-data tests. These do not construct an E4 audit
//! view, certify a source, register a provider, or publish a grant.
use super::records::{MindRenewalOperationV1, MindRenewalReceiptV1};
use mush_core::{
    ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId,
    mechanics::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
    },
};
use mush_economy::{
    AccountId, Amount, ContractId, InstrumentId,
    managed_entitlement::{GrossRecipientCut, Recipient},
    managed_renewal::{RenewalBasisV1, RenewalLineageV1, RenewalOffer, RenewalOfferTermsV1},
};
use mush_roll::commerce::{
    CharterId, FiledRevisionId, LicenceId, LineId, RevisionIdentity, TermGeneration, TermWindow,
    WorldIcSeconds,
    entitlement::{
        DownstreamTermsV1, FiledRoyaltyTermsV1, GrantEconomicsV1, PaidTermEconomicsV1,
        RenewalTemplateV1,
    },
    paid::{PaidForm, PaidTermRecord, PaidTerms},
    purchase::PurchaseParticipants,
    rating::RatingClass,
    records::RevisionPolicy,
    renewal::{
        RenewalAttemptV1, RenewalBindingV1, RenewalOperationV1, RenewalStateV1,
        protocol::{self, Transition},
    },
};
use serde_json::{Value, json};
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
fn digest(n: u8) -> DefinitionDigest {
    DefinitionDigest::from_bytes([n; 32])
}
fn stamp(at: i64) -> ClockStamp {
    ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at,
    }
}
fn definition(owner: &str, kind: &str, id: &str, n: u8) -> TestResult<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(
        OwnerId::new(owner)?,
        DefinitionDescriptor::new(
            DefinitionKind::new(kind)?,
            DefinitionId::new(id)?,
            1,
            json!({"format":1,"fixture":n}),
        )?,
    )?)
}
fn action(
    transition: Transition,
    manifest: &DefinitionRef,
    refs: &[DefinitionRef; 2],
) -> TestResult<DefinitionRef> {
    let [economy, retention] = refs;
    let p = PurchaseParticipants::new(economy.clone(), retention.clone())?;
    Ok(DefinitionRegistryBuilder::new().register(
        OwnerId::new("market")?,
        protocol::descriptor(transition, manifest, &p)?,
    )?)
}
pub(super) fn fixture() -> TestResult<MindRenewalOperationV1> {
    let manifest = definition("tacsim", "registry_manifest", "combat", 1)?;
    let participants = [
        definition("economy", "owner_participant", "initial_filing", 2)?,
        definition("roll", "owner_participant", "commercial_roots", 3)?,
    ];
    let identity: RevisionIdentity = serde_json::from_value(json!({
        "format":1,"revision":{"key":{"character":1,"mind":1,"number":1},"digest":digest(1)},"source":digest(2)
    }))?;
    let licence = LicenceId::new(7)?;
    let one = TermGeneration::new(1)?;
    let terms: PaidTerms = serde_json::from_value(
        json!({"seats":1,"class":RatingClass::I,"form":PaidForm::OpenResale{},"transferable":false}),
    )?;
    let filed = FiledRevisionId::new(3)?;
    let policy = RevisionPolicy::Pin {
        filed,
        revision: identity,
    };
    let chain: FiledRoyaltyTermsV1 = serde_json::from_value(
        json!({"format":1,"filed":filed,"revision":identity,"publication":token(2),"edges":[]}),
    )?;
    let template = RenewalTemplateV1::new(
        InstrumentId::new("cr").ok_or("instrument")?,
        Amount::from_minor(500),
        100,
        20,
        terms,
        policy,
        definition("market", "entitlement_policy", "personal", 4)?,
    )?;
    let grant = GrantEconomicsV1::new(
        DownstreamTermsV1::Source {
            royalty_permille: 73,
        },
        template,
        chain.reference()?,
    )?;
    let window = TermWindow::new(
        WorldIcSeconds::from_stamp(&stamp(10))?,
        WorldIcSeconds::from_stamp(&stamp(10))?,
        WorldIcSeconds::from_stamp(&stamp(110))?,
        WorldIcSeconds::from_stamp(&stamp(130))?,
    )?;
    let term: PaidTermRecord = serde_json::from_value(
        json!({"format":1,"licence":licence,"generation":one,"window":window,"terms":terms,"revision":policy,"operation":token(1)}),
    )?;
    let economics: PaidTermEconomicsV1 = serde_json::from_value(
        json!({"format":1,"licence":licence,"generation":one,"operation":token(1),"product_digest":digest(9),"grant":grant}),
    )?;
    let prior_digest = DefinitionDigest::for_canonical_value(
        "roll.mind.renewal_predecessor.v1",
        &serde_json::to_value(term)?,
    )?;
    let binding: RenewalBindingV1 = serde_json::from_value(json!({
        "format":1,"licence":licence,"holder":ObjId(5),"charter":CharterId::new(9)?,"line":LineId::new(2)?,
        "birth_operation":token(1),"predecessor":one,"next":TermGeneration::new(2)?,
        "predecessor_operation":token(1),"predecessor_term":prior_digest,"predecessor_economics":economics.digest()?,
        "filed":filed,"revision":identity,"terms":terms,"term_seconds":100,"grace_seconds":20,
        "grant":grant,"product_digest":digest(9)
    }))?;
    let offer = RenewalOffer {
        format: 1,
        basis: RenewalBasisV1 {
            format: 1,
            licence: licence.get(),
            predecessor_generation: 1,
            next_generation: 2,
            predecessor_operation: token(1),
            prior_quote: token(2),
            prior_quote_digest: digest(3),
            product_digest: digest(9),
            template_digest: binding.template_digest()?,
            grant_economics_digest: binding.grant().digest()?,
            royalty_chain_digest: chain.digest()?,
        },
        terms: RenewalOfferTermsV1 {
            format: 1,
            lineage: RenewalLineageV1::Original,
            buyer: ObjId(5),
            seller: ObjId(6),
            charter: ObjId(9),
            title_revision: 1,
            payer: AccountId(10),
            instrument: InstrumentId::new("cr").ok_or("instrument")?,
            price: Amount::from_minor(500),
            registry: GrossRecipientCut {
                permille: 0,
                recipient: None,
            },
            venue: None,
            parents: Vec::new(),
            remainder: Recipient {
                account: AccountId(11),
                owner: mush_org::PrincipalId::Object(ObjId(6)),
            },
            not_before: stamp(100),
            expires_at: stamp(200),
        },
    };
    let row = MindRenewalOperationV1 {
        format: 1,
        operation: token(10),
        action: action(Transition::Draft, &manifest, &participants)?,
        attempt: RenewalAttemptV1::Automatic {},
        manifest,
        participants,
        binding: Box::new(binding),
        offer_digest: offer.digest()?,
        offer: Box::new(offer),
        contract: ContractId(20),
        consent_hash: [4; 32],
        created_at_ic: stamp(100),
        created_at_utc: 1000,
        state: RenewalStateV1::Drafted {},
    };
    row.validate()?;
    Ok(row)
}
fn funded(row: &MindRenewalOperationV1) -> TestResult<MindRenewalOperationV1> {
    let funding = serde_json::from_value(json!({
        "format":1,"commit":token(11),"action":row.expected_action(Transition::Fund)?,
        "contract":row.contract,"hold":31,"transaction":41,"journal_seq":51,
        "consent_hash":row.consent_hash,"at_ic":stamp(110),"posted_at_utc":1010
    }))?;
    let row = MindRenewalOperationV1 {
        state: RenewalStateV1::Funded { funding },
        ..row.clone()
    };
    row.validate()?;
    Ok(row)
}
fn terminal(row: &MindRenewalOperationV1, grant: bool) -> TestResult<MindRenewalOperationV1> {
    let funding = row.state.funding().ok_or("funding")?.clone();
    let terminal = serde_json::from_value(json!({
        "format":1,"commit":token(12),"action":row.expected_action(if grant{Transition::Grant}else{Transition::Refund})?,
        "outcome":if grant{"granted"}else{"refunded"},"transaction":42,"journal_seq":52,
        "at_ic":stamp(120),"posted_at_utc":1020,"issued_at_ic":if grant{Some(stamp(120))}else{None}
    }))?;
    let row = MindRenewalOperationV1 {
        state: if grant {
            RenewalStateV1::Granted { funding, terminal }
        } else {
            RenewalStateV1::Refunded { funding, terminal }
        },
        ..row.clone()
    };
    row.validate()?;
    Ok(row)
}
fn closed(row: &MindRenewalOperationV1, expired: bool) -> TestResult<MindRenewalOperationV1> {
    let closure = serde_json::from_value(json!({
        "commit":token(12),"action":row.expected_action(Transition::Close)?,
        "at_ic":stamp(if expired{200}else{120}),"posted_at_utc":1020,
        "reason":if expired{"expired"}else{"declined"}
    }))?;
    let row = MindRenewalOperationV1 {
        state: RenewalStateV1::UnfundedClosed { closure },
        ..row.clone()
    };
    row.validate()?;
    Ok(row)
}
fn receipt(row: &MindRenewalOperationV1) -> TestResult<MindRenewalReceiptV1> {
    let value = MindRenewalReceiptV1 {
        format: 1,
        operation: row.operation,
        licence: row.binding.licence(),
        predecessor: row.binding.predecessor(),
        next: row.binding.next(),
        contract: row.contract,
        consent_hash: row.consent_hash,
        state: row.state.clone(),
    };
    value.matches_operation(row)?;
    Ok(value)
}
fn put(value: &mut Value, path: &[&str], replacement: Value) -> TestResult {
    let (last, parents) = path.split_last().ok_or("empty path")?;
    let mut cursor = value;
    for key in parents {
        cursor = cursor.get_mut(*key).ok_or("path missing")?;
    }
    *cursor.get_mut(*last).ok_or("field missing")? = replacement;
    Ok(())
}
fn parsed(value: Value) -> Result<MindRenewalOperationV1, serde_json::Error> {
    serde_json::from_value(value)
}

#[test]
fn five_states_and_explicit_retry_roundtrip_exact_shared_data() -> TestResult {
    let draft = fixture()?;
    let fund = funded(&draft)?;
    let rows: [fn(
        &MindRenewalOperationV1,
        &MindRenewalOperationV1,
    ) -> TestResult<MindRenewalOperationV1>; 6] = [
        |draft, _| Ok(draft.clone()),
        |_, fund| Ok(fund.clone()),
        |_, fund| terminal(fund, true),
        |_, fund| terminal(fund, false),
        |draft, _| closed(draft, false),
        |draft, _| closed(draft, true),
    ];
    for make in rows {
        let row = make(&draft, &fund)?;
        let bytes = serde_json::to_vec(&row)?;
        check_eq!(MindRenewalOperationV1::decode(&bytes)?, row);
        row.matches_roll(&row.roll_projection()?)?;
        if !row.state.pending() {
            let r = receipt(&row)?;
            check_eq!(MindRenewalReceiptV1::decode(&serde_json::to_vec(&r)?)?, r);
        }
    }
    let retry = MindRenewalOperationV1 {
        attempt: RenewalAttemptV1::BuyerRetry {
            previous_operation: token(8),
        },
        ..draft
    };
    retry.validate()?;
    check_eq!(
        MindRenewalOperationV1::decode(&serde_json::to_vec(&retry)?)?,
        retry
    );
    // Structural decoding cannot prove the previous terminal or current head.
    Ok(())
}

#[test]
fn raw_decoders_reject_duplicates_unknown_null_missing_and_oversized_collections() -> TestResult {
    let row = fixture()?;
    let raw = serde_json::to_string(&row)?;
    for needle in ["\"format\":", "\"offer\":", "\"attempt\":"] {
        let value = match needle {
            "\"format\":" => "1",
            "\"offer\":" => "null",
            _ => "null",
        };
        let duplicate = raw.replacen(needle, &format!("{needle}{value},{needle}"), 1);
        check!(MindRenewalOperationV1::decode(duplicate.as_bytes()).is_err());
    }
    for key in ["attempt", "manifest", "participants", "offer", "state"] {
        let mut absent = serde_json::to_value(&row)?;
        absent.as_object_mut().ok_or("object")?.remove(key);
        check!(parsed(absent).is_err());
        let mut null = serde_json::to_value(&row)?;
        put(&mut null, &[key], Value::Null)?;
        check!(parsed(null).is_err());
    }
    let mut value = serde_json::to_value(&row)?;
    value
        .get_mut("offer")
        .and_then(|v| v.get_mut("terms"))
        .and_then(Value::as_object_mut)
        .ok_or("terms")?
        .insert("actor".into(), json!(ObjId(1)));
    check!(parsed(value).is_err());
    let mut value = serde_json::to_value(&row)?;
    put(
        &mut value,
        &["participants"],
        json!([row.participants(), row.participants()]),
    )?;
    check!(parsed(value).is_err());
    let mut value = serde_json::to_value(&row)?;
    put(
        &mut value,
        &["offer", "terms", "parents"],
        json!([
            {"charter":1,"permille":0,"title":null},{"charter":2,"permille":0,"title":null},
            {"charter":3,"permille":0,"title":null},{"charter":4,"permille":0,"title":null}
        ]),
    )?;
    check!(parsed(value).is_err());
    check!(
        MindRenewalOperationV1::decode(&vec![b' '; MindRenewalOperationV1::MAX_BYTES + 1]).is_err()
    );
    check!(MindRenewalReceiptV1::decode(&vec![b' '; MindRenewalReceiptV1::MAX_BYTES + 1]).is_err());
    Ok(())
}

#[test]
fn exact_pair_rejects_individually_valid_crossed_roll_rows() -> TestResult {
    let row = fixture()?;
    let roll = row.roll_projection()?;
    let mutations: Vec<(&[&str], Value)> = vec![
        (&["operation"], json!(token(14))),
        (&["contract"], json!(99)),
        (&["consent_hash"], json!(vec![8u8; 32])),
        (&["created_at_utc"], json!(1001)),
        (&["created_at_ic"], json!(stamp(101))),
        (
            &["attempt"],
            json!({"kind":"buyer_retry","previous_operation":token(8)}),
        ),
        (&["binding", "line"], json!(LineId::new(4)?)),
    ];
    for (path, replacement) in mutations {
        let mut value = serde_json::to_value(&roll)?;
        put(&mut value, path, replacement)?;
        let crossed: RenewalOperationV1 = serde_json::from_value(value)?;
        crossed.validate()?;
        check!(row.matches_roll(&crossed).is_err(), "crossed paired field");
    }
    Ok(())
}

#[test]
fn new_money_actions_require_exact_original_refs_but_cleanup_can_be_current() -> TestResult {
    let draft = fixture()?;
    let newer = definition("tacsim", "registry_manifest", "combat", 99)?;
    let mut crossed = draft.clone();
    crossed.manifest = newer.clone();
    check!(crossed.validate().is_err());
    let mut crossed = draft.clone();
    crossed.participants.swap(0, 1);
    check!(crossed.validate().is_err());
    let fund = funded(&draft)?;
    let mut value = serde_json::to_value(&fund)?;
    put(
        &mut value,
        &["state", "funding", "action"],
        json!(action(Transition::Fund, &newer, &draft.participants)?),
    )?;
    check!(parsed(value).is_err());
    let grant = terminal(&fund, true)?;
    let mut value = serde_json::to_value(&grant)?;
    put(
        &mut value,
        &["state", "terminal", "action"],
        json!(action(Transition::Grant, &newer, &draft.participants)?),
    )?;
    check!(parsed(value).is_err());
    let refund = terminal(&fund, false)?;
    let mut value = serde_json::to_value(&refund)?;
    put(
        &mut value,
        &["state", "terminal", "action"],
        json!(action(Transition::Refund, &newer, &draft.participants)?),
    )?;
    let data = parsed(value)?;
    receipt(&data)?;
    let mut value = serde_json::to_value(closed(&draft, false)?)?;
    put(
        &mut value,
        &["state", "closure", "action"],
        json!(action(Transition::Close, &newer, &draft.participants)?),
    )?;
    receipt(&parsed(value)?)?;
    // Cleanup data still needs actual E4 view + current registered dispatch.
    Ok(())
}

#[test]
fn repaired_offer_digest_cannot_change_the_accepted_template_or_binding() -> TestResult {
    let row = fixture()?;
    for case in 0..7 {
        let mut changed = row.clone();
        match case {
            0 => changed.offer.terms.price = Amount::from_minor(501),
            1 => changed.offer.terms.instrument = InstrumentId::new("other").ok_or("instrument")?,
            2 => changed.offer.basis.product_digest = digest(40),
            3 => changed.offer.basis.template_digest = digest(41),
            4 => changed.offer.basis.grant_economics_digest = digest(42),
            5 => changed.offer.basis.royalty_chain_digest = digest(43),
            6 => changed.offer.terms.buyer = ObjId(8),
            _ => return Err("case".into()),
        }
        changed.offer_digest = changed.offer.digest()?;
        check!(
            changed.validate().is_err(),
            "changed binding/template case {case}"
        );
    }
    Ok(())
}

#[test]
fn exact_clocks_enforce_admission_and_grant_but_refund_does_not_extend_terms() -> TestResult {
    let row = fixture()?;
    for at in [99, 200] {
        let mut changed = row.clone();
        changed.created_at_ic = stamp(at);
        check!(changed.validate().is_err());
    }
    let fund = funded(&row)?;
    let mut value = serde_json::to_value(&fund)?;
    put(
        &mut value,
        &["state", "funding", "at_ic"],
        json!(stamp(200)),
    )?;
    check!(parsed(value).is_err());
    let grant = terminal(&fund, true)?;
    let mut value = serde_json::to_value(&grant)?;
    put(
        &mut value,
        &["state", "terminal", "at_ic"],
        json!(stamp(200)),
    )?;
    put(
        &mut value,
        &["state", "terminal", "issued_at_ic"],
        json!(stamp(200)),
    )?;
    check!(parsed(value).is_err());
    let mut value = serde_json::to_value(closed(&row, true)?)?;
    put(
        &mut value,
        &["state", "closure", "at_ic"],
        json!(stamp(199)),
    )?;
    check!(parsed(value).is_err());
    let mut value = serde_json::to_value(terminal(&fund, false)?)?;
    put(
        &mut value,
        &["state", "terminal", "at_ic"],
        json!(ClockStamp {
            clock: ClockRef::WorldIc { generation: 2 },
            at: 1
        }),
    )?;
    let refund = parsed(value)?;
    receipt(&refund)?;
    check!(
        refund
            .state
            .terminal()
            .ok_or("terminal")?
            .issued_at_ic()
            .is_none()
    );
    Ok(())
}

#[test]
fn terminal_receipt_cannot_claim_pending_or_a_different_operation() -> TestResult {
    let draft = fixture()?;
    check!(receipt(&draft).is_err());
    let fund = funded(&draft)?;
    check!(receipt(&fund).is_err());
    let grant = terminal(&fund, true)?;
    let r = receipt(&grant)?;
    let mut different = r.clone();
    different.licence = LicenceId::new(8)?;
    different.validate()?;
    check!(different.matches_operation(&grant).is_err());
    let mut different = r.clone();
    different.operation = token(15);
    different.validate()?;
    check!(different.matches_operation(&grant).is_err());
    let refund = terminal(&fund, false)?;
    check!(r.matches_operation(&refund).is_err());
    let mut raw = serde_json::to_value(&r)?;
    raw.get_mut("state")
        .and_then(|v| v.get_mut("terminal"))
        .and_then(Value::as_object_mut)
        .ok_or("terminal")?
        .remove("issued_at_ic");
    check!(serde_json::from_value::<MindRenewalReceiptV1>(raw).is_err());
    Ok(())
}

#[test]
fn receipt_wire_has_only_exact_fixed_terminal_fields_and_bounds() -> TestResult {
    let row = closed(&fixture()?, false)?;
    let r = receipt(&row)?;
    let raw = serde_json::to_string(&r)?;
    let duplicate = raw.replacen(
        "\"operation\":",
        &format!(
            "\"operation\":{},\"operation\":",
            serde_json::to_string(&token(14))?
        ),
        1,
    );
    check!(MindRenewalReceiptV1::decode(duplicate.as_bytes()).is_err());
    let mut unknown = serde_json::to_value(&r)?;
    unknown
        .as_object_mut()
        .ok_or("receipt")?
        .insert("issued_at_ic".into(), json!(stamp(200)));
    check!(serde_json::from_value::<MindRenewalReceiptV1>(unknown).is_err());
    let mut value = serde_json::to_value(&r)?;
    put(&mut value, &["next"], json!(TermGeneration::new(3)?))?;
    check!(serde_json::from_value::<MindRenewalReceiptV1>(value).is_err());
    check!(serde_json::to_vec(&row)?.len() < MindRenewalOperationV1::MAX_BYTES);
    check!(serde_json::to_vec(&r)?.len() < MindRenewalReceiptV1::MAX_BYTES);
    Ok(())
}

#[path = "provider_tests.rs"]
mod provider_tests;
