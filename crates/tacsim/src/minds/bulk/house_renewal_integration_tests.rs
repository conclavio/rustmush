//! Actual granted House purchase continues through genuine offered-contract renewal.
//! No receipt/certificate is invented. All signed actions install the real owner union.
use super::*;
use mush_market::mind_house_renewal::{MindRenewalOperationV1, MindRenewalReceiptV1};
use mush_roll::commerce::house_renewal::{RenewalAttemptV1, RenewalHeadV1, RenewalStateV1};
use mush_roll::commerce::records::OwnerTerm;
use mush_roll::commerce::{TermGeneration, TermPhase, WorldIcSeconds};
#[path = "house_renewal_service_integration_tests.rs"]
mod service;
const START: u128 = 20_000_000;
const DRAFT: &str = "draft_house_mind_renewal";
const FUND: &str = "fund_house_mind_renewal";
const GRANT: &str = "settle_house_mind_renewal";
const REFUND: &str = "refund_house_mind_renewal";
const CLOSE: &str = "close_house_mind_renewal";
const OP: &str = "market_house_mind_renewal_operation";
const ROLL_OP: &str = "roll_house_mind_renewal_operation";
const HEAD: &str = "roll_house_mind_renewal_head";
const RECEIPT: &str = "market_house_mind_renewal_receipt";
fn row<T: serde::de::DeserializeOwned>(
    w: &PlayerWorld,
    owner: &str,
    family: &str,
    field: &str,
    value: Value,
) -> TestResult<T> {
    let mut all = rows(&disk(w), owner, family)
        .into_iter()
        .filter(|r| r.get(field) == Some(&value));
    let actual = all.next().ok_or("actual renewal row")?;
    check!(all.next().is_none(), "exact row is unique");
    Ok(serde_json::from_value(actual)?)
}
fn licence(w: &PlayerWorld, id: LicenceId) -> TestResult<PersonalPaidLicence> {
    row(
        w,
        "roll",
        "roll_mind_personal_paid_licence",
        "id",
        json!(id),
    )
}
fn paid(w: &PlayerWorld, id: LicenceId, generation: u64) -> TestResult<PaidTermRecord> {
    let mut all = rows(&disk(w), "roll", "roll_mind_paid_term")
        .into_iter()
        .filter(|r| {
            // A term generation is stored as its canonical decimal string.
            r.get("licence") == Some(&json!(id))
                && r.get("generation") == Some(&json!(generation.to_string()))
        });
    let value = all.next().ok_or("actual generation")?;
    check!(all.next().is_none());
    Ok(serde_json::from_value(value)?)
}
fn operation(w: &PlayerWorld, n: u128) -> TestResult<MindRenewalOperationV1> {
    row(w, "market", OP, "operation", json!(token(n)))
}
fn step(w: &mut PlayerWorld, id: &str, n: u128, params: Value) -> TestResult<Vec<ReplayWrite>> {
    perform(w, id, BUYER, n, params)
}
fn boot(registry: &Fixture, snapshot: &Durable) -> TestResult<Box<PlayerWorld>> {
    Ok(Box::new(super::boot_options(registry, snapshot, true)?))
}

