//! Tier 2A & Tier 2B security regression tests for `mush-economy`:
//! - Unconsented `@contract/party` multi-org officer fan-out caps.
//! - Existence oracles across `@econ/pay`, `@contract/deliver`, and `@hold`.
//! - P2-MOD-12: external-org offer sliding-window rate limit & unaccepted self-cancel/amend
//!   notice suppression.
//! - P2-MOD-13: `@contract/split`, `/remainder`, `/deadline forfeit=`, `/outcome pay=`, and
//!   `/deliver collateral|title` existence oracles closed.
//! - P2-MOD-42: `@econ/pay` masks unviewable destinations behind approval rules and masks
//!   `InactiveInstrument`.
//! - P2-MOD-44: `@asset/list` post-filter pagination hides foreign `ObjId`s and bounds scan.
//! - P2-MOD-45: `join_job` / `accept_retrieval` reject mismatched acceptor instrument.
//! - P2-MOD-75: `@asset/release` indexes only visible encumbrances, hiding covert stolen reports.
//! - P2-MOD-76: `reprice_player_obligation` checks `WorldAuth` against the new amount, and
//!   `job_bond` uses exact `i128` arithmetic without `i64` saturation under-computation.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mush_core::module::{GameModule, ModuleBootData, Registrar, WorldEvent};
use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, Deferred, Effects, GOD, LIMBO, ModuleOp, ModuleWrite,
    ObjId, ObjKind, Outbox,
};
use mush_economy::{
    ASSET_PAGE_VISIT, AccountId, Amount, AuthEval, ContractAuth, ContractError, ContractId,
    ContractState, ContractStateKind, EXTERNAL_ORG_OFFER_WINDOW_SECS, EconomicClock, EconomicEvent,
    EconomicSchedule, EconomyError, EconomyObserver, EconomySnapshot, EconomyStore, ExternalRef,
    FactProvider, FactState, GracePolicy, HoldId, JobSpec, JoinTerms,
    MAX_EXTERNAL_ORG_OFFERS_PER_WINDOW, MAX_PENDING_NOTICES, ObligationCause, ObligationSpec,
    ObligationStatus, RetrievalSpec,
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
    // Grant `asset.assign` (view standing, NOT `asset.title.transfer`) so `build_deliver`
    // passes P2-MOD-13's view gate and still exercises P2-MOD-1's re-offer transfer gate.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@org/grant #{}=VaultCorp asset.assign scope=#{}",
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
        &format!("@contract/split {c2}={alice_acct} 1000‰"),
    );
    ok(
        &mut tw,
        alice,
        &format!("@contract/remainder {c2}={alice_acct}"),
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

#[test]
fn p2_mod_14_scoped_fact_tombstones_survive_sweep_and_stay_bounded_independently() {
    #[derive(Debug)]
    struct ScopedTestProvider;
    impl mush_economy::FactProvider for ScopedTestProvider {
        fn provider(&self) -> &str {
            "scoped_test"
        }
        fn requires_scope(&self, _fact: &mush_economy::ExternalRef) -> bool {
            true
        }
        fn resolve(
            &self,
            _: &mush_core::World,
            _: &str,
            _: &str,
            _: Option<u64>,
        ) -> Option<mush_economy::FactState> {
            None
        }
        fn authorize_scope(
            &self,
            _ctx: &mush_economy::ScopedFactContext<'_>,
            _fact: &mush_economy::ExternalRef,
        ) -> bool {
            true
        }
        fn resolve_scoped(
            &self,
            _ctx: &mush_economy::ScopedFactContext<'_>,
            _fact: &mush_economy::ExternalRef,
        ) -> Option<mush_economy::FactState> {
            None
        }
    }

    let vocab: std::collections::BTreeSet<mush_org::CapabilityId> = mush_economy::CAPABILITIES
        .iter()
        .map(|s| mush_org::CapabilityId::new(*s).expect("capability"))
        .collect();
    let mut tw = TestWorld::with_modules(vec![
        Box::new(
            mush_org::OrgModule::new()
                .with_capabilities(vocab)
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(
            mush_economy::EconomyModule::with_custody(vec![])
                .with_facts(vec![Arc::new(ScopedTestProvider)]),
        ),
    ]);
    tw.set_now(1_000);
    ok(&mut tw, GOD, "@instrument/register cr=0");
    let pilot = player(&mut tw, "Pilot");
    let spammer = player(&mut tw, "Spammer");
    ok(&mut tw, GOD, &format!("@account/open #{}=cr", pilot.0));
    ok(&mut tw, GOD, &format!("@account/open #{}=cr", spammer.0));
    let pilot_acct = account_of(&tw, &PrincipalId::Object(pilot), "cr");
    let spammer_acct = account_of(&tw, &PrincipalId::Object(spammer), "cr");

    let offer_scoped =
        |tw: &mut TestWorld, who: ObjId, acct: AccountId, idx: usize| -> ContractId {
            ok(tw, who, &format!("@contract/draft Scoped{idx}"));
            let id = latest_contract(tw);
            ok(tw, who, &format!("@contract/party {id}=#{}", GOD.0));
            ok(tw, who, &format!("@contract/remainder {id}={acct}"));
            ok(
                tw,
                who,
                &format!("@contract/condition {id}=fact scoped_test done item-{idx}"),
            );
            ok(tw, who, &format!("@contract/deadline {id}=60"));
            ok(tw, who, &format!("@contract/offer {id}"));
            ContractId(id)
        };

    // 1. A cancelled zero-history scoped contract survives economy sweeps, and
    //    ordinary `@contract/draft` spam cannot evict it from its separate pool.
    let kept_cancelled = offer_scoped(&mut tw, pilot, pilot_acct, 0);
    assert!(
        snap(&tw)
            .contract(kept_cancelled)
            .expect("offered")
            .consent
            .fact_consent
            .is_some()
    );
    ok(
        &mut tw,
        pilot,
        &format!("@contract/cancel #{}", kept_cancelled.0),
    );
    for i in 0..(mush_economy::MAX_RETAINED_CANCELLED_DRAFTS + 4) {
        ok(&mut tw, spammer, &format!("@contract/draft Plain{i}"));
        let id = latest_contract(&tw);
        ok(&mut tw, spammer, &format!("@contract/cancel {id}"));
    }
    sweep_at(&mut tw, 1_030);
    sweep_at(&mut tw, 1_050);
    assert_eq!(
        snap(&tw).contract(kept_cancelled).map(|c| c.state),
        Some(ContractState::Cancelled),
        "scoped fact_consent cancelled contract must survive sweeps and ordinary draft spam"
    );

    // 2. An expired zero-history scoped contract also survives subsequent sweeps.
    let kept_expired = offer_scoped(&mut tw, pilot, pilot_acct, 1);
    sweep_at(&mut tw, 1_100);
    sweep_at(&mut tw, 1_200);
    assert_eq!(
        snap(&tw).contract(kept_expired).map(|c| c.state),
        Some(ContractState::Expired),
        "scoped fact_consent expired contract must survive sweeps"
    );

    // 3. Repeated cancellation of scoped contracts stays bounded per issuer.
    let cap = mush_economy::MAX_RETAINED_CANCELLED_DRAFTS_PER_ISSUER;
    for i in 2..(cap + 6) {
        let id = offer_scoped(&mut tw, spammer, spammer_acct, i);
        ok(&mut tw, spammer, &format!("@contract/cancel #{}", id.0));
    }
    sweep_at(&mut tw, 1_300);
    let spammer_scoped_count = snap(&tw)
        .contracts()
        .filter(|c| c.issuer == PrincipalId::Object(spammer) && c.consent.fact_consent.is_some())
        .count();
    assert!(
        spammer_scoped_count <= cap,
        "scoped fact_consent tombstones must stay bounded per issuer ({spammer_scoped_count} > {cap})"
    );
    assert_eq!(
        snap(&tw).contract(kept_cancelled).map(|c| c.state),
        Some(ContractState::Cancelled),
        "other issuer's scoped tombstones remain intact"
    );
}

// ---- Tier 2B security regression tests (P2-MOD-12/13/42/44/45/75/76) -------

type ProbeFn = Box<dyn FnOnce(&mut Ctx<'_>) + Send>;

#[derive(Clone, Default)]
struct ProbeQueue(Arc<Mutex<Vec<ProbeFn>>>);

struct ProbeModule;

impl GameModule for ProbeModule {
    fn name(&self) -> &'static str {
        "econ_t2b_probe"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), mush_core::module::ModError> {
        reg.state(ProbeQueue::default());
        reg.classify_commands(mush_core::CommandCompatibility::Authoritative);
        reg.command(
            CommandSpec {
                name: "+t2bprobe".into(),
                shape: ArgShape::Raw,
                help: "internal test probe".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let job = ctx
                    .w
                    .module_state::<ProbeQueue>()
                    .and_then(|q| q.0.lock().unwrap().pop());
                if let Some(f) = job {
                    f(ctx);
                }
                Ok(())
            }),
        )?;
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        Ok(())
    }
}

fn with_probe<T: Send + 'static>(
    tw: &mut TestWorld,
    actor: ObjId,
    f: impl FnOnce(&mut Ctx<'_>) -> T + Send + 'static,
) -> T {
    let slot: Arc<Mutex<Option<T>>> = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&slot);
    let queue = tw
        .world()
        .module_state::<ProbeQueue>()
        .expect("ProbeModule installed")
        .clone();
    queue.0.lock().unwrap().push(Box::new(move |ctx| {
        *sink.lock().unwrap() = Some(f(ctx));
    }));
    ok(tw, actor, "+t2bprobe");
    slot.lock().unwrap().take().expect("probe executed")
}

