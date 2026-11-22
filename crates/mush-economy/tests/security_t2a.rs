//! Tier 2A security regression tests for `mush-economy`:
//! - Unconsented `@contract/party` multi-org officer fan-out caps.
//! - Existence oracles across `@econ/pay`, `@contract/deliver`, and `@hold`.

use std::collections::HashMap;
use std::sync::Arc;

use mush_core::module::{ModuleBootData, WorldEvent};
use mush_core::{Deferred, Effects, GOD, LIMBO, ModuleOp, ModuleWrite, ObjId, ObjKind, Outbox};
use mush_economy::{
    AccountId, AuthEval, ContractAuth, ContractId, ContractState, EconomySnapshot, EconomyStore,
    HoldId, ObligationCause, ObligationStatus,
};
use mush_org::PrincipalId;
use mush_testkit::TestWorld;
use mush_testkit::fixtures::{err, ok};
use smol_str::SmolStr;

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

fn boot_full() -> TestWorld {
    let vocab: std::collections::BTreeSet<mush_org::CapabilityId> = mush_economy::CAPABILITIES
        .iter()
        .map(|s| mush_org::CapabilityId::new(*s).expect("capability"))
        .collect();
    TestWorld::with_modules(vec![
        Box::new(
            mush_org::OrgModule::new()
                .with_capabilities(vocab)
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(
            mush_economy::EconomyModule::with_custody(vec![])
                .with_canon_outcome_kinds(vec![SmolStr::new_static("op.done")]),
        ),
    ])
}

fn sweep_at(tw: &mut TestWorld, t: i64) {
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1000));
}

fn bal(tw: &TestWorld, id: AccountId) -> i64 {
    snap(tw)
        .account(id)
        .expect("account")
        .posted_balance
        .minor()
}

fn fold_kv(tw: &mut TestWorld) -> HashMap<SmolStr, ModuleBootData> {
    let mut by_ns: HashMap<SmolStr, Vec<(SmolStr, serde_json::Value)>> = HashMap::new();
    for op in tw.take_module_ops() {
        let kv = by_ns.entry(op.ns).or_default();
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                kv.retain(|(k, _)| *k != key);
                kv.push((key, value));
            }
            ModuleWrite::Del { key } => kv.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    by_ns
        .into_iter()
        .map(|(ns, kv)| {
            (
                ns,
                ModuleBootData {
                    kv,
                    streams: HashMap::new(),
                    ..Default::default()
                },
            )
        })
        .collect()
}

fn reboot_all(tw: &mut TestWorld, boot: HashMap<SmolStr, ModuleBootData>) {
    tw.try_cold_loaded(boot).expect("cold load");
}

fn org_scope_id(tw: &TestWorld, org: mush_org::OrgId, name: &str) -> mush_org::OrgScopeId {
    tw.world()
        .module_state::<mush_org::OrgStore>()
        .expect("org store")
        .state()
        .scopes_in(org)
        .find(|s| s.name == name)
        .expect("scope")
        .id
}

fn emit_canon(tw: &mut TestWorld, canon_id: u64, kind: &str, source: &str, entities: &[ObjId]) {
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    out.defer(Deferred::Persist(ModuleOp {
        ns: SmolStr::new_static("canon"),
        op: ModuleWrite::JsonSet {
            key: format!("log:canon:{canon_id}").into(),
            value: serde_json::json!({ "id": canon_id, "kind": kind, "source": source }),
        },
    }));
    tw.engine_mut().emit_world_event(
        &WorldEvent::CanonRecorded {
            canon_id,
            kind: kind.to_string(),
            entities: entities.to_vec(),
            source: source.to_string(),
        },
        &mut out,
        &mut fx,
    );
    let _ = tw.take_module_ops();
}

