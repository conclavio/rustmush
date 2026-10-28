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

/// 17g C2d test 1 (C2 plan §2.1's renewal exception): the factored player
/// branch keeps `@mindrenewal` and `@housemindrenewal` byte-identical.
/// - Status: for no operation and for every state of a real operation row
///   (drafted, funded, granted, refunded, declined and expired), each
///   service's status sentence through `player_step`'s view
///   (`status_view`, then the command's own `command_line`, the one call
///   `command` prints) is exactly the pre-C2d sentence, recomputed here from
///   the row by the pre-C2d code (the oracle below); each command's line is
///   its own service's sentence, never the other's (the C2d fix round,
///   reviewer T2).
/// - The submitting switches print exactly the pre-C2d queued and
///   unresolved lines, and the command's switches select the same branch
///   as before (`Switches::of_command`); `player_step`'s `Request` is the
///   request branch and, once attempted, the retry branch.
/// - `player_step`'s Status view carries the drafted offer's frozen price,
///   instrument, expiry and term, and the current term's end.
///
/// The service's own admission, entry, key and submission are unchanged
/// code (`step`); its live composition is the renewal services' integration
/// stories' (`tacsim::minds::bulk::renewal_service_integration_tests`). Not
/// proven here: the command reading its switches through
/// `Switches::of_command` and the term end from `own`'s entry under a
/// dispatched context; those stories run `@mindrenewal` and
/// `@housemindrenewal` for real and check their sentences by substring.
#[test]
fn renewal_player_step_keeps_the_service_commands_byte_identical() -> TestResult {
    use super::player::{RenewalPlayerOutcome, RenewalPlayerStep, Switches};
    use super::service::command_line as personal_line;
    use crate::mind_house_renewal::command_line as house_line;
    let term_end = 130;
    personal_status_is_byte_identical(term_end)?;
    house_status_is_byte_identical(term_end)?;

    // The submitting branches' two lines, for both services.
    for line in [personal_line, house_line] {
        check_eq!(
            line(RenewalPlayerOutcome::Queued),
            "Renewal request queued. Status or its acknowledged receipt establishes completion."
        );
        check_eq!(
            line(RenewalPlayerOutcome::Unresolved),
            "A renewal request is still unresolved; inspect status before retrying."
        );
    }

    // The command's switches select exactly what they did; player_step's
    // steps select the same branches (Request: request, then retry).
    let of = |names: &[&str]| {
        Switches::of_command(&mush_core::SwitchSet::from_names(names.iter().copied()))
    };
    let s = |status, request, accept, decline, retry| Switches {
        status,
        request,
        accept,
        decline,
        retry,
    };
    check_eq!(of(&[]), s(true, false, false, false, false));
    check_eq!(of(&["status"]), s(true, false, false, false, false));
    check_eq!(of(&["request"]), s(false, true, false, false, false));
    check_eq!(of(&["accept"]), s(false, false, true, false, false));
    check_eq!(of(&["decline"]), s(false, false, false, true, false));
    check_eq!(of(&["retry"]), s(false, false, false, false, true));
    check_eq!(
        Switches::of(RenewalPlayerStep::Status),
        s(true, false, false, false, false)
    );
    check_eq!(
        Switches::of(RenewalPlayerStep::Request),
        s(false, true, false, false, true)
    );
    check_eq!(Switches::of(RenewalPlayerStep::Accept), of(&["accept"]));
    check_eq!(Switches::of(RenewalPlayerStep::Decline), of(&["decline"]));
    Ok(())
}