/// **P2-MOD-12**:
/// 1. Sliding-window rate limit caps per-actor/per-external-org offers at
///    `MAX_EXTERNAL_ORG_OFFERS_PER_WINDOW` per `EXTERNAL_ORG_OFFER_WINDOW_SECS`,
///    preventing `Offer -> Cancel` churn from evicting an offline officer's 32-slot queue.
/// 2. Self-cancel and amend on contracts where an external org has not accepted suppress
///    notice fan-out (both `deliver_notice_for` offline queue and `principal_holders` live toasts),
///    while cancelling after the external org HAS accepted still delivers the notice.
#[test]
fn p2_mod_12_external_org_offer_rate_limit_and_self_cancel_amend_notice_suppression() {
    #[derive(Debug)]
    struct TestOrgInbox {
        live_toasts: Arc<Mutex<Vec<(ObjId, ContractId, ContractStateKind)>>>,
    }

    impl EconomyObserver for TestOrgInbox {
        fn on_economic_event(&self, ctx: &mut Ctx<'_>, event: &EconomicEvent) {
            let EconomicEvent::ContractChanged {
                id: contract,
                state,
                ..
            } = *event
            else {
                return;
            };
            let parties: Vec<PrincipalId> = ctx
                .w
                .module_state::<EconomyStore>()
                .and_then(|s| s.snapshot().contract(contract))
                .map(|c| c.consent.parties.clone())
                .unwrap_or_default();
            for party in parties {
                if !matches!(party, PrincipalId::Organization(_)) {
                    continue;
                }
                for holder in mush_economy::principal_holders(ctx.w, &party) {
                    self.live_toasts
                        .lock()
                        .unwrap()
                        .push((holder, contract, state));
                    mush_economy::deliver_notice_for(
                        ctx,
                        holder,
                        &party,
                        format!("Contract #{contract} -> {state:?}"),
                    );
                }
            }
        }
    }

    let live_toasts = Arc::new(Mutex::new(Vec::new()));
    let vocab: std::collections::BTreeSet<mush_org::CapabilityId> = mush_economy::CAPABILITIES
        .iter()
        .map(|s| mush_org::CapabilityId::new(*s).expect("capability"))
        .collect();
    let mut tw = TestWorld::with_modules(vec![
        Box::new(
            mush_org::OrgModule::new()
                .with_capabilities(vocab)
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(
            mush_economy::EconomyModule::with_custody(vec![]).with_observers(vec![Arc::new(
                TestOrgInbox {
                    live_toasts: Arc::clone(&live_toasts),
                },
            )]),
        ),
    ]);
    tw.set_now(10_000);
    ok(&mut tw, GOD, "@instrument/register cr=0");

    let spammer = player(&mut tw, "Spammer");
    let officer = player(&mut tw, "TargetOfficer");
    ok(&mut tw, GOD, "@org/create TargetOrg");
    let target_org = org_id(&tw, "TargetOrg");
    for cap in ["contract.accept", "economy.account.post"] {
        ok(
            &mut tw,
            GOD,
            &format!("@org/grant #{}=TargetOrg {cap}", officer.0),
        );
    }
    ok(&mut tw, GOD, &format!("@account/open #{}=cr", spammer.0));
    ok(&mut tw, GOD, "@account/open org:TargetOrg=cr");
    let spammer_acct = account_of(&tw, &PrincipalId::Object(spammer), "cr");

    let pending_for_officer = |tw: &TestWorld| -> usize {
        tw.world()
            .module_state::<EconomyStore>()
            .unwrap()
            .pending_notices(officer)
            .len()
    };

    // 1. Draft -> Party org:TargetOrg -> Cancel emits ZERO notices (unaccepted external org).
    ok(&mut tw, spammer, "@contract/draft DraftOnly");
    let d0 = latest_contract(&tw);
    ok(
        &mut tw,
        spammer,
        &format!("@contract/party {d0}=org:TargetOrg"),
    );
    ok(&mut tw, spammer, &format!("@contract/cancel {d0}"));
    assert_eq!(pending_for_officer(&tw), 0);
    assert!(live_toasts.lock().unwrap().is_empty());

    // 2. Offer -> Cancel emits ONLY 1 notice (for Offer), ZERO for self-cancel!
    //    Offer -> Amend emits ONLY 1 notice (for Offer), ZERO for amend!
    let make_draft = |tw: &mut TestWorld, name: &str| -> u64 {
        ok(tw, spammer, &format!("@contract/draft {name}"));
        let id = latest_contract(tw);
        ok(tw, spammer, &format!("@contract/party {id}=org:TargetOrg"));
        ok(
            tw,
            spammer,
            &format!("@contract/remainder {id}={spammer_acct}"),
        );
        id
    };

    // Offer #1 -> Cancel: 1 notice total (Offer only).
    let c1 = make_draft(&mut tw, "Spam1");
    ok(&mut tw, spammer, &format!("@contract/offer {c1}"));
    assert_eq!(pending_for_officer(&tw), 1);
    assert_eq!(live_toasts.lock().unwrap().len(), 1);
    ok(&mut tw, spammer, &format!("@contract/cancel {c1}"));
    assert_eq!(
        pending_for_officer(&tw),
        1,
        "self-cancel on unaccepted external org must not enqueue a notice"
    );
    assert_eq!(
        live_toasts.lock().unwrap().len(),
        1,
        "self-cancel on unaccepted external org must not emit a live toast"
    );

    // Offer #2 -> Amend (back to Draft) -> Cancel: 1 notice total (Offer only, 0 for Amend/Cancel).
    let c2 = make_draft(&mut tw, "Spam2");
    ok(&mut tw, spammer, &format!("@contract/offer {c2}"));
    assert_eq!(pending_for_officer(&tw), 2);
    ok(&mut tw, spammer, &format!("@contract/amend {c2}"));
    assert_eq!(
        pending_for_officer(&tw),
        2,
        "amend on unaccepted external org must not enqueue a notice"
    );
    assert_eq!(
        live_toasts.lock().unwrap().len(),
        2,
        "amend on unaccepted external org must not emit a live toast"
    );
    ok(&mut tw, spammer, &format!("@contract/cancel {c2}"));
    assert_eq!(pending_for_officer(&tw), 2);

    // Offers #3 and #4 succeed up to MAX_EXTERNAL_ORG_OFFERS_PER_WINDOW (4).
    assert_eq!(MAX_EXTERNAL_ORG_OFFERS_PER_WINDOW, 4);
    for idx in 3..=MAX_EXTERNAL_ORG_OFFERS_PER_WINDOW {
        let cid = make_draft(&mut tw, &format!("Spam{idx}"));
        ok(&mut tw, spammer, &format!("@contract/offer {cid}"));
        ok(&mut tw, spammer, &format!("@contract/cancel {cid}"));
    }
    assert_eq!(pending_for_officer(&tw), MAX_EXTERNAL_ORG_OFFERS_PER_WINDOW);
    assert!(pending_for_officer(&tw) < MAX_PENDING_NOTICES);

    // 5th offer within EXTERNAL_ORG_OFFER_WINDOW_SECS is rate-limited!
    let c5 = make_draft(&mut tw, "Spam5");
    let rate_err = err(&mut tw, spammer, &format!("@contract/offer {c5}"));
    assert!(
        rate_err.contains("recently offered") || rate_err.contains("wait"),
        "expected external-org sliding-window rate limit refusal, got: {rate_err}"
    );
    assert_eq!(
        pending_for_officer(&tw),
        MAX_EXTERNAL_ORG_OFFERS_PER_WINDOW,
        "rate-limited offer must not deliver any notice"
    );

    // 3. After EXTERNAL_ORG_OFFER_WINDOW_SECS elapses, offering succeeds again; and if
    //    TargetOrg ACCEPTS before spammer cancels, the cancellation notice IS delivered!
    tw.set_now(10_000 + EXTERNAL_ORG_OFFER_WINDOW_SECS + 1);
    ok(&mut tw, spammer, &format!("@contract/offer {c5}"));
    let after_offer = pending_for_officer(&tw);
    assert_eq!(after_offer, MAX_EXTERNAL_ORG_OFFERS_PER_WINDOW + 1);
    ok(&mut tw, officer, &format!("@contract/accept {c5}"));
    assert!(
        snap(&tw)
            .contract(ContractId(c5))
            .unwrap()
            .accepted_by(&PrincipalId::Organization(target_org))
    );
    // Spammer cancels while TargetOrg has already accepted -> TargetOrg MUST be notified!
    let before_cancel = pending_for_officer(&tw);
    let toasts_before_cancel = live_toasts.lock().unwrap().len();
    ok(&mut tw, spammer, &format!("@contract/cancel {c5}"));
    assert_eq!(
        pending_for_officer(&tw),
        before_cancel + 1,
        "cancellation AFTER external org accepted must still deliver notice"
    );
    assert_eq!(
        live_toasts.lock().unwrap().len(),
        toasts_before_cancel + 1,
        "cancellation AFTER external org accepted must still emit live toast"
    );
}

/// **P2-MOD-13**: `@contract/split`, `/remainder`, `/deadline forfeit=`, `/outcome pay=`,
/// and `/deliver collateral|title` must enforce account/asset viewability and title ownership
/// with uniform existence-hiding errors for non-staff, while preserving legitimate party
/// accounts and buyer-drafted two-party asset delivery.
#[test]
fn p2_mod_13_contract_builders_close_account_and_asset_existence_oracles() {
    let mut tw = boot_full();
    tw.set_now(1_000);
    ok(&mut tw, GOD, "@instrument/register cr=0");
    ok(&mut tw, GOD, "@account/system mint=cr");
    let mint = account_of(
        &tw,
        &PrincipalId::System(mush_org::SystemPrincipalId::new("mint").unwrap()),
        "cr",
    );

    let attacker = player(&mut tw, "Attacker");
    let victim = player(&mut tw, "Victim");
    let bystander = player(&mut tw, "Bystander");
    ok(&mut tw, GOD, "@org/create SecretCorp");
    let corp = org_id(&tw, "SecretCorp");
    let corp_root = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .unwrap()
        .state()
        .org(corp)
        .unwrap()
        .root_scope;

    ok(&mut tw, GOD, &format!("@account/open #{}=cr", attacker.0));
    ok(&mut tw, GOD, &format!("@account/open #{}=cr", victim.0));
    ok(&mut tw, GOD, &format!("@account/open #{}=cr", bystander.0));
    ok(
        &mut tw,
        GOD,
        &format!("@account/open org:SecretCorp/{}=cr", corp_root.0),
    );
    // Also create a closed account to verify closed vs nonexistent uniformity.
    let ghost = player(&mut tw, "Ghost");
    ok(&mut tw, GOD, &format!("@account/open #{}=cr", ghost.0));
    let closed_acct = account_of(&tw, &PrincipalId::Object(ghost), "cr");
    ok(&mut tw, GOD, &format!("@account/close {closed_acct}"));

    let attacker_acct = account_of(&tw, &PrincipalId::Object(attacker), "cr");
    let victim_acct = account_of(&tw, &PrincipalId::Object(victim), "cr");
    let bystander_acct = account_of(&tw, &PrincipalId::Object(bystander), "cr");
    let corp_acct = account_of(&tw, &PrincipalId::Organization(corp), "cr");

    let victim_asset = thing(&mut tw, "VictimShip");
    let corp_asset = thing(&mut tw, "CorpShip");
    let attacker_asset = thing(&mut tw, "AttackerShip");
    ok(
        &mut tw,
        GOD,
        &format!("@asset/title #{}=#{}", victim_asset.0, victim.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@asset/title #{}=org:SecretCorp scope=#{}",
            corp_asset.0, corp_root.0
        ),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@asset/title #{}=#{}", attacker_asset.0, attacker.0),
    );

    ok(&mut tw, attacker, "@contract/draft OracleProbe");
    let cid = latest_contract(&tw);
    ok(
        &mut tw,
        attacker,
        &format!("@contract/party {cid}=#{}", victim.0),
    );

    let missing_acct = AccountId(99_999);
    let hidden_targets = [corp_acct, mint, bystander_acct, closed_acct, missing_acct];

    // 1. `@contract/split`
    let base_split = err(
        &mut tw,
        attacker,
        &format!("@contract/split {cid}={missing_acct} 500‰"),
    );
    assert_eq!(base_split, "No such account.");
    for acct in hidden_targets {
        let got = err(
            &mut tw,
            attacker,
            &format!("@contract/split {cid}={acct} 500‰"),
        );
        assert_eq!(
            got, base_split,
            "@contract/split leaked account {acct:?}: {got}"
        );
    }

    // 2. `@contract/remainder`
    let base_rem = err(
        &mut tw,
        attacker,
        &format!("@contract/remainder {cid}={missing_acct}"),
    );
    assert_eq!(base_rem, "No such account.");
    for acct in hidden_targets {
        let got = err(
            &mut tw,
            attacker,
            &format!("@contract/remainder {cid}={acct}"),
        );
        assert_eq!(
            got, base_rem,
            "@contract/remainder leaked account {acct:?}: {got}"
        );
    }

    // 3. `@contract/deadline ... forfeit=`
    let base_forfeit = err(
        &mut tw,
        attacker,
        &format!("@contract/deadline {cid}=60 forfeit={missing_acct}"),
    );
    assert_eq!(base_forfeit, "No such account.");
    for acct in hidden_targets {
        let got = err(
            &mut tw,
            attacker,
            &format!("@contract/deadline {cid}=60 forfeit={acct}"),
        );
        assert_eq!(
            got, base_forfeit,
            "@contract/deadline forfeit= leaked account {acct:?}: {got}"
        );
    }

    // 4. `@contract/outcome ... pay=`
    ok(
        &mut tw,
        attacker,
        &format!("@contract/condition {cid}=before 9999"),
    );
    let base_outcome = err(
        &mut tw,
        attacker,
        &format!("@contract/outcome {cid}=win when=0 pay={missing_acct}:1000‰"),
    );
    assert_eq!(base_outcome, "No such account.");
    for acct in hidden_targets {
        let got = err(
            &mut tw,
            attacker,
            &format!("@contract/outcome {cid}=win when=0 pay={acct}:1000‰"),
        );
        assert_eq!(
            got, base_outcome,
            "@contract/outcome pay= leaked account {acct:?}: {got}"
        );
    }

    // 5. `@contract/deliver collateral` & `@contract/deliver title`
    let base_col = err(
        &mut tw,
        attacker,
        &format!("@contract/deliver {cid}=collateral #99999 #{}", attacker.0),
    );
    assert_eq!(base_col, "No such asset.");
    for cmd in [
        format!(
            "@contract/deliver {cid}=collateral #{} #{}",
            victim_asset.0, victim.0
        ),
        format!(
            "@contract/deliver {cid}=collateral #{} #{}",
            victim_asset.0, attacker.0
        ),
        format!(
            "@contract/deliver {cid}=collateral #{} org:SecretCorp",
            corp_asset.0
        ),
        format!(
            "@contract/deliver {cid}=collateral #{} #{}",
            attacker_asset.0, victim.0
        ),
        format!(
            "@contract/deliver {cid}=title #{} #{}->#{}",
            victim_asset.0, victim.0, attacker.0
        ),
        format!(
            "@contract/deliver {cid}=title #{} #{}->#{}",
            victim_asset.0, attacker.0, victim.0
        ),
        format!(
            "@contract/deliver {cid}=title #{} org:SecretCorp->#{}",
            corp_asset.0, attacker.0
        ),
        format!(
            "@contract/deliver {cid}=title #99999 #{}->#{}",
            attacker.0, victim.0
        ),
    ] {
        let got = err(&mut tw, attacker, &cmd);
        assert_eq!(
            got, base_col,
            "deliver command `{cmd}` leaked asset/owner existence: {got}"
        );
    }

    // 6. Legitimate flows still work: own/party accounts + own asset + listed-party self-delivery!
    ok(
        &mut tw,
        attacker,
        &format!("@contract/split {cid}={victim_acct} 500‰"),
    );
    ok(
        &mut tw,
        attacker,
        &format!("@contract/remainder {cid}={attacker_acct}"),
    );
    ok(
        &mut tw,
        attacker,
        &format!("@contract/deadline {cid}=60 forfeit={victim_acct}"),
    );
    ok(
        &mut tw,
        attacker,
        &format!("@contract/outcome {cid}=win when=0 pay={victim_acct}:1000‰"),
    );
    ok(
        &mut tw,
        attacker,
        &format!(
            "@contract/deliver {cid}=collateral #{} #{}",
            attacker_asset.0, attacker.0
        ),
    );
    // Listed party `victim` adds their own asset delivery on `attacker`'s draft (buyer-drafted sale).
    ok(
        &mut tw,
        victim,
        &format!(
            "@contract/deliver {cid}=title #{} #{}->#{}",
            victim_asset.0, victim.0, attacker.0
        ),
    );
    // Preserving an already-named org forfeit destination (e.g. a system/staff-built contract
    // amended by its player issuer) succeeds without reopening probing of other hidden accounts.
    ok(
        &mut tw,
        GOD,
        &format!("@contract/deadline {cid}=60 forfeit={corp_acct}"),
    );
    ok(
        &mut tw,
        attacker,
        &format!("@contract/deadline {cid}=120 forfeit={corp_acct}"),
    );
    assert_eq!(
        err(
            &mut tw,
            attacker,
            &format!("@contract/deadline {cid}=120 forfeit={bystander_acct}")
        ),
        "No such account."
    );
}

/// **P2-MOD-42**:
/// 1. With an approval-ruled source account, `@econ/pay` (plain and `asset=`) must return an
///    identical `"Approval required"` response for every unviewable destination (nonexistent,
///    closed, cross-instrument, inactive-instrument, valid personal, hidden org, System),
///    and still post cleanly once approved for a valid destination.
/// 2. Without an approval rule, `@econ/pay` to a foreign account on an inactive/retired
///    instrument must mask `InactiveInstrument` identically to `"No such account."`.
#[test]
fn p2_mod_42_econ_pay_masks_unviewable_destinations_behind_approval_and_inactive_instrument() {
    let mut tw = boot_full();
    tw.set_now(1_000);
    ok(&mut tw, GOD, "@instrument/register cr=0");
    ok(&mut tw, GOD, "@instrument/register au=0");
    ok(&mut tw, GOD, "@instrument/register old=0");
    ok(&mut tw, GOD, "@account/system mint=cr");
    let mint = account_of(
        &tw,
        &PrincipalId::System(mush_org::SystemPrincipalId::new("mint").unwrap()),
        "cr",
    );

    let prober = player(&mut tw, "Prober");
    let approver = player(&mut tw, "Approver");
    let victim = player(&mut tw, "Victim");
    let ghost = player(&mut tw, "Ghost");
    ok(&mut tw, GOD, "@org/create ProberOrg");
    ok(&mut tw, GOD, "@org/create SecretCorp");
    let prober_org = org_id(&tw, "ProberOrg");
    let secret_corp = org_id(&tw, "SecretCorp");
    let (p_root, s_root) = {
        let orgs = tw.world().module_state::<mush_org::OrgStore>().unwrap();
        (
            orgs.state().org(prober_org).unwrap().root_scope,
            orgs.state().org(secret_corp).unwrap().root_scope,
        )
    };
    ok(
        &mut tw,
        GOD,
        &format!("@account/open org:ProberOrg/{}=cr", p_root.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@account/open org:SecretCorp/{}=cr", s_root.0),
    );
    ok(&mut tw, GOD, &format!("@account/open #{}=cr", prober.0));
    ok(&mut tw, GOD, &format!("@account/open #{}=cr", victim.0));
    ok(&mut tw, GOD, &format!("@account/open #{}=au", victim.0));
    ok(&mut tw, GOD, &format!("@account/open #{}=old", victim.0));
    ok(&mut tw, GOD, &format!("@account/open #{}=cr", ghost.0));

    let src_org_acct = account_of(&tw, &PrincipalId::Organization(prober_org), "cr");
    let prober_cr = account_of(&tw, &PrincipalId::Object(prober), "cr");
    let victim_cr = account_of(&tw, &PrincipalId::Object(victim), "cr");
    let victim_au = account_of(&tw, &PrincipalId::Object(victim), "au");
    let victim_old = account_of(&tw, &PrincipalId::Object(victim), "old");
    let secret_corp_cr = account_of(&tw, &PrincipalId::Organization(secret_corp), "cr");
    let closed_acct = account_of(&tw, &PrincipalId::Object(ghost), "cr");
    ok(&mut tw, GOD, &format!("@account/close {closed_acct}"));

    ok(
        &mut tw,
        GOD,
        &format!("@econ/mint {mint}={src_org_acct} 5000"),
    );
    ok(&mut tw, GOD, &format!("@econ/mint {mint}={prober_cr} 5000"));

    // Grant prober posting+view+title.transfer on ProberOrg, approver approval on ProberOrg.
    for cap in [
        "economy.account.post",
        "economy.account.view",
        "asset.title.transfer",
    ] {
        ok(
            &mut tw,
            GOD,
            &format!(
                "@org/grant #{}=ProberOrg {cap} scope=#{}",
                prober.0, p_root.0
            ),
        );
    }
    ok(
        &mut tw,
        GOD,
        &format!(
            "@org/grant #{}=ProberOrg economy.account.approve scope=#{}",
            approver.0, p_root.0
        ),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@account/approval {src_org_acct}=100 named:1:#{}",
            approver.0
        ),
    );

    let prober_asset = thing(&mut tw, "ProberCargo");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@asset/title #{}=org:ProberOrg scope=#{}",
            prober_asset.0, p_root.0
        ),
    );

    // Retire `old` instrument in persisted boot data and reboot so `victim_old` sits on an
    // inactive instrument.
    let mut boot = mush_testkit::fold_module_ops(tw.take_module_ops());
    let econ_boot = boot.get_mut(mush_economy::NS).expect("economy boot");
    let old_key = mush_economy::instrument_key(&mush_economy::InstrumentId::new("old").unwrap());
    let (_, old_val) = econ_boot
        .kv
        .iter_mut()
        .find(|(k, _)| k == &old_key)
        .expect("old instrument in KV");
    old_val["status"] = serde_json::json!("Retired");
    tw.modules_loaded(boot);

    // 1. Without approval rule (`prober_cr`), probing `victim_old` (retired instrument) must
    //    return the exact same `"No such account."` as nonexistent `99999` (never expose
    //    `InactiveInstrument`).
    let e_missing = err(&mut tw, prober, &format!("@econ/pay {prober_cr}=99999 10"));
    let e_retired = err(
        &mut tw,
        prober,
        &format!("@econ/pay {prober_cr}={victim_old} 10"),
    );
    assert_eq!(e_missing, "No such account.");
    assert_eq!(
        e_retired, e_missing,
        "@econ/pay leaked InactiveInstrument on foreign account: {e_retired}"
    );

    // 2. Behind an approval rule (`src_org_acct`, threshold 100), `@econ/pay` of 200 to EVERY
    //    unviewable destination (nonexistent, closed, cross-instrument, inactive-instrument,
    //    valid personal, hidden org, System) returns `"Approval required"` identically!
    //    Strip the sequential request id before comparing the response shape.
    let normalize = |line: &str| -> String {
        line.split_whitespace()
            .map(|w| {
                let core = w.trim_end_matches('.');
                if core.starts_with('#') && core[1..].chars().all(|c| c.is_ascii_digit()) {
                    if w.ends_with('.') { "#<id>." } else { "#<id>" }
                } else {
                    w
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    let extract_req_id = |out: &str| -> u64 {
        let digits: String = out
            .split("request #")
            .nth(1)
            .unwrap_or_else(|| panic!("no request id in: {out}"))
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse::<u64>().unwrap()
    };
    let targets = [
        AccountId(99_999),
        closed_acct,
        victim_au,
        victim_old,
        secret_corp_cr,
        mint,
        victim_cr,
    ];
    let mut shapes = Vec::new();
    let mut victim_req_id = None;
    for target in targets {
        let out = ok(
            &mut tw,
            prober,
            &format!("@econ/pay {src_org_acct}={target} 200"),
        );
        assert!(
            out.contains("Approval required"),
            "expected Approval required for target {target}, got: {out}"
        );
        let req_id = extract_req_id(&out);
        if target == victim_cr {
            victim_req_id = Some(req_id);
        } else {
            // Clear pending slot so we can probe all 7 targets without hitting
            // `MAX_PENDING_PER_REQUESTER`.
            ok(&mut tw, approver, &format!("@econ/deny {req_id}"));
        }
        shapes.push((target, normalize(&out)));
    }
    let baseline = &shapes[0].1;
    for (target, shape) in &shapes {
        assert_eq!(
            shape, baseline,
            "approval-ruled @econ/pay differentiated target {target:?}: {shape} vs {baseline}"
        );
    }

    // 3. Same with `asset=` on an approval-ruled source: nonexistent vs real foreign target
    //    return identical `"Approval required"`!
    let asset_missing = ok(
        &mut tw,
        prober,
        &format!(
            "@econ/pay {src_org_acct}=99999 200 asset=#{}",
            prober_asset.0
        ),
    );
    let r_am = extract_req_id(&asset_missing);
    ok(&mut tw, approver, &format!("@econ/deny {r_am}"));
    let asset_real = ok(
        &mut tw,
        prober,
        &format!(
            "@econ/pay {src_org_acct}={victim_cr} 200 asset=#{}",
            prober_asset.0
        ),
    );
    let r_ar = extract_req_id(&asset_real);
    ok(&mut tw, approver, &format!("@econ/deny {r_ar}"));
    assert!(asset_missing.contains("Approval required"));
    assert_eq!(normalize(&asset_missing), normalize(&asset_real));

    // 4. Legitimate approval still posts cleanly to `victim_cr` when re-submitted!
    let req_id = victim_req_id.expect("victim approval request id");
    ok(&mut tw, approver, &format!("@econ/approve {req_id}"));
    ok(
        &mut tw,
        prober,
        &format!("@econ/pay {src_org_acct}={victim_cr} 200"),
    );
    assert_eq!(bal(&tw, victim_cr), 200);
}

/// **P2-MOD-44**: `@asset/list` must filter visibility BEFORE computing pagination cursors
/// (never emitting `after=#<hidden_obj_id>`) and bound total scanned records per call.
#[test]
fn p2_mod_44_asset_list_hides_cursor_for_hidden_assets_and_bounds_scan() {
    let mut tw = boot_full();
    let victim = player(&mut tw, "Victim");
    let alice = player(&mut tw, "Alice");
    let bob = player(&mut tw, "Bob");
    ok(&mut tw, GOD, "@org/create SecretOrg");
    let secret_org = org_id(&tw, "SecretOrg");
    let secret_scope = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .unwrap()
        .state()
        .org(secret_org)
        .unwrap()
        .root_scope;

    // Create `ASSET_PAGE_VISIT + 20` (276) hidden assets owned by SecretOrg in secret_scope
    // (> ASSET_PAGE_VISIT = 256, within ASSET_PAGE_SCAN = 1024), then 1 asset owned by Alice!
    let mut hidden_ids = Vec::new();
    for i in 0..(ASSET_PAGE_VISIT + 20) {
        let obj = thing(&mut tw, &format!("SecretAsset{i}"));
        ok(
            &mut tw,
            GOD,
            &format!(
                "@asset/title #{}=org:SecretOrg scope=#{}",
                obj.0, secret_scope.0
            ),
        );
        hidden_ids.push(obj);
    }
    let alice_asset = thing(&mut tw, "AliceAsset");
    ok(
        &mut tw,
        GOD,
        &format!("@asset/title #{}=#{}", alice_asset.0, alice.0),
    );

    // 1. Alice sees her own asset and NO `after=#` cursor leaking any SecretAsset ObjId
    //    (previously pre-filter pagination emitted `after=#<256th hidden SecretAsset>`!).
    let alice_out = ok(&mut tw, alice, "@asset/list");
    assert!(
        alice_out.contains(&format!("#{}", alice_asset.0)),
        "Alice should see her own asset: {alice_out}"
    );
    assert!(
        !alice_out.contains("after=#"),
        "Alice must not receive a pagination cursor for hidden assets: {alice_out}"
    );
    for hid in &hidden_ids {
        assert!(
            !alice_out.contains(&format!("#{} ", hid.0)),
            "hidden asset #{} leaked to Alice: {alice_out}",
            hid.0
        );
    }

    // 2. Add `ASSET_PAGE_VISIT + 10` more hidden victim assets (total > 2 * ASSET_PAGE_VISIT):
    //    Bob (0 visible assets) gets "No assets." with NO cursor for bare, owner=, and scope=.
    for i in 0..(ASSET_PAGE_VISIT + 10) {
        let obj = thing(&mut tw, &format!("VictimAsset{i}"));
        ok(
            &mut tw,
            GOD,
            &format!("@asset/title #{}=#{}", obj.0, victim.0),
        );
    }
    for cmd in [
        "@asset/list".to_owned(),
        format!("@asset/list owner=#{}", victim.0),
        format!("@asset/list scope=#{}", secret_scope.0),
    ] {
        let bob_out = ok(&mut tw, bob, &cmd);
        assert_eq!(
            bob_out.trim(),
            "No assets.",
            "`{cmd}` leaked hidden assets or cursor to Bob: {bob_out}"
        );
    }

    // 3. Staff (`GOD`) still paginates cleanly with `(more — @asset/list after=#<256th>)`.
    let god_p1 = ok(&mut tw, GOD, "@asset/list");
    assert!(
        god_p1.contains("(more — @asset/list after=#"),
        "staff should get a pagination cursor: {god_p1}"
    );
    let cursor: String = god_p1
        .split("after=#")
        .nth(1)
        .unwrap()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    assert_eq!(
        cursor.parse::<u32>().unwrap(),
        hidden_ids[ASSET_PAGE_VISIT - 1].0,
        "staff page 1 cursor must point at the 256th visible asset"
    );
    let god_p2 = ok(&mut tw, GOD, &format!("@asset/list after=#{cursor}"));
    assert!(
        god_p2.contains("(more — @asset/list after=#"),
        "staff page 2 should continue paginating: {god_p2}"
    );
}

/// **P2-MOD-45**: `join_job` and `accept_retrieval` must reject an acceptor account whose
/// instrument does not match the job's fee account (`ContractError::InstrumentMismatch`) before
/// mutating contract state or placing a bond hold, while accepting a matching-instrument account.
#[test]
fn p2_mod_45_join_job_and_accept_retrieval_reject_mismatched_instrument() {
    #[derive(Debug)]
    struct ToggleFact(Arc<std::sync::atomic::AtomicBool>);

    impl FactProvider for ToggleFact {
        fn provider(&self) -> &str {
            "jobtest"
        }
        fn resolve(
            &self,
            _: &mush_core::World,
            _: &str,
            _: &str,
            _: Option<u64>,
        ) -> Option<FactState> {
            Some(FactState {
                satisfied: self.0.load(std::sync::atomic::Ordering::Relaxed),
                actor: None,
                at: 0,
            })
        }
    }

    let done_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut tw = TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(
            mush_economy::EconomyModule::with_custody(vec![])
                .with_facts(vec![Arc::new(ToggleFact(Arc::clone(&done_flag)))]),
        ),
        Box::new(ProbeModule),
    ]);
    tw.set_now(1_000);
    ok(&mut tw, GOD, "@instrument/register cr=0");
    ok(&mut tw, GOD, "@instrument/register au=0");
    ok(&mut tw, GOD, "@account/system mint_cr=cr");
    ok(&mut tw, GOD, "@account/system mint_au=au");
    let mint_cr = account_of(
        &tw,
        &PrincipalId::System(mush_org::SystemPrincipalId::new("mint_cr").unwrap()),
        "cr",
    );
    let mint_au = account_of(
        &tw,
        &PrincipalId::System(mush_org::SystemPrincipalId::new("mint_au").unwrap()),
        "au",
    );

    let poster = player(&mut tw, "Poster");
    let acceptor = player(&mut tw, "Acceptor");
    ok(&mut tw, GOD, &format!("@account/open #{}=cr", poster.0));
    ok(&mut tw, GOD, &format!("@account/open #{}=cr", acceptor.0));
    ok(&mut tw, GOD, &format!("@account/open #{}=au", acceptor.0));
    let poster_cr = account_of(&tw, &PrincipalId::Object(poster), "cr");
    let acceptor_cr = account_of(&tw, &PrincipalId::Object(acceptor), "cr");
    let acceptor_au = account_of(&tw, &PrincipalId::Object(acceptor), "au");
    ok(
        &mut tw,
        GOD,
        &format!("@econ/mint {mint_cr}={poster_cr} 5000000"),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@econ/mint {mint_cr}={acceptor_cr} 1000000"),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@econ/mint {mint_au}={acceptor_au} 1000000"),
    );

    let hull = thing(&mut tw, "DerelictHull");
    ok(
        &mut tw,
        GOD,
        &format!("@asset/title #{}=#{}", hull.0, poster.0),
    );

    let deadline_ic = tw.world().world_ic_now().at + 86_400;

    // 1. Bonded `join_job`: fee 2_000_000 cr -> bond 400_000 cr. Joining with `au` account
    //    must fail with `ContractError::InstrumentMismatch` without mutating or holding!
    let job_id = with_probe(&mut tw, poster, move |ctx| {
        mush_economy::post_job(
            ctx,
            JobSpec {
                name: SmolStr::new_static("Haul"),
                poster: PrincipalId::Object(poster),
                poster_account: poster_cr,
                fee: Amount::from_minor(2_000_000),
                fact: ExternalRef {
                    provider: SmolStr::new_static("jobtest"),
                    kind: SmolStr::new_static("done"),
                    id: SmolStr::new_static("haul-1"),
                    revision: None,
                },
                deadline_ic,
            },
        )
        .expect("post_job")
    });
    let bond = mush_economy::job_bond(Amount::from_minor(2_000_000));
    assert_eq!(bond, Some(Amount::from_minor(400_000)));

    let wrong_join = with_probe(&mut tw, acceptor, move |ctx| {
        mush_economy::join_job(
            ctx,
            job_id,
            JoinTerms {
                acceptor: PrincipalId::Object(acceptor),
                account: acceptor_au,
                bond,
                term_ic: None,
            },
        )
    });
    assert!(
        matches!(wrong_join, Err(ContractError::InstrumentMismatch)),
        "join_job with mismatched instrument must return InstrumentMismatch, got: {wrong_join:?}"
    );
    assert!(
        mush_economy::job_acceptor(snap(&tw).contract(job_id).unwrap()).is_none(),
        "rejected join_job must leave job unaccepted"
    );
    assert_eq!(
        snap(&tw).account(acceptor_au).unwrap().held_amount.minor(),
        0,
        "rejected join_job must not place bond hold on mismatched account"
    );

    // Retry with matching `cr` account succeeds and settles cleanly when fact turns true!
    with_probe(&mut tw, acceptor, move |ctx| {
        mush_economy::join_job(
            ctx,
            job_id,
            JoinTerms {
                acceptor: PrincipalId::Object(acceptor),
                account: acceptor_cr,
                bond,
                term_ic: None,
            },
        )
        .expect("join_job with matching instrument")
    });
    assert_eq!(
        snap(&tw).account(acceptor_cr).unwrap().held_amount.minor(),
        400_000
    );
    done_flag.store(true, std::sync::atomic::Ordering::Relaxed);
    sweep_at(&mut tw, 1_100);
    assert!(matches!(
        snap(&tw).contract(job_id).unwrap().state,
        ContractState::Settled { .. }
    ));

    // 2. `accept_retrieval` with mismatched `au` account must fail with `InstrumentMismatch`.
    done_flag.store(false, std::sync::atomic::Ordering::Relaxed);
    let ret_id = with_probe(&mut tw, poster, move |ctx| {
        mush_economy::post_retrieval(
            ctx,
            RetrievalSpec {
                name: SmolStr::new_static("Retrieve"),
                poster: PrincipalId::Object(poster),
                title_holder: None,
                poster_account: poster_cr,
                hull,
                fee: Amount::from_minor(5_000),
                delivered: ExternalRef {
                    provider: SmolStr::new_static("jobtest"),
                    kind: SmolStr::new_static("done"),
                    id: SmolStr::new_static("ret-1"),
                    revision: None,
                },
                deadline_ic,
            },
        )
        .expect("post_retrieval")
    });
    let wrong_ret = with_probe(&mut tw, acceptor, move |ctx| {
        mush_economy::accept_retrieval(ctx, ret_id, &PrincipalId::Object(acceptor), acceptor_au)
    });
    assert!(
        matches!(wrong_ret, Err(ContractError::InstrumentMismatch)),
        "accept_retrieval with mismatched instrument must return InstrumentMismatch, got: {wrong_ret:?}"
    );
    assert!(
        mush_economy::retrieval_acceptor(snap(&tw).contract(ret_id).unwrap()).is_none(),
        "rejected accept_retrieval must leave retrieval unaccepted"
    );
    // Matching `cr` account succeeds!
    with_probe(&mut tw, acceptor, move |ctx| {
        mush_economy::accept_retrieval(ctx, ret_id, &PrincipalId::Object(acceptor), acceptor_cr)
            .expect("accept_retrieval with matching instrument")
    });
    assert_eq!(
        mush_economy::retrieval_acceptor(snap(&tw).contract(ret_id).unwrap()),
        Some(&PrincipalId::Object(acceptor))
    );
}

