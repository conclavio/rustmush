//! Tier 2A security regression tests for `mush-economy`:
//! - Unconsented `@contract/party` multi-org officer fan-out caps.
//! - Existence oracles across `@econ/pay`, `@contract/deliver`, and `@hold`.

use mush_core::{GOD, LIMBO, ObjId, ObjKind};
use mush_economy::{AccountId, EconomySnapshot, EconomyStore, HoldId};
use mush_org::PrincipalId;
use mush_testkit::TestWorld;
use mush_testkit::fixtures::{err, ok};

fn boot() -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(mush_economy::EconomyModule::with_custody(vec![])),
    ])
}

fn snap(tw: &TestWorld) -> &EconomySnapshot {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy store")
        .snapshot()
}

fn player(tw: &mut TestWorld, name: &str) -> ObjId {
    tw.world_mut()
        .create(ObjKind::Player, name, GOD, Some(LIMBO))
        .expect("create player")
}

fn thing(tw: &mut TestWorld, name: &str) -> ObjId {
    tw.world_mut()
        .create(ObjKind::Thing, name, GOD, Some(LIMBO))
        .expect("create thing")
}

fn account_of(tw: &TestWorld, owner: &PrincipalId, instrument: &str) -> AccountId {
    snap(tw)
        .accounts()
        .find(|a| a.owner == *owner && a.instrument.as_str() == instrument)
        .expect("account")
        .id
}

fn org_id(tw: &TestWorld, name: &str) -> mush_org::OrgId {
    tw.world()
        .module_state::<mush_org::OrgStore>()
        .expect("org store")
        .state()
        .orgs()
        .find(|o| o.name == name)
        .expect("org")
        .id
}

fn latest_contract(tw: &TestWorld) -> u64 {
    snap(tw)
        .contracts()
        .map(|c| c.id.0)
        .max()
        .expect("contract")
}

fn latest_hold(tw: &TestWorld) -> HoldId {
    snap(tw).holds().map(|h| h.id).max().expect("hold")
}

#[test]
fn contract_party_caps_unconsented_org_fanout_and_preserves_legitimate_flows() {
    let mut tw = boot();
    ok(&mut tw, GOD, "@instrument/register cr=0");
    let spammer = player(&mut tw, "Spammer");
    let officer = player(&mut tw, "Officer");
    for name in ["Alpha", "Beta", "Gamma"] {
        ok(&mut tw, GOD, &format!("@org/create {name}"));
    }

    // Spammer does not act for any org: may add 1 external org, refused on 2nd.
    ok(&mut tw, spammer, "@contract/draft deal1");
    let d1 = latest_contract(&tw);
    ok(&mut tw, spammer, &format!("@contract/party {d1}=org:Alpha"));
    let second = err(&mut tw, spammer, &format!("@contract/party {d1}=org:Beta"));
    assert!(
        second.contains("do not act for"),
        "expected unconsented org cap refusal, got: {second}"
    );

    // Spammer may hold at most MAX_UNACCEPTED_CONTRACTS_PER_EXTERNAL_ORG (2)
    // unaccepted open contracts naming the same external org.
    ok(&mut tw, spammer, "@contract/draft deal2");
    let d2 = latest_contract(&tw);
    ok(&mut tw, spammer, &format!("@contract/party {d2}=org:Alpha"));
    ok(&mut tw, spammer, "@contract/draft deal3");
    let d3 = latest_contract(&tw);
    let third = err(&mut tw, spammer, &format!("@contract/party {d3}=org:Alpha"));
    assert!(
        third.contains("unaccepted open contracts"),
        "expected per-external-org cap refusal, got: {third}"
    );

    // Legitimate org officer can draft an org-to-org deal (own org + 1 external org),
    // but cannot exceed MAX_ORG_PARTIES_PER_CONTRACT (2).
    ok(
        &mut tw,
        GOD,
        &format!("@org/grant #{}=Alpha contract.accept", officer.0),
    );
    ok(&mut tw, officer, "@contract/draft org_deal");
    let od = latest_contract(&tw);
    ok(&mut tw, officer, &format!("@contract/party {od}=org:Alpha"));
    ok(&mut tw, officer, &format!("@contract/party {od}=org:Beta"));
    let third_org = err(&mut tw, officer, &format!("@contract/party {od}=org:Gamma"));
    assert!(
        third_org.contains("at most 2 organizations"),
        "expected total org cap refusal, got: {third_org}"
    );

    // TOCTOU check: officer acts for Alpha and Beta, adds both, then loses standing
    // on both before `@contract/offer` — offer must refuse.
    ok(
        &mut tw,
        GOD,
        &format!("@org/grant #{}=Beta contract.accept", officer.0),
    );
    ok(&mut tw, officer, "@contract/draft toctou");
    let td = latest_contract(&tw);
    ok(&mut tw, officer, &format!("@contract/party {td}=org:Alpha"));
    ok(&mut tw, officer, &format!("@contract/party {td}=org:Beta"));
    ok(&mut tw, GOD, "@org/status Alpha=suspended");
    ok(&mut tw, GOD, "@org/status Beta=suspended");
    let offer_err = err(&mut tw, officer, &format!("@contract/offer {td}"));
    assert!(
        offer_err.contains("do not act for"),
        "expected TOCTOU org cap refusal at offer, got: {offer_err}"
    );
    ok(&mut tw, GOD, "@org/status Alpha=active");
    ok(&mut tw, GOD, "@org/status Beta=active");

    // Staff remains unrestricted up to MAX_PARTIES.
    ok(&mut tw, GOD, "@contract/draft staff_deal");
    let sd = latest_contract(&tw);
    ok(&mut tw, GOD, &format!("@contract/party {sd}=org:Alpha"));
    ok(&mut tw, GOD, &format!("@contract/party {sd}=org:Beta"));
    ok(&mut tw, GOD, &format!("@contract/party {sd}=org:Gamma"));
}