#[test]
fn p2_mod_1_contract_accept_enforces_deliverable_authority_cadence_and_non_lease_termination() {
    let mut tw = boot_full();
    tw.set_now(1_000);
    ok(&mut tw, GOD, "@instrument/register cr=0");
    ok(&mut tw, GOD, "@account/system mint=cr");
    let mint = account_of(
        &tw,
        &PrincipalId::System(mush_org::SystemPrincipalId::new("mint").unwrap()),
        "cr",
    );

    let mallory = player(&mut tw, "Mallory");
    let officer = player(&mut tw, "Officer");
    let bob = player(&mut tw, "Bob");
    ok(&mut tw, GOD, "@org/create VaultCorp");
    let corp = org_id(&tw, "VaultCorp");
    ok(&mut tw, GOD, "@org/scope create VaultCorp=SubDesk");
    let root_scope = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("orgs")
        .state()
        .org(corp)
        .expect("corp")
        .root_scope;
    let sub_scope = org_scope_id(&tw, corp, "SubDesk");

    // Root treasury + root asset live in `root_scope`; sub_acct lives in `sub_scope`.
    ok(
        &mut tw,
        GOD,
        &format!("@account/open org:VaultCorp/{}=cr", root_scope.0),
    );
    let root_treasury = snap(&tw)
        .accounts()
        .find(|a| a.owner == PrincipalId::Organization(corp) && a.scope == Some(root_scope))
        .expect("root treasury")
        .id;
    ok(
        &mut tw,
        GOD,
        &format!("@account/open org:VaultCorp/{}=cr", sub_scope.0),
    );
    let sub_acct = snap(&tw)
        .accounts()
        .find(|a| a.owner == PrincipalId::Organization(corp) && a.scope == Some(sub_scope))
        .expect("sub acct")
        .id;
    ok(&mut tw, GOD, &format!("@account/open #{}=cr", mallory.0));
    ok(&mut tw, GOD, &format!("@account/open #{}=cr", bob.0));
    let mallory_acct = account_of(&tw, &PrincipalId::Object(mallory), "cr");
    let bob_acct = account_of(&tw, &PrincipalId::Object(bob), "cr");
    ok(
        &mut tw,
        GOD,
        &format!("@econ/mint {mint}={root_treasury} 10000"),
    );
    ok(&mut tw, GOD, &format!("@econ/mint {mint}={sub_acct} 5000"));
    ok(&mut tw, GOD, &format!("@econ/mint {mint}={bob_acct} 2000"));

    let root_asset = thing(&mut tw, "CrownJewel");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@asset/title #{}=org:VaultCorp scope=#{}",
            root_asset.0, root_scope.0
        ),
    );

    // Mallory holds contract.draft + contract.accept + asset.title.transfer ONLY on SubDesk,
    // plus economy.account.view on root (NOT economy.account.post or asset.title.transfer on root!).
    for cap in ["contract.draft", "contract.accept", "asset.title.transfer"] {
        ok(
            &mut tw,
            GOD,
            &format!(
                "@org/grant #{}=VaultCorp {cap} scope=#{}",
                mallory.0, sub_scope.0
            ),
        );
    }
    ok(
        &mut tw,
        GOD,
        &format!(
            "@org/grant #{}=VaultCorp economy.account.view scope=#{}",
            mallory.0, root_scope.0
        ),
    );

    // 1. Sub-minute recurring obligation cadence (< 60s) is rejected at draft time.
    ok(&mut tw, mallory, "@contract/draft FastPersonal");
    let fast_personal = latest_contract(&tw);
    ok(
        &mut tw,
        mallory,
        &format!("@contract/party {fast_personal}=#{}", bob.0),
    );
    err(
        &mut tw,
        mallory,
        &format!(
            "@contract/deliver {fast_personal}=obligation {bob_acct} {mallory_acct} 10 every=1"
        ),
    );
    err(
        &mut tw,
        mallory,
        &format!(
            "@contract/deliver {fast_personal}=obligation {bob_acct} {mallory_acct} 10 every=59"
        ),
    );
    ok(
        &mut tw,
        mallory,
        &format!("@contract/cancel {fast_personal}"),
    );

    // 2. Zero-consideration Deliverable::Obligation debiting root_treasury MUST fail
    //    when Mallory tries to accept on behalf of VaultCorp via her SubDesk `contract.accept` grant!
    ok(
        &mut tw,
        GOD,
        &format!("@contract/draft FastDrain scope=#{}", sub_scope.0),
    );
    let fast_id = latest_contract(&tw);
    ok(
        &mut tw,
        GOD,
        &format!("@contract/party {fast_id}=#{}", bob.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/deliver {fast_id}=obligation {root_treasury} {bob_acct} 500 every=60"),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/remainder {fast_id}={bob_acct}"),
    );
    ok(&mut tw, GOD, &format!("@contract/offer {fast_id}"));
    err(&mut tw, mallory, &format!("@contract/accept {fast_id}"));
    assert!(
        !snap(&tw)
            .contract(ContractId(fast_id))
            .expect("c")
            .accepted_by(&PrincipalId::Organization(corp)),
        "unauthorized obligation deliverable must not record org acceptance"
    );
    assert_eq!(
        snap(&tw).obligations().count(),
        0,
        "no obligation may be created on refused acceptance"
    );

    // 3. SubDesk `contract.accept` + `asset.title.transfer` on SubDesk CANNOT steal
    //    or encumber `root_asset` living in `root_scope`!
    ok(
        &mut tw,
        GOD,
        &format!("@contract/draft AssetTheft scope=#{}", sub_scope.0),
    );
    let theft_id = latest_contract(&tw);
    ok(
        &mut tw,
        GOD,
        &format!("@contract/party {theft_id}=#{}", bob.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@contract/deliver {theft_id}=title #{} org:VaultCorp->#{}",
            root_asset.0, bob.0
        ),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/remainder {theft_id}={bob_acct}"),
    );
    ok(&mut tw, GOD, &format!("@contract/offer {theft_id}"));
    err(&mut tw, mallory, &format!("@contract/accept {theft_id}"));
    assert!(
        !snap(&tw)
            .contract(ContractId(theft_id))
            .expect("c")
            .accepted_by(&PrincipalId::Organization(corp)),
        "SubDesk officer must not accept TitleTransfer on root_scope asset"
    );

    ok(
        &mut tw,
        GOD,
        &format!("@contract/draft AssetLock scope=#{}", sub_scope.0),
    );
    let lock_id = latest_contract(&tw);
    ok(
        &mut tw,
        GOD,
        &format!("@contract/party {lock_id}=#{}", bob.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@contract/deliver {lock_id}=collateral #{} org:VaultCorp",
            root_asset.0
        ),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/remainder {lock_id}={bob_acct}"),
    );
    ok(&mut tw, GOD, &format!("@contract/offer {lock_id}"));
    err(&mut tw, mallory, &format!("@contract/accept {lock_id}"));
    assert!(
        snap(&tw)
            .asset(root_asset)
            .expect("asset")
            .encumbrances
            .is_empty(),
        "SubDesk officer must not encumber root_scope asset"
    );

    // 4. `@contract/amend` + `@contract/deliver` + `@contract/offer` cannot bypass
    //    acceptance deliverable authorization via a surviving acceptance!
    //    Give an officer `economy.account.post` + `contract.accept` + `contract.draft` on root
    //    (NO `asset.title.transfer` on root!), accept an empty draft, amend (acceptance survives
    //    under Option A'), add a TitleTransfer on `root_asset`, and try to re-offer.
    let amender = player(&mut tw, "Amender");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@org/grant #{}=VaultCorp economy.account.post scope=#{}",
            amender.0, root_scope.0
        ),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@org/grant #{}=VaultCorp contract.accept scope=#{}",
            amender.0, root_scope.0
        ),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@org/grant #{}=VaultCorp contract.draft scope=#{}",
            amender.0, root_scope.0
        ),
    );
    ok(
        &mut tw,
        amender,
        &format!("@contract/draft AmendBypass scope=#{}", root_scope.0),
    );
    let amend_id = latest_contract(&tw);
    ok(
        &mut tw,
        amender,
        &format!("@contract/party {amend_id}=#{}", bob.0),
    );
    ok(
        &mut tw,
        amender,
        &format!("@contract/remainder {amend_id}={bob_acct}"),
    );
    ok(&mut tw, amender, &format!("@contract/offer {amend_id}"));
    // Amender accepts as VaultCorp while there are no deliverables yet.
    ok(&mut tw, amender, &format!("@contract/accept {amend_id}"));
    // Amend keeps VaultCorp's acceptance (A'), then add unauthorized TitleTransfer on root_asset.
    ok(&mut tw, amender, &format!("@contract/amend {amend_id}"));
    ok(
        &mut tw,
        amender,
        &format!(
            "@contract/deliver {amend_id}=title #{} org:VaultCorp->#{}",
            root_asset.0, bob.0
        ),
    );
    err(&mut tw, amender, &format!("@contract/offer {amend_id}"));
    assert_eq!(
        snap(&tw).contract(ContractId(amend_id)).expect("c").state,
        ContractState::Draft,
        "amended contract with unauthorized deliverable must be refused at re-offer"
    );

    // 5. GrantLimit AND ApprovalRule (threshold + rolling window) are enforced on
    //    Deliverable::Obligation at acceptance.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@org/grant #{}=VaultCorp contract.accept scope=#{}",
            officer.0, sub_scope.0
        ),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@org/grant #{}=VaultCorp economy.account.post scope=#{} limit=200",
            officer.0, sub_scope.0
        ),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@org/grant #{}=VaultCorp economy.account.view scope=#{}",
            officer.0, sub_scope.0
        ),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@account/approval {sub_acct}=180 threshold:1 window=300/3600"),
    );

    // 5a. Over GrantLimit (250 > 200) -> refused.
    ok(
        &mut tw,
        GOD,
        &format!("@contract/draft OverLimit scope=#{}", sub_scope.0),
    );
    let lim_id = latest_contract(&tw);
    ok(
        &mut tw,
        GOD,
        &format!("@contract/deliver {lim_id}=obligation {sub_acct} {bob_acct} 250 every=3600"),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/remainder {lim_id}={bob_acct}"),
    );
    ok(&mut tw, GOD, &format!("@contract/offer {lim_id}"));
    err(&mut tw, officer, &format!("@contract/accept {lim_id}"));

    // 5b. Below GrantLimit (50 <= 200) & below threshold (50 <= 180) BUT rapid cadence
    //     exceeds rolling window (50 * (3600 / 60) = 3000 > 300) -> refused!
    ok(
        &mut tw,
        GOD,
        &format!("@contract/draft OverWindow scope=#{}", sub_scope.0),
    );
    let win_id = latest_contract(&tw);
    ok(
        &mut tw,
        GOD,
        &format!("@contract/deliver {win_id}=obligation {sub_acct} {bob_acct} 50 every=60"),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/remainder {win_id}={bob_acct}"),
    );
    ok(&mut tw, GOD, &format!("@contract/offer {win_id}"));
    err(&mut tw, officer, &format!("@contract/accept {win_id}"));

    // 5c. Over approval threshold (190 > 180) -> refused!
    ok(
        &mut tw,
        GOD,
        &format!("@contract/draft OverThresh scope=#{}", sub_scope.0),
    );
    let thr_id = latest_contract(&tw);
    ok(
        &mut tw,
        GOD,
        &format!("@contract/deliver {thr_id}=obligation {sub_acct} {bob_acct} 190 every=3600"),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/remainder {thr_id}={bob_acct}"),
    );
    ok(&mut tw, GOD, &format!("@contract/offer {thr_id}"));
    err(&mut tw, officer, &format!("@contract/accept {thr_id}"));

    // 5d. Within GrantLimit (150 <= 200), threshold (150 <= 180), AND window (150 <= 300) -> allowed!
    ok(
        &mut tw,
        GOD,
        &format!("@contract/draft ValidOblig scope=#{}", sub_scope.0),
    );
    let ok_id = latest_contract(&tw);
    ok(
        &mut tw,
        GOD,
        &format!("@contract/deliver {ok_id}=obligation {sub_acct} {bob_acct} 150 every=3600"),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/remainder {ok_id}={bob_acct}"),
    );
    ok(&mut tw, GOD, &format!("@contract/offer {ok_id}"));
    ok(&mut tw, officer, &format!("@contract/accept {ok_id}"));

    // 6. Non-lease contract obligation terminates when the contract enters `Settled`.
    ok(&mut tw, bob, "@contract/draft NonLeaseSettle");
    let nl_id = latest_contract(&tw);
    ok(
        &mut tw,
        bob,
        &format!("@contract/party {nl_id}=#{}", mallory.0),
    );
    ok(
        &mut tw,
        bob,
        &format!("@contract/pay {nl_id}={bob_acct} 100"),
    );
    ok(
        &mut tw,
        bob,
        &format!("@contract/split {nl_id}={mallory_acct} 1000‰"),
    );
    ok(
        &mut tw,
        bob,
        &format!("@contract/remainder {nl_id}={mallory_acct}"),
    );
    ok(
        &mut tw,
        bob,
        &format!("@contract/deliver {nl_id}=obligation {bob_acct} {mallory_acct} 25 every=60"),
    );
    ok(
        &mut tw,
        bob,
        &format!("@contract/condition {nl_id}=attest party:#{}", bob.0),
    );
    ok(&mut tw, bob, &format!("@contract/offer {nl_id}"));
    ok(&mut tw, bob, &format!("@contract/accept {nl_id}"));
    ok(&mut tw, mallory, &format!("@contract/accept {nl_id}"));
    assert_eq!(
        snap(&tw).contract(ContractId(nl_id)).expect("c").state,
        ContractState::Active
    );
    let nl_ob = snap(&tw)
        .obligations()
        .find(|o| o.cause == ObligationCause::Contract(ContractId(nl_id)))
        .expect("non-lease obligation")
        .id;
    assert_eq!(
        snap(&tw).obligation(nl_ob).expect("ob").status,
        ObligationStatus::Active
    );
    // Satisfy the condition -> settles the non-lease contract and cancels its obligation!
    ok(&mut tw, bob, &format!("@contract/attest {nl_id}=0"));
    assert!(
        matches!(
            snap(&tw).contract(ContractId(nl_id)).expect("c").state,
            ContractState::Settled { .. }
        ),
        "non-lease contract should settle"
    );
    assert_eq!(
        snap(&tw).obligation(nl_ob).expect("ob").status,
        ObligationStatus::Cancelled,
        "non-lease contract obligation must be cancelled on Settled"
    );
    let bob_after_settle = bal(&tw, bob_acct);
    sweep_at(&mut tw, 1_500);
    assert_eq!(
        bal(&tw, bob_acct),
        bob_after_settle,
        "cancelled non-lease obligation must not debit after settlement"
    );
}