/// **P2-MOD-75**: `@asset/release` must index only visible encumbrances (consistently with
/// `@asset/show`) so a covert stolen report at raw index 0 neither leaks via index shift nor
/// prevents a creditor from releasing their visible lien at visible index 1.
#[test]
fn p2_mod_75_asset_release_indexes_only_visible_encumbrances_hiding_covert_stolen() {
    let mut tw = boot_full();
    ok(&mut tw, GOD, "@instrument/register cr=0");
    let owner = player(&mut tw, "Owner");
    let assignee = player(&mut tw, "Assignee");
    let hull = thing(&mut tw, "StolenHull");

    ok(
        &mut tw,
        GOD,
        &format!("@asset/title #{}=#{}", hull.0, owner.0),
    );
    ok(
        &mut tw,
        owner,
        &format!("@asset/assign #{}=#{}", hull.0, assignee.0),
    );
    // Place a COVERT stolen report at raw index 0, then a visible lien held by Assignee at raw
    // index 1 (Assignee passes `may_view_asset` and holds standing to release their own lien).
    ok(
        &mut tw,
        GOD,
        &format!("@asset/stolen #{}=covert silent tracking", hull.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@asset/lien #{}=#{} 500 cr", hull.0, assignee.0),
    );
    assert_eq!(
        snap(&tw).asset(hull).unwrap().encumbrances.len(),
        2,
        "raw encumbrances holds [Stolen(Covert), Lien(Assignee)]"
    );

    // 1. Assignee (cannot view covert stolen report) sees `encumbrances: 1` (covert hidden).
    let show_assignee = ok(&mut tw, assignee, &format!("@asset/show #{}", hull.0));
    assert!(
        show_assignee.contains("encumbrances: 1"),
        "assignee should see 1 visible encumbrance: {show_assignee}"
    );
    assert!(
        !show_assignee.to_lowercase().contains("stolen"),
        "covert stolen report must not appear in @asset/show: {show_assignee}"
    );

    // 2. `@asset/release <hull>=2` by assignee must return `"No such encumbrance."`
    //    (cannot probe raw index 1 to discover the hidden covert stolen report at raw index 0!).
    let probe_assignee = err(&mut tw, assignee, &format!("@asset/release #{}=2", hull.0));
    assert_eq!(probe_assignee, "No such encumbrance.");
    // Owner (who CAN view their own covert stolen report) sees `encumbrances: 2`.
    let show_owner = ok(&mut tw, owner, &format!("@asset/show #{}", hull.0));
    assert!(
        show_owner.contains("encumbrances: 2") && show_owner.to_lowercase().contains("stolen"),
        "owner should still see both encumbrances including covert stolen report: {show_owner}"
    );

    // 3. Assignee releases their visible lien using visible index `1` -> removes raw index 1
    //    (the lien) while leaving the covert stolen report at raw index 0 completely intact!
    ok(&mut tw, assignee, &format!("@asset/release #{}=1", hull.0));
    let remaining = &snap(&tw).asset(hull).unwrap().encumbrances;
    assert_eq!(remaining.len(), 1);
    assert!(
        matches!(remaining[0], mush_economy::Encumbrance::Disputed { .. })
            && snap(&tw).asset(hull).unwrap().stolen.is_some(),
        "covert stolen report must remain intact after creditor releases visible index 1"
    );

    // 4. Staff (`GOD`) still sees the covert stolen report and can release it at index 1.
    let show_god = ok(&mut tw, GOD, &format!("@asset/show #{}", hull.0));
    assert!(
        show_god.contains("encumbrances: 1") && show_god.to_lowercase().contains("stolen"),
        "staff should see covert stolen report: {show_god}"
    );
    ok(&mut tw, GOD, &format!("@asset/release #{}=1", hull.0));
    assert!(snap(&tw).asset(hull).unwrap().encumbrances.is_empty());
}

/// **P2-MOD-76**:
/// 1. `reprice_player_obligation` must authorize `WorldAuth` against the NEW amount (not the old
///    amount), blocking a `GrantLimit`-capped org officer from escalating an obligation past their
///    spending cap.
/// 2. `job_bond` must use exact `i128` arithmetic so large fees never saturate `i64` on `* 200`
///    and under-compute the 20% bond.
#[test]
fn p2_mod_76_reprice_player_obligation_checks_new_amount_and_job_bond_exact_i128() {
    // 1. Exact `job_bond` arithmetic across threshold boundaries and near `i64::MAX`.
    assert_eq!(
        mush_economy::job_bond(Amount::from_minor(mush_economy::JOB_BOND_ABOVE_MINOR)),
        None
    );
    assert_eq!(
        mush_economy::job_bond(Amount::from_minor(mush_economy::JOB_BOND_ABOVE_MINOR + 1)),
        Some(Amount::from_minor(200_000))
    );
    assert_eq!(
        mush_economy::job_bond(Amount::from_minor(100_000_000_000_000_000)),
        Some(Amount::from_minor(20_000_000_000_000_000)),
        "1e17 fee must yield exact 2e16 bond (old saturating i64 collapsed to ~9.22e15)"
    );
    assert_eq!(
        mush_economy::job_bond(Amount::from_minor(i64::MAX)),
        Some(Amount::from_minor(1_844_674_407_370_955_161)),
        "i64::MAX fee must yield exact floor(i64::MAX * 200 / 1000)"
    );

    // 2. `reprice_player_obligation` checks `WorldAuth` against NEW amount.
    let vocab: std::collections::BTreeSet<mush_org::CapabilityId> = mush_economy::CAPABILITIES
        .iter()
        .map(|s| mush_org::CapabilityId::new(*s).expect("capability"))
        .collect();
    let mut tw = TestWorld::with_modules(vec![
        Box::new(
            mush_org::OrgModule::new()
                .with_capabilities(vocab)
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(mush_economy::EconomyModule::with_custody(vec![])),
        Box::new(ProbeModule),
    ]);
    tw.set_now(1_000);
    ok(&mut tw, GOD, "@instrument/register cr=0");
    ok(&mut tw, GOD, "@org/create TenantOrg");
    ok(&mut tw, GOD, "@org/create PortOrg");
    let tenant_org = org_id(&tw, "TenantOrg");
    let port_org = org_id(&tw, "PortOrg");
    let tenant_root = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .unwrap()
        .state()
        .org(tenant_org)
        .unwrap()
        .root_scope;
    ok(
        &mut tw,
        GOD,
        &format!("@account/open org:TenantOrg/{}=cr", tenant_root.0),
    );
    ok(&mut tw, GOD, "@account/open org:PortOrg=cr");
    let debit_acct = account_of(&tw, &PrincipalId::Organization(tenant_org), "cr");
    let credit_acct = account_of(&tw, &PrincipalId::Organization(port_org), "cr");

    let officer = player(&mut tw, "CappedOfficer");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@org/grant #{}=TenantOrg economy.account.post scope=#{} limit=100",
            officer.0, tenant_root.0
        ),
    );

    let ob_id = with_probe(&mut tw, officer, move |ctx| {
        mush_economy::create_player_obligation(
            ctx,
            ObligationSpec {
                debit_account: debit_acct,
                credit_account: credit_acct,
                amount: Amount::from_minor(50),
                schedule: EconomicSchedule {
                    clock: EconomicClock::WorldIc,
                    next_due: 4_600,
                    cadence: Some(3_600),
                    maximum_catch_up: 1,
                    manual_now: 0,
                },
                grace: GracePolicy::new(2),
                notify_on_success: false,
                cause: ObligationCause::Ruleset {
                    namespace: SmolStr::new_static("test.locker"),
                },
            },
            "test.locker",
        )
        .expect("create_player_obligation within cap 100")
    });
    assert_eq!(snap(&tw).obligation(ob_id).unwrap().amount.minor(), 50);

    // Reprice 50 -> 80 (within officer's limit=100): allowed!
    let within_cap = with_probe(&mut tw, officer, move |ctx| {
        mush_economy::reprice_player_obligation(ctx, ob_id, "test.locker", Amount::from_minor(80))
    });
    assert!(
        within_cap.is_ok(),
        "reprice within limit=100 should pass: {within_cap:?}"
    );
    assert_eq!(snap(&tw).obligation(ob_id).unwrap().amount.minor(), 80);

    // Reprice 80 -> 500 (ABOVE officer's limit=100): MUST fail with Unauthorized!
    let over_cap = with_probe(&mut tw, officer, move |ctx| {
        mush_economy::reprice_player_obligation(ctx, ob_id, "test.locker", Amount::from_minor(500))
    });
    assert!(
        matches!(over_cap, Err(EconomyError::Unauthorized)),
        "reprice past GrantLimit=100 must return Unauthorized, got: {over_cap:?}"
    );
    assert_eq!(
        snap(&tw).obligation(ob_id).unwrap().amount.minor(),
        80,
        "refused reprice must leave obligation amount unchanged"
    );

    // Staff (`GOD`) may still reprice to 500.
    let staff_reprice = with_probe(&mut tw, GOD, move |ctx| {
        mush_economy::reprice_player_obligation(ctx, ob_id, "test.locker", Amount::from_minor(500))
    });
    assert!(staff_reprice.is_ok());
    assert_eq!(snap(&tw).obligation(ob_id).unwrap().amount.minor(), 500);
}