/// M8c B4: the House attestation's action and its operation family.
const HOUSE_ATTEST: &str = "attest_house_mind";
const PERSONAL_ATTEST: &str = "attest_owner_mind";
const ATTEST_OPS: &str = "roll_mind_owner_attestation_operation";
/// A Roll action by id (`house_purchase`'s `action` finds the market's).
fn roll_action(w: &PlayerWorld, id: &str) -> TestResult<DefinitionRef> {
    w.engine
        .registry
        .definitions()
        .ok_or("definitions")?
        .iter()
        .map(|e| e.reference())
        .find(|r| {
            r.key().owner.as_str() == "roll"
                && r.key().kind.as_str() == "action"
                && r.key().id.as_str() == id
                && r.version() == 1
        })
        .cloned()
        .ok_or_else(|| format!("roll action {id}").into())
}
/// One direct preparation of a Roll attestation (no offer: the fence and the
/// provider are what is under test).
fn attest(
    w: &mut PlayerWorld,
    id: &str,
    actor: ObjId,
    n: u128,
    params: Value,
) -> TestResult<(
    PreparedComposedTxn,
    mush_core::action::PreparedAuthoritativeAction,
)> {
    let a = roll_action(w, id)?;
    Ok(w.engine.prepare_registered_composed_authoritative_action(
        &a,
        &request(n, actor, "roll", params)?,
        token(n),
    )?)
}
fn owner_generation(w: &PlayerWorld, owner: LicenceId, generation: u64) -> TestResult<OwnerTerm> {
    Ok(serde_json::from_value(
        rows(&disk(w), "roll", "roll_mind_owner_term")
            .into_iter()
            .find(|v| {
                v.get("licence") == Some(&json!(owner))
                    && v.get("generation") == Some(&json!(generation.to_string()))
            })
            .ok_or("actual House owner generation")?,
    )?)
}
fn current_generation(w: &PlayerWorld, owner: LicenceId) -> TestResult<Value> {
    Ok(rows(&disk(w), "roll", "roll_mind_owner_licence")
        .into_iter()
        .find(|v| v.get("id") == Some(&json!(owner)))
        .and_then(|v| v.get("term").cloned())
        .ok_or("actual House owner licence")?)
}