/// The personal half of
/// `renewal_player_step_keeps_the_service_commands_byte_identical`'s status
/// proof, apart from it so each frame stays under the M7 memory-size limit
/// (`.github/clippy-m7`).
fn personal_status_is_byte_identical(term_end: i64) -> TestResult {
    use super::player::RenewalPlayerOutcome;
    use super::service::command_line as personal_line;
    // The pre-C2d status branch of `@mindrenewal` (the oracle).
    let personal_before = |row: Option<&MindRenewalOperationV1>| -> String {
        let label = match row.map(|r| r.state()) {
            None => "No renewal offer has been requested.",
            Some(RenewalStateV1::Drafted {}) => {
                "A renewal offer awaits your explicit acceptance or decline."
            }
            Some(RenewalStateV1::Funded { .. }) => "Payment is held; completion is pending.",
            Some(RenewalStateV1::Granted { .. }) => "Renewal completed.",
            Some(_) => {
                "The last renewal closed without a new term. You may explicitly request a retry."
            }
        };
        let details=row.map(|row| format!(" Price: {} minor units of {}; offer expires at world-IC {}. Term: {} seconds plus {} seconds grace. Registry cut: {} per mille; venue cut: {} per mille; parent royalties: {} per mille.",
            row.offer().terms.price.minor(),row.offer().terms.instrument,row.offer().terms.expires_at.at,
            row.binding().term_seconds(),row.binding().grace_seconds(),row.offer().terms.registry.permille,
            row.offer().terms.venue.as_ref().map_or(0,|c|c.permille),row.offer().terms.parents.iter().map(|p|u32::from(p.permille)).sum::<u32>())).unwrap_or_default();
        format!("{label}{details}")
    };
    // Personal: no operation, then every state of one real row.
    let drafted = fixture()?;
    for (row, state) in &personal_rows(&drafted)? {
        let view = super::service::status_view(term_end, row.as_ref());
        check_eq!(view.state(), *state);
        check_eq!(view.term_end_ic(), term_end);
        check_eq!(
            personal_line(RenewalPlayerOutcome::Status(view.clone())),
            personal_before(row.as_ref()),
            "{state:?}"
        );
    }
    // The drafted offer's frozen terms reach the view.
    let view = super::service::status_view(term_end, Some(&drafted));
    check_eq!(
        view.price_minor(),
        Some(drafted.offer().terms.price.minor())
    );
    check_eq!(
        view.instrument(),
        Some(drafted.offer().terms.instrument.to_string().as_str())
    );
    check_eq!(
        view.offer_expires_ic(),
        Some(drafted.offer().terms.expires_at.at)
    );
    check_eq!(view.term_seconds(), Some(drafted.binding().term_seconds()));
    check_eq!(
        view.grace_seconds(),
        Some(drafted.binding().grace_seconds())
    );
    check!(view.price_minor().is_some_and(|p| p > 0), "a real price");
    let none = super::service::status_view(term_end, None);
    check_eq!(none.price_minor(), None);
    check_eq!(none.offer_expires_ic(), None);
    Ok(())
}

/// The House half of
/// `renewal_player_step_keeps_the_service_commands_byte_identical`'s status
/// proof (see `personal_status_is_byte_identical`).
fn house_status_is_byte_identical(term_end: i64) -> TestResult {
    use super::player::RenewalPlayerOutcome;
    use super::service::command_line as personal_line;
    use crate::mind_house_renewal::command_line as house_line;
    use crate::mind_house_renewal::tests as house;
    // The pre-C2d status branch of `@housemindrenewal` (the oracle).
    let house_before = |row: Option<&crate::mind_house_renewal::MindRenewalOperationV1>| -> String {
        use mush_roll::commerce::house_renewal::RenewalStateV1 as State;
        let label = match row.map(|r| r.state()) {
            None => "No renewal offer has been requested.",
            Some(State::Drafted {}) => {
                "A renewal offer awaits your explicit acceptance or decline."
            }
            Some(State::Funded { .. }) => "Payment is held; completion is pending.",
            Some(State::Granted { .. }) => "Renewal completed.",
            Some(_) => {
                "The last renewal closed without a new term. You may explicitly request a retry."
            }
        };
        let details=row.map(|row| format!(" Price: {} minor units of {}; offer expires at world-IC {}. Term: {} seconds plus {} seconds grace. Registry cut: {} per mille; venue cut: {} per mille.",
            row.offer().terms.price.minor(),row.offer().terms.instrument,row.offer().terms.expires_at.at,
            row.binding().term_seconds(),row.binding().grace_seconds(),row.offer().terms.registry.permille,
            row.offer().terms.venue.as_ref().map_or(0,|c|c.permille))).unwrap_or_default();
        format!("{label}{details}")
    };
    // House: the same, against the House service's own row and sentence.
    let drafted = house::fixture()?;
    for (row, state) in &house_rows(&drafted)? {
        let view = crate::mind_house_renewal::status_view(term_end, row.as_ref());
        check_eq!(view.state(), *state);
        check_eq!(
            house_line(RenewalPlayerOutcome::Status(view.clone())),
            house_before(row.as_ref()),
            "House {state:?}"
        );
        if row.is_some() {
            check!(
                house_line(RenewalPlayerOutcome::Status(view.clone()))
                    != personal_line(RenewalPlayerOutcome::Status(view.clone())),
                "the House command prints its own sentence"
            );
        }
    }
    let view = crate::mind_house_renewal::status_view(term_end, Some(&drafted));
    check_eq!(
        view.price_minor(),
        Some(drafted.offer().terms.price.minor())
    );
    check_eq!(
        view.offer_expires_ic(),
        Some(drafted.offer().terms.expires_at.at)
    );
    check_eq!(view.parent_royalty_permille(), Some(0));
    Ok(())
}