#[test]
fn p2_mod_10_cross_org_title_transfer_never_corrupts_invariant_18_or_hijacks_scope() {
    let mut tw = boot_full();
    tw.set_now(1_000);
    ok(&mut tw, GOD, "@instrument/register cr=0");
    ok(&mut tw, GOD, "@account/system mint=cr");
    let mint = account_of(
        &tw,
        &PrincipalId::System(mush_org::SystemPrincipalId::new("mint").unwrap()),
        "cr",
    );

    let seller_officer = player(&mut tw, "SellerOfficer");
    let buyer_officer = player(&mut tw, "BuyerOfficer");
    ok(&mut tw, GOD, "@org/create OrgA");
    ok(&mut tw, GOD, "@org/create OrgB");
    let org_a = org_id(&tw, "OrgA");
    let org_b = org_id(&tw, "OrgB");
    ok(&mut tw, GOD, "@org/scope create OrgA=DivisionA");
    let scope_a = org_scope_id(&tw, org_a, "DivisionA");
    let root_b = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("orgs")
        .state()
        .org(org_b)
        .expect("org b")
        .root_scope;

    for cap in ["contract.draft", "contract.accept", "asset.title.transfer"] {
        ok(
            &mut tw,
            GOD,
            &format!(
                "@org/grant #{}=OrgA {cap} scope=#{}",
                seller_officer.0, scope_a.0
            ),
        );
    }
    for cap in [
        "contract.accept",
        "economy.account.post",
        "economy.account.view",
    ] {
        ok(
            &mut tw,
            GOD,
            &format!(
                "@org/grant #{}=OrgB {cap} scope=#{}",
                buyer_officer.0, root_b.0
            ),
        );
    }

    ok(
        &mut tw,
        GOD,
        &format!("@account/open org:OrgA/{}=cr", scope_a.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@account/open org:OrgB/{}=cr", root_b.0),
    );
    let acct_a = account_of(&tw, &PrincipalId::Organization(org_a), "cr");
    let acct_b = account_of(&tw, &PrincipalId::Organization(org_b), "cr");
    ok(&mut tw, GOD, &format!("@econ/mint {mint}={acct_b} 500"));

    let hull = thing(&mut tw, "Freighter");
    ok(
        &mut tw,
        GOD,
        &format!("@asset/title #{}=org:OrgA scope=#{}", hull.0, scope_a.0),
    );

    // OrgA drafts a contract scoped to `scope_a` selling `hull` to OrgB.
    ok(
        &mut tw,
        GOD,
        &format!("@contract/draft CrossOrgSale scope=#{}", scope_a.0),
    );
    let cid = latest_contract(&tw);
    ok(&mut tw, GOD, &format!("@contract/party {cid}=org:OrgB"));
    ok(&mut tw, GOD, &format!("@contract/pay {cid}={acct_b} 100"));
    ok(
        &mut tw,
        GOD,
        &format!("@contract/split {cid}={acct_a} 1000‰"),
    );
    ok(&mut tw, GOD, &format!("@contract/remainder {cid}={acct_a}"));
    ok(
        &mut tw,
        GOD,
        &format!(
            "@contract/deliver {cid}=title #{} org:OrgA->org:OrgB",
            hull.0
        ),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/condition {cid}=before 2000"),
    );
    ok(&mut tw, GOD, &format!("@contract/offer {cid}"));
    ok(&mut tw, seller_officer, &format!("@contract/accept {cid}"));
    ok(&mut tw, buyer_officer, &format!("@contract/accept {cid}"));

    // Directly prove ContractAuth refuses cross-org `to_scope = Some(scope_a)` for `OrgB`!
    {
        let c = snap(&tw).contract(ContractId(cid)).expect("c");
        let auth = ContractAuth::new(c, snap(&tw));
        let to_b = PrincipalId::Organization(org_b);
        assert!(
            !auth.authorize_title_receipt(&c.issuer, &to_b, Some(scope_a)),
            "ContractAuth must refuse cross-org to_scope"
        );
        assert!(
            !auth.scope_coherent(Some(scope_a), &to_b),
            "ContractAuth::scope_coherent must reject OrgA scope for OrgB"
        );
        assert!(
            auth.authorize_title_receipt(&c.issuer, &to_b, None),
            "ContractAuth must allow unscoped receipt for OrgB"
        );
    }

    // Settle via sweep: title moves to OrgB with `scope == None` (NO Invariant 18 corruption!).
    sweep_at(&mut tw, 1_100);
    assert!(
        matches!(
            snap(&tw).contract(ContractId(cid)).expect("c").state,
            ContractState::Settled { .. }
        ),
        "cross-org sale should settle cleanly"
    );
    let asset = snap(&tw).asset(hull).expect("hull");
    assert_eq!(asset.title_owner, PrincipalId::Organization(org_b));
    assert_eq!(
        asset.scope, None,
        "recipient OrgB's asset must NOT inherit issuer OrgA's scope_a"
    );
}

#[test]
fn p2_mod_11_immutable_created_by_stops_role_author_quota_lockout_and_cap_bypass() {
    let mut tw = boot_full();
    tw.set_now(1_000);
    ok(&mut tw, GOD, "@instrument/register cr=0");
    ok(&mut tw, GOD, "@org/create TargetOrg");
    ok(&mut tw, GOD, "@org/create AttackerOrg");

    let attacker = player(&mut tw, "Attacker");
    let victim = player(&mut tw, "Victim");
    ok(
        &mut tw,
        GOD,
        &format!("@org/grant #{}=AttackerOrg contract.draft", attacker.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@org/grant #{}=AttackerOrg economy.account.post",
            attacker.0
        ),
    );

    let attacker_org = org_id(&tw, "AttackerOrg");
    let attacker_org_root = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("orgs")
        .state()
        .org(attacker_org)
        .expect("attacker org")
        .root_scope;

    // 1. Attacker drafts MAX_OPEN_CONTRACTS_PER_ISSUER contracts under AttackerOrg
    //    and points `roles.author` on every single one to `#victim`.
    for i in 0..mush_economy::MAX_OPEN_CONTRACTS_PER_ISSUER {
        ok(
            &mut tw,
            attacker,
            &format!("@contract/draft Spam{i} scope=#{}", attacker_org_root.0),
        );
        let id = latest_contract(&tw);
        ok(
            &mut tw,
            attacker,
            &format!("@contract/role {id}=author #{}", victim.0),
        );
    }

    // Victim must NOT be locked out by Attacker forging `roles.author`!
    ok(&mut tw, victim, "@contract/draft LegitVictimDraft");

    // Attacker MUST still hit their own immutable `created_by` creator cap even
    // when drafting personally (different issuer!) with `roles.author` mutated!
    let cap_err = err(&mut tw, attacker, "@contract/draft BypassViaRoleAuthor");
    assert!(
        cap_err.contains("You already have"),
        "attacker bypassed creator cap by reassigning roles.author: {cap_err}"
    );
    // And drafting via AttackerOrg also hits issuer/creator cap.
    let org_cap_err = err(
        &mut tw,
        attacker,
        &format!(
            "@contract/draft BypassViaOrg scope=#{}",
            attacker_org_root.0
        ),
    );
    assert!(!org_cap_err.is_empty());

    // Reboot: `created_by` persists across cold boot so Attacker STILL cannot bypass!
    let boot = fold_kv(&mut tw);
    reboot_all(&mut tw, boot);
    let post_reboot_err = err(&mut tw, attacker, "@contract/draft BypassAfterReboot");
    assert!(
        post_reboot_err.contains("You already have"),
        "created_by did not persist across reboot: {post_reboot_err}"
    );
    ok(&mut tw, victim, "@contract/draft LegitVictimAfterReboot");

    // 2. External-org cap (`MAX_UNACCEPTED_CONTRACTS_PER_EXTERNAL_ORG`) cannot be
    //    bypassed by clearing `roles.author=none` or pinning it to `#victim`.
    let spammer2 = player(&mut tw, "Spammer2");
    for i in 0..mush_economy::MAX_UNACCEPTED_CONTRACTS_PER_EXTERNAL_ORG {
        ok(&mut tw, spammer2, &format!("@contract/draft Ext{i}"));
        let id = latest_contract(&tw);
        ok(
            &mut tw,
            spammer2,
            &format!("@contract/party {id}=org:TargetOrg"),
        );
        ok(
            &mut tw,
            spammer2,
            &format!("@contract/role {id}=author none"),
        );
    }
    ok(&mut tw, spammer2, "@contract/draft ExtOverflow");
    let over_id = latest_contract(&tw);
    let ext_err = err(
        &mut tw,
        spammer2,
        &format!("@contract/party {over_id}=org:TargetOrg"),
    );
    assert!(
        ext_err.contains("unaccepted open contracts"),
        "clearing roles.author bypassed external-org cap: {ext_err}"
    );
}

#[test]
fn p2_mod_14_self_accepted_amended_and_expired_zero_history_contracts_prune_cleanly() {
    let mut tw = boot_full();
    tw.set_now(1_000);
    ok(&mut tw, GOD, "@instrument/register cr=0");
    let spammer = player(&mut tw, "Spammer");
    let partner = player(&mut tw, "Partner");
    ok(&mut tw, GOD, &format!("@account/open #{}=cr", spammer.0));
    let acct = account_of(&tw, &PrincipalId::Object(spammer), "cr");
    let cap = mush_economy::MAX_RETAINED_CANCELLED_DRAFTS_PER_ISSUER;

    // 1. Self-accepted zero-consideration offers cancelled repeatedly stay bounded at cap.
    for i in 0..(cap + 6) {
        ok(&mut tw, spammer, &format!("@contract/draft AccSpam{i}"));
        let id = latest_contract(&tw);
        ok(
            &mut tw,
            spammer,
            &format!("@contract/party {id}=#{}", partner.0),
        );
        ok(
            &mut tw,
            spammer,
            &format!("@contract/remainder {id}={acct}"),
        );
        ok(&mut tw, spammer, &format!("@contract/offer {id}"));
        ok(&mut tw, spammer, &format!("@contract/accept {id}"));
        ok(&mut tw, spammer, &format!("@contract/cancel {id}"));
        assert_eq!(
            snap(&tw).contract(ContractId(id)).expect("latest").state,
            ContractState::Cancelled
        );
        assert!(
            snap(&tw).contracts().count() <= cap,
            "self-accepted cancelled contracts exceeded retention cap: {}",
            snap(&tw).contracts().count()
        );
    }

    // 2. Amended zero-consideration offers cancelled repeatedly stay bounded at cap.
    for i in 0..(cap + 6) {
        ok(&mut tw, spammer, &format!("@contract/draft AmendSpam{i}"));
        let id = latest_contract(&tw);
        ok(
            &mut tw,
            spammer,
            &format!("@contract/remainder {id}={acct}"),
        );
        ok(&mut tw, spammer, &format!("@contract/offer {id}"));
        ok(&mut tw, spammer, &format!("@contract/amend {id}"));
        ok(&mut tw, spammer, &format!("@contract/cancel {id}"));
        assert!(
            snap(&tw).contracts().count() <= cap,
            "amended cancelled contracts exceeded retention cap: {}",
            snap(&tw).contracts().count()
        );
    }

    // Clear previously cancelled drafts cleanly on one sweep.
    sweep_at(&mut tw, 2_000);
    assert_eq!(
        snap(&tw).contracts().count(),
        0,
        "sweep must collect retained self-accepted/amended cancelled contracts"
    );

    // 3. Expired zero-consideration offers stay bounded at cap immediately on expiry
    //    and clear completely on the following sweep.
    for i in 0..(cap + 6) {
        ok(&mut tw, spammer, &format!("@contract/draft ExpSpam{i}"));
        let id = latest_contract(&tw);
        ok(
            &mut tw,
            spammer,
            &format!("@contract/remainder {id}={acct}"),
        );
        ok(&mut tw, spammer, &format!("@contract/deadline {id}=60"));
        ok(&mut tw, spammer, &format!("@contract/offer {id}"));
    }
    sweep_at(&mut tw, 2_100);
    assert!(
        snap(&tw).contracts().count() <= cap,
        "expired zero-history contracts exceeded per-issuer retention cap: {}",
        snap(&tw).contracts().count()
    );
    sweep_at(&mut tw, 2_200);
    assert_eq!(
        snap(&tw).contracts().count(),
        0,
        "following sweep must collect all retained zero-history expired contracts"
    );
}

#[test]
fn p2_mod_43_canon_attestation_names_a_party_blocks_staff_wildcard() {
    let mut tw = boot_full();
    tw.set_now(1_000);
    ok(&mut tw, GOD, "@instrument/register cr=0");
    ok(&mut tw, GOD, "@account/system mint=cr");
    let mint = account_of(
        &tw,
        &PrincipalId::System(mush_org::SystemPrincipalId::new("mint").unwrap()),
        "cr",
    );

    let alice = player(&mut tw, "Alice");
    let bob = player(&mut tw, "Bob");
    let member = player(&mut tw, "IronMember");
    ok(&mut tw, GOD, "@org/create Ironhold");
    ok(
        &mut tw,
        GOD,
        &format!("@org/grant #{}=Ironhold economy.account.post", member.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@org/grant #{}=Ironhold contract.accept", member.0),
    );
    ok(&mut tw, GOD, &format!("@account/open #{}=cr", alice.0));
    ok(&mut tw, GOD, &format!("@account/open #{}=cr", bob.0));
    let alice_acct = account_of(&tw, &PrincipalId::Object(alice), "cr");
    let bob_acct = account_of(&tw, &PrincipalId::Object(bob), "cr");
    ok(
        &mut tw,
        GOD,
        &format!("@econ/mint {mint}={alice_acct} 1000"),
    );

    // 1. Personal contract between Alice and Bob gated on `canon:op.done:op:1`.
    ok(&mut tw, alice, "@contract/draft PersonalCanon");
    let c1 = latest_contract(&tw);
    ok(&mut tw, alice, &format!("@contract/party {c1}=#{}", bob.0));
    ok(
        &mut tw,
        alice,
        &format!("@contract/pay {c1}={alice_acct} 100"),
    );
    ok(
        &mut tw,
        alice,
        &format!("@contract/split {c1}={bob_acct} 1000‰"),
    );
    ok(
        &mut tw,
        alice,
        &format!("@contract/remainder {c1}={bob_acct}"),
    );
    ok(
        &mut tw,
        alice,
        &format!("@contract/condition {c1}=attest canon:op.done:op:1"),
    );
    ok(&mut tw, alice, &format!("@contract/offer {c1}"));
    ok(&mut tw, alice, &format!("@contract/accept {c1}"));
    ok(&mut tw, bob, &format!("@contract/accept {c1}"));
    assert_eq!(
        snap(&tw).contract(ContractId(c1)).expect("c1").state,
        ContractState::Active
    );

    // Canon naming staff (GOD), who is NOT a party, must NOT satisfy or settle c1!
    emit_canon(&mut tw, 1, "op.done", "op:1", &[GOD]);
    assert_eq!(
        snap(&tw).contract(ContractId(c1)).expect("c1").state,
        ContractState::Active,
        "canon naming staff GOD must NOT wildcard-match non-staff contract parties"
    );
    assert!(
        snap(&tw).contract(ContractId(c1)).expect("c1").satisfied[0].is_none(),
        "condition must remain unsatisfied after staff-only canon record"
    );

    // Canon naming an actual party (Alice) satisfies and settles c1!
    emit_canon(&mut tw, 2, "op.done", "op:1", &[alice]);
    assert!(
        matches!(
            snap(&tw).contract(ContractId(c1)).expect("c1").state,
            ContractState::Settled { .. }
        ),
        "canon naming an actual party must settle the contract"
    );

    // 2. Org contract (`parties = [Alice, org:Ironhold]`): canon naming GOD (staff,
    //    NOT a member of Ironhold) must NOT match, whereas canon naming an active
    //    member of Ironhold DOES match!
    ok(&mut tw, alice, "@contract/draft OrgCanon");
    let c2 = latest_contract(&tw);
    ok(
        &mut tw,
        alice,
        &format!("@contract/party {c2}=org:Ironhold"),
    );
    ok(
        &mut tw,
        alice,
        &format!("@contract/pay {c2}={alice_acct} 100"),
    );
    ok(
        &mut tw,
        alice,
        &format!("@contract/split {c2}={bob_acct} 1000‰"),
    );
    ok(
        &mut tw,
        alice,
        &format!("@contract/remainder {c2}={bob_acct}"),
    );
    ok(
        &mut tw,
        alice,
        &format!("@contract/condition {c2}=attest canon:op.done:op:2"),
    );
    ok(&mut tw, alice, &format!("@contract/offer {c2}"));
    ok(&mut tw, alice, &format!("@contract/accept {c2}"));
    ok(&mut tw, member, &format!("@contract/accept {c2}"));
    assert_eq!(
        snap(&tw).contract(ContractId(c2)).expect("c2").state,
        ContractState::Active
    );

    emit_canon(&mut tw, 3, "op.done", "op:2", &[GOD]);
    assert_eq!(
        snap(&tw).contract(ContractId(c2)).expect("c2").state,
        ContractState::Active,
        "canon naming staff GOD must NOT wildcard-match unrelated org party"
    );

    emit_canon(&mut tw, 4, "op.done", "op:2", &[member]);
    assert!(
        matches!(
            snap(&tw).contract(ContractId(c2)).expect("c2").state,
            ContractState::Settled { .. }
        ),
        "canon naming an active member of org party must settle the contract"
    );
}

#[test]
fn p2_mod_14_salvage_claims_survive_gc_while_spoofed_drafts_prune_and_leases_allow_15s_cadence() {
    let mut tw = boot_full();
    tw.set_now(1_000);
    ok(&mut tw, GOD, "@instrument/register cr=0");
    let owner = player(&mut tw, "Owner");
    let salvor = player(&mut tw, "Salvor");
    let spammer = player(&mut tw, "Spammer");
    let hull = thing(&mut tw, "Derelict");
    let loaner = thing(&mut tw, "Loaner");
    ok(&mut tw, GOD, &format!("@account/open #{}=cr", owner.0));
    ok(&mut tw, GOD, &format!("@account/open #{}=cr", salvor.0));
    ok(&mut tw, GOD, &format!("@account/open #{}=cr", spammer.0));
    ok(
        &mut tw,
        GOD,
        &format!("@asset/title #{}=#{}", hull.0, owner.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@asset/title #{}=#{}", loaner.0, owner.0),
    );
    let owner_acct = account_of(&tw, &PrincipalId::Object(owner), "cr");
    let salvor_acct = account_of(&tw, &PrincipalId::Object(salvor), "cr");
    let spammer_acct = account_of(&tw, &PrincipalId::Object(spammer), "cr");

    // 1. A real Rust-API `file_salvage_claim` survives expiry and subsequent sweeps.
    let claim_id = {
        let mut sink = Outbox::new();
        let engine = tw.engine_mut();
        let mut ctx = mush_core::Ctx {
            w: &mut engine.world,
            actor: salvor,
            switches: mush_core::SwitchSet::default(),
            args: mush_core::ParsedArgs::default(),
            out: &mut sink,
            reg: &engine.registry,
            module: Some(SmolStr::new_static(mush_economy::NS)),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        mush_economy::file_salvage_claim(
            &mut ctx,
            mush_economy::SalvageClaimSpec {
                name: SmolStr::new_static("salvage claim"),
                claimant: salvor,
                hull,
                audience: vec![PrincipalId::Object(owner)],
                window_secs: 60,
                remainder_to: salvor_acct,
            },
        )
        .expect("file_salvage_claim")
    };

    // 2. A player-drafted contract mimicking salvage-claim shape must STILL be pruned.
    ok(&mut tw, spammer, "@contract/draft FakeSalvage");
    let fake_id = latest_contract(&tw);
    ok(
        &mut tw,
        spammer,
        &format!("@contract/remainder {fake_id}={spammer_acct}"),
    );
    ok(
        &mut tw,
        spammer,
        &format!("@contract/condition {fake_id}=attest staff"),
    );
    ok(
        &mut tw,
        spammer,
        &format!(
            "@contract/condition {fake_id}=title #{} #{}",
            hull.0, spammer.0
        ),
    );
    ok(
        &mut tw,
        spammer,
        &format!("@contract/deadline {fake_id}=60"),
    );
    // 2. A player-drafted contract mimicking salvage-claim shape is refused at
    //    `@contract/offer` and pruned by P2-MOD-14 GC once expired/cancelled.
    let offer_err = err(&mut tw, spammer, &format!("@contract/offer {fake_id}"));
    assert!(
        offer_err.contains("salvage claim"),
        "expected salvage-claim offer refusal, got: {offer_err}"
    );
    ok(&mut tw, GOD, &format!("@contract/cancel {fake_id}"));

    // Expire the filed salvage claim, then sweep again to trigger `collect_prunable_contracts`.
    sweep_at(&mut tw, 1_100);
    sweep_at(&mut tw, 1_200);
    assert_eq!(
        snap(&tw).contract(claim_id).map(|c| c.state),
        Some(ContractState::Expired),
        "Rust-API filed salvage claim must survive sweep GC after expiry"
    );
    assert!(
        snap(&tw).contract(ContractId(fake_id)).is_none(),
        "player-drafted salvage-shaped contract must still be pruned by P2-MOD-14 GC"
    );

    // 3. Module `offer_lease` with 15s cadence succeeds while `@contract/deliver`
    //    still enforces MIN_PLAYER_OBLIGATION_EVERY_SECS (60s) on player drafts.
    let lease_id = {
        let mut sink = Outbox::new();
        let engine = tw.engine_mut();
        let mut ctx = mush_core::Ctx {
            w: &mut engine.world,
            actor: GOD,
            switches: mush_core::SwitchSet::default(),
            args: mush_core::ParsedArgs::default(),
            out: &mut sink,
            reg: &engine.registry,
            module: Some(SmolStr::new_static(mush_economy::NS)),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        mush_economy::offer_lease(
            &mut ctx,
            mush_economy::LeaseSpec {
                name: SmolStr::new_static("recovery loaner"),
                lessor: PrincipalId::Object(owner),
                scope: None,
                tenants: vec![PrincipalId::Object(salvor)],
                premises: loaner,
                rent: mush_economy::ObligationTemplate {
                    debtor: PrincipalId::Object(salvor),
                    creditor: PrincipalId::Object(owner),
                    debit_account: salvor_acct,
                    credit_account: owner_acct,
                    amount: mush_economy::Amount::from_minor(10),
                    schedule: mush_economy::EconomicSchedule::new(
                        mush_economy::EconomicClock::RealUtc,
                        1_230,
                        Some(mush_economy::MIN_LEASE_OBLIGATION_EVERY_SECS),
                        1,
                    ),
                    grace: mush_economy::GracePolicy::default(),
                    notify_on_success: false,
                },
            },
        )
        .expect("15s module lease offer")
    };
    ok(&mut tw, salvor, &format!("@contract/accept {}", lease_id.0));
    assert_eq!(
        snap(&tw).contract(lease_id).map(|c| c.state),
        Some(ContractState::Active)
    );

    ok(&mut tw, spammer, "@contract/draft TooFastPlayer");
    let too_fast = latest_contract(&tw);
    ok(
        &mut tw,
        spammer,
        &format!("@contract/party {too_fast}=#{}", salvor.0),
    );
    err(
        &mut tw,
        spammer,
        &format!(
            "@contract/deliver {too_fast}=obligation {salvor_acct} {spammer_acct} 10 every=15"
        ),
    );
}