/// M8c B4 (the brief's test 2, here rather than in the personal attestation
/// story because the House filing lives in this world): the House filing's
/// owner licence, due at its term end, attested by its curator holding
/// `roll.holder.org` on the House's scope through the House action, writing
/// generation 2 under the House terms (M8c F10: the rig composes
/// `with_house_owner_terms` at 365 days, as the launch does, so the new
/// window is told from the personal term) with the curator recorded as the
/// acting character; refused before the term is due, refused for the
/// curator without the grant and for a second character of the curator's
/// login without it, refused by the personal action with and without the
/// organisation, refused (by its sentence) for another organisation the
/// curator is granted on and for generation 2 before it is due; the
/// retained generation survives a restart and needs the attestation
/// composed. The House is organisation 0 (F8).
pub(super) fn attest_house(
    registry: &Fixture,
    snapshot: &Durable,
    organization: OrgId,
    scope: OrgScopeId,
) -> TestResult {
    let birth = one(rows(&snapshot.disk, "roll", "roll_mind_initial_filing"))?;
    let owner: LicenceId =
        serde_json::from_value(birth.get("licence").cloned().ok_or("owner licence")?)?;
    let params = json!({"owner_licence": owner.get(), "org": organization.0});
    let personal = json!({"owner_licence": owner.get()});
    let plain = boot(registry, snapshot)?;
    check!(
        roll_action(&plain, HOUSE_ATTEST).is_err(),
        "the House attestation is composed with the owner attestations alone"
    );
    drop(plain);
    let mut w = boot_attestations(registry, snapshot)?;
    check_eq!(organization, OrgId(0), "F8: organisation 0 attests");
    check_ne!(
        roll_action(&w, HOUSE_ATTEST)?,
        roll_action(&w, PERSONAL_ATTEST)?,
        "the House action's definition is its own"
    );
    let uniform =
        "The owner licence is unavailable, not due, or no longer eligible for attestation.";
    let refusal = |r: TestResult<(
        PreparedComposedTxn,
        mush_core::action::PreparedAuthoritativeAction,
    )>|
     -> TestResult<String> { Ok(r.err().ok_or("refused")?.to_string()) };
    let first = owner_generation(&w, owner, 1)?;
    check_eq!(current_generation(&w, owner)?, json!("1"));
    check!(
        attest(&mut w, HOUSE_ATTEST, GOD, START + 200, params.clone()).is_err(),
        "refused before the term is due"
    );
    set_ic(&mut w, first.window().term_end().get())?;
    check!(
        attest(&mut w, HOUSE_ATTEST, GOD, START + 201, params.clone()).is_err(),
        "the curator's publication grant is not the holder grant"
    );
    for (n, p) in [(202, &personal), (203, &params)] {
        check!(
            attest(&mut w, PERSONAL_ATTEST, GOD, START + n, p.clone()).is_err(),
            "the personal action never attests a House filing"
        );
    }
    // A second character of the curator's login, without the grant.
    let alt = w
        .engine
        .world
        .create(ObjKind::Player, "Curator alt", GOD, Some(ROOM))?;
    {
        let mut accounts = registry.accounts.values.lock().map_err(module_error)?;
        let curator = *accounts.get(&GOD).ok_or("the curator's login")?;
        registry.accounts.epoch.invalidate();
        accounts.insert(alt, curator);
    }
    check!(
        attest(&mut w, HOUSE_ATTEST, alt, START + 204, params.clone()).is_err(),
        "the curator's alt holds no grant of its own"
    );
    command(
        &mut w,
        &format!(
            "@org/grant #{}=House {} scope=#{}",
            GOD.0,
            mush_roll::commerce::org_holder::HOLDER_ORG_CAPABILITY,
            scope.0
        ),
    )?;
    check!(
        attest(&mut w, HOUSE_ATTEST, alt, START + 205, params.clone()).is_err(),
        "the grant is the curator's, not its login's"
    );
    // F11: another organisation, the curator granted on its root, does not
    // hold the title: the uniform sentence, never a grant refusal.
    command(&mut w, "@org/create Rival")?;
    let other = w
        .engine
        .world
        .module_state::<OrgStore>()
        .ok_or("Org")?
        .state()
        .orgs()
        .find(|o| o.name.as_str() == "Rival")
        .ok_or("the other organisation")?
        .id;
    command(
        &mut w,
        &format!(
            "@org/grant #{}=Rival {}",
            GOD.0,
            mush_roll::commerce::org_holder::HOLDER_ORG_CAPABILITY
        ),
    )?;
    let error = refusal(attest(
        &mut w,
        HOUSE_ATTEST,
        GOD,
        START + 206,
        json!({"owner_licence": owner.get(), "org": other.0}),
    ))?;
    check!(
        error.contains(uniform),
        "another organisation does not hold the title: {error}"
    );
    let before = balance(
        &w,
        wallet(&w, EconomicOwner::Organization(organization), Some(scope))?,
    )?;
    let (prepared, _) = attest(&mut w, HOUSE_ATTEST, GOD, START + 210, params.clone())?;
    let writes = apply(&mut w, prepared)?;
    let families = typed_families(&writes);
    for family in [
        ATTEST_OPS,
        "roll_mind_owner_term",
        "roll_mind_owner_licence",
    ] {
        check!(families.contains(family), "{family} in {families:?}");
    }
    check_eq!(
        balance(
            &w,
            wallet(&w, EconomicOwner::Organization(organization), Some(scope))?
        )?,
        before,
        "the attestation is free"
    );
    check_eq!(current_generation(&w, owner)?, json!("2"));
    let second = owner_generation(&w, owner, 2)?;
    check_eq!(
        second.window().term_end().get() - second.window().issued_at().get(),
        HOUSE_OWNER_TERM_SECONDS,
        "the House terms (365 days), not the personal term"
    );
    check_ne!(HOUSE_OWNER_TERM_SECONDS, TERM_SECONDS);
    check!(second.window().term_end() > first.window().term_end());
    check_eq!(
        owner_generation(&w, owner, 1)?,
        first,
        "the first generation is retained"
    );
    let operation = rows(&disk(&w), "roll", ATTEST_OPS)
        .into_iter()
        .find(|v| v.get("licence") == Some(&json!(owner)))
        .ok_or("the House attestation operation")?;
    check_eq!(
        operation.get("owner"),
        Some(&json!(GOD)),
        "the acting character"
    );
    check_eq!(operation.get("format"), Some(&json!(1)));
    check_eq!(
        operation.get("house"),
        Some(&json!({"organization": organization, "scope": scope}))
    );
    let error = refusal(attest(
        &mut w,
        HOUSE_ATTEST,
        GOD,
        START + 211,
        params.clone(),
    ))?;
    check!(
        error.contains(uniform),
        "generation 2 is not due, after the grant: {error}"
    );
    let attested = saved(&w);
    let cold = boot_attestations(registry, &attested)?;
    check_eq!(current_generation(&cold, owner)?, json!("2"));
    check_eq!(owner_generation(&cold, owner, 2)?, second);
    drop(cold);
    check!(
        crate::test_checks::refused(boot(registry, &attested))?,
        "a retained House generation needs the attestation composed"
    );
    Ok(())
}
fn current_same(w: &PlayerWorld, old: &PersonalPaidLicence, term: &PaidTermRecord) -> TestResult {
    check_eq!(licence(w, old.id())?, *old);
    check_eq!(paid(w, old.id(), term.generation().get())?, *term);
    Ok(())
}
fn fresh_fences(
    registry: &Fixture,
    due: &Durable,
    drafted: &Durable,
    funded: &Durable,
    id: LicenceId,
) -> TestResult {
    for (base, action, params) in [
        (due, DRAFT, json!({"licence":id.get()})),
        (drafted, FUND, json!({"operation":token(START)})),
        (funded, GRANT, json!({"operation":token(START)})),
    ] {
        for boundary in 0..3 {
            let mut w = boot(registry, base)?;
            let before = balance(&w, wallet(&w, EconomicOwner::Object(BUYER), None)?)?;
            if boundary == 0 {
                command(&mut w, "@org/status House=suspended")?;
                check!(direct(&mut w, action, BUYER, START + 70, params.clone()).is_err());
            } else {
                let (p, o) = direct(&mut w, action, BUYER, START + 70, params.clone())?;
                if boundary == 1 {
                    command(&mut w, "@org/status House=suspended")?;
                    check!(stage(&mut w, action, BUYER, START + 70, params.clone(), p, o).is_err());
                } else {
                    stage(&mut w, action, BUYER, START + 70, params.clone(), p, o)?;
                    command(&mut w, "@org/status House=suspended")?;
                    let _ = w.engine.world.take_dirty();
                    check!(w.engine.take_staged_action().is_err());
                }
            }
            check_eq!(
                balance(&w, wallet(&w, EconomicOwner::Object(BUYER), None)?)?,
                before
            );
            check!(
                rows(&disk(&w), "market", OP) == rows(&base.disk, "market", OP),
                "no renewed rows after issuer loss"
            );
        }
    }
    for (base, action, params) in [
        (drafted, FUND, json!({"operation":token(START)})),
        (funded, GRANT, json!({"operation":token(START)})),
    ] {
        for after_stage in [false, true] {
            let mut w = boot(registry, base)?;
            let before = disk(&w);
            let (p, o) = direct(&mut w, action, BUYER, START + 71, params.clone())?;
            if after_stage {
                stage(&mut w, action, BUYER, START + 71, params.clone(), p, o)?;
                registry.accounts.epoch.invalidate();
                check!(w.engine.take_staged_action().is_err());
            } else {
                registry.accounts.epoch.invalidate();
                check!(stage(&mut w, action, BUYER, START + 71, params.clone(), p, o).is_err());
            }
            check!(
                disk(&w) == before,
                "stale account publishes no private rows"
            );
        }
    }
    Ok(())
}
#[inline(never)]
pub(super) fn exercise(
    registry: &Fixture,
    snapshot: &Durable,
    id: LicenceId,
    issuer_grace_end: i64,
) -> TestResult<Durable> {
    service::exercise(registry, snapshot, id)?;
    let mut w = boot(registry, snapshot)?;
    let old = licence(&w, id)?;
    let predecessor = paid(&w, id, 1)?;
    let birth = rows(&disk(&w), "roll", "roll_mind_house_purchase");
    let birth_economics = rows(&disk(&w), "roll", "roll_mind_house_term_economics");
    check!(
        direct(&mut w, DRAFT, BUYER, START, json!({"licence":id.get()})).is_err(),
        "no early term truncation"
    );
    set_ic(&mut w, predecessor.window().term_end().get())?;
    check!(
        w.engine.world.world_ic_now().at < issuer_grace_end,
        "issuer still has actual Grace"
    );
    let due = saved(&w);
    step(&mut w, DRAFT, START, json!({"licence":id.get()}))?;
    let draft = operation(&w, START)?;
    check!(matches!(draft.state(), RenewalStateV1::Drafted {}));
    check_eq!(draft.offer().terms.price, Amount::from_minor(300));
    check_eq!(draft.binding().term_seconds(), TERM_SECONDS - 60);
    check_eq!(draft.binding().grace_seconds(), GRACE_SECONDS - 60);
    check_eq!(
        draft.offer().terms.organization,
        draft.binding().issuer().organization()
    );
    check_eq!(draft.offer().terms.scope, draft.binding().issuer().scope());
    let contract = economy(&w)?
        .snapshot()
        .contract(draft.contract())
        .ok_or("actual offered contract")?;
    let contract_value = serde_json::to_value(contract)?;
    // The buyer issues their own renewal (the House is paid through the split).
    check_eq!(
        contract_value.get("issuer"),
        Some(&serde_json::to_value(
            draft.offer().terms.buyer.principal()
        )?)
    );
    check_eq!(
        contract.acceptances.len(),
        0,
        "Draft never accepts buyer consent"
    );
    current_same(&w, &old, &predecessor)?;
    check!(
        direct(&mut w, DRAFT, BUYER, START + 1, json!({"licence":id.get()})).is_err(),
        "automatic predecessor offered once"
    );
    let drafted = saved(&w);
    let _cold_draft = boot(registry, &drafted)?;
    check!(
        super::boot(registry, &drafted).is_err(),
        "disabled renewal cannot ignore native offer"
    );
    let payer = draft.offer().terms.payer;
    let treasury = draft.offer().terms.remainder.account;
    let money = balance(&w, payer)?;
    let treasury_money = balance(&w, treasury)?;
    let registry_money = balance(&w, WalletId(1))?;
    let writes = step(&mut w, FUND, START + 2, json!({"operation":token(START)}))?;
    check_eq!(
        typed_families(&writes),
        BTreeSet::from([OP.to_string(), ROLL_OP.to_string()])
    );
    let held = operation(&w, START)?;
    check_eq!(held.contract(), draft.contract());
    check_eq!(held.consent_hash(), draft.consent_hash());
    let RenewalStateV1::Funded { funding } = held.state() else {
        return Err("actual funded renewal".into());
    };
    let hold = funding.hold();
    check!(
        economy(&w)?
            .snapshot()
            .hold(hold)
            .ok_or("actual hold")?
            .status
            .is_held()
    );
    check_eq!(balance(&w, payer)?, (money.0, Amount::from_minor(300)));
    current_same(&w, &old, &predecessor)?;
    let funded = saved(&w);
    let _cold_funded = boot(registry, &funded)?;
    fresh_fences(registry, &due, &drafted, &funded, id)?;
    // A real held refund survives issuer loss and cannot extend the predecessor.
    let mut refund = boot(registry, &funded)?;
    command(&mut refund, "@org/status House=suspended")?;
    step(
        &mut refund,
        REFUND,
        START + 3,
        json!({"operation":token(START)}),
    )?;
    check_eq!(balance(&refund, payer)?, money);
    current_same(&refund, &old, &predecessor)?;
    let _cold_refund = boot(registry, &saved(&refund))?;
    service::retention(registry, &saved(&refund), START)?;
    check!(
        direct(
            &mut refund,
            DRAFT,
            BUYER,
            START + 4,
            json!({"previous_operation":token(START)})
        )
        .is_err()
    );
    // Fresh buyer consent is mandatory on a linked retry, even at same next generation.
    command(&mut refund, "@org/status House=active")?;
    step(
        &mut refund,
        DRAFT,
        START + 4,
        json!({"previous_operation":token(START)}),
    )?;
    let retry = operation(&refund, START + 4)?;
    check_eq!(
        retry.attempt(),
        RenewalAttemptV1::BuyerRetry {
            previous_operation: token(START)
        }
    );
    check_ne!(retry.contract(), draft.contract());
    check_ne!(retry.consent_hash(), draft.consent_hash());
    step(
        &mut refund,
        CLOSE,
        START + 5,
        json!({"operation":token(START+4),"reason":"declined"}),
    )?;
    step(
        &mut refund,
        DRAFT,
        START + 6,
        json!({"previous_operation":token(START+4)}),
    )?;
    step(
        &mut refund,
        FUND,
        START + 7,
        json!({"operation":token(START+6)}),
    )?;
    step(
        &mut refund,
        GRANT,
        START + 8,
        json!({"operation":token(START+6)}),
    )?;
    let retry_cold = boot(registry, &saved(&refund))?;
    check_eq!(
        licence(&retry_cold, id)?.term().get(),
        2,
        "closed attempts allow exact later next row"
    );
    // Original independent branch reaches actual Grant with exact seven target rows.
    let writes = step(&mut w, GRANT, START + 9, json!({"operation":token(START)}))?;
    check_eq!(
        typed_families(&writes),
        BTreeSet::from(
            [
                OP,
                ROLL_OP,
                HEAD,
                RECEIPT,
                "roll_mind_personal_paid_licence",
                "roll_mind_paid_term",
                "roll_mind_house_term_economics"
            ]
            .map(str::to_owned)
        )
    );
    let actual = licence(&w, id)?;
    let next = paid(&w, id, 2)?;
    check_eq!(actual.operation(), old.operation());
    check_eq!(actual.term().get(), 2);
    check_eq!(next.operation(), token(START));
    check_eq!(next.policy(), predecessor.policy());
    check_eq!(next.terms(), predecessor.terms());
    check_eq!(
        next.window().term_end().get() - next.window().eligibility_start().get(),
        TERM_SECONDS - 60
    );
    check_eq!(
        balance(&w, payer)?,
        (Amount::from_minor(money.0.minor() - 300), Amount::ZERO)
    );
    check_eq!(
        balance(&w, treasury)?.0,
        Amount::from_minor(treasury_money.0.minor() + 270)
    );
    check_eq!(
        balance(&w, WalletId(1))?.0,
        Amount::from_minor(registry_money.0.minor() + 30)
    );
    check!(
        !economy(&w)?
            .snapshot()
            .hold(hold)
            .ok_or("retained terminal hold")?
            .status
            .is_held()
    );
    check_eq!(paid(&w, id, 1)?, predecessor);
    check!(
        rows(&disk(&w), "roll", "roll_mind_house_purchase") == birth,
        "immutable birth remains"
    );
    check!(
        birth_economics
            .iter()
            .all(|r| rows(&disk(&w), "roll", "roll_mind_house_term_economics").contains(r)),
        "birth economics remain"
    );
    let granted = saved(&w);
    service::retention(registry, &granted, START)?;
    let cold = boot(registry, &granted)?;
    check_eq!(licence(&cold, id)?, actual);
    let receipt: MindRenewalReceiptV1 =
        row(&cold, "market", RECEIPT, "operation", json!(token(START)))?;
    check!(matches!(receipt.state(), RenewalStateV1::Granted { .. }));
    let head: RenewalHeadV1 = row(&cold, "roll", HEAD, "licence", json!(id))?;
    check_eq!(head.current(), TermGeneration::new(2)?);
    for (owner, family) in [
        ("market", OP),
        ("market", RECEIPT),
        ("roll", ROLL_OP),
        ("roll", HEAD),
    ] {
        let mut missing = granted.clone();
        check!(remove_family(&mut missing, owner, family) > 0);
        check!(
            boot(registry, &missing).is_err(),
            "missing renewal inverse accepted"
        );
    }
    let mut missing = granted.clone();
    for (owner, family) in [
        ("market", OP),
        ("market", RECEIPT),
        ("roll", ROLL_OP),
        ("roll", HEAD),
    ] {
        remove_family(&mut missing, owner, family);
    }
    check!(
        boot(registry, &missing).is_err(),
        "all missing typed history cannot hide native renewal"
    );
    let mut crossed = granted.clone();
    change_one_family(&mut crossed, "roll", ROLL_OP, |r| {
        *r.get_mut("binding")
            .and_then(|v| v.get_mut("issuer"))
            .and_then(|v| v.get_mut("scope"))
            .ok_or("scope")? = json!(draft.binding().issuer().scope().0 + 1);
        Ok(())
    })?;
    check!(boot(registry, &crossed).is_err());
    // Existing paid generation is independent of issuer loss; no automatic extension.
    set_ic(&mut w, issuer_grace_end)?;
    command(&mut w, "@org/status House=suspended")?;
    check_eq!(
        next.window()
            .phase_at(WorldIcSeconds::from_stamp(&w.engine.world.world_ic_now())?),
        TermPhase::Active
    );
    selection(&mut w, id, true)?;
    set_ic(&mut w, next.window().term_end().get())?;
    check!(
        direct(
            &mut w,
            DRAFT,
            BUYER,
            START + 10,
            json!({"licence":id.get()})
        )
        .is_err(),
        "renewal still needs live issuer"
    );
    check_eq!(
        next.window()
            .phase_at(WorldIcSeconds::from_stamp(&w.engine.world.world_ic_now())?),
        TermPhase::Grace
    );
    selection(&mut w, id, true)?;
    let independent = saved(&w);
    let mut cold = boot(registry, &independent)?;
    selection(&mut cold, id, true)?;
    set_ic(&mut cold, next.window().grace_end().get())?;
    selection(&mut cold, id, false)?;
    Ok(independent)
}