#[test]
fn econ_pay_closes_empty_source_cross_instrument_and_asset_existence_oracles() {
    let mut tw = boot();
    ok(&mut tw, GOD, "@instrument/register cr=0");
    ok(&mut tw, GOD, "@instrument/register au=0");
    ok(&mut tw, GOD, "@account/system mint=cr");
    let mint = account_of(
        &tw,
        &PrincipalId::System(mush_org::SystemPrincipalId::new("mint").unwrap()),
        "cr",
    );

    let prober = player(&mut tw, "Prober");
    let victim = player(&mut tw, "Victim");
    ok(&mut tw, GOD, "@org/create SecretCorp");
    let corp = org_id(&tw, "SecretCorp");

    ok(&mut tw, GOD, &format!("@account/open #{}=cr", prober.0));
    ok(&mut tw, GOD, &format!("@account/open #{}=cr", victim.0));
    ok(&mut tw, GOD, &format!("@account/open #{}=au", victim.0));
    ok(&mut tw, GOD, "@account/open org:SecretCorp=cr");
    let prober_cr = account_of(&tw, &PrincipalId::Object(prober), "cr");
    let victim_cr = account_of(&tw, &PrincipalId::Object(victim), "cr");
    let victim_au = account_of(&tw, &PrincipalId::Object(victim), "au");
    let corp_cr = account_of(&tw, &PrincipalId::Organization(corp), "cr");

    // 1. Empty source account must NOT distinguish missing vs. existing vs. cross-instrument `to`.
    let e_missing = err(&mut tw, prober, &format!("@econ/pay {prober_cr}=9999 1"));
    let e_active = err(
        &mut tw,
        prober,
        &format!("@econ/pay {prober_cr}={victim_cr} 1"),
    );
    let e_diff_ccy = err(
        &mut tw,
        prober,
        &format!("@econ/pay {prober_cr}={victim_au} 1"),
    );
    assert_eq!(
        e_missing, e_active,
        "empty source leaked existence of active destination"
    );
    assert_eq!(
        e_missing, e_diff_ccy,
        "empty source leaked existence of cross-instrument destination"
    );

    // 2. Non-staff probing arbitrary org scoped account as `from` with `asset=` must NOT
    //    distinguish an org scoped account from a missing or stranger personal account.
    let f_missing = err(&mut tw, prober, "@econ/pay 9998=9999 1 asset=#99999");
    let f_personal = err(
        &mut tw,
        prober,
        &format!("@econ/pay {victim_cr}=9999 1 asset=#99999"),
    );
    let f_org = err(
        &mut tw,
        prober,
        &format!("@econ/pay {corp_cr}=9999 1 asset=#99999"),
    );
    assert_eq!(f_missing, f_personal);
    assert_eq!(
        f_missing, f_org,
        "scoped org account leaked through @econ/pay asset= branch"
    );

    // 3. Funded source paying missing vs. cross-instrument `to` must BOTH say "No such account."
    ok(&mut tw, GOD, &format!("@econ/mint {mint}={prober_cr} 500"));
    let p_missing = err(&mut tw, prober, &format!("@econ/pay {prober_cr}=9999 1"));
    let p_diff_ccy = err(
        &mut tw,
        prober,
        &format!("@econ/pay {prober_cr}={victim_au} 1"),
    );
    assert!(p_missing.contains("No such account."), "{p_missing}");
    assert_eq!(
        p_missing, p_diff_ccy,
        "cross-instrument NotBalanced leaked existence of destination account"
    );

    // 4. Funded source + `asset=` with a frozen own asset or unauthorized vault must NOT
    //    distinguish missing `to` from existing `to`.
    let item = thing(&mut tw, "Relic");
    ok(
        &mut tw,
        GOD,
        &format!("@asset/title #{}=#{}", item.0, prober.0),
    );
    let vault_box = thing(&mut tw, "SecretChest");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@vault/create secret=container:#{} owner=#{}",
            vault_box.0, victim.0
        ),
    );
    let victim_vault = snap(&tw).vaults().next().expect("vault").id;
    let v_missing = err(
        &mut tw,
        prober,
        &format!(
            "@econ/pay {prober_cr}=9999 1 asset=#{} to_vault={}",
            item.0, victim_vault.0
        ),
    );
    let v_real = err(
        &mut tw,
        prober,
        &format!(
            "@econ/pay {prober_cr}={victim_cr} 1 asset=#{} to_vault={}",
            item.0, victim_vault.0
        ),
    );
    assert!(v_missing.contains("No such vault."), "{v_missing}");
    assert_eq!(
        v_missing, v_real,
        "to_vault= leaked existence of destination account"
    );
}