/// No operation, then every state of one real personal row, each with the
/// state its status reads; a `Vec` built apart so the rows stay off the
/// status proof's frame.
fn personal_rows(
    drafted: &MindRenewalOperationV1,
) -> TestResult<Vec<(Option<MindRenewalOperationV1>, super::player::RenewalState)>> {
    use super::player::RenewalState;
    let funded = funded(drafted)?;
    Ok(vec![
        (None, RenewalState::None),
        (Some(drafted.clone()), RenewalState::Drafted),
        (Some(funded.clone()), RenewalState::Funded),
        (Some(terminal(&funded, true)?), RenewalState::Granted),
        (Some(terminal(&funded, false)?), RenewalState::Closed),
        (Some(closed(drafted, false)?), RenewalState::Closed),
        (Some(closed(drafted, true)?), RenewalState::Closed),
    ])
}

/// The House service's rows, as `personal_rows`.
fn house_rows(
    drafted: &crate::mind_house_renewal::MindRenewalOperationV1,
) -> TestResult<
    Vec<(
        Option<crate::mind_house_renewal::MindRenewalOperationV1>,
        super::player::RenewalState,
    )>,
> {
    use super::player::RenewalState;
    use crate::mind_house_renewal::tests as house;
    let funded = house::funded(drafted)?;
    Ok(vec![
        (None, RenewalState::None),
        (Some(drafted.clone()), RenewalState::Drafted),
        (Some(funded.clone()), RenewalState::Funded),
        (Some(house::terminal(&funded, true)?), RenewalState::Granted),
        (Some(house::terminal(&funded, false)?), RenewalState::Closed),
        (Some(house::closed(drafted, false)?), RenewalState::Closed),
        (Some(house::closed(drafted, true)?), RenewalState::Closed),
    ])
}