#[test]
fn p2_tsc_17_open_account_refuses_when_economy_is_frozen() {
    let mut tw = TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(mush_economy::EconomyModule::with_custody(vec![])),
        Box::new(ProbeModule),
    ]);
    ok(&mut tw, GOD, "@instrument/register cr=2");
    let alice = player(&mut tw, "Alice");
    let _ = tw.take_module_ops();

    // Freeze the economy latch.
    let frozen = with_probe(&mut tw, GOD, |ctx| {
        mush_economy::set_economy_frozen(ctx, true)
    });
    assert!(frozen);
    assert!(
        tw.world()
            .module_state::<EconomyStore>()
            .expect("economy store")
            .is_frozen()
    );
    let _ = tw.take_module_ops();

    let accounts_before = snap(&tw).accounts().count();
    let candidate = mush_economy::Account {
        id: AccountId(999),
        owner: PrincipalId::Object(alice),
        instrument: mush_economy::InstrumentId::new("cr").expect("cr"),
        scope: None,
        name: SmolStr::new_static("Alice"),
        status: mush_economy::AccountStatus::Active,
        posted_balance: Amount::ZERO,
        held_amount: Amount::ZERO,
        last_hold_seq: 0,
        credit: mush_economy::CreditPolicy::forbid(),
        last_journal_seq: 0,
        revision: 1,
        approval: None,
    };
    let candidate_frozen = candidate.clone();
    let opened_frozen = with_probe(&mut tw, GOD, move |ctx| {
        mush_economy::open_account(ctx, candidate_frozen)
    });
    assert!(
        !opened_frozen,
        "open_account must refuse when EconomyStore::frozen is true"
    );
    assert_eq!(
        snap(&tw).accounts().count(),
        accounts_before,
        "frozen open_account must not insert into snapshot"
    );
    assert!(
        snap(&tw).account(AccountId(999)).is_none(),
        "frozen account must not exist"
    );
    assert!(
        tw.take_module_ops().is_empty(),
        "frozen open_account must not emit persistent account or meta writes"
    );

    // Thaw and verify open_account succeeds normally.
    assert!(with_probe(&mut tw, GOD, |ctx| {
        mush_economy::set_economy_frozen(ctx, false)
    }));
    let _ = tw.take_module_ops();
    let opened = with_probe(&mut tw, GOD, move |ctx| {
        mush_economy::open_account(ctx, candidate)
    });
    assert!(opened, "open_account should succeed once unfrozen");
    assert!(snap(&tw).account(AccountId(999)).is_some());
    assert!(!tw.take_module_ops().is_empty());
}