#[test]
fn contract_deliver_obligation_closes_org_compartment_and_closed_account_oracles() {
    let mut tw = boot();
    ok(&mut tw, GOD, "@instrument/register cr=0");
    let prober = player(&mut tw, "Prober");
    let party = player(&mut tw, "Party");
    ok(&mut tw, GOD, "@org/create TargetOrg");
    let org = org_id(&tw, "TargetOrg");

    ok(&mut tw, GOD, &format!("@account/open #{}=cr", prober.0));
    ok(&mut tw, GOD, &format!("@account/open #{}=cr", party.0));
    ok(&mut tw, GOD, "@account/open org:TargetOrg=cr");
    let prober_acct = account_of(&tw, &PrincipalId::Object(prober), "cr");
    let party_acct = account_of(&tw, &PrincipalId::Object(party), "cr");
    let org_acct = account_of(&tw, &PrincipalId::Organization(org), "cr");

    // Draft naming TargetOrg and Party:
    ok(&mut tw, prober, "@contract/draft probe");
    let cid = latest_contract(&tw);
    ok(
        &mut tw,
        prober,
        &format!("@contract/party {cid}=org:TargetOrg"),
    );
    ok(
        &mut tw,
        prober,
        &format!("@contract/party {cid}=#{}", party.0),
    );

    // Prober cannot enumerate TargetOrg's hidden compartment account IDs via obligation deliverable.
    let e_missing = err(
        &mut tw,
        prober,
        &format!("@contract/deliver {cid}=obligation 9999 {prober_acct} 1 every=60"),
    );
    let e_org = err(
        &mut tw,
        prober,
        &format!("@contract/deliver {cid}=obligation {org_acct} {prober_acct} 1 every=60"),
    );
    assert!(e_missing.contains("No such account."), "{e_missing}");
    assert_eq!(
        e_missing, e_org,
        "org compartment account leaked through @contract/deliver obligation"
    );

    // Legitimate active personal account of a named party STILL succeeds.
    ok(
        &mut tw,
        prober,
        &format!("@contract/deliver {cid}=obligation {party_acct} {prober_acct} 1 every=60"),
    );

    // Closed personal account of a named party must be indistinguishable from a missing one.
    ok(&mut tw, GOD, &format!("@account/close {party_acct}"));
    let e_closed = err(
        &mut tw,
        prober,
        &format!("@contract/deliver {cid}=obligation {party_acct} {prober_acct} 1 every=60"),
    );
    assert_eq!(
        e_missing, e_closed,
        "closed party account leaked through @contract/deliver obligation"
    );
}