pub(super) async fn checkout(
    registry: &Fixture,
    snapshot: &Durable,
    licence: LicenceId,
) -> TestResult {
    let mut w = boot(registry, snapshot)?;
    let tasks = w.tasks.clone();
    let result: TestResult = async {
        let term = paid(&w, licence, self::licence(&w, licence)?.term().get())?;
        selection(&mut w, licence, true)?;
        for line in [
            format!("@mind/source filed:{}", licence.get()),
            "@mind/source Candidate@1".into(),
        ] {
            let output = w.say(BUYER, &line)?;
            assert!(
                PlayerWorld::private_text(&output, BUYER)
                    .contains("No such owned mind or revision.")
            );
        }
        w.start()?;
        w.board()?;
        assert_eq!(w.state()?, MatchState::Running);
        let id = match_::centre_of(&w.engine.world)
            .and_then(|c| c.match_id)
            .ok_or("running match")?;
        let record = w.record(id)?;
        let unit = record
            .teams
            .iter()
            .flat_map(|t| t.pilots.iter())
            .find_map(|(pilot, unit)| (*pilot == BUYER).then_some(*unit))
            .ok_or("customer hull")?;
        let seal = record
            .minds
            .get(&unit)
            .ok_or("actual house seal")?
            .seal
            .clone();
        assert_eq!(seal.origin.licence_label(), licence.get().to_string());
        assert_eq!(seal.engine, crate::arena::SealEngine::Scripted);
        assert!(seal.requested.origin.certificate().is_some());
        w.read_sealed_mind(&seal.mind).await?;
        let deadline = Instant::now() + WAIT;
        while !w.snapshots.contains_key(&id) {
            w.pump()?;
            if Instant::now() >= deadline {
                return Err("house opening snapshot".into());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let opening = w.snapshots.get(&id).ok_or("opening")?;
        let ground = crate::arena_bridge::ground_of(&w.engine.world, id).ok_or("ground")?;
        let arena = Snapshot::decode(&opening.document, (*ground).clone())?;
        assert_eq!(
            arena
                .units
                .iter()
                .find(|u| u.id == unit)
                .ok_or("opening unit")?
                .controller,
            Controller::Pilot
        );
        set_ic(&mut w, term.window().grace_end().get())?;
        let before = w.snapshots.get(&id).map_or(-1, |s| s.at_ms);
        w.say(BUYER, "EXIT")?;
        let first = w.progressed_snapshot(id, unit, before, 0, -1).await?;
        let (thinks, landed) = assert_captured(&w, id, unit, &seal, &first)?;
        let later = w
            .progressed_snapshot(id, unit, first.at_ms, thinks, landed)
            .await?;
        let (next_thinks, next_landed) = assert_captured(&w, id, unit, &seal, &later)?;
        assert!(
            next_thinks > thinks && next_landed > landed,
            "captured house mind runs after own expiry"
        );
        Ok(())
    }
    .await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}