/// 17g C2h (the lead's ruling R3, the C2e plan §5.1): the market's `front`
/// arm, over the C2d test 1 oracle's real operation rows of both services.
/// - Each state's `+minds` words, through the arm's own pieces
///   (`status_view`, then `front_request`, `front_line`): with no operation,
///   or after a grant or a closure, `RENEW_NOT_DUE` with the whole IC days
///   left while the term runs and the request-or-retry step after it; a
///   drafted offer `RENEW_OFFER` (its frozen price, renewed term and Grace in
///   whole IC days, the IC hours left) and `RENEW_HOW`, submitting nothing;
///   a funded one `RENEW_FUNDED`; the submitting lines `RENEWING`,
///   `RENEW_PAYING` with the drafted row's frozen price, and
///   `RENEW_DECLINING`, verbatim the C2d words.
/// - A wrong operation is refused (`front_bound`): pay and decline act only
///   on the licence's exact last operation, the one the holder saw.
/// - A typed `@mindrenewal/front` line and TacSim's continuation print the
///   same words: the continuation's argument is `front_argument`, and
///   `parse_front` reads it back to the same licence and step for every
///   step, so the one handler takes the one path either way; any other
///   spelling is refused (the uniform refusal).
/// - `submit` reporting that it sent nothing is `Unresolved`
///   (`sent_outcome`): `Submission::may_send`, the one rule both services'
///   `submit` decide by, sends nothing while the retained entry's earlier
///   attempt is open or while the actor already has an authoritative
///   submission awaiting its receipt (`authoritative_pending`); the arm
///   prints no request line for it (`front_submitted`), and `@mindrenewal`
///   prints `UNRESOLVED_LINE`, where before C2h a pending authoritative
///   submission read `QUEUED_LINE`. That `step` hands `submit`'s answer to
///   `sent_outcome` is not a fast test (it needs a dispatched context over a
///   granted licence, which the fast suites cannot build); it is structural:
///   `Submission` is `#[must_use]`, so the pre-C2h spelling that dropped it
///   (`submit(..)?;` then `Queued`) is a warning the workspace's
///   `-D warnings` refuses.
///
/// The dispatched flow (the delegation, the continuation, the real reads and
/// submissions) is the player market story's part 3; the delegation rule is
/// mush-core's `continuation::tests`.
#[test]
fn the_front_prints_the_minds_words_for_every_state_and_binds_the_operation() -> TestResult {
    use super::player::{
        FrontAnswer, FrontStep, RenewalPlayerOutcome, Submission, front_argument, front_bound,
        front_line, front_request, front_submitted, parse_front, sent_outcome,
    };
    use super::service::command_line as personal_line;
    let term_end = 130;
    let drafted = fixture()?;
    let licence = drafted.binding().licence();
    let shown = licence.get().to_string();
    check_eq!(shown, "7");
    let not_due = "Licence 7 is not due for renewal; its term has 1 IC days left.".to_owned();
    for (row, state) in &personal_rows(&drafted)? {
        let view = super::service::status_view(term_end, row.as_ref());
        let before = front_request(&shown, &view, term_end - 30);
        let after = front_request(&shown, &view, term_end);
        match state {
            super::player::RenewalState::Drafted => {
                let lines = vec![
                    "Licence 7 renews for 5cr (1+1 IC days); the offer is open 1 more IC hours."
                        .to_owned(),
                    "Pay it with +minds/renew/pay 7, or turn it down with +minds/renew/decline 7."
                        .to_owned(),
                ];
                check_eq!(front_request(&shown, &view, 150), FrontAnswer::Say(lines));
                check_eq!(before, after, "a drafted offer shows whatever the term");
            }
            super::player::RenewalState::Funded => {
                let funded = FrontAnswer::Say(vec![
                    "Your renewal payment for licence 7 is held; the renewal completes on its own."
                        .to_owned(),
                ]);
                check_eq!(before, funded.clone());
                check_eq!(after, funded);
            }
            _ => {
                check_eq!(before, FrontAnswer::Refuse(not_due.clone()), "{state:?}");
                check_eq!(after, FrontAnswer::Ask, "{state:?}");
            }
        }
    }
    let price = drafted.offer().terms.price.minor();
    let operation = drafted.operation();
    check_eq!(
        front_line(&shown, FrontStep::Request, 0),
        "Asking for a renewal offer on licence 7. +minds/renew 7 shows it once it is confirmed."
    );
    check_eq!(
        front_line(&shown, FrontStep::Accept(operation), price),
        "Paying 5cr for the renewal of licence 7 into escrow; it completes on its own."
    );
    check_eq!(
        front_line(&shown, FrontStep::Decline(operation), price),
        "Turning down the renewal offer on licence 7."
    );
    // The operation binds the step to the offer the holder saw.
    let other = token(99);
    for step in [FrontStep::Accept(operation), FrontStep::Decline(operation)] {
        check!(front_bound(step, Some(operation)));
        check!(!front_bound(step, None), "{step:?} with no operation");
    }
    for step in [FrontStep::Accept(other), FrontStep::Decline(other)] {
        check!(
            !front_bound(step, Some(operation)),
            "{step:?}: another operation"
        );
    }
    check!(front_bound(FrontStep::Request, None));
    // A typed `/front` and the continuation take the one path.
    for step in [
        FrontStep::Request,
        FrontStep::Accept(operation),
        FrontStep::Decline(operation),
    ] {
        let argument = front_argument(licence, step);
        check_eq!(parse_front(&argument), Some((licence, step)), "{argument}");
    }
    check_eq!(
        front_argument(licence, FrontStep::Accept(operation)),
        format!("7 accept {operation}")
    );
    let text = operation.to_string();
    for bad in [
        String::new(),
        "7".to_owned(),
        "07 request".to_owned(),
        "0 request".to_owned(),
        "+7 request".to_owned(),
        "7  request".to_owned(),
        " 7 request".to_owned(),
        "7 request ".to_owned(),
        "7 request now".to_owned(),
        "7 REQUEST".to_owned(),
        "7 pay".to_owned(),
        "7 accept".to_owned(),
        format!("7 accept {}", text.to_uppercase()),
        format!("7 accept {}", text.get(1..).unwrap_or_default()),
        format!("7 accept {text}0"),
        format!("7 decline {text} more"),
        format!("7 request {text}"),
    ] {
        check_eq!(parse_front(&bad), None, "{bad:?}");
    }
    // A step that sent nothing: the rule `submit` decides by, then its
    // outcome.
    check!(Submission::may_send(false, false), "nothing pending: sent");
    for (attempt, authoritative) in [(true, false), (false, true), (true, true)] {
        check!(
            !Submission::may_send(attempt, authoritative),
            "attempt open {attempt}, authoritative pending {authoritative}: nothing sent"
        );
    }
    check_eq!(sent_outcome(Submission::Sent), RenewalPlayerOutcome::Queued);
    check_eq!(
        sent_outcome(Submission::Nothing),
        RenewalPlayerOutcome::Unresolved
    );
    let line = front_line(&shown, FrontStep::Request, 0);
    check_eq!(
        front_submitted(&RenewalPlayerOutcome::Queued, line.clone()),
        Some(line.clone())
    );
    check_eq!(
        front_submitted(&RenewalPlayerOutcome::Unresolved, line),
        None
    );
    check_eq!(
        personal_line(sent_outcome(Submission::Nothing)),
        "A renewal request is still unresolved; inspect status before retrying."
    );
    house_front_words(term_end)
}