#[test]
fn hold_release_and_capture_close_hidden_escrow_masked_source_and_oversized_oracles() {
    let mut tw = boot();
    ok(&mut tw, GOD, "@instrument/register cr=0");
    ok(&mut tw, GOD, "@instrument/register au=0");
    ok(&mut tw, GOD, "@account/system mint=cr");
    let mint = account_of(
        &tw,
        &PrincipalId::System(mush_org::SystemPrincipalId::new("mint").unwrap()),
        "cr",
    );

    let payer = player(&mut tw, "Payer");
    let payee = player(&mut tw, "Payee");
    let auditor = player(&mut tw, "Auditor");
    ok(&mut tw, GOD, "@org/create CovertOrg");
    let org = org_id(&tw, "CovertOrg");

    ok(&mut tw, GOD, &format!("@account/open #{}=cr", payer.0));
    ok(&mut tw, GOD, &format!("@account/open #{}=cr", payee.0));
    ok(&mut tw, GOD, &format!("@account/open #{}=au", payee.0));
    ok(&mut tw, GOD, "@account/open org:CovertOrg=cr");
    let payer_cr = account_of(&tw, &PrincipalId::Object(payer), "cr");
    let payee_cr = account_of(&tw, &PrincipalId::Object(payee), "cr");
    let payee_au = account_of(&tw, &PrincipalId::Object(payee), "au");
    let org_cr = account_of(&tw, &PrincipalId::Organization(org), "cr");
    ok(&mut tw, GOD, &format!("@econ/mint {mint}={payer_cr} 500"));
    ok(&mut tw, GOD, &format!("@econ/mint {mint}={org_cr} 500"));

    // 1. Hidden Staff-visibility contract escrow viewed by an org account auditor:
    //    @hold/release and @hold/capture must NOT disclose the hidden contract ID or "escrow".
    ok(
        &mut tw,
        GOD,
        &format!("@org/grant #{}=CovertOrg economy.account.view", auditor.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@org/grant #{}=CovertOrg contract.accept", payer.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@org/grant #{}=CovertOrg economy.account.post", payer.0),
    );
    ok(&mut tw, GOD, "@contract/draft covert visibility=staff");
    let covert = latest_contract(&tw);
    ok(
        &mut tw,
        GOD,
        &format!("@contract/party {covert}=org:CovertOrg"),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/pay {covert}={org_cr} 100"),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/remainder {covert}={payee_cr}"),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/condition {covert}=before 3600"),
    );
    ok(&mut tw, GOD, &format!("@contract/offer {covert}"));
    ok(&mut tw, payer, &format!("@contract/accept {covert}"));
    let escrow_hold = latest_hold(&tw);

    let rel_err = err(&mut tw, auditor, &format!("@hold/release {escrow_hold}"));
    assert!(
        !rel_err.contains(&format!("#{covert}"))
            && !rel_err.to_ascii_lowercase().contains("escrow"),
        "hidden Staff contract leaked via @hold/release: {rel_err}"
    );
    let cap_err = err(
        &mut tw,
        auditor,
        &format!("@hold/capture {escrow_hold}={payee_cr} 10"),
    );
    assert!(
        !cap_err.contains(&format!("#{covert}"))
            && !cap_err.to_ascii_lowercase().contains("escrow"),
        "hidden Staff contract leaked via @hold/capture: {cap_err}"
    );

    // 2. Beneficiary capturing a manual hold whose source account `payer_cr` is masked:
    //    - Naming `payer_cr` as `to` must NOT reveal "An account cannot pay itself."
    //    - Oversized capture must NOT distinguish missing vs. existing `to`.
    ok(
        &mut tw,
        payer,
        &format!("@hold/place {payer_cr}=50 for=#{}", payee.0),
    );
    let h = latest_hold(&tw);

    let cap_missing = err(&mut tw, payee, &format!("@hold/capture {h}=9999 10"));
    let cap_self_masked = err(&mut tw, payee, &format!("@hold/capture {h}={payer_cr} 10"));
    let cap_diff_ccy = err(&mut tw, payee, &format!("@hold/capture {h}={payee_au} 10"));
    assert!(cap_missing.contains("No such account."), "{cap_missing}");
    assert_eq!(
        cap_missing, cap_self_masked,
        "masked source account leaked via self-capture refusal"
    );
    assert!(
        !cap_self_masked.contains("cannot pay itself"),
        "raw self-capture message leaked masked source account: {cap_self_masked}"
    );
    // Payee CAN view payee_au (their own account), so check oversized capture on hidden accounts:
    let over_missing = err(&mut tw, payee, &format!("@hold/capture {h}=9999 100"));
    let over_hidden = err(&mut tw, payee, &format!("@hold/capture {h}={org_cr} 100"));
    assert_eq!(
        over_missing, over_hidden,
        "oversized capture leaked existence of hidden destination account"
    );
    let _ = cap_diff_ccy;

    // 3. Released hold must return HoldNotHeld before inspecting destination accounts.
    ok(&mut tw, payer, &format!("@hold/release {h}"));
    let rel_missing = err(&mut tw, payee, &format!("@hold/capture {h}=9999 10"));
    let rel_hidden = err(&mut tw, payee, &format!("@hold/capture {h}={org_cr} 10"));
    assert_eq!(
        rel_missing, rel_hidden,
        "released hold leaked existence of hidden destination account"
    );
}