/// The House half of
/// `the_front_prints_the_minds_words_for_every_state_and_binds_the_operation`:
/// the House service's real rows read the same `+minds` words (the arm is
/// the same code over the House service's own row), its frozen price among
/// them, and its `@housemindrenewal` prints `UNRESOLVED_LINE` for a step
/// that sent nothing.
fn house_front_words(term_end: i64) -> TestResult {
    use super::player::{FrontAnswer, FrontStep, RenewalState, front_line, front_request};
    use crate::mind_house_renewal::command_line as house_line;
    use crate::mind_house_renewal::tests as house;
    let drafted = house::fixture()?;
    let shown = drafted.binding().licence().get().to_string();
    for (row, state) in &house_rows(&drafted)? {
        let view = crate::mind_house_renewal::status_view(term_end, row.as_ref());
        let before = front_request(&shown, &view, term_end - 30);
        match state {
            RenewalState::Drafted => check_eq!(
                before,
                FrontAnswer::Say(vec![
                    "Licence 7 renews for 5cr (1+1 IC days); the offer is open 1 more IC hours."
                        .to_owned(),
                    "Pay it with +minds/renew/pay 7, or turn it down with +minds/renew/decline 7."
                        .to_owned(),
                ])
            ),
            RenewalState::Funded => check_eq!(
                before,
                FrontAnswer::Say(vec![
                    "Your renewal payment for licence 7 is held; the renewal completes on its own."
                        .to_owned()
                ])
            ),
            _ => {
                check_eq!(
                    before,
                    FrontAnswer::Refuse(
                        "Licence 7 is not due for renewal; its term has 1 IC days left.".to_owned()
                    ),
                    "House {state:?}"
                );
                check_eq!(front_request(&shown, &view, term_end), FrontAnswer::Ask);
            }
        }
    }
    check_eq!(
        front_line(
            &shown,
            FrontStep::Accept(drafted.operation()),
            drafted.offer().terms.price.minor()
        ),
        "Paying 5cr for the renewal of licence 7 into escrow; it completes on its own."
    );
    check_eq!(
        house_line(super::player::sent_outcome(
            super::player::Submission::Nothing
        )),
        "A renewal request is still unresolved; inspect status before retrying."
    );
    Ok(())
}

#[path = "provider_tests.rs"]
mod provider_tests;
