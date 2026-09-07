//! E7 cut 9 acceptance (10b §3.9): **a declared operation, funded and resolved
//! exactly once**.
//!
//! Everything here runs through the real `Engine` via `TestWorld`, on the production
//! composition order (org → faction → canon → economy → campaign), and every command
//! goes through the real **command → handler → document / economy post** path. A
//! test that only reads back the document it just wrote does not count: each one
//! asserts what a second actor would actually **see** or be **able to do** — the
//! rendered bytes a non-audience viewer receives, the balance another account can
//! spend, the refusal a stranger gets — in addition to the record.
//!
//! The three lenses this cut is verified on:
//!
//! - **Z (freeze integrity)** — can any path change what a defender consented to be
//!   told, or progress an operation whose surface no longer hashes?
//! - **A (audience)** — can a viewer learn a covert operation exists, or its plan,
//!   through any render, error or timing?
//! - **I (idempotent fan-out)** — can a crash, retry or duplicate root fact
//!   double-apply a consequence, or strand an operation in `Resolving`?

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use mush_campaign::{
    CampaignStore, Composition, ConsumerId, FormationAuthority, FormationKind, NO_SUCH_OPERATION,
    OperationId, OperationState, RoleSpec, UnitRef,
};
use mush_core::{Audience, GOD, LIMBO, ModuleBootData, ModuleWrite, ObjId, ObjKind, World};
use mush_economy::{AccountId, EconomySnapshot, EconomyStore, Hold, HoldStatus};
use mush_org::CapabilityId;
use mush_org::PrincipalId;
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

/// One IC hour, at the default `ic_ratio` of 1.
const HOUR: i64 = 3_600;

// ---- rig ------------------------------------------------------------------

fn vocabulary() -> BTreeSet<CapabilityId> {
    mush_economy::CAPABILITIES
        .iter()
        .copied()
        .chain(mush_campaign::CAPABILITIES.iter().copied())
        .chain(["org.role.manage", "org.grant.delegate"])
        .map(|c| CapabilityId::new(c).expect("well-formed"))
        .collect()
}

/// A stand-in for `mush_server::campaign_formations::TacsimFormationAuthority`:
/// the same seam, with two kinds authored in the test rather than in a pack, so
/// this crate's tests need no ruleset edge.
#[derive(Clone, Copy, Debug)]
struct TestKinds;

fn kind(id: &str, echelon: u8, minimum: u8, maximum: u8) -> FormationKind {
    FormationKind {
        id: SmolStr::new(id),
        branch: SmolStr::new_static("mech"),
        echelon,
        composition: Composition {
            minimum,
            maximum,
            preferred: minimum,
            allowed: BTreeSet::new(),
            required_command: Vec::new(),
            required_support: Vec::new(),
        },
        command_roles: vec![RoleSpec {
            id: SmolStr::new_static("commander"),
            minimum: 0,
        }],
    }
}

impl FormationAuthority for TestKinds {
    fn kinds(&self, _world: &World) -> Vec<FormationKind> {
        vec![kind("lance", 1, 4, 4), kind("star", 1, 5, 5)]
    }
}

fn boot() -> TestWorld {
    boot_windowed(mush_economy::DEFAULT_IDEMPOTENCY_WINDOW_SECS)
}

/// [`boot`] with an explicit org capability vocabulary, so a test can compose a
/// build that does **not** know a `force.*` id.
fn boot_with_vocabulary(vocab: BTreeSet<CapabilityId>) -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(
            mush_org::OrgModule::new()
                .with_capabilities(vocab)
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(mush_faction::FactionModule::new()),
        Box::new(mush_canon::CanonModule::new()),
        Box::new(
            mush_economy::EconomyModule::with_custody(vec![]).with_canon_outcome_kinds(vec![
                SmolStr::new_static(mush_campaign::KIND_OPERATION_RESOLVED),
            ]),
        ),
        Box::new(mush_campaign::CampaignModule::new().with_formation_kinds(Arc::new(TestKinds))),
    ])
}

/// [`boot`] with the economy's idempotency window kept for `secs` (doc 13, W2).
/// Zero keeps nothing past the next sweep and nothing at boot, which is how a
/// campaign key that has aged out of the window is simulated.
fn boot_windowed(secs: i64) -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(
            mush_org::OrgModule::new()
                .with_capabilities(vocabulary())
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(mush_faction::FactionModule::new()),
        Box::new(mush_canon::CanonModule::new()),
        Box::new(
            mush_economy::EconomyModule::with_custody(vec![])
                .with_canon_outcome_kinds(vec![SmolStr::new_static(
                    mush_campaign::KIND_OPERATION_RESOLVED,
                )])
                .with_config(mush_economy::EconomyConfig {
                    idempotency_window_secs: secs,
                    ..mush_economy::EconomyConfig::default()
                }),
        ),
        Box::new(mush_campaign::CampaignModule::new().with_formation_kinds(Arc::new(TestKinds))),
    ])
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
/// `ok`-or-`err` that asserts nothing: for a test whose point is **which way it
/// went**, not that it went one way.
fn run(tw: &mut TestWorld, actor: ObjId, line: &str) -> (bool, String) {
    let ev = tw.run(actor, line);
    (!has_error(&ev), texts(&ev))
}
/// One unit the given player really controls — a thing they own, not another
/// player. `spawn_player` makes a player its OWN owner, so one player never
/// controls another, which is exactly the authority `@op/join` now asks about.
fn unit_of(tw: &mut TestWorld, owner: ObjId, name: &str) -> ObjId {
    tw.world_mut()
        .create(ObjKind::Thing, name, owner, Some(LIMBO))
        .expect("a unit")
}
fn snap(tw: &TestWorld) -> &EconomySnapshot {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy store")
        .snapshot()
}
fn camp(tw: &TestWorld) -> &CampaignStore {
    tw.world()
        .module_state::<CampaignStore>()
        .expect("campaign store")
}
fn bal(tw: &TestWorld, id: AccountId) -> i64 {
    snap(tw)
        .account(id)
        .expect("account")
        .posted_balance
        .minor()
}
fn held(tw: &TestWorld, id: AccountId) -> i64 {
    snap(tw).account(id).expect("account").held_amount.minor()
}
/// **The conservation probe**: the sum of every posted balance. A double-entry
/// journal keeps it constant, so any campaign path that minted or burned value
/// moves it.
fn ledger_total(tw: &TestWorld) -> i64 {
    snap(tw).accounts().map(|a| a.posted_balance.minor()).sum()
}
fn account_of(tw: &TestWorld, who: ObjId) -> AccountId {
    snap(tw)
        .accounts()
        .find(|a| a.owner == PrincipalId::Object(who))
        .expect("an opened account")
        .id
}
/// The reservation backing one operation's bond, whatever its status.
fn bond_hold(tw: &TestWorld, id: OperationId) -> Option<Hold> {
    snap(tw)
        .holds()
        .find(|h| h.purpose == mush_campaign::bond::purpose(id))
        .cloned()
}
fn state_of(tw: &TestWorld, id: OperationId) -> OperationState {
    camp(tw)
        .operation(id)
        .expect("the operation exists")
        .runtime
        .state
        .clone()
}
fn pending_of(tw: &TestWorld, id: OperationId) -> BTreeSet<ConsumerId> {
    camp(tw)
        .operation(id)
        .expect("the operation exists")
        .runtime
        .pending()
}

/// The whole economy set up once: an instrument, a faucet, and `funds` minted into
/// each named player's own account.
fn money(tw: &mut TestWorld, players: &[(ObjId, i64)]) {
    ok(tw, GOD, "@instrument/register gold=0");
    ok(tw, GOD, "@account/system faucet=gold");
    let faucet = snap(tw)
        .accounts()
        .find(|a| matches!(a.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id;
    for (who, funds) in players {
        ok(tw, GOD, &format!("@account/open #{}=gold", who.0));
        if *funds > 0 {
            let acct = account_of(tw, *who);
            ok(tw, GOD, &format!("@econ/mint {faucet}={acct} {funds}"));
        }
    }
}

type Kv = BTreeMap<SmolStr, Vec<(SmolStr, serde_json::Value)>>;

/// Fold the module ops written since the last call **into** `kv`, so a test may
/// reboot more than once without losing the earlier writes.
fn fold_into(tw: &mut TestWorld, kv: &mut Kv) {
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
                    ..Default::default()
                },
            )
        })
        .collect();
    tw.modules_loaded(boot);
}

/// **One campaign tick fire.** The tick is `Coarse(TICK_PERIOD_SECS)`, so the world
/// clock has to move a whole period for the engine to consider it due — firing at
/// second 1, 2, 3 fires nothing at all, which is exactly the trap cut 8's fold
/// warned about.
fn fire(tw: &mut TestWorld) {
    let period = i64::try_from(mush_campaign::TICK_PERIOD_SECS).unwrap_or(15);
    let t = tw.world().now() + period + 1;
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1_000));
}

fn fire_n(tw: &mut TestWorld, n: usize) {
    for _ in 0..n {
        fire(tw);
    }
}

/// Jump the world clock to `t` seconds and fire everything due there.
fn jump_to(tw: &mut TestWorld, t: i64) {
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1_000));
}

/// A live E4 contract that really attests from `operation`'s own resolution: the
/// exact `AttestorSet::Canon { kind: campaign.operation.resolved, source: op:<id> }`
/// a `stake.outcome` must name since the fold (C9 / decision E6(a)). `payee` becomes
/// the party the settlement pays, so a test can put them on the canon record.
fn contract_attesting(tw: &mut TestWorld, issuer: ObjId, operation: OperationId) -> u64 {
    let payee = tw.spawn_player(&format!("Payee{}", operation.0));
    ok(tw, GOD, &format!("@account/open #{}=gold", payee.0));
    let mine = account_of(tw, issuer);
    let theirs = account_of(tw, payee);
    let before: BTreeSet<u64> = snap(tw).contracts().map(|c| c.id.0).collect();
    ok(tw, issuer, "@contract/draft Operation");
    let id = snap(tw)
        .contracts()
        .map(|c| c.id.0)
        .find(|id| !before.contains(id))
        .expect("one new contract");
    ok(tw, issuer, &format!("@contract/party {id}=#{}", payee.0));
    ok(tw, issuer, &format!("@contract/pay {id}={mine} 10"));
    ok(tw, issuer, &format!("@contract/split {id}={theirs} 10"));
    ok(tw, issuer, &format!("@contract/remainder {id}={mine}"));
    ok(
        tw,
        issuer,
        &format!(
            "@contract/condition {id}=attest canon:{}:{}",
            mush_campaign::KIND_OPERATION_RESOLVED,
            mush_campaign::canon_source(operation)
        ),
    );
    ok(tw, issuer, &format!("@contract/offer {id}"));
    ok(tw, issuer, &format!("@contract/accept {id}"));
    ok(tw, payee, &format!("@contract/accept {id}"));
    id
}

/// Declare a bonded operation for `who`, with a 24 h notice and a 1 h window, and
/// return its id. Everything goes through the real commands.
fn declare(tw: &mut TestWorld, who: ObjId, bond: i64) -> OperationId {
    let before: BTreeSet<u64> = camp(tw).operations().map(|d| d.id().0).collect();
    ok(tw, who, "@op/declare hold the refinery");
    let id = camp(tw)
        .operations()
        .map(|d| d.id())
        .find(|id| !before.contains(&id.0))
        .expect("one new operation");
    if bond > 0 {
        let acct = account_of(tw, who);
        ok(tw, who, &format!("@op/terms {}/bond={acct}:{bond}", id.0));
    }
    ok(tw, who, &format!("@op/terms {}/notice=24", id.0));
    id
}

// ===========================================================================
// Hazard axis 1 — a campaign that mints value or goes negative
// ===========================================================================

/// **A1. Declare, activate and resolve leave the ledger's total unchanged.**
///
/// The bond moves money from *available* to *held* and back; it never creates or
/// destroys any. The observable at each step is what the attacker could actually
/// spend, read from the account another actor would be paid out of.
#[test]
fn e7c9_a1_declare_activate_and_resolve_leave_the_ledger_total_unchanged() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let acct = account_of(&tw, attacker);
    let total = ledger_total(&tw);

    let id = declare(&mut tw, attacker, 400);
    assert_eq!(ledger_total(&tw), total, "declaring minted something");
    assert_eq!(held(&tw, acct), 0, "declaring reserved something");

    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    assert_eq!(ledger_total(&tw), total, "activating minted something");
    // The OBSERVABLE consequence: 400 of the attacker's own money is spoken for,
    // so a 700 transfer they could make a moment ago is now refused by the engine.
    assert_eq!(held(&tw, acct), 400);
    assert_eq!(bal(&tw, acct), 1_000);
    let other = tw.spawn_player("Other");
    ok(&mut tw, GOD, &format!("@account/open #{}=gold", other.0));
    let other_acct = account_of(&tw, other);
    err(
        &mut tw,
        GOD,
        &format!("@econ/pay {acct}={other_acct} 700 too much"),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@econ/pay {acct}={other_acct} 500 fine"),
    );

    ok(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
    fire_n(&mut tw, 7);
    assert_eq!(state_of(&tw, id), OperationState::Resolved);
    assert_eq!(held(&tw, acct), 0, "the bond came back");
    assert_eq!(ledger_total(&tw), total, "resolving minted something");
    // …and the released money is spendable again, which is the observable form of
    // "the bond came back".
    ok(
        &mut tw,
        GOD,
        &format!("@econ/pay {acct}={other_acct} 400 released"),
    );
}

/// **A2. An unfunded attacker cannot activate, and the operation stays `Declared`.**
///
/// The refusal is the engine's own `InsufficientAvailable` (invariant 26), reached
/// through the real `@op/activate` → `mush_economy::post` path.
#[test]
fn e7c9_a2_an_unfunded_attacker_cannot_activate_and_the_operation_stays_declared() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Broke");
    money(&mut tw, &[(attacker, 10)]);
    let acct = account_of(&tw, attacker);
    let total = ledger_total(&tw);
    let id = declare(&mut tw, attacker, 500);

    let refusal = err(&mut tw, attacker, &format!("@op/activate {}", id.0));
    assert!(refusal.contains("cannot fund the bond"), "{refusal}");
    assert_eq!(state_of(&tw, id), OperationState::Declared);
    assert_eq!(held(&tw, acct), 0, "nothing was reserved");
    assert_eq!(bal(&tw, acct), 10, "and nothing was spent");
    assert_eq!(ledger_total(&tw), total);
    // The OBSERVABLE outcome another actor sees: the operation is still declared.
    let shown = ok(&mut tw, attacker, &format!("@op/show/full {}", id.0));
    assert!(shown.contains("State: declared"), "{shown}");
    assert!(shown.contains("not yet placed"), "{shown}");
    // And a tick does not sneak it forward.
    fire(&mut tw);
    assert_eq!(state_of(&tw, id), OperationState::Declared);
}

/// **A3. The bond is a `Hold` on the attacker's own account and nothing else.**
#[test]
fn e7c9_a3_the_bond_is_a_hold_on_the_attackers_own_account_and_nothing_else() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    let bystander = tw.spawn_player("Bystander");
    money(&mut tw, &[(attacker, 900), (bystander, 900)]);
    let mine = account_of(&tw, attacker);
    let theirs = account_of(&tw, bystander);
    let id = declare(&mut tw, attacker, 300);
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));

    let holds: Vec<_> = snap(&tw).holds().collect();
    assert_eq!(holds.len(), 1, "exactly one hold exists");
    let hold = holds[0];
    assert_eq!(hold.account, mine);
    assert_eq!(hold.placed_by, PrincipalId::Object(attacker));
    assert_eq!(
        hold.purpose,
        mush_economy::HoldPurpose::Reservation {
            kind: SmolStr::new_static("campaign.operation"),
            id: id.0,
        }
    );
    assert_eq!(hold.holder.namespace.as_str(), "campaign.op.bond");
    // The bystander is untouched and can still spend every unit they have.
    assert_eq!(held(&tw, theirs), 0);
    ok(&mut tw, GOD, &format!("@econ/pay {theirs}={mine} 900 all"));
}

/// **A4. The bond settles exactly once across a restart.**
///
/// The release is keyed off the root (`campaign.resolve/{op}/bond_settlement`) and
/// is an `AlreadyPosted` HIT on replay, so a reboot that re-runs the whole fan-out
/// cannot release twice. The observable: the attacker's spendable balance is the
/// same after the second drain as after the first.
#[test]
fn e7c9_a4_the_bond_settles_exactly_once_across_a_restart() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let acct = account_of(&tw, attacker);
    let id = declare(&mut tw, attacker, 250);
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    ok(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
    fire_n(&mut tw, 7);
    assert_eq!(state_of(&tw, id), OperationState::Resolved);
    let after_first = (bal(&tw, acct), held(&tw, acct), ledger_total(&tw));
    let holds_after_first = snap(&tw).holds().count();

    let mut kv = Kv::new();
    fold_into(&mut tw, &mut kv);
    reboot(&mut tw, &kv);
    // The reboot re-runs the tick — which IS the reconcile — many times over.
    fire_n(&mut tw, 10);
    assert_eq!(state_of(&tw, id), OperationState::Resolved, "it re-opened");
    assert_eq!(
        (bal(&tw, acct), held(&tw, acct), ledger_total(&tw)),
        after_first,
        "the bond settled twice across the restart"
    );
    assert_eq!(snap(&tw).holds().count(), holds_after_first);
    assert!(
        snap(&tw)
            .holds()
            .all(|h| matches!(h.status, HoldStatus::Released)),
        "the one hold is released, exactly once"
    );
}

/// **A5. A forfeit captures at most the hold, and the beneficiary can spend it.**
#[test]
fn e7c9_a5_a_forfeit_captures_at_most_the_hold_and_the_beneficiary_can_spend_it() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    let defender = tw.spawn_player("Defender");
    money(&mut tw, &[(attacker, 1_000), (defender, 0)]);
    let mine = account_of(&tw, attacker);
    let theirs = account_of(&tw, defender);
    let total = ledger_total(&tw);
    let id = declare(&mut tw, attacker, 300);
    ok(
        &mut tw,
        attacker,
        &format!("@op/terms {}/bond.forfeit=#{}:{theirs}", id.0, defender.0),
    );
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    // A forfeit is STAFF's alone since the E7 cut 9 fold (C1): it is the only path
    // that moves a player's bond to somebody else, and the tick never produces one.
    err(&mut tw, attacker, &format!("@op/resolve {}=forfeit", id.0));
    ok(&mut tw, GOD, &format!("@op/resolve {}=forfeit", id.0));
    fire_n(&mut tw, 7);
    assert_eq!(state_of(&tw, id), OperationState::Resolved);
    assert_eq!(bal(&tw, mine), 700, "exactly the hold left the attacker");
    assert_eq!(held(&tw, mine), 0, "and nothing stays reserved");
    assert_eq!(bal(&tw, theirs), 300);
    assert_eq!(ledger_total(&tw), total, "a forfeit is a move, not a mint");
    let consumed = snap(&tw)
        .holds()
        .filter(|h| matches!(h.status, HoldStatus::Consumed { .. }))
        .count();
    assert_eq!(consumed, 1);
    // The OBSERVABLE outcome: the defender can actually spend what they won.
    ok(
        &mut tw,
        GOD,
        &format!("@econ/pay {theirs}={mine} 300 spent"),
    );
}

/// **A6. `mush-campaign` reaches no mint path.** A source probe over the whole
/// crate: no `create_system_obligation`, no minting `AssetOp`, no `Amount`
/// arithmetic, and no authoritative submit.
#[test]
fn e7c9_a6_the_crate_reaches_no_mint_path_and_no_authoritative_submit() {
    let mut sources = Vec::new();
    for name in [
        "lib.rs",
        "bond.rs",
        "commands.rs",
        "facts.rs",
        "formation.rs",
        "ids.rs",
        "record.rs",
        "render.rs",
        "settle.rs",
        "store.rs",
        "terms.rs",
        "tick.rs",
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join(name);
        sources.push((
            name,
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}")),
        ));
    }
    for (name, text) in &sources {
        // The doc comments name several of these deliberately, to say the crate
        // does NOT reach them; only CODE is probed, so strip comment lines first.
        let code: String = text
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in [
            // No mint path, no faucet, no credit line, no title.
            "create_system_obligation",
            "set_system_obligation",
            "AssetOp::",
            "issue_title",
            "CreditPolicy",
            // No authoritative submit, and none of the registrations cut 9 is
            // forbidden to make.
            "submit_authoritative",
            "reg.schedule_source",
            "reg.state_family",
            "reg.revision_validator",
            "reg.clock_provider",
            "reg.domain_clock_provider",
            "reg.subscribe",
            "AuthorityGuardKind",
            // No `Amount` arithmetic: the only money numbers cut 9 handles are
            // read from the operator's own input or from a live hold.
            "amount.checked_add",
            "amount.checked_sub",
            "amount.checked_mul",
            "Amount::from_widened",
        ] {
            assert!(
                !code.contains(forbidden),
                "{name} reaches {forbidden}, which cut 9 must not"
            );
        }
    }
    // The ONE arithmetic the crate performs on money is `checked_neg` on a live
    // hold's OWN amount, to write the debit line of a capture — a sign flip on a
    // number the engine gave it, never a sum it computed.
    let bond = &sources
        .iter()
        .find(|(n, _)| *n == "bond.rs")
        .expect("bond.rs")
        .1;
    assert!(bond.contains("amount.checked_neg()"));
    assert_eq!(bond.matches("checked_neg").count(), 1);
}

/// **A7. Funds exhausted mid-run pauses and pages, never negative, never
/// continuing.**
///
/// Staff release the bond out from under a running operation (`@hold/release`, the
/// economy's own verb). The next campaign tick finds no live hold and pauses **that
/// operation only**, with the recommended command in the page.
#[test]
fn e7c9_a7_funds_exhausted_mid_run_pauses_only_the_subject_and_never_goes_negative() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let acct = account_of(&tw, attacker);
    let a = declare(&mut tw, attacker, 200);
    let b = declare(&mut tw, attacker, 100);
    ok(&mut tw, attacker, &format!("@op/activate {}", a.0));
    ok(&mut tw, attacker, &format!("@op/activate {}", b.0));
    // Since the E7 cut 9 fold (C2 / decision E2(a)) a campaign bond is an
    // **engine-owned** reservation: neither its placer nor anyone else reaches it
    // through the economy's own verbs, so this is how the money STAYS reserved.
    let doomed = bond_hold(&tw, a).expect("a's hold").id;
    for who in [attacker, GOD] {
        let refusal = err(&mut tw, who, &format!("@hold/release {}", doomed.0));
        assert!(refusal.contains("campaign.operation"), "{refusal}");
    }
    assert_eq!(held(&tw, acct), 300, "an @hold verb reached the bond");

    // The fault this test is about is the bond going away underneath a live
    // operation. With the economy's verbs closed, it is reached the way a lost or
    // repaired document would produce it: the reservation really is settled, and the
    // operation document says it is still activated.
    ok(&mut tw, attacker, &format!("@op/resolve {}=success", a.0));
    fire_n(&mut tw, 9);
    assert_eq!(held(&tw, acct), 100, "only b's bond is still reserved");
    let mut kv = Kv::new();
    fold_into(&mut tw, &mut kv);
    let entry = kv
        .get_mut(&SmolStr::new_static("campaign"))
        .expect("campaign wrote");
    for (k, v) in entry.iter_mut() {
        if k.as_str() == format!("op:{}", a.0) {
            v["runtime"]["state"] = serde_json::json!({ "state": "activated" });
            v["runtime"]["root"] = serde_json::Value::Null;
            v["runtime"]["required"] = serde_json::json!([]);
        }
    }
    reboot(&mut tw, &kv);
    assert_eq!(state_of(&tw, a), OperationState::Activated);

    fire(&mut tw);
    let shown = ok(&mut tw, attacker, &format!("@op/show/full {}", a.0));
    assert!(shown.contains("paused: unfunded"), "{shown}");
    // Only the subject is stopped: b is untouched and still moves.
    let other = ok(&mut tw, attacker, &format!("@op/show/full {}", b.0));
    assert!(!other.contains("paused"), "{other}");
    assert_eq!(held(&tw, acct), 100, "b's bond still stands");
    assert!(bal(&tw, acct) >= 0, "an account went negative");
    // And a paused operation stays paused across further fires rather than
    // silently continuing unfunded.
    fire_n(&mut tw, 4);
    assert_eq!(state_of(&tw, a), OperationState::Activated);
}

// ===========================================================================
// Hazard axis 2 — an objective evaluated twice across a crash
// ===========================================================================

/// **I1. The tick fires TWICE in sequence and the second stages rather than
/// repeating a key or freezing** (cut 8's fold, addendum 4).
#[test]
fn e7c9_i1_the_campaign_tick_fires_twice_and_the_second_fire_stages() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let id = declare(&mut tw, attacker, 100);
    // A front stake and a contract stake, so all five consumers are required and
    // the fan-out really has more than one fire's worth of work.
    ok(
        &mut tw,
        attacker,
        &format!("@op/terms {}/stake.front=1:defacto", id.0),
    );
    // The contract stake names a REAL contract that really attests from this
    // operation's own resolution: since the fold (C9 / decision E6(a)) a stake that
    // nothing could ever settle is refused where it is written.
    let contract = contract_attesting(&mut tw, attacker, id);
    ok(
        &mut tw,
        attacker,
        &format!("@op/terms {}/stake.outcome={contract}:success", id.0),
    );
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    ok(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
    let required = pending_of(&tw, id).len();
    assert_eq!(required, 5, "every consumer arm must be required here");

    fire(&mut tw);
    let after_one = pending_of(&tw, id).len();
    assert_eq!(
        after_one,
        required - 1,
        "the first fire advanced exactly one"
    );
    fire(&mut tw);
    let after_two = pending_of(&tw, id).len();
    assert_eq!(
        after_two,
        required - 2,
        "the second fire staged, rather than repeating or freezing"
    );
    assert_eq!(
        camp(&tw).last_fire().consumers,
        1,
        "one per operation, per fire"
    );
}

/// **I2. A re-run of the tick over the same documents is a no-op past the applied
/// point, and `Resolved` is unreachable while a consumer is pending.**
#[test]
fn e7c9_i2_a_rerun_is_a_no_op_and_resolved_is_unreachable_while_a_consumer_pends() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let id = declare(&mut tw, attacker, 100);
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    ok(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
    // Every fire but the last leaves at least one consumer pending, and the state
    // is `Resolving` for every one of them.
    let mut t = 1;
    while !pending_of(&tw, id).is_empty() {
        assert!(
            matches!(state_of(&tw, id), OperationState::Resolving { .. }),
            "Resolved was reached with {} consumer(s) pending",
            pending_of(&tw, id).len()
        );
        fire(&mut tw);
        t += 1;
        assert!(t < 20, "the fan-out never drained");
    }
    assert_eq!(state_of(&tw, id), OperationState::Resolved);
    let settled = camp(&tw).operation(id).expect("op").runtime.revision;
    let total = ledger_total(&tw);
    // Ten more fires change nothing at all.
    fire_n(&mut tw, 10);
    assert_eq!(
        camp(&tw).operation(id).expect("op").runtime.revision,
        settled
    );
    assert_eq!(ledger_total(&tw), total);
}

/// **I3. A crash mid-fan-out resumes and does not double-pay.**
#[test]
fn e7c9_i3_a_crash_after_part_of_the_fan_out_resumes_and_does_not_double_pay() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let acct = account_of(&tw, attacker);
    let id = declare(&mut tw, attacker, 350);
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    ok(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
    // Two consumers, then the process dies.
    fire(&mut tw);
    fire(&mut tw);
    let mid = pending_of(&tw, id);
    assert!(!mid.is_empty() && mid.len() < 5, "mid fan-out: {mid:?}");

    let mut kv = Kv::new();
    fold_into(&mut tw, &mut kv);
    reboot(&mut tw, &kv);
    assert_eq!(
        pending_of(&tw, id),
        mid,
        "the stored set moved across a reboot"
    );
    fire_n(&mut tw, 10);
    assert_eq!(state_of(&tw, id), OperationState::Resolved);
    assert_eq!(bal(&tw, acct), 1_000, "the bond was paid twice");
    assert_eq!(held(&tw, acct), 0);
    assert_eq!(
        snap(&tw)
            .holds()
            .filter(|h| matches!(h.status, HoldStatus::Released))
            .count(),
        1
    );
}

/// **I4. The visited and worked budgets are enforced independently, and the cursor
/// resumes past the last VISITED id** — skipped ones included.
#[test]
fn e7c9_i4_the_visited_and_worked_budgets_are_independent_and_the_cursor_resumes() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 100_000)]);
    // More operations than one fire may WORK, all of them resolving.
    let mut ids = Vec::new();
    const {
        assert!(
            mush_campaign::MAX_WORKED_PER_FIRE + 4 <= mush_campaign::MAX_OPERATIONS_PER_ISSUER,
            "the budget fixture must fit inside one issuer's cap"
        );
    }
    for _ in 0..mush_campaign::MAX_WORKED_PER_FIRE + 4 {
        let id = declare(&mut tw, attacker, 1);
        ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
        ok(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
        ids.push(id);
    }
    let full: usize = pending_of(&tw, ids[0]).len();
    fire(&mut tw);
    let stats = camp(&tw).last_fire();
    assert_eq!(
        stats.worked,
        mush_campaign::MAX_WORKED_PER_FIRE,
        "the worked budget did not bind"
    );
    assert!(
        stats.visited <= mush_campaign::MAX_VISITED_PER_FIRE,
        "the visited budget was exceeded"
    );
    assert_eq!(
        stats.visited, stats.worked,
        "every visit here is workable, so the two budgets bind at the same place"
    );
    assert_eq!(stats.consumers, mush_campaign::MAX_WORKED_PER_FIRE);
    // The first `MAX_WORKED_PER_FIRE` advanced; the rest were never reached.
    let advanced = |tw: &TestWorld, id: OperationId| pending_of(tw, id).len() < full;
    for id in &ids[..mush_campaign::MAX_WORKED_PER_FIRE] {
        assert!(advanced(&tw, *id), "{id} should have advanced");
    }
    for id in &ids[mush_campaign::MAX_WORKED_PER_FIRE..] {
        assert!(!advanced(&tw, *id), "{id} was worked past the budget");
    }
    // The next fire resumes PAST the last VISITED id rather than at the top, so
    // the four that were cut off are the ones that move now.
    fire(&mut tw);
    for id in &ids[mush_campaign::MAX_WORKED_PER_FIRE..] {
        assert!(
            advanced(&tw, *id),
            "{id} was starved by the rotation cursor"
        );
    }
}

/// **I5. A permanently failing consumer pauses only its own operation and does not
/// starve another's fan-out** (08c §5.2 B4's wall-of-disputes shape).
///
/// The failure is real: the operation's own canon root is dropped from the store
/// before a reboot, so `FrontControl`/`ContractOutcomes` can never find it.
#[test]
fn e7c9_i5_a_permanently_failing_consumer_starves_nobody_else() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let doomed = declare(&mut tw, attacker, 50);
    let healthy = declare(&mut tw, attacker, 50);
    // A front stake makes `FrontControl` required — and that consumer is
    // discharged ONLY by the root canon record, which this test then removes.
    ok(
        &mut tw,
        attacker,
        &format!("@op/terms {}/stake.front=1:defacto", doomed.0),
    );
    for id in [doomed, healthy] {
        ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    }
    ok(
        &mut tw,
        attacker,
        &format!("@op/resolve {}=success", doomed.0),
    );
    let mut kv = Kv::new();
    fold_into(&mut tw, &mut kv);
    // Drop the canon ledger entirely AND the durable marker that names the record:
    // since the E7 cut 9 fold (D1 / decision E4(a)) the commit-once evidence is the
    // operation's own `RootFact.canon`, so a canon-backed consumer refuses exactly
    // when the record it names is not there to have been committed.
    kv.remove(&SmolStr::new_static("canon"));
    let entry = kv
        .get_mut(&SmolStr::new_static("campaign"))
        .expect("campaign wrote");
    for (k, v) in entry.iter_mut() {
        if k.as_str() == format!("op:{}", doomed.0) {
            v["runtime"]["root"]["canon"] = serde_json::Value::Null;
        }
    }
    reboot(&mut tw, &kv);
    ok(
        &mut tw,
        attacker,
        &format!("@op/resolve {}=success", healthy.0),
    );

    fire_n(&mut tw, 24);
    // The doomed one is paused, with the reason and the command in the page.
    let shown = ok(&mut tw, GOD, &format!("@op/consumers {}", doomed.0));
    assert!(shown.contains("paused: consumer-failed"), "{shown}");
    assert!(!matches!(state_of(&tw, doomed), OperationState::Resolved));
    // The healthy one drained anyway: one operation's wall of refusals starves
    // nobody.
    assert_eq!(state_of(&tw, healthy), OperationState::Resolved);
}

/// **I6. A resolution advancing a front writes ONE `ControlRecord` change, and
/// re-running the consumer writes none.**
///
/// Cut 9 writes no `ControlRecord` at all: the ONE root canon record is the whole
/// mechanism, through `mush-faction`'s pre-existing `on_canon_recorded` observer.
#[test]
fn e7c9_i6_a_resolution_advances_a_front_once_and_a_rerun_writes_nothing() {
    let mut tw = boot();
    let commander = tw.spawn_player("Nick");
    money(&mut tw, &[(commander, 1_000)]);
    ok(&mut tw, GOD, "@faction/create Undertow");
    ok(&mut tw, GOD, "@faction/add Undertow=Nick");
    ok(&mut tw, GOD, "@region/create The Waterfront");
    ok(&mut tw, GOD, "@front/create Undertow=Seize the waterfront");
    ok(&mut tw, GOD, "@clock/create 0=Waterfront falls/1");
    ok(&mut tw, GOD, "@clock/set 0=on:campaign.operation.resolved");
    ok(&mut tw, GOD, "@clock/set 0=control:The Waterfront/defacto");
    let before = ok(&mut tw, GOD, "+territory");

    let id = declare(&mut tw, commander, 100);
    ok(
        &mut tw,
        commander,
        &format!("@op/terms {}/stake.front=0:defacto", id.0),
    );
    ok(&mut tw, commander, &format!("@op/activate {}", id.0));
    // The commander's own object is a participant, so the root record names an
    // entity the faction's relevance gate recognises.
    ok(
        &mut tw,
        commander,
        &format!("@op/join {}=#{}", id.0, commander.0),
    );
    ok(&mut tw, commander, &format!("@op/resolve {}=success", id.0));
    fire_n(&mut tw, 9);
    assert_eq!(state_of(&tw, id), OperationState::Resolved);
    let after = ok(&mut tw, GOD, "+territory");
    assert_ne!(after, before, "the front consequence never fired");
    assert!(after.contains("Undertow"), "{after}");

    // Re-running the consumer writes none: the operation is `Resolved`, twenty
    // more fires change nothing, and the territory reads exactly the same.
    fire_n(&mut tw, 20);
    assert_eq!(ok(&mut tw, GOD, "+territory"), after);
}

// ===========================================================================
// Hazard axis 3 — an outcome that leaks a fogged truth
// ===========================================================================

/// **A-lens 1. `@op/show` for a non-audience viewer is byte-identical for a covert
/// operation and for an id that never existed, at EVERY verbosity.**
#[test]
fn e7c9_x1_a_covert_operation_and_a_nonexistent_id_are_one_answer_at_every_verbosity() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    let stranger = tw.spawn_player("Stranger");
    money(&mut tw, &[(attacker, 1_000)]);
    let id = declare(&mut tw, attacker, 100);
    ok(
        &mut tw,
        attacker,
        &format!("@op/terms {}/visibility=covert", id.0),
    );
    for verb in ["@op/show", "@op/show/brief", "@op/show/full"] {
        let covert = err(&mut tw, stranger, &format!("{verb} {}", id.0));
        let absent = err(&mut tw, stranger, &format!("{verb} 99999"));
        assert_eq!(covert, absent, "{verb} distinguishes covert from absent");
        assert!(covert.contains(NO_SUCH_OPERATION), "{covert}");
    }
    // …and the listing does not name it either.
    let list = ok(&mut tw, stranger, "@op/list");
    assert!(!list.contains(&format!("Operation {id}")), "{list}");
}

/// **A-lens 2. An audience member sees the window and the notice but NOT the plan
/// when `announce_issuer` is false — and the issuer joins only when it is true.**
#[test]
fn e7c9_x2_an_audience_member_sees_the_window_but_not_the_plan() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    let defender = tw.spawn_player("Defender");
    money(&mut tw, &[(attacker, 1_000)]);
    let id = declare(&mut tw, attacker, 100);
    ok(
        &mut tw,
        attacker,
        &format!("@op/terms {}/defender=#{}", id.0, defender.0),
    );
    ok(
        &mut tw,
        attacker,
        &format!("@op/terms {}/visibility=audience", id.0),
    );
    for verb in ["@op/show", "@op/show/full"] {
        let seen = ok(&mut tw, defender, &format!("{verb} {}", id.0));
        assert!(seen.contains("window"), "{seen}");
        assert!(seen.contains("entitled to defend"), "{seen}");
        for leak in [
            "hold the refinery",
            "Bond",
            "Participants",
            "State:",
            "declared by",
        ] {
            assert!(!seen.contains(leak), "{verb} leaked {leak}: {seen}");
        }
    }
    // With `announce`, the issuer — and only the issuer — joins the notice.
    ok(
        &mut tw,
        attacker,
        &format!("@op/terms {}/visibility=announce", id.0),
    );
    let seen = ok(&mut tw, defender, &format!("@op/show/full {}", id.0));
    assert!(
        seen.contains(&format!("declared by #{}", attacker.0)),
        "{seen}"
    );
    assert!(!seen.contains("Bond"), "{seen}");
}

/// **A-lens 3. A recursive leak scan over every player-facing serialization finds
/// no covert operation's existence, no account id, and no unobserved unit.**
#[test]
fn e7c9_x3_a_leak_scan_over_every_player_facing_render_finds_nothing() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    let unit = unit_of(&mut tw, attacker, "Unit");
    let stranger = tw.spawn_player("Stranger");
    money(&mut tw, &[(attacker, 1_000)]);
    let id = declare(&mut tw, attacker, 100);
    let acct = account_of(&tw, attacker);
    ok(&mut tw, attacker, &format!("@op/join {}=#{}", id.0, unit.0));
    ok(
        &mut tw,
        attacker,
        &format!("@op/terms {}/visibility=covert", id.0),
    );
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    ok(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
    fire_n(&mut tw, 9);

    let mut seen = String::new();
    for line in [
        "@op/list".to_owned(),
        format!("@op/show {}", id.0),
        format!("@op/show/brief {}", id.0),
        format!("@op/show/full {}", id.0),
        format!("@op/consumers {}", id.0),
        format!("@op/terms {}/objective=x", id.0),
        format!("@op/activate {}", id.0),
        format!("@op/join {}=#{}", id.0, stranger.0),
        format!("@op/withdraw {}=#{}", id.0, unit.0),
        format!("@op/resolve {}=abort", id.0),
        "@formation/list".to_owned(),
        "+chronicle".to_owned(),
    ] {
        let events = tw.run(stranger, &line);
        seen.push('\n');
        seen.push_str(&texts(&events));
    }
    for leak in [
        &format!("Operation {id}"),
        "hold the refinery",
        &format!("account #{}", acct.0),
        &format!("#{}", unit.0),
        "campaign.operation",
        "bond",
    ] {
        assert!(
            !seen.contains(leak),
            "a stranger's renders leaked {leak:?}:\n{seen}"
        );
    }
    // The operation really did resolve — the fog is what a stranger sees, not the
    // absence of the thing.
    assert_eq!(state_of(&tw, id), OperationState::Resolved);
}

/// **A-lens 4. An audience over `MAX_AUDIENCE` refuses, and `@op/consumers` is
/// staff-only with the same bytes a nonexistent id gets.**
#[test]
fn e7c9_x4_an_oversized_audience_refuses_and_consumers_is_staff_only() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let id = declare(&mut tw, attacker, 100);
    for n in 0..mush_economy::MAX_AUDIENCE {
        ok(
            &mut tw,
            attacker,
            &format!("@op/terms {}/defender=#{}", id.0, n + 10),
        );
    }
    let refusal = err(
        &mut tw,
        attacker,
        &format!("@op/terms {}/defender=#9001", id.0),
    );
    assert!(refusal.contains("over its cap"), "{refusal}");
    // …and the stored surface is still the last legal one, so the refusal changed
    // nothing a defender consented to.
    let stored = camp(&tw)
        .operation(id)
        .expect("op")
        .terms
        .eligible_defenders
        .members
        .len();
    assert_eq!(stored, mush_economy::MAX_AUDIENCE);

    let staff_only = err(&mut tw, attacker, &format!("@op/consumers {}", id.0));
    let absent = err(&mut tw, attacker, "@op/consumers 99999");
    assert_eq!(staff_only, absent);
    assert!(staff_only.contains(NO_SUCH_OPERATION));
    ok(&mut tw, GOD, &format!("@op/consumers {}", id.0));
}

/// **A-lens 5. A covert resolution is `Secret` canon and is hidden from every
/// non-staff viewer's chronicle.**
#[test]
fn e7c9_x5_a_covert_resolution_is_secret_canon_and_a_public_one_is_not() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    let stranger = tw.spawn_player("Stranger");
    money(&mut tw, &[(attacker, 1_000)]);
    let covert = declare(&mut tw, attacker, 10);
    ok(
        &mut tw,
        attacker,
        &format!("@op/terms {}/visibility=covert", covert.0),
    );
    ok(
        &mut tw,
        attacker,
        &format!("@op/resolve {}=success", covert.0),
    );
    fire_n(&mut tw, 7);
    let chronicle = texts(&tw.run(stranger, "+chronicle"));
    assert!(
        !chronicle.contains(&format!("Operation {covert}")),
        "{chronicle}"
    );
    // Staff DO see it, so the record exists and it is the scope doing the work.
    let staff = ok(&mut tw, GOD, "@canon");
    assert!(staff.contains(&format!("Operation {covert}")), "{staff}");

    let public = declare(&mut tw, attacker, 10);
    ok(
        &mut tw,
        attacker,
        &format!("@op/terms {}/visibility=public", public.0),
    );
    ok(
        &mut tw,
        attacker,
        &format!("@op/resolve {}=success", public.0),
    );
    fire_n(&mut tw, 8);
    let chronicle = texts(&tw.run(stranger, "+chronicle"));
    assert!(
        chronicle.contains(&format!("Operation {public}")),
        "a public resolution should reach the chronicle: {chronicle}"
    );
}

// ===========================================================================
// Freeze (Z)
// ===========================================================================

/// **Z1. A terms edit after activation refuses; a roster edit leaves the consent
/// hash byte-identical.**
#[test]
fn e7c9_z1_terms_freeze_at_activation_while_the_roster_keeps_moving() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let id = declare(&mut tw, attacker, 100);
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    let frozen = camp(&tw).operation(id).expect("op").terms.consent_hash;

    let refusal = err(
        &mut tw,
        attacker,
        &format!("@op/terms {}/objective=something else", id.0),
    );
    assert!(
        refusal.contains("froze when the operation was activated"),
        "{refusal}"
    );
    assert_eq!(
        camp(&tw).operation(id).expect("op").terms.consent_hash,
        frozen
    );

    // Eight roster moves through the REAL commands; the frozen half never moves.
    let mut units = Vec::new();
    for n in 0..8 {
        let u = unit_of(&mut tw, attacker, &format!("Unit{n}"));
        ok(&mut tw, attacker, &format!("@op/join {}=#{}", id.0, u.0));
        units.push(u);
        assert_eq!(
            camp(&tw).operation(id).expect("op").terms.consent_hash,
            frozen
        );
    }
    for u in &units {
        ok(
            &mut tw,
            attacker,
            &format!("@op/withdraw {}=#{}", id.0, u.0),
        );
        assert_eq!(
            camp(&tw).operation(id).expect("op").terms.consent_hash,
            frozen
        );
    }
    // The runtime moved sixteen times and the terms hash zero times.
    assert!(camp(&tw).operation(id).expect("op").runtime.revision > 16);
    // …and the operation is still intact, so nothing about it disputes.
    assert!(camp(&tw).operation(id).expect("op").consent_intact());
}

/// **Z2. A mutated surface makes the intactness check false and the operation
/// disputes and pages rather than settling.**
#[test]
fn e7c9_z2_a_mutated_surface_disputes_and_pages_rather_than_settling() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let acct = account_of(&tw, attacker);
    let id = declare(&mut tw, attacker, 300);
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));

    // Mutate the stored surface out from under the frozen hash — the shape a
    // corrupted document, a bad migration or a hand edit would take — and reboot.
    let mut kv = Kv::new();
    fold_into(&mut tw, &mut kv);
    let entry = kv
        .get_mut(&SmolStr::new_static("campaign"))
        .expect("campaign wrote");
    for (k, v) in entry.iter_mut() {
        if k.as_str() == format!("op:{}", id.0) {
            v["terms"]["objective"]["summary"] = serde_json::json!("something else entirely");
        }
    }
    reboot(&mut tw, &kv);
    assert!(!camp(&tw).operation(id).expect("op").consent_intact());

    // It refuses to settle: neither the command nor the tick moves it, and the
    // money stays exactly where it is.
    err(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
    fire_n(&mut tw, 9);
    assert!(!matches!(state_of(&tw, id), OperationState::Resolved));
    assert_eq!(
        held(&tw, acct),
        300,
        "a broken surface still settled the bond"
    );
    let shown = ok(&mut tw, attacker, &format!("@op/show/full {}", id.0));
    assert!(shown.contains("paused: terms-broken"), "{shown}");
}

/// **Z3. The notice window sits inside `OPERATION_NOTICE_MIN`/`MAX`.**
#[test]
fn e7c9_z3_the_notice_window_sits_inside_the_declared_band() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let id = declare(&mut tw, attacker, 0);
    for hours in ["1", "23", "73", "1000"] {
        let refusal = err(
            &mut tw,
            attacker,
            &format!("@op/terms {}/notice={hours}", id.0),
        );
        assert!(refusal.contains("outside"), "{hours}: {refusal}");
    }
    for hours in ["24", "48", "72"] {
        ok(
            &mut tw,
            attacker,
            &format!("@op/terms {}/notice={hours}", id.0),
        );
    }
    let terms = &camp(&tw).operation(id).expect("op").terms;
    let lead = terms.window.0.at - terms.notice.at;
    assert_eq!(lead, 72 * HOUR);
}

// ===========================================================================
// One operation, composition, and the objective pull
// ===========================================================================

/// **R1. A unit already in an operation refuses a second; withdrawal releases it.**
#[test]
fn e7c9_r1_a_unit_is_in_at_most_one_operation_and_withdrawal_releases_it() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let unit = unit_of(&mut tw, attacker, "Unit");
    let a = declare(&mut tw, attacker, 10);
    let b = declare(&mut tw, attacker, 10);
    ok(&mut tw, attacker, &format!("@op/join {}=#{}", a.0, unit.0));
    let refusal = err(&mut tw, attacker, &format!("@op/join {}=#{}", b.0, unit.0));
    assert!(refusal.contains("already committed"), "{refusal}");
    // The OBSERVABLE outcome: b's roster really is empty.
    let shown = ok(&mut tw, attacker, &format!("@op/show {}", b.0));
    assert!(shown.contains("Participants: 0"), "{shown}");

    ok(
        &mut tw,
        attacker,
        &format!("@op/withdraw {}=#{}", a.0, unit.0),
    );
    ok(&mut tw, attacker, &format!("@op/join {}=#{}", b.0, unit.0));
    let shown = ok(&mut tw, attacker, &format!("@op/show {}", b.0));
    assert!(shown.contains("Participants: 1"), "{shown}");
}

/// **R2/R3. A formation exists, deploys and keeps its administrative parent when it
/// is attached to an operation — and every `force.*` id is in the vocabulary.**
#[test]
fn e7c9_r2_an_understrength_formation_exists_deploys_and_keeps_its_parent() {
    let mut tw = boot();
    let commander = tw.spawn_player("Commander");
    money(&mut tw, &[(commander, 1_000)]);
    ok(&mut tw, GOD, "@org/create Ironhold");
    let scope = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("org")
        .state()
        .orgs()
        .next()
        .expect("one org")
        .root_scope;

    ok(&mut tw, GOD, &format!("@force/create lance={}", scope.0));
    ok(&mut tw, GOD, &format!("@force/create lance={}", scope.0));
    let ids: Vec<_> = camp(&tw).formations().map(|n| n.id).collect();
    assert_eq!(ids.len(), 2);
    let (child, parent) = (ids[0], ids[1]);
    ok(&mut tw, GOD, &format!("@force/attach {child}={parent}"));

    // Understrength — two of four — and it still exists and still deploys.
    let unit_a = tw.spawn_player("A");
    let unit_b = tw.spawn_player("B");
    for u in [unit_a, unit_b] {
        ok(&mut tw, GOD, &format!("@force/assign {child}=#{}", u.0));
    }
    let shown = ok(&mut tw, GOD, &format!("@formation/show {child}"));
    assert!(shown.contains("understrength (2 of 4)"), "{shown}");
    assert!(shown.contains("still exists and still deploys"), "{shown}");
    // The same constraint set is reported in every strictness scope.
    for scope_label in ["creation", "doctrine", "operation"] {
        assert!(shown.contains(scope_label), "{scope_label}: {shown}");
    }

    // Deploying it into an operation does NOT change its administrative parent.
    let id = declare(&mut tw, commander, 10);
    ok(&mut tw, commander, &format!("@op/activate {}", id.0));
    // `@op/join <id>=fm<n>` asks `force.command` on the node's OWN scope, so the
    // commander cannot deploy a formation they hold no authority over; staff can.
    let refusal = err(&mut tw, commander, &format!("@op/join {}={child}", id.0));
    assert!(refusal.contains("Permission denied"), "{refusal}");
    ok(&mut tw, GOD, &format!("@op/join {}={child}", id.0));
    let node = camp(&tw).formation(child).expect("node").clone();
    assert_eq!(
        node.parent,
        Some(parent),
        "attachment moved the admin parent"
    );
    let shown = ok(&mut tw, commander, &format!("@op/show {}", id.0));
    assert!(shown.contains("Participants: 2"), "{shown}");
    let shown = ok(&mut tw, GOD, &format!("@formation/show {child}"));
    assert!(shown.contains(&format!("parent {parent}")), "{shown}");
}

/// **R4. A `force.*` id missing from the vocabulary fails at REGISTRATION, not
/// silently at use; and one granted before it was listed still authorizes nothing
/// while `@org/show` flags rather than deletes it.**
#[test]
fn e7c9_r4_a_force_capability_outside_the_vocabulary_fails_at_the_grant() {
    let mut tw = boot();
    let officer = tw.spawn_player("Officer");
    ok(&mut tw, GOD, "@org/create Ironhold");
    let scope = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("org")
        .state()
        .orgs()
        .next()
        .expect("one org")
        .root_scope;
    // Every declared id is grantable…
    for capability in mush_campaign::CAPABILITIES {
        ok(
            &mut tw,
            GOD,
            &format!(
                "@org/grant #{}=Ironhold {capability} scope={}",
                officer.0, scope.0
            ),
        );
    }
    // …and a typo is refused AT THE GRANT rather than stored as an inert record.
    let refusal = err(
        &mut tw,
        GOD,
        &format!(
            "@org/grant #{}=Ironhold force.comand scope={}",
            officer.0, scope.0
        ),
    );
    assert!(!refusal.is_empty());
    // …and `@org/show` lists exactly the declared ids: the typo became no record
    // at all, rather than an inert grant nobody would notice.
    let shown = ok(&mut tw, GOD, "@org/show Ironhold");
    for capability in mush_campaign::CAPABILITIES {
        assert!(shown.contains(capability), "{capability}: {shown}");
    }
    assert!(!shown.contains("force.comand"), "{shown}");
}

/// **R5. A non-officer cannot create, attach or assign a formation, and the
/// refusals are the same "Permission denied." on every arm.**
#[test]
fn e7c9_r5_force_authority_is_checked_on_every_arm() {
    let mut tw = boot();
    let stranger = tw.spawn_player("Stranger");
    ok(&mut tw, GOD, "@org/create Ironhold");
    let scope = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("org")
        .state()
        .orgs()
        .next()
        .expect("one org")
        .root_scope;
    ok(&mut tw, GOD, &format!("@force/create lance={}", scope.0));
    ok(&mut tw, GOD, &format!("@force/create star={}", scope.0));
    let ids: Vec<_> = camp(&tw).formations().map(|n| n.id).collect();
    let (a, b) = (ids[0], ids[1]);
    // `@force/create` names a scope the caller supplied, so the refusal is the
    // authority's. The three that name an existing NODE answer `No such formation.`
    // since the E7 cut 9 fold (B5): the node was looked up BEFORE the capability was
    // asked about, so "not yours" and "no such thing" were two different answers and
    // a stranger could enumerate the order of battle by refusal.
    let refusal = err(
        &mut tw,
        stranger,
        &format!("@force/create lance={}", scope.0),
    );
    assert!(refusal.contains("Permission denied"), "{refusal}");
    for line in [
        format!("@force/attach {a}={b}"),
        format!("@force/detach {a}"),
        format!("@force/assign {a}=#{}", stranger.0),
    ] {
        let refusal = err(&mut tw, stranger, &line);
        assert!(
            refusal.contains(mush_campaign::NO_SUCH_FORMATION),
            "{line}: {refusal}"
        );
        // …and byte-identical to the answer for a node that never existed.
        let absent = err(&mut tw, stranger, &line.replace(&a.to_string(), "fm99999"));
        assert_eq!(absent, refusal, "{line}: an existence oracle");
    }
    // Nothing moved.
    assert_eq!(camp(&tw).formations().count(), 2);
    assert!(camp(&tw).formation(a).expect("a").members.is_empty());
}

/// **O1. The objective is a PULL over durable state.** A `canon:` objective is
/// discharged by a canon record the campaign layer never observed being written —
/// the tick reads canon's own ledger on its next fire.
#[test]
fn e7c9_o1_a_canon_objective_is_discharged_by_a_pull_not_an_observer() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let id = declare(&mut tw, attacker, 10);
    ok(
        &mut tw,
        attacker,
        &format!(
            "@op/terms {}/objective.kind=canon:front.progress:clock:0",
            id.0
        ),
    );
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    // Move IC time past the window's open so the operation is Running…
    jump_to(&mut tw, 25 * HOUR);
    assert_eq!(state_of(&tw, id), OperationState::Running);
    // …and it stays Running while the canon record does not exist.
    fire(&mut tw);
    assert_eq!(state_of(&tw, id), OperationState::Running);

    // Now a completely unrelated subsystem records the canon the objective names.
    ok(&mut tw, GOD, "@faction/create Undertow");
    ok(&mut tw, GOD, "@front/create Undertow=Seize the waterfront");
    ok(&mut tw, GOD, "@clock/create 0=Waterfront falls/1");
    ok(&mut tw, GOD, "@clock/advance 0");
    // The very next fire pulls it and resolves.
    fire(&mut tw);
    assert!(
        matches!(
            state_of(&tw, id),
            OperationState::Resolving { .. } | OperationState::Resolved
        ),
        "the objective was not pulled: {:?}",
        state_of(&tw, id)
    );
}

// ===========================================================================
// Wiring / projection (P)
// ===========================================================================

/// **P1. The crate's edge set is exactly `{core, economy, canon, org, sim}`** —
/// no faction, no tactics, no propagation, no ruleset. `cargo tree -p
/// mush-campaign` is the gate; this is the same fact read from the manifest, so a
/// stray edge fails a unit test rather than only a human's reading.
///
/// **E7 cut 10 added the ONE `mush-sim` edge** (10b §3.10 OPEN 1, as the owner
/// decided): `handoff.rs` is a private module and `ModuleActionState` is
/// owner-scoped, so the K6 request path and the `control_leases` family are
/// unreachable from outside `mush-sim`. The campaign layer crosses that edge
/// through exactly **two** exported `pub fn` seams —
/// `mush_sim::{request_seat_activation, channel_lease_holder}` — and the edge is
/// acyclic (`mush-sim -> {core, economy, scenes, proto}`, never campaign).
#[test]
fn e7c9_p1_the_manifest_declares_exactly_five_edges() {
    let manifest = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
    )
    .expect("the manifest reads");
    let deps = manifest
        .split("[dependencies]")
        .nth(1)
        .expect("a dependencies table")
        .split("[dev-dependencies]")
        .next()
        .expect("a dev-dependencies table follows");
    let edges: BTreeSet<&str> = deps
        .lines()
        .filter_map(|l| l.split('.').next())
        .map(str::trim)
        .filter(|l| l.starts_with("mush-"))
        .collect();
    assert_eq!(
        edges,
        BTreeSet::from([
            "mush-canon",
            "mush-core",
            "mush-economy",
            "mush-org",
            "mush-sim",
        ]),
        "the campaign crate's edge set moved"
    );
}

/// **P2. The fact provider answers from the applied marker, never from a status.**
#[test]
fn e7c9_p2_the_campaign_fact_answers_from_the_applied_marker_never_a_status() {
    use mush_economy::FactProvider;
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let id = declare(&mut tw, attacker, 10);
    let provider = mush_campaign::CampaignFactProvider;
    assert_eq!(provider.provider(), "campaign");
    let ask = |tw: &TestWorld| {
        provider.resolve(
            tw.world(),
            mush_campaign::KIND_OPERATION_RESOLVED,
            &id.0.to_string(),
            None,
        )
    };
    // Declared, activated and running all answer UNKNOWN — which is unsatisfied,
    // which never settles money.
    assert!(ask(&tw).is_none());
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    assert!(ask(&tw).is_none());
    ok(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
    // Entering `Resolving` writes the applied root marker, so the fact holds from
    // there — the marker, not the terminal status.
    assert!(matches!(
        state_of(&tw, id),
        OperationState::Resolving { .. }
    ));
    let answered = ask(&tw).expect("the applied marker answers");
    assert!(answered.satisfied);
    // An unknown kind, an unknown id and a stale revision all answer unknown.
    assert!(
        provider
            .resolve(tw.world(), "campaign.nope", &id.0.to_string(), None)
            .is_none()
    );
    assert!(
        provider
            .resolve(
                tw.world(),
                mush_campaign::KIND_OPERATION_RESOLVED,
                "99999",
                None
            )
            .is_none()
    );
    assert!(
        provider
            .resolve(
                tw.world(),
                mush_campaign::KIND_OPERATION_RESOLVED,
                &id.0.to_string(),
                Some(7)
            )
            .is_none()
    );
}

/// **P3. A corrupt operation document is skipped loudly, HELD not deleted, with its
/// subject marked — and its id is never re-issued underneath it.**
#[test]
fn e7c9_p3_a_corrupt_operation_document_is_held_and_its_id_is_never_reissued() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let doomed = declare(&mut tw, attacker, 100);
    let healthy = declare(&mut tw, attacker, 100);
    let mut kv = Kv::new();
    fold_into(&mut tw, &mut kv);
    let entry = kv
        .get_mut(&SmolStr::new_static("campaign"))
        .expect("campaign wrote");
    for (k, v) in entry.iter_mut() {
        if k.as_str() == format!("op:{}", doomed.0) {
            // A roster over the cap: refused on load, never truncated.
            v["runtime"]["participants"] = serde_json::Value::Array(
                (0..=mush_campaign::MAX_OPERATION_PARTICIPANTS)
                    .map(|n| {
                        serde_json::json!({
                            "unit": n as u32 + 1,
                            "side": "attack",
                            "command_parent": null,
                            "joined_at": 0
                        })
                    })
                    .collect(),
            );
        }
    }
    reboot(&mut tw, &kv);
    assert!(
        camp(&tw).operation(doomed).is_none(),
        "it was installed anyway"
    );
    assert!(
        camp(&tw).operation(healthy).is_some(),
        "the healthy one loaded"
    );
    let held_docs = camp(&tw).held();
    assert_eq!(held_docs.len(), 1);
    assert_eq!(held_docs[0].key, format!("op:{}", doomed.0));
    // The world booted, and the next declare does NOT re-issue the held id.
    let next = declare(&mut tw, attacker, 0);
    assert_ne!(next, doomed);
    assert!(next.0 > healthy.0);
}

/// **P4. A stored consumer id this build does not know fails CLOSED**: it is kept,
/// blocks `Resolved`, and `@op/consumers` says so with the command filled in.
#[test]
fn e7c9_p4_an_unknown_stored_consumer_pages_rather_than_completing() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let id = declare(&mut tw, attacker, 100);
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    ok(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
    let mut kv = Kv::new();
    fold_into(&mut tw, &mut kv);
    let entry = kv
        .get_mut(&SmolStr::new_static("campaign"))
        .expect("campaign wrote");
    for (k, v) in entry.iter_mut() {
        if k.as_str() == format!("op:{}", id.0) {
            let required = v["runtime"]["required"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let mut with_sixth = required;
            with_sixth.push(serde_json::json!("orbital_bombardment"));
            v["runtime"]["required"] = serde_json::Value::Array(with_sixth.clone());
            v["runtime"]["state"] =
                serde_json::json!({ "state": "resolving", "pending": with_sixth });
        }
    }
    reboot(&mut tw, &kv);
    fire_n(&mut tw, 19);
    assert!(
        !matches!(state_of(&tw, id), OperationState::Resolved),
        "an unknown consumer was dropped and the operation completed"
    );
    let shown = ok(&mut tw, GOD, &format!("@op/consumers {}", id.0));
    assert!(shown.contains("orbital_bombardment"), "{shown}");
    assert!(shown.contains("does not know"), "{shown}");
    assert!(shown.contains("paused: unknown-consumer"), "{shown}");
    // …and it survives another restart rather than being written away.
    let mut kv2 = Kv::new();
    fold_into(&mut tw, &mut kv2);
    reboot(&mut tw, &kv2);
    let shown = ok(&mut tw, GOD, &format!("@op/consumers {}", id.0));
    assert!(shown.contains("orbital_bombardment"), "{shown}");
}

/// **P5. A `Resolved` operation never re-opens, and a later build's sixth consumer
/// leaves it untouched.**
#[test]
fn e7c9_p5_a_resolved_operation_never_reopens() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let id = declare(&mut tw, attacker, 100);
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    ok(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
    fire_n(&mut tw, 9);
    assert_eq!(state_of(&tw, id), OperationState::Resolved);
    // Every route back in refuses.
    err(&mut tw, attacker, &format!("@op/resolve {}=forfeit", id.0));
    err(&mut tw, attacker, &format!("@op/activate {}", id.0));
    err(
        &mut tw,
        attacker,
        &format!("@op/join {}=#{}", id.0, attacker.0),
    );
    err(
        &mut tw,
        attacker,
        &format!("@op/terms {}/objective=new", id.0),
    );
    assert_eq!(state_of(&tw, id), OperationState::Resolved);
    // A stored set that *grew* after the fact does not re-open it either: the
    // state is terminal, and the fan-out never looks at a `Resolved` operation.
    let mut kv = Kv::new();
    fold_into(&mut tw, &mut kv);
    let entry = kv
        .get_mut(&SmolStr::new_static("campaign"))
        .expect("campaign wrote");
    for (k, v) in entry.iter_mut() {
        if k.as_str() == format!("op:{}", id.0) {
            v["runtime"]["required"] = serde_json::json!([
                "campaign_outcome",
                "bond_settlement",
                "contract_outcomes",
                "front_control",
                "force_retirement",
                "orbital_bombardment"
            ]);
        }
    }
    reboot(&mut tw, &kv);
    fire_n(&mut tw, 10);
    assert_eq!(state_of(&tw, id), OperationState::Resolved);
}

/// **P6. The one-operation index and the retirement release units observably.**
#[test]
fn e7c9_p6_force_retirement_releases_every_unit_for_another_operation() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let unit = unit_of(&mut tw, attacker, "Unit");
    let first = declare(&mut tw, attacker, 10);
    ok(
        &mut tw,
        attacker,
        &format!("@op/join {}=#{}", first.0, unit.0),
    );
    assert_eq!(
        camp(&tw).operation_holding(UnitRef(unit)),
        Some(first),
        "the index did not see the join"
    );
    ok(
        &mut tw,
        attacker,
        &format!("@op/resolve {}=success", first.0),
    );
    fire_n(&mut tw, 9);
    assert_eq!(state_of(&tw, first), OperationState::Resolved);
    // The OBSERVABLE outcome: the unit can now join a different operation, which
    // it could not do a moment ago.
    let second = declare(&mut tw, attacker, 10);
    ok(
        &mut tw,
        attacker,
        &format!("@op/join {}=#{}", second.0, unit.0),
    );
    let shown = ok(&mut tw, attacker, &format!("@op/show {}", second.0));
    assert!(shown.contains("Participants: 1"), "{shown}");
}

/// **A8 (doc 13, W1/W2). The bond settles exactly once even when its idempotency
/// key has been EVICTED from the window.**
///
/// A key older than the window is **not** a HIT — `idempotency_resolved` answers
/// through the gate and the post decides — so a settlement that leaned on the key
/// alone would release a second time after a month. Cut 9 does not lean on it: the
/// durable evidence is the `Hold`'s own status, read from the economy's journal
/// projection before anything is posted, so an evicted key changes nothing.
#[test]
fn e7c9_a8_the_bond_settles_once_even_with_the_idempotency_key_evicted() {
    // Window zero: nothing survives the sweep, and nothing is seeded at boot.
    let mut tw = boot_windowed(0);
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let acct = account_of(&tw, attacker);
    let id = declare(&mut tw, attacker, 250);
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    ok(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
    fire_n(&mut tw, 8);
    assert_eq!(state_of(&tw, id), OperationState::Resolved);
    let settled = (bal(&tw, acct), held(&tw, acct), ledger_total(&tw));

    // Age the world well past the window and sweep, which is what evicts the map.
    let mut kv = Kv::new();
    fold_into(&mut tw, &mut kv);
    jump_to(&mut tw, 60 * 24 * HOUR);
    fire_n(&mut tw, 4);
    // Boot from the folded documents: the campaign document is intact, the
    // idempotency map is not.
    reboot(&mut tw, &kv);
    assert!(
        snap(&tw)
            .idempotency(&mush_campaign::bond::consumer_key(
                id,
                ConsumerId::BondSettlement
            ))
            .is_none(),
        "the fixture must actually evict the key it is testing"
    );
    let posts_before = tw
        .world()
        .module_state::<EconomyStore>()
        .expect("economy")
        .transactions()
        .count();
    fire_n(&mut tw, 12);
    assert_eq!(
        (bal(&tw, acct), held(&tw, acct), ledger_total(&tw)),
        settled,
        "an evicted idempotency key released the bond a second time"
    );
    // The settlement did not merely fail to *move* money — it POSTED NOTHING. The
    // `live_hold` read is the durable evidence, and a hold that is no longer held
    // ends the consumer before a plan is built, so the evicted key is never asked.
    assert_eq!(
        tw.world()
            .module_state::<EconomyStore>()
            .expect("economy")
            .transactions()
            .count(),
        posts_before,
        "an evicted key produced a second settlement post"
    );
    assert_eq!(
        snap(&tw).holds().filter(|h| h.status.is_held()).count(),
        0,
        "no hold is live after the drain"
    );
}

/// **Z4. Staff can abandon an operation whose surface no longer hashes, and doing
/// so RELEASES the bond rather than stranding it.**
///
/// The one path past the intactness gate, and the reason it exists: without it a
/// corrupted document would reserve a player's money against a record nothing could
/// ever settle (08c §5.2's M2/M3 shape).
#[test]
fn e7c9_z4_staff_can_abandon_a_broken_operation_and_the_bond_comes_back() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let acct = account_of(&tw, attacker);
    let id = declare(&mut tw, attacker, 400);
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    let mut kv = Kv::new();
    fold_into(&mut tw, &mut kv);
    let entry = kv
        .get_mut(&SmolStr::new_static("campaign"))
        .expect("campaign wrote");
    for (k, v) in entry.iter_mut() {
        if k.as_str() == format!("op:{}", id.0) {
            v["terms"]["objective"]["summary"] = serde_json::json!("something else entirely");
        }
    }
    reboot(&mut tw, &kv);
    assert!(!camp(&tw).operation(id).expect("op").consent_intact());
    assert_eq!(held(&tw, acct), 400, "the money really is reserved");

    // The owner cannot force it — an abort from a non-staff actor still goes
    // through the intactness gate and refuses.
    err(&mut tw, attacker, &format!("@op/resolve {}=abort", id.0));
    assert_eq!(held(&tw, acct), 400);
    // Staff can, and the bond comes back.
    ok(&mut tw, GOD, &format!("@op/resolve {}=abort", id.0));
    fire_n(&mut tw, 10);
    // D7: an abandonment now really reaches `Aborted`, and `@op/show` says so.
    assert_eq!(state_of(&tw, id), OperationState::Aborted);
    assert!(
        ok(&mut tw, GOD, &format!("@op/show {}", id.0)).contains("aborted"),
        "an abandoned operation still reads as resolved"
    );
    assert_eq!(held(&tw, acct), 0, "the bond was stranded");
    assert_eq!(bal(&tw, acct), 1_000, "an abort must never capture");
    // The OBSERVABLE outcome: the money is spendable again.
    let other = tw.spawn_player("Other");
    ok(&mut tw, GOD, &format!("@account/open #{}=gold", other.0));
    let other_acct = account_of(&tw, other);
    ok(
        &mut tw,
        GOD,
        &format!("@econ/pay {acct}={other_acct} 1000 released"),
    );
}

/// **P7. `with_facts` takes exactly one `campaign.*` provider, and a duplicate
/// namespace PANICS** (D-E7-30's "one provider per crate, no exceptions").
#[test]
#[should_panic(expected = "campaign")]
fn e7c9_p7_a_duplicate_campaign_fact_namespace_panics_at_wiring() {
    let _ = mush_economy::EconomyModule::with_custody(vec![]).with_facts(vec![
        Arc::new(mush_campaign::CampaignFactProvider),
        Arc::new(mush_campaign::CampaignFactProvider),
    ]);
}

/// …and the composed world carries exactly one namespace, answering exactly one
/// kind: every other kind — cut 10's seats, cut 8's tactics facts included — is
/// unknown to cut 9, which is unsatisfied, which never settles money.
#[test]
fn e7c9_p8_exactly_one_campaign_namespace_answers_exactly_one_kind() {
    use mush_economy::FactProvider;
    let tw = boot();
    assert!(tw.world().module_state::<CampaignStore>().is_some());
    let provider = mush_campaign::CampaignFactProvider;
    assert_eq!(provider.provider(), mush_campaign::NS);
    assert_eq!(mush_campaign::NS, "campaign");
    assert_eq!(
        mush_campaign::KIND_OPERATION_RESOLVED,
        "campaign.operation.resolved"
    );
    for kind in [
        "campaign.operation.declared",
        "campaign.seat.granted",
        "sim.station.served",
        "tactics.sortie.started",
    ] {
        assert!(
            provider.resolve(tw.world(), kind, "1", None).is_none(),
            "{kind} must be unknown to cut 9"
        );
    }
}

/// **R6. A `force.*` grant stored before the vocabulary could catch it still
/// authorizes NOTHING, and `@org/show` FLAGS it rather than deleting it.**
///
/// The vocabulary is a **typo catcher, not an authorization gate**: `authorize`
/// matches capability ids by equality, so a misspelled grant confers nothing
/// whatever the vocabulary says. The first world wires an **empty** vocabulary —
/// E1's accept-everything behaviour, which is what a server running before cut 9
/// listed the ids had — and stores `force.comand`. The second wires the real
/// vocabulary, which knows `force.command` and not the typo.
#[test]
fn e7c9_r6_a_grant_made_before_the_id_was_listed_authorizes_nothing_and_is_flagged() {
    let mut lax = boot_with_vocabulary(BTreeSet::new());
    let officer = lax.spawn_player("Officer");
    ok(&mut lax, GOD, "@org/create Ironhold");
    let scope = lax
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("org")
        .state()
        .orgs()
        .next()
        .expect("one org")
        .root_scope;
    // Accepted, because an EMPTY vocabulary accepts everything — the exact record a
    // server running before cut 9 listed the ids could have stored.
    ok(
        &mut lax,
        GOD,
        &format!(
            "@org/grant #{}=Ironhold force.comand scope={}",
            officer.0, scope.0
        ),
    );
    ok(&mut lax, GOD, &format!("@force/create lance={}", scope.0));
    let mut kv = Kv::new();
    fold_into(&mut lax, &mut kv);

    // A build that does not know the id: the stored grant survives, is FLAGGED,
    // and authorizes nothing.
    let mut strict = boot_with_vocabulary(vocabulary());
    let officer2 = strict.spawn_player("Officer");
    assert_eq!(officer, officer2, "the two rigs must allocate the same ids");
    reboot(&mut strict, &kv);
    let shown = ok(&mut strict, GOD, "@org/show Ironhold");
    assert!(
        shown.contains("force.comand"),
        "the grant was deleted: {shown}"
    );
    assert!(
        shown.contains("[UNKNOWN CAPABILITY]"),
        "it was not flagged: {shown}"
    );
    // …and the correctly spelled id IS grantable here, so the vocabulary really is
    // the one that knows it and the typo really is the thing it caught.
    ok(
        &mut strict,
        GOD,
        &format!(
            "@org/grant #{}=Ironhold force.command scope={}",
            GOD.0, scope.0
        ),
    );
    // …and it authorizes nothing: the formation exists, and the holder still
    // cannot deploy it.
    let id = declare_bondless(&mut strict, officer);
    let node = camp(&strict)
        .formations()
        .next()
        .expect("the node loaded")
        .id;
    let refusal = err(&mut strict, officer, &format!("@op/join {}={node}", id.0));
    assert!(refusal.contains("Permission denied"), "{refusal}");
}

/// A bondless operation declared by `who` — the money rig is not composed in every
/// world this file boots.
fn declare_bondless(tw: &mut TestWorld, who: ObjId) -> OperationId {
    let before: BTreeSet<u64> = camp(tw).operations().map(|d| d.id().0).collect();
    ok(tw, who, "@op/declare hold the refinery");
    camp(tw)
        .operations()
        .map(|d| d.id())
        .find(|id| !before.contains(&id.0))
        .expect("one new operation")
}

// ===========================================================================
// ---- E7 cut 9 independent verify fold ----
//
// 26 findings (7 HIGH, 11 MEDIUM, 8 LOW) folded in, with the six owner decisions
// E1(a)–E6(a) taken. Every test here asserts the OBSERVABLE outcome through the real
// command path — the balance another account can spend, the bytes a stranger
// receives, the unit a different operation can take, whether a hold is still `Held`
// after a reboot and a full drain — and names every arm its guard covers.
// ===========================================================================

/// A world with **no canon module at all**, for the composition D6 turns on.
fn boot_without_canon() -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(
            mush_org::OrgModule::new()
                .with_capabilities(vocabulary())
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(mush_faction::FactionModule::new()),
        Box::new(mush_economy::EconomyModule::with_custody(vec![])),
        Box::new(mush_campaign::CampaignModule::new().with_formation_kinds(Arc::new(TestKinds))),
    ])
}

/// A test-only module carrying the seam e4c uses for the same job: the
/// whole-economy latch has a staff CLEAR (`@econ/unfreeze`) and no staff set, so
/// this drives it the way boot's recovery pass does.
#[derive(Default)]
struct Latch;

impl mush_core::module::GameModule for Latch {
    fn name(&self) -> &'static str {
        "campaign_test_latch"
    }
    fn register(
        &mut self,
        reg: &mut mush_core::module::Registrar<'_>,
    ) -> Result<(), mush_core::module::ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.command(
            mush_core::CommandSpec {
                name: "@econfreeze".into(),
                shape: mush_core::ArgShape::Raw,
                help: "test: set the whole-economy latch".into(),
                ..mush_core::CommandSpec::default()
            },
            Arc::new(
                |ctx: &mut mush_core::Ctx<'_>| -> Result<(), mush_core::CmdError> {
                    mush_economy::set_economy_frozen(ctx, true);
                    Ok(())
                },
            ),
        )?;
        Ok(())
    }
}

/// [`boot`] plus the latch seam.
fn boot_freezable() -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(
            mush_org::OrgModule::new()
                .with_capabilities(vocabulary())
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(mush_faction::FactionModule::new()),
        Box::new(mush_canon::CanonModule::new()),
        Box::new(
            mush_economy::EconomyModule::with_custody(vec![]).with_canon_outcome_kinds(vec![
                SmolStr::new_static(mush_campaign::KIND_OPERATION_RESOLVED),
            ]),
        ),
        Box::new(mush_campaign::CampaignModule::new().with_formation_kinds(Arc::new(TestKinds))),
        Box::new(Latch),
    ])
}

/// Rewrite one folded `op:<id>` document in place.
fn edit_op(kv: &mut Kv, id: OperationId, edit: impl Fn(&mut serde_json::Value)) {
    let entry = kv
        .get_mut(&SmolStr::new_static("campaign"))
        .expect("campaign wrote");
    let key = format!("op:{}", id.0);
    for (k, v) in entry.iter_mut() {
        if k.as_str() == key {
            edit(v);
        }
    }
}

// ---- C1 (HIGH; merges D4 and B2): the disclosure lens is not the write gate ----

/// **C1a. A named side reads the plan and may move nothing.** All five mutating
/// verbs, one lens, and the issuer's money never leaves their own hands.
#[test]
fn vf_c1a_a_named_side_may_read_the_plan_and_may_not_amend_activate_join_withdraw_or_resolve_it() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    let defender = tw.spawn_player("Defender");
    money(&mut tw, &[(attacker, 1_000), (defender, 0)]);
    let mine = account_of(&tw, attacker);
    let theirs = account_of(&tw, defender);
    let unit = unit_of(&mut tw, attacker, "Tank");
    let id = declare(&mut tw, attacker, 400);
    ok(
        &mut tw,
        attacker,
        &format!("@op/terms {}/side=defend:#{}", id.0, defender.0),
    );
    ok(&mut tw, attacker, &format!("@op/join {}=#{}", id.0, unit.0));
    assert!(
        !mush_core::perm::has_power(tw.world(), defender, mush_core::Powers::SIM_ADMIN),
        "the fixture must not be staff"
    );

    // The lens is unchanged: a named side is entitled to READ the plan.
    let shown = ok(&mut tw, defender, &format!("@op/show/full {}", id.0));
    assert!(
        shown.contains("Objective:"),
        "the side lost the plan: {shown}"
    );

    // …and entitled to move nothing at all. Every arm, named.
    for line in [
        format!("@op/terms {}/bond.forfeit=#{}:{theirs}", id.0, defender.0),
        format!("@op/terms {}/objective=whatever I like", id.0),
        format!("@op/terms {}/visibility=public", id.0),
        format!("@op/activate {}", id.0),
        format!("@op/join {}=#{}", id.0, attacker.0),
        format!("@op/withdraw {}=#{}", id.0, unit.0),
        format!("@op/resolve {}=success", id.0),
        format!("@op/resolve {}=forfeit", id.0),
        format!("@op/resolve {}=abort", id.0),
        format!("@op/resolve {}=retry", id.0),
    ] {
        let refusal = err(&mut tw, defender, &line);
        assert!(refusal.contains("Permission denied"), "{line}: {refusal}");
    }
    // The observables: nothing was reserved, nothing was amended, the roster stands.
    assert_eq!(
        held(&tw, mine),
        0,
        "a third party reserved the issuer's money"
    );
    assert_eq!(bal(&tw, theirs), 0);
    assert_eq!(state_of(&tw, id), OperationState::Declared);
    assert_eq!(
        camp(&tw).operation(id).expect("op").terms.objective.summary,
        "hold the refinery"
    );
    assert_eq!(
        camp(&tw)
            .operation(id)
            .expect("op")
            .runtime
            .participants
            .len(),
        1
    );
}

/// **C1b. Only staff may name a forfeit**, and the beneficiary's own spendable
/// balance is the proof.
#[test]
fn vf_c1b_only_staff_may_name_a_forfeit_and_the_beneficiarys_spendable_balance_proves_it() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    let defender = tw.spawn_player("Defender");
    money(&mut tw, &[(attacker, 1_000), (defender, 0)]);
    let mine = account_of(&tw, attacker);
    let theirs = account_of(&tw, defender);
    let id = declare(&mut tw, attacker, 400);
    ok(
        &mut tw,
        attacker,
        &format!("@op/terms {}/side=defend:#{}", id.0, defender.0),
    );
    ok(
        &mut tw,
        attacker,
        &format!("@op/terms {}/bond.forfeit=#{}:{theirs}", id.0, defender.0),
    );
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));

    // The beneficiary cannot take it, and neither can the issuer.
    for who in [defender, attacker] {
        err(&mut tw, who, &format!("@op/resolve {}=forfeit", id.0));
    }
    fire_n(&mut tw, 4);
    assert_eq!(
        bal(&tw, theirs),
        0,
        "a non-staff forfeit paid the beneficiary"
    );
    assert_eq!(held(&tw, mine), 400, "and the bond is still reserved");

    // Staff can, and only then does the money move.
    ok(&mut tw, GOD, &format!("@op/resolve {}=forfeit", id.0));
    fire_n(&mut tw, 8);
    assert_eq!(bal(&tw, theirs), 400);
    ok(
        &mut tw,
        GOD,
        &format!("@econ/pay {theirs}={mine} 400 spent"),
    );
}

/// **C1c. The issuer resolves an outcome that takes nobody's money; a side does
/// not.**
#[test]
fn vf_c1c_the_attacker_may_resolve_a_non_forfeit_outcome_and_a_side_may_not() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    let defender = tw.spawn_player("Defender");
    money(&mut tw, &[(attacker, 1_000), (defender, 0)]);
    let acct = account_of(&tw, attacker);
    let id = declare(&mut tw, attacker, 100);
    ok(
        &mut tw,
        attacker,
        &format!("@op/terms {}/side=defend:#{}", id.0, defender.0),
    );
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    err(&mut tw, defender, &format!("@op/resolve {}=failure", id.0));
    assert_eq!(state_of(&tw, id), OperationState::Activated);
    ok(&mut tw, attacker, &format!("@op/resolve {}=failure", id.0));
    fire_n(&mut tw, 8);
    assert_eq!(state_of(&tw, id), OperationState::Resolved);
    assert_eq!(held(&tw, acct), 0, "a failure releases the bond");
    assert_eq!(bal(&tw, acct), 1_000);
}

// ---- C2 (HIGH; merges C5 and D3): the bond's own lane ----------------------

/// **C2a. Neither the placer nor the beneficiary reaches a campaign bond through
/// the economy's own verbs** — and it is still `Held` after a reboot and a drain.
#[test]
fn vf_c2a_neither_the_placer_nor_the_beneficiary_may_reach_a_campaign_bond_through_hold_release_or_hold_capture()
 {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    let defender = tw.spawn_player("Defender");
    money(&mut tw, &[(attacker, 1_000), (defender, 0)]);
    let mine = account_of(&tw, attacker);
    let theirs = account_of(&tw, defender);
    let id = declare(&mut tw, attacker, 400);
    ok(
        &mut tw,
        attacker,
        &format!("@op/terms {}/bond.forfeit=#{}:{theirs}", id.0, defender.0),
    );
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    let hold = bond_hold(&tw, id).expect("the bond").id;

    // The beneficiary's capture, the placer's release, and staff's release: all
    // three are refused, and the refusal says where the reservation is settled.
    for (who, line) in [
        (defender, format!("@hold/capture {}={theirs} 400", hold.0)),
        (attacker, format!("@hold/release {}", hold.0)),
        (GOD, format!("@hold/release {}", hold.0)),
        (GOD, format!("@hold/capture {}={theirs} 400", hold.0)),
    ] {
        let refusal = err(&mut tw, who, &line);
        assert!(refusal.contains("campaign.operation"), "{line}: {refusal}");
    }
    // The observables: nobody was paid and the money is still spoken for, across a
    // reboot and a full drain of the tick.
    assert_eq!(bal(&tw, theirs), 0);
    assert_eq!(held(&tw, mine), 400);
    let mut kv = Kv::new();
    fold_into(&mut tw, &mut kv);
    reboot(&mut tw, &kv);
    fire_n(&mut tw, 12);
    assert_eq!(held(&tw, mine), 400, "the bond did not survive the reboot");
    assert!(
        bond_hold(&tw, id).expect("the bond").status.is_held(),
        "the reservation is no longer held"
    );
}

/// **C2b. A bond settled by something other than this operation is a DISPUTE, not
/// a replay hit**: the settlement pauses and pages instead of acknowledging.
#[test]
fn vf_c2b_a_bond_settled_outside_this_operation_pauses_and_pages_instead_of_acknowledging_bond_settlement()
 {
    // The window at zero, so the settlement's own idempotency key really is gone.
    let mut tw = boot_windowed(0);
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let acct = account_of(&tw, attacker);
    let id = declare(&mut tw, attacker, 250);
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    ok(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
    fire_n(&mut tw, 8);
    assert_eq!(state_of(&tw, id), OperationState::Resolved);
    assert_eq!(held(&tw, acct), 0, "the bond really was released");

    // Age the world past the window, then boot a document that still owes the bond
    // consumer for a reservation this settlement's key can no longer claim.
    let mut kv = Kv::new();
    fold_into(&mut tw, &mut kv);
    jump_to(&mut tw, 60 * 24 * HOUR);
    fire_n(&mut tw, 4);
    edit_op(&mut kv, id, |v| {
        v["runtime"]["state"] =
            serde_json::json!({ "state": "resolving", "pending": ["bond_settlement"] });
    });
    reboot(&mut tw, &kv);
    assert!(
        snap(&tw)
            .idempotency(&mush_campaign::bond::consumer_key(
                id,
                ConsumerId::BondSettlement
            ))
            .is_none(),
        "the fixture must actually evict the key it is testing"
    );
    fire_n(&mut tw, 12);
    // It did NOT reach `Resolved`, and the page names the dispute.
    assert!(
        !matches!(state_of(&tw, id), OperationState::Resolved),
        "a hold nothing here settled was acknowledged as this operation's settlement"
    );
    let shown = ok(&mut tw, GOD, &format!("@op/show/full {}", id.0));
    assert!(shown.contains("paused: unfunded"), "{shown}");
    // …and the exit E3(a) added really does clear it.
    ok(&mut tw, GOD, &format!("@op/resolve {}=abort", id.0));
    fire_n(&mut tw, 10);
    assert_eq!(state_of(&tw, id), OperationState::Aborted);
    assert_eq!(bal(&tw, acct), 1_000);
}

/// **C2c. A hold this settlement CONSUMED is still the replay hit**, across a
/// reboot, and it pays nobody a second time.
#[test]
fn vf_c2c_a_hold_this_settlement_consumed_is_still_the_replay_hit_across_a_reboot() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    let defender = tw.spawn_player("Defender");
    money(&mut tw, &[(attacker, 1_000), (defender, 0)]);
    let theirs = account_of(&tw, defender);
    let id = declare(&mut tw, attacker, 300);
    ok(
        &mut tw,
        attacker,
        &format!("@op/terms {}/bond.forfeit=#{}:{theirs}", id.0, defender.0),
    );
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    ok(&mut tw, GOD, &format!("@op/resolve {}=forfeit", id.0));
    fire_n(&mut tw, 8);
    assert_eq!(bal(&tw, theirs), 300);
    assert!(matches!(
        bond_hold(&tw, id).expect("the bond").status,
        HoldStatus::Consumed { .. }
    ));

    let mut kv = Kv::new();
    fold_into(&mut tw, &mut kv);
    edit_op(&mut kv, id, |v| {
        v["runtime"]["state"] =
            serde_json::json!({ "state": "resolving", "pending": ["bond_settlement"] });
    });
    reboot(&mut tw, &kv);
    let total = ledger_total(&tw);
    fire_n(&mut tw, 10);
    assert_eq!(
        state_of(&tw, id),
        OperationState::Resolved,
        "the replay stalled"
    );
    assert_eq!(bal(&tw, theirs), 300, "the forfeit paid twice");
    assert_eq!(ledger_total(&tw), total);
}

// ---- C3 (HIGH; merges D5 and B1): the write path is the gate ---------------

/// **C3a. Ordinary play past the withdrawal cap still LOADS**, and the bond is
/// still reachable by every command afterwards. The encoding form: built through the
/// real commands, folded, rebooted, and read back.
#[test]
fn vf_c3a_a_join_withdraw_cycle_past_the_cap_still_loads_and_its_bond_is_still_reachable() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let acct = account_of(&tw, attacker);
    let id = declare(&mut tw, attacker, 500);
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    // One more distinct unit than the cap can hold, each joined and withdrawn
    // through the real commands.
    for n in 0..=mush_campaign::MAX_OPERATION_PARTICIPANTS {
        let unit = unit_of(&mut tw, attacker, &format!("U{n}"));
        ok(&mut tw, attacker, &format!("@op/join {}=#{}", id.0, unit.0));
        ok(
            &mut tw,
            attacker,
            &format!("@op/withdraw {}=#{}", id.0, unit.0),
        );
    }
    assert_eq!(held(&tw, acct), 500);
    let mut kv = Kv::new();
    fold_into(&mut tw, &mut kv);
    reboot(&mut tw, &kv);

    // It loaded. It was not held. Its bond is reachable.
    assert!(
        camp(&tw).operation(id).is_some(),
        "ordinary play wrote a document the boot refused"
    );
    assert!(camp(&tw).held().is_empty(), "{:?}", camp(&tw).held());
    assert_eq!(
        camp(&tw)
            .operation(id)
            .expect("op")
            .runtime
            .withdrawals
            .len(),
        mush_campaign::MAX_OPERATION_PARTICIPANTS,
        "the audit tail is bounded by construction"
    );
    ok(&mut tw, GOD, &format!("@op/show {}", id.0));
    ok(&mut tw, GOD, &format!("@op/resolve {}=abort", id.0));
    fire_n(&mut tw, 10);
    assert_eq!(
        held(&tw, acct),
        0,
        "the bond was stranded by the write path"
    );
    assert_eq!(bal(&tw, acct), 1_000);
}

/// **C3b. The stored document always round-trips**: what the write path produced is
/// exactly what the strict wire accepts back, at the cap.
#[test]
fn vf_c3b_put_operation_never_writes_a_runtime_the_boot_would_refuse() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let id = declare(&mut tw, attacker, 10);
    for n in 0..mush_campaign::MAX_OPERATION_PARTICIPANTS + 20 {
        let unit = unit_of(&mut tw, attacker, &format!("U{n}"));
        ok(&mut tw, attacker, &format!("@op/join {}=#{}", id.0, unit.0));
        ok(
            &mut tw,
            attacker,
            &format!("@op/withdraw {}=#{}", id.0, unit.0),
        );
        let doc = camp(&tw).operation(id).expect("op").clone();
        let encoded = serde_json::to_value(&doc).expect("serializes");
        serde_json::from_value::<mush_campaign::OperationDoc>(encoded)
            .unwrap_or_else(|e| panic!("cycle {n} stored a document the wire refuses: {e}"));
    }
}

/// **C3c. A full roster's retirement writes a document the wire accepts**, on top of
/// withdrawals that are already there.
#[test]
fn vf_c3c_force_retirement_of_a_full_roster_writes_a_document_the_wire_accepts() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let id = declare(&mut tw, attacker, 20);
    // Twenty already withdrawn…
    for n in 0..20 {
        let unit = unit_of(&mut tw, attacker, &format!("Gone{n}"));
        ok(&mut tw, attacker, &format!("@op/join {}=#{}", id.0, unit.0));
        ok(
            &mut tw,
            attacker,
            &format!("@op/withdraw {}=#{}", id.0, unit.0),
        );
    }
    // …and a FULL roster on top of them, which the retirement drains into the same
    // list.
    for n in 0..mush_campaign::MAX_OPERATION_PARTICIPANTS {
        let unit = unit_of(&mut tw, attacker, &format!("On{n}"));
        ok(&mut tw, attacker, &format!("@op/join {}=#{}", id.0, unit.0));
    }
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    ok(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
    fire_n(&mut tw, 10);
    assert_eq!(state_of(&tw, id), OperationState::Resolved);
    let mut kv = Kv::new();
    fold_into(&mut tw, &mut kv);
    reboot(&mut tw, &kv);
    assert!(
        camp(&tw).operation(id).is_some(),
        "the retirement wrote a document the boot refused"
    );
    assert!(camp(&tw).held().is_empty());
}

// ---- C4 (HIGH; merges C12, D2 and deviation (h)): the exit from Resolving --

/// **C4a. An unknown stored consumer strands the fan-out, and `abort` from
/// `Resolving` releases the bond** — the state the old `abandon` refused.
#[test]
fn vf_c4a_an_unknown_consumer_stranding_a_bond_in_resolving_can_be_abandoned_and_the_hold_is_released()
 {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let acct = account_of(&tw, attacker);
    let id = declare(&mut tw, attacker, 400);
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    ok(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
    let mut kv = Kv::new();
    fold_into(&mut tw, &mut kv);
    edit_op(&mut kv, id, |v| {
        v["runtime"]["required"] = serde_json::json!([
            "campaign_outcome",
            "bond_settlement",
            "force_retirement",
            "orbital_bombardment"
        ]);
        v["runtime"]["state"] = serde_json::json!({
            "state": "resolving",
            "pending": ["bond_settlement", "force_retirement", "orbital_bombardment"]
        });
    });
    reboot(&mut tw, &kv);
    fire_n(&mut tw, 6);
    assert_eq!(held(&tw, acct), 400, "the fixture must actually strand it");
    let shown = ok(&mut tw, GOD, &format!("@op/consumers {}", id.0));
    assert!(shown.contains("does not know consumer"), "{shown}");

    // The command the runbook prints. Before the fold it answered "That operation
    // cannot resolve right now" from exactly this state.
    ok(&mut tw, GOD, &format!("@op/resolve {}=abort", id.0));
    fire_n(&mut tw, 12);
    assert_eq!(held(&tw, acct), 0, "the bond stayed stranded");
    assert_eq!(bal(&tw, acct), 1_000, "an abandonment must never capture");
    // …and the id nobody can discharge is still kept, so the operation does not
    // claim to have finished: the money moved, the record did not lie.
    assert!(
        matches!(state_of(&tw, id), OperationState::Resolving { .. }),
        "an unknown consumer stopped blocking the terminal state"
    );
    assert!(
        !camp(&tw)
            .operation(id)
            .expect("op")
            .runtime
            .unknown_consumers
            .is_empty()
    );
}

/// **C4b. A terms break found MID-DRAIN can be abandoned**, and the fan-out
/// completes past the broken surface.
#[test]
fn vf_c4b_a_terms_break_found_mid_drain_can_be_abandoned_and_the_fan_out_completes() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let acct = account_of(&tw, attacker);
    let id = declare(&mut tw, attacker, 400);
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    ok(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
    let mut kv = Kv::new();
    fold_into(&mut tw, &mut kv);
    edit_op(&mut kv, id, |v| {
        v["terms"]["objective"]["summary"] = serde_json::json!("something else entirely");
        v["runtime"]["state"] = serde_json::json!({
            "state": "resolving",
            "pending": ["bond_settlement", "force_retirement"]
        });
    });
    reboot(&mut tw, &kv);
    fire_n(&mut tw, 4);
    let shown = ok(&mut tw, GOD, &format!("@op/show/full {}", id.0));
    assert!(shown.contains("paused: terms-broken"), "{shown}");
    assert_eq!(held(&tw, acct), 400);

    let (went, text) = run(&mut tw, GOD, &format!("@op/resolve {}=abort", id.0));
    assert!(went, "the escape hatch is still unreachable: {text}");
    fire_n(&mut tw, 12);
    assert_eq!(state_of(&tw, id), OperationState::Aborted);
    assert_eq!(held(&tw, acct), 0);
    assert_eq!(bal(&tw, acct), 1_000);
}

/// **C4c. The level-2 runbook names a command that runs.** The bytes the page
/// prints are asserted against the verb that accepts them.
#[test]
fn vf_c4c_the_unknown_consumer_page_names_a_command_that_runs() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let id = declare(&mut tw, attacker, 100);
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    ok(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
    let mut kv = Kv::new();
    fold_into(&mut tw, &mut kv);
    edit_op(&mut kv, id, |v| {
        v["runtime"]["required"] = serde_json::json!(["campaign_outcome", "orbital_bombardment"]);
        v["runtime"]["state"] =
            serde_json::json!({ "state": "resolving", "pending": ["orbital_bombardment"] });
    });
    reboot(&mut tw, &kv);
    fire_n(&mut tw, 4);
    // The runbook's own sentence, verbatim from `settle.rs`, and the verb it names.
    let printed = format!("@op/resolve {}=abort", id.0);
    let (went, text) = run(&mut tw, GOD, &printed);
    assert!(went, "the page recommends a command that refuses: {text}");
}

// ---- C6 (MEDIUM; merges B4): the roster asks about the unit ---------------

/// **C6a. Joining an object you do not control is refused**, and it stays free for
/// its own controller to commit.
#[test]
fn vf_c6a_joining_an_object_you_do_not_control_is_refused_and_it_stays_free_for_its_own_controller()
{
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    let bystander = tw.spawn_player("Bystander");
    money(&mut tw, &[(attacker, 1_000), (bystander, 1_000)]);
    let theirs = unit_of(&mut tw, bystander, "Theirs");
    let mine = declare(&mut tw, attacker, 100);
    let yours = declare(&mut tw, bystander, 100);

    let refusal = err(
        &mut tw,
        attacker,
        &format!("@op/join {}=#{}", mine.0, theirs.0),
    );
    assert!(refusal.contains("Permission denied"), "{refusal}");
    // The OBSERVABLE outcome: the object was never locked out of its own
    // controller's operation, which the global one-operation index would have done.
    assert_eq!(camp(&tw).operation_holding(UnitRef(theirs)), None);
    ok(
        &mut tw,
        bystander,
        &format!("@op/join {}=#{}", yours.0, theirs.0),
    );
    assert_eq!(camp(&tw).operation_holding(UnitRef(theirs)), Some(yours));
    // …and staff may still commit anything.
    let other = unit_of(&mut tw, bystander, "Other");
    ok(&mut tw, GOD, &format!("@op/join {}=#{}", mine.0, other.0));
}

/// **C6b. `@force/assign` asks about the member as well as the node.**
#[test]
fn vf_c6b_force_assign_asks_about_the_member_as_well_as_the_node() {
    let mut tw = boot();
    let officer = tw.spawn_player("Officer");
    let bystander = tw.spawn_player("Bystander");
    ok(&mut tw, GOD, "@org/create Ironhold");
    let scope = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("org")
        .state()
        .orgs()
        .next()
        .expect("one org")
        .root_scope;
    ok(&mut tw, GOD, &format!("@force/create lance={}", scope.0));
    let node = camp(&tw).formations().next().expect("node").id;
    for capability in ["force.assign", "force.command"] {
        ok(
            &mut tw,
            GOD,
            &format!(
                "@org/grant #{}=Ironhold {capability} scope={}",
                officer.0, scope.0
            ),
        );
    }
    let theirs = unit_of(&mut tw, bystander, "Theirs");
    let ours = unit_of(&mut tw, officer, "Ours");
    // Full authority over the NODE, and none over the member.
    let refusal = err(
        &mut tw,
        officer,
        &format!("@force/assign {node}=#{}", theirs.0),
    );
    assert!(refusal.contains("Permission denied"), "{refusal}");
    assert!(camp(&tw).formation(node).expect("node").members.is_empty());
    // Their own unit assigns.
    ok(
        &mut tw,
        officer,
        &format!("@force/assign {node}=#{}", ours.0),
    );
    assert_eq!(camp(&tw).formation(node).expect("node").members.len(), 1);
}

/// **C6c. A bystander whose object staff committed gains no command over the
/// operation** — the disclosure lens still admits them, and the write gate does not.
#[test]
fn vf_c6c_a_bystanders_object_committed_by_staff_gives_the_bystander_no_command_over_the_operation()
{
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    let bystander = tw.spawn_player("Bystander");
    money(&mut tw, &[(attacker, 1_000), (bystander, 0)]);
    let mine = account_of(&tw, attacker);
    let theirs = account_of(&tw, bystander);
    let unit = unit_of(&mut tw, bystander, "Theirs");
    let id = declare(&mut tw, attacker, 400);
    ok(&mut tw, GOD, &format!("@op/join {}=#{}", id.0, unit.0));
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));

    // The lens: they are inside the operation and see the plan.
    let shown = ok(&mut tw, bystander, &format!("@op/show/full {}", id.0));
    assert!(shown.contains("Objective:"), "{shown}");
    // The gate: they command nothing.
    for line in [
        format!("@op/resolve {}=forfeit", id.0),
        format!("@op/resolve {}=success", id.0),
        format!("@op/withdraw {}=#{}", id.0, unit.0),
    ] {
        // …except taking their OWN unit back out, which is the one roster move a
        // unit's controller keeps.
        let (went, text) = run(&mut tw, bystander, &line);
        if line.contains("withdraw") {
            assert!(went, "a unit's controller may withdraw it: {text}");
        } else {
            assert!(!went, "{line} was allowed: {text}");
        }
    }
    fire_n(&mut tw, 6);
    assert_eq!(bal(&tw, theirs), 0, "a bystander took the issuer's bond");
    assert_eq!(held(&tw, mine), 400);
}

// ---- C7 (MEDIUM; merges B10): the abandonment flag is the transition's --------

/// **C7a. A non-staff `abort` is refused**, and the operation stays exactly where it
/// was — so the mid-drain intactness re-check cannot be pre-disabled by choosing a
/// word.
#[test]
fn vf_c7a_a_non_staff_abort_is_refused_and_the_operation_stays_where_it_was() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let id = declare(&mut tw, attacker, 100);
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    let refusal = err(&mut tw, attacker, &format!("@op/resolve {}=abort", id.0));
    assert!(refusal.contains("Permission denied"), "{refusal}");
    assert_eq!(state_of(&tw, id), OperationState::Activated);
    assert!(camp(&tw).operation(id).expect("op").runtime.root.is_none());
    assert!(!camp(&tw).operation(id).expect("op").runtime.abandoned);
}

/// **C7b. A surface broken AFTER a staff abandonment still drains and releases the
/// bond** — the abandonment is the one resolution allowed past a broken surface.
#[test]
fn vf_c7b_a_surface_broken_after_a_staff_abandonment_still_drains_and_releases_the_bond() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let acct = account_of(&tw, attacker);
    let id = declare(&mut tw, attacker, 300);
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    ok(&mut tw, GOD, &format!("@op/resolve {}=abort", id.0));
    let mut kv = Kv::new();
    fold_into(&mut tw, &mut kv);
    edit_op(&mut kv, id, |v| {
        v["terms"]["objective"]["summary"] = serde_json::json!("tampered with mid-drain");
    });
    reboot(&mut tw, &kv);
    assert!(!camp(&tw).operation(id).expect("op").consent_intact());
    fire_n(&mut tw, 12);
    assert_eq!(state_of(&tw, id), OperationState::Aborted);
    assert_eq!(held(&tw, acct), 0);
    let shown = ok(&mut tw, GOD, &format!("@op/show/full {}", id.0));
    assert!(!shown.contains("terms-broken"), "{shown}");
}

/// **C7c. A surface broken after any OTHER outcome pauses `terms-broken`
/// mid-drain** — the same fixture, one word different.
#[test]
fn vf_c7c_a_surface_broken_after_any_other_outcome_pauses_terms_broken_mid_drain() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let acct = account_of(&tw, attacker);
    let id = declare(&mut tw, attacker, 300);
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    ok(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
    let mut kv = Kv::new();
    fold_into(&mut tw, &mut kv);
    edit_op(&mut kv, id, |v| {
        v["terms"]["objective"]["summary"] = serde_json::json!("tampered with mid-drain");
        v["runtime"]["state"] = serde_json::json!({
            "state": "resolving",
            "pending": ["bond_settlement", "force_retirement"]
        });
    });
    reboot(&mut tw, &kv);
    fire_n(&mut tw, 8);
    let shown = ok(&mut tw, GOD, &format!("@op/show/full {}", id.0));
    assert!(shown.contains("paused: terms-broken"), "{shown}");
    assert_eq!(held(&tw, acct), 300, "a tampered surface still paid out");
}

// ---- C8 (MEDIUM): the bond's accounts are resolved at the edit --------------

/// **C8a. A forfeit account that does not exist is refused at the terms edit**, not
/// at a capture inside a state that used to have no exit.
#[test]
fn vf_c8a_a_forfeit_account_that_does_not_exist_is_refused_at_the_terms_edit() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    let defender = tw.spawn_player("Defender");
    money(&mut tw, &[(attacker, 1_000), (defender, 0)]);
    let acct = account_of(&tw, attacker);
    let id = declare(&mut tw, attacker, 400);
    let refusal = err(
        &mut tw,
        attacker,
        &format!("@op/terms {}/bond.forfeit=#{}:99999", id.0, defender.0),
    );
    assert!(refusal.contains("does not exist"), "{refusal}");
    // The observable: the terms never took it, so nothing can ever strand on it.
    assert!(
        camp(&tw)
            .operation(id)
            .expect("op")
            .terms
            .bond
            .as_ref()
            .expect("bond")
            .forfeit_to
            .is_none()
    );
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    assert_eq!(held(&tw, acct), 400);
}

/// **C8b. A forfeit account on a different instrument is refused at the edit.**
#[test]
fn vf_c8b_a_forfeit_account_on_a_different_instrument_is_refused_at_the_terms_edit() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    let defender = tw.spawn_player("Defender");
    money(&mut tw, &[(attacker, 1_000), (defender, 0)]);
    ok(&mut tw, GOD, "@instrument/register silver=0");
    ok(
        &mut tw,
        GOD,
        &format!("@account/open #{}=silver", defender.0),
    );
    let silver = snap(&tw)
        .accounts()
        .filter(|a| a.owner == PrincipalId::Object(defender))
        .map(|a| a.id)
        .max()
        .expect("the silver account");
    let id = declare(&mut tw, attacker, 400);
    let refusal = err(
        &mut tw,
        attacker,
        &format!("@op/terms {}/bond.forfeit=#{}:{silver}", id.0, defender.0),
    );
    assert!(refusal.contains("different instrument"), "{refusal}");
}

/// **C8c. A bond on an account the issuer does not own is refused at the edit**,
/// rather than at activation.
#[test]
fn vf_c8c_a_bond_account_the_issuer_does_not_own_is_refused_at_the_terms_edit_not_at_activation() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    let other = tw.spawn_player("Other");
    money(&mut tw, &[(attacker, 1_000), (other, 1_000)]);
    let theirs = account_of(&tw, other);
    let before: BTreeSet<u64> = camp(&tw).operations().map(|d| d.id().0).collect();
    ok(&mut tw, attacker, "@op/declare hold the refinery");
    let id = camp(&tw)
        .operations()
        .map(|d| d.id())
        .find(|id| !before.contains(&id.0))
        .expect("one new operation");
    let refusal = err(
        &mut tw,
        attacker,
        &format!("@op/terms {}/bond={theirs}:100", id.0),
    );
    assert!(refusal.contains("issuer's own account"), "{refusal}");
    assert!(camp(&tw).operation(id).expect("op").terms.bond.is_none());
    assert_eq!(held(&tw, theirs), 0);
}

// ---- C9 (MEDIUM; merges C13 and deviation (d)'s contract arm) --------------

/// **C9a. A stake naming a contract that does not exist is refused at the edit.**
#[test]
fn vf_c9a_a_stake_naming_a_contract_that_does_not_exist_is_refused_at_the_terms_edit() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let id = declare(&mut tw, attacker, 100);
    let refusal = err(
        &mut tw,
        attacker,
        &format!("@op/terms {}/stake.outcome=4242:success", id.0),
    );
    assert!(refusal.contains("does not exist"), "{refusal}");
    assert!(camp(&tw).operation(id).expect("op").terms.stakes.is_empty());
}

/// **C9b. A stake whose contract carries no matching canon attestor is refused** —
/// right contract, wrong (or missing) evidence binding.
#[test]
fn vf_c9b_a_stake_whose_contract_carries_no_matching_canon_attestor_is_refused() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let mine = declare(&mut tw, attacker, 100);
    let other = declare(&mut tw, attacker, 100);
    // A contract bound to a DIFFERENT operation's resolution.
    let contract = contract_attesting(&mut tw, attacker, other);
    let refusal = err(
        &mut tw,
        attacker,
        &format!("@op/terms {}/stake.outcome={contract}:success", mine.0),
    );
    assert!(
        refusal.contains("no `campaign.operation.resolved` attestor"),
        "{refusal}"
    );
    // …and the correctly bound one IS accepted.
    ok(
        &mut tw,
        attacker,
        &format!("@op/terms {}/stake.outcome={contract}:success", other.0),
    );
}

/// **C9c. A correctly bound contract really settles from the resolution**, and the
/// consumer re-runs twenty times writing nothing.
#[test]
fn vf_c9c_a_correctly_bound_contract_settles_and_the_consumer_reruns_twenty_times_writing_nothing()
{
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let id = declare(&mut tw, attacker, 100);
    let contract = contract_attesting(&mut tw, attacker, id);
    let payee = tw
        .world()
        .module_state::<EconomyStore>()
        .expect("economy")
        .snapshot()
        .contract(mush_economy::ContractId(contract))
        .expect("contract")
        .consent
        .parties
        .iter()
        .find_map(|p| match p {
            PrincipalId::Object(o) if *o != attacker => Some(*o),
            _ => None,
        })
        .expect("a payee party");
    let theirs = account_of(&tw, payee);
    ok(
        &mut tw,
        attacker,
        &format!("@op/terms {}/stake.outcome={contract}:success", id.0),
    );
    // The payee must be ON the canon record for the economy's attestation observer
    // to match it, so they are a named side.
    ok(
        &mut tw,
        attacker,
        &format!("@op/terms {}/side=defend:#{}", id.0, payee.0),
    );
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    ok(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
    fire_n(&mut tw, 10);
    assert_eq!(state_of(&tw, id), OperationState::Resolved);
    assert_eq!(bal(&tw, theirs), 10, "the bound contract did not settle");
    let total = ledger_total(&tw);
    fire_n(&mut tw, 20);
    assert_eq!(bal(&tw, theirs), 10, "a re-run paid twice");
    assert_eq!(ledger_total(&tw), total);
}

// ---- D1 / D6 (HIGH): what makes the root durable --------------------------

/// **D1a. A root record canon can no longer answer for still discharges
/// `FrontControl` and `ContractOutcomes`.** The evidence is the operation's own
/// durable marker, so emptying canon's window changes nothing.
#[test]
fn vf_d1a_a_root_record_canon_can_no_longer_answer_for_still_discharges_front_control() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let acct = account_of(&tw, attacker);
    let id = declare(&mut tw, attacker, 200);
    ok(
        &mut tw,
        attacker,
        &format!("@op/terms {}/stake.front=7:orbit", id.0),
    );
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    ok(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
    assert!(pending_of(&tw, id).contains(&ConsumerId::FrontControl));

    // The record is committed and the marker carries its id…
    let canon = camp(&tw)
        .operation(id)
        .expect("op")
        .runtime
        .root
        .as_ref()
        .and_then(|r| r.canon);
    assert!(canon.is_some(), "the marker was stamped without a canon id");

    // …and now canon's whole ledger is gone from the boot data, which is a harsher
    // version of the record ageing out of the RAM window.
    let mut kv = Kv::new();
    fold_into(&mut tw, &mut kv);
    kv.remove(&SmolStr::new_static("canon"));
    reboot(&mut tw, &kv);
    fire_n(&mut tw, 12);
    assert_eq!(
        state_of(&tw, id),
        OperationState::Resolved,
        "the canon-backed consumers refused for ever"
    );
    assert_eq!(held(&tw, acct), 0);
    let shown = ok(&mut tw, GOD, &format!("@op/show/full {}", id.0));
    assert!(!shown.contains("paused"), "{shown}");
}

/// **D1b. Ordinary canon traffic after the resolution starves nobody.** Thirty more
/// resolutions, each committing their own record, and the first still drains.
#[test]
fn vf_d1b_ordinary_canon_traffic_after_the_resolution_still_reaches_resolved() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let id = declare(&mut tw, attacker, 0);
    ok(
        &mut tw,
        attacker,
        &format!("@op/terms {}/stake.front=7:orbit", id.0),
    );
    ok(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
    // Staff traffic: thirty further operations, each resolving into canon.
    for _ in 0..30 {
        let other = declare(&mut tw, GOD, 0);
        ok(&mut tw, GOD, &format!("@op/resolve {}=success", other.0));
    }
    fire_n(&mut tw, 40);
    assert_eq!(state_of(&tw, id), OperationState::Resolved);
    // …and `=retry` was never needed.
    let shown = ok(&mut tw, GOD, &format!("@op/show/full {}", id.0));
    assert!(!shown.contains("paused"), "{shown}");
}

/// **D1c. A canon objective the bounded seam cannot answer resolves as a failure**
/// at window close, rather than hanging — and it says which outcome it recorded.
#[test]
fn vf_d1c_a_canon_objective_the_seam_cannot_answer_resolves_as_a_failure_at_window_close() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let id = declare(&mut tw, attacker, 50);
    ok(
        &mut tw,
        attacker,
        &format!(
            "@op/terms {}/objective.kind=canon:front.progress:clock:9",
            id.0
        ),
    );
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    // The declared window runs 24 h to 48 h, so this is well past its close.
    jump_to(&mut tw, 49 * HOUR);
    fire_n(&mut tw, 10);
    assert_eq!(state_of(&tw, id), OperationState::Resolved);
    assert_eq!(
        camp(&tw)
            .operation(id)
            .expect("op")
            .runtime
            .root
            .as_ref()
            .expect("root")
            .outcome,
        "failure"
    );
    assert_eq!(held(&tw, account_of(&tw, attacker)), 0);
}

/// **D6a. A composition with no canon refuses to enter `Resolving`**, and the
/// boundary fact answers unknown rather than satisfied.
#[test]
fn vf_d6a_a_canon_less_composition_refuses_to_enter_resolving_and_the_fact_answers_unknown() {
    use mush_economy::FactProvider;
    let mut tw = boot_without_canon();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let acct = account_of(&tw, attacker);
    let id = declare(&mut tw, attacker, 300);
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    let refusal = err(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
    assert!(refusal.contains("cannot resolve right now"), "{refusal}");

    // No marker, no boundary fact, and the bond is exactly where it was.
    assert!(camp(&tw).operation(id).expect("op").runtime.root.is_none());
    assert!(
        mush_campaign::CampaignFactProvider
            .resolve(
                tw.world(),
                mush_campaign::KIND_OPERATION_RESOLVED,
                &id.0.to_string(),
                None
            )
            .is_none(),
        "a contract could settle from an operation with no root record"
    );
    assert_eq!(held(&tw, acct), 300);
    assert!(!matches!(
        state_of(&tw, id),
        OperationState::Resolving { .. }
    ));

    // …and the exit is still there, because an abandonment causes no consequence
    // and answers no boundary fact: refusing it for want of a chronicle line would
    // strand the bond the hatch exists to release.
    ok(&mut tw, GOD, &format!("@op/resolve {}=abort", id.0));
    fire_n(&mut tw, 12);
    assert_eq!(state_of(&tw, id), OperationState::Aborted);
    assert_eq!(held(&tw, acct), 0);
    assert_eq!(bal(&tw, acct), 1_000);
    assert!(
        mush_campaign::CampaignFactProvider
            .resolve(
                tw.world(),
                mush_campaign::KIND_OPERATION_RESOLVED,
                &id.0.to_string(),
                None
            )
            .is_none(),
        "an abandonment with no canon line answered the boundary fact"
    );
}

/// **D6c / D9. Every stamped marker carries a canon id, and it survives a reboot.**
#[test]
fn vf_d6c_every_stamped_marker_carries_a_canon_id_and_it_survives_a_reboot() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let mut ids = Vec::new();
    for outcome in ["success", "failure", "partial"] {
        let id = declare(&mut tw, attacker, 10);
        ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
        ok(
            &mut tw,
            attacker,
            &format!("@op/resolve {}={outcome}", id.0),
        );
        ids.push(id);
    }
    let abandoned = declare(&mut tw, attacker, 10);
    ok(&mut tw, attacker, &format!("@op/activate {}", abandoned.0));
    ok(&mut tw, GOD, &format!("@op/resolve {}=abort", abandoned.0));
    ids.push(abandoned);
    fire_n(&mut tw, 12);
    let mut kv = Kv::new();
    fold_into(&mut tw, &mut kv);
    reboot(&mut tw, &kv);
    for id in ids {
        let root = camp(&tw)
            .operation(id)
            .expect("op")
            .runtime
            .root
            .clone()
            .expect("a marker");
        assert!(root.canon.is_some(), "{id}'s marker carries no canon id");
    }
}

// ---- D7 (MEDIUM; merges D10): abandoning is not settling -------------------

/// **D7a. An abandoned operation leaves a contract attesting on
/// `campaign.operation.resolved` unsettled** — and D7b proves the same contract
/// settles for a completion, so the difference really is the abandonment.
#[test]
fn vf_d7a_an_abandoned_operation_leaves_a_contract_attesting_on_the_resolution_unsettled() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let id = declare(&mut tw, attacker, 100);
    let contract = contract_attesting(&mut tw, attacker, id);
    let payee = tw.spawn_player("Watcher");
    let _ = payee;
    let party = snap(&tw)
        .contract(mush_economy::ContractId(contract))
        .expect("contract")
        .consent
        .parties
        .iter()
        .find_map(|p| match p {
            PrincipalId::Object(o) if *o != attacker => Some(*o),
            _ => None,
        })
        .expect("a payee");
    let theirs = account_of(&tw, party);
    ok(
        &mut tw,
        attacker,
        &format!("@op/terms {}/side=defend:#{}", id.0, party.0),
    );
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    ok(&mut tw, GOD, &format!("@op/resolve {}=abort", id.0));
    fire_n(&mut tw, 12);
    assert_eq!(state_of(&tw, id), OperationState::Aborted);
    assert_eq!(bal(&tw, theirs), 0, "an abandonment settled the contract");
    // …and the boundary fact says the same thing.
    use mush_economy::FactProvider;
    assert!(
        mush_campaign::CampaignFactProvider
            .resolve(
                tw.world(),
                mush_campaign::KIND_OPERATION_RESOLVED,
                &id.0.to_string(),
                None
            )
            .is_none()
    );
}

/// **D7b. A successful operation settles that same contract** — the control.
#[test]
fn vf_d7b_a_successful_operation_settles_that_same_contract() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let id = declare(&mut tw, attacker, 100);
    let contract = contract_attesting(&mut tw, attacker, id);
    let party = snap(&tw)
        .contract(mush_economy::ContractId(contract))
        .expect("contract")
        .consent
        .parties
        .iter()
        .find_map(|p| match p {
            PrincipalId::Object(o) if *o != attacker => Some(*o),
            _ => None,
        })
        .expect("a payee");
    let theirs = account_of(&tw, party);
    ok(
        &mut tw,
        attacker,
        &format!("@op/terms {}/side=defend:#{}", id.0, party.0),
    );
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    ok(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
    fire_n(&mut tw, 12);
    assert_eq!(bal(&tw, theirs), 10, "the completion did not settle it");
}

/// **D7c. After a full drain an abandoned operation reads `aborted`, the tick stops
/// visiting it, and its units are free.**
#[test]
fn vf_d7c_after_a_full_drain_an_abandoned_operation_reads_aborted_and_the_tick_stops_visiting_it() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let unit = unit_of(&mut tw, attacker, "Tank");
    let id = declare(&mut tw, attacker, 100);
    ok(&mut tw, attacker, &format!("@op/join {}=#{}", id.0, unit.0));
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    ok(&mut tw, GOD, &format!("@op/resolve {}=abort", id.0));
    fire_n(&mut tw, 12);
    assert_eq!(state_of(&tw, id), OperationState::Aborted);
    let shown = ok(&mut tw, GOD, &format!("@op/show {}", id.0));
    assert!(shown.contains("aborted"), "{shown}");
    // The tick stops working it: ten more fires move nothing at all.
    let revision = camp(&tw).operation(id).expect("op").runtime.revision;
    fire_n(&mut tw, 10);
    assert_eq!(
        camp(&tw).operation(id).expect("op").runtime.revision,
        revision
    );
    // …and the unit is free for another operation, observably.
    assert_eq!(camp(&tw).operation_holding(UnitRef(unit)), None);
    let next = declare(&mut tw, attacker, 10);
    ok(
        &mut tw,
        attacker,
        &format!("@op/join {}=#{}", next.0, unit.0),
    );
}

// ---- D8 (MEDIUM): a wait is not a failure ---------------------------------

/// **D8a. A frozen economy defers the bond consumer without counting an attempt**,
/// and it drains when the economy comes back.
#[test]
fn vf_d8a_a_frozen_economy_defers_the_bond_consumer_without_counting_an_attempt() {
    let mut tw = boot_freezable();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let acct = account_of(&tw, attacker);
    let id = declare(&mut tw, attacker, 200);
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    ok(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
    ok(&mut tw, GOD, "@econfreeze");
    // Four times `MAX_CONSUMER_ATTEMPTS`, and it neither pauses nor counts.
    fire_n(&mut tw, 12);
    let doc = camp(&tw).operation(id).expect("op").clone();
    assert!(doc.runtime.paused.is_none(), "a wait paused the operation");
    assert!(
        doc.runtime.attempts.is_empty(),
        "a wait was counted as a failure: {:?}",
        doc.runtime.attempts
    );
    assert!(pending_of(&tw, id).contains(&ConsumerId::BondSettlement));
    assert_eq!(held(&tw, acct), 200);

    ok(&mut tw, GOD, "@econ/unfreeze");
    fire_n(&mut tw, 10);
    assert_eq!(state_of(&tw, id), OperationState::Resolved);
    assert_eq!(held(&tw, acct), 0);
}

/// **D8b. A zero-window key lookup never pauses the operation.**
#[test]
fn vf_d8b_a_zero_window_key_lookup_never_pauses_the_operation() {
    let mut tw = boot_windowed(0);
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let acct = account_of(&tw, attacker);
    let id = declare(&mut tw, attacker, 150);
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    ok(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
    fire_n(&mut tw, 12);
    assert_eq!(state_of(&tw, id), OperationState::Resolved);
    assert!(
        camp(&tw)
            .operation(id)
            .expect("op")
            .runtime
            .paused
            .is_none()
    );
    assert_eq!(held(&tw, acct), 0);
}

// ---- B3 (HIGH): the refusal goes through the lens too ---------------------

/// **B3a. A join conflict with a covert operation names no operation and no
/// roster.** The existence-hiding pin `@op/show` carries, extended to the verb
/// beside it.
#[test]
fn vf_b3a_a_join_conflict_with_a_covert_operation_names_no_operation_and_no_roster() {
    let mut tw = boot();
    let hider = tw.spawn_player("Hider");
    let officer = tw.spawn_player("Officer");
    money(&mut tw, &[(hider, 1_000), (officer, 1_000)]);
    ok(&mut tw, GOD, "@org/create Ironhold");
    let scope = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("org")
        .state()
        .orgs()
        .next()
        .expect("one org")
        .root_scope;
    ok(&mut tw, GOD, &format!("@force/create lance={}", scope.0));
    let node = camp(&tw).formations().next().expect("node").id;
    // A unit the OFFICER commands through the formation and does not control as an
    // object — which is exactly the caller who reaches the conflict without ever
    // being entitled to see the operation holding it.
    let unit = unit_of(&mut tw, hider, "Theirs");
    ok(&mut tw, GOD, &format!("@force/assign {node}=#{}", unit.0));
    ok(
        &mut tw,
        GOD,
        &format!(
            "@org/grant #{}=Ironhold force.command scope={}",
            officer.0, scope.0
        ),
    );
    let covert = declare(&mut tw, hider, 10);
    ok(
        &mut tw,
        hider,
        &format!("@op/terms {}/visibility=covert", covert.0),
    );
    ok(&mut tw, GOD, &format!("@op/join {}=#{}", covert.0, unit.0));

    // The officer cannot see it exists…
    assert_eq!(
        err(&mut tw, officer, &format!("@op/show {}", covert.0)),
        NO_SUCH_OPERATION
    );
    // …and the join conflict does not tell them either.
    let mine = declare(&mut tw, officer, 10);
    let refusal = err(
        &mut tw,
        officer,
        &format!("@op/join {}=#{}", mine.0, unit.0),
    );
    assert!(refusal.contains("already committed elsewhere"), "{refusal}");
    for leak in [
        format!("op{}", covert.0),
        format!("operation {covert}"),
        covert.to_string(),
    ] {
        assert!(
            !refusal.to_lowercase().contains(&leak.to_lowercase()),
            "the refusal leaked {leak:?}: {refusal}"
        );
    }
    // The roster is not disclosed either: nothing about the covert operation's
    // membership appears in any answer the officer can obtain.
    let listed = ok(&mut tw, officer, "@op/list");
    assert!(!listed.contains(&covert.to_string()), "{listed}");
}

/// **B3b. A join conflict with an operation the caller CAN see still names it** —
/// the fix is a lens, not a blanket blackout.
#[test]
fn vf_b3b_a_join_conflict_with_an_operation_the_caller_can_see_still_names_it() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let unit = unit_of(&mut tw, attacker, "Tank");
    let a = declare(&mut tw, attacker, 10);
    let b = declare(&mut tw, attacker, 10);
    ok(&mut tw, attacker, &format!("@op/join {}=#{}", a.0, unit.0));
    let refusal = err(&mut tw, attacker, &format!("@op/join {}=#{}", b.0, unit.0));
    assert!(refusal.contains(&a.to_string()), "{refusal}");
}

// ---- B5 (MEDIUM): the formation surface is lensed too ---------------------

/// **B5a. A stranger learns neither the org compartment nor the roster from
/// `@formation/list` or `/show`.**
#[test]
fn vf_b5a_a_stranger_learns_neither_the_org_scope_nor_the_roster_from_formation_list_or_show() {
    let mut tw = boot();
    let stranger = tw.spawn_player("Stranger");
    ok(&mut tw, GOD, "@org/create Ironhold");
    let scope = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("org")
        .state()
        .orgs()
        .next()
        .expect("one org")
        .root_scope;
    ok(&mut tw, GOD, &format!("@force/create lance={}", scope.0));
    let node = camp(&tw).formations().next().expect("node").id;
    let a = unit_of(&mut tw, GOD, "A");
    let b = unit_of(&mut tw, GOD, "B");
    for u in [a, b] {
        ok(&mut tw, GOD, &format!("@force/assign {node}=#{}", u.0));
    }
    // Staff still see everything.
    let staff = ok(&mut tw, GOD, &format!("@formation/show {node}"));
    assert!(staff.contains("understrength (2 of 4)"), "{staff}");

    // The stranger sees the same world as one with no formations in it.
    let listed = ok(&mut tw, stranger, "@formation/list");
    assert_eq!(listed.trim(), "No formations exist.", "{listed}");
    let refusal = err(&mut tw, stranger, &format!("@formation/show {node}"));
    assert_eq!(refusal, mush_campaign::NO_SUCH_FORMATION);
    assert_eq!(
        refusal,
        err(&mut tw, stranger, "@formation/show fm99999"),
        "an existence oracle"
    );
    for leak in [&format!("scope {}", scope.0), "member(s)", "understrength"] {
        assert!(
            !listed.contains(leak) && !refusal.contains(leak),
            "the stranger's render leaked {leak:?}"
        );
    }
}

// ---- B6 (MEDIUM; merges C11): the depth graft ----------------------------

/// Three formations chained root → child → grandchild, returning them in order.
fn chain(tw: &mut TestWorld, scope: u64, prefix: &str) -> Vec<mush_campaign::FormationId> {
    let mut made = Vec::new();
    for _ in 0..3 {
        let before: BTreeSet<u64> = camp(tw).formations().map(|n| n.id.0).collect();
        ok(tw, GOD, &format!("@force/create lance={scope}"));
        let id = camp(tw)
            .formations()
            .map(|n| n.id)
            .find(|id| !before.contains(&id.0))
            .expect("one new node");
        made.push(id);
    }
    let _ = prefix;
    ok(tw, GOD, &format!("@force/attach {}={}", made[1], made[0]));
    ok(tw, GOD, &format!("@force/attach {}={}", made[2], made[1]));
    made
}

/// The ancestor chain of one node, walked in the test rather than in the code.
fn chain_len(tw: &TestWorld, id: mush_campaign::FormationId) -> usize {
    let mut cursor = Some(id);
    let mut n = 0usize;
    while let Some(current) = cursor {
        n += 1;
        assert!(n < 64, "the walk did not terminate");
        cursor = camp(tw).formation(current).and_then(|node| node.parent);
    }
    n
}

/// **B6a. Grafting a three-deep subtree under a three-deep chain is refused.**
#[test]
fn vf_b6a_grafting_a_three_deep_subtree_under_a_three_deep_chain_is_refused() {
    let mut tw = boot();
    ok(&mut tw, GOD, "@org/create Ironhold");
    let scope = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("org")
        .state()
        .orgs()
        .next()
        .expect("one org")
        .root_scope
        .0;
    let a = chain(&mut tw, scope, "a");
    let b = chain(&mut tw, scope, "b");
    assert_eq!(chain_len(&tw, a[2]), 3);
    // 3 (the parent's own chain) + 3 (the child's own subtree) = 6, past the cap.
    let refusal = err(&mut tw, GOD, &format!("@force/attach {}={}", b[0], a[2]));
    assert!(refusal.contains("deep"), "{refusal}");
    assert_eq!(camp(&tw).formation(b[0]).expect("b0").parent, None);
    assert_eq!(chain_len(&tw, b[2]), 3, "the graft happened anyway");
}

/// **B6b. The deepest node of any accepted tree has an ancestor chain inside the
/// cap** — including the graft that IS legal.
#[test]
fn vf_b6b_the_deepest_node_of_any_accepted_tree_has_an_ancestor_chain_inside_the_cap() {
    let mut tw = boot();
    ok(&mut tw, GOD, "@org/create Ironhold");
    let scope = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("org")
        .state()
        .orgs()
        .next()
        .expect("one org")
        .root_scope
        .0;
    let a = chain(&mut tw, scope, "a");
    let b = chain(&mut tw, scope, "b");
    // 2 + 3 = 5 exactly, which the cap admits.
    ok(&mut tw, GOD, &format!("@force/attach {}={}", b[0], a[1]));
    assert_eq!(chain_len(&tw, b[2]), mush_campaign::MAX_FORMATION_DEPTH);
    for node in camp(&tw).formations().map(|n| n.id).collect::<Vec<_>>() {
        assert!(
            chain_len(&tw, node) <= mush_campaign::MAX_FORMATION_DEPTH,
            "{node} is past the cap"
        );
    }
    // …and one more node under the deepest is refused.
    let c = chain(&mut tw, scope, "c");
    err(&mut tw, GOD, &format!("@force/attach {}={}", c[0], b[2]));
}

/// **B6c. A cycle is still refused and the walk still terminates.**
#[test]
fn vf_b6c_a_cycle_is_still_refused_and_the_walk_still_terminates() {
    let mut tw = boot();
    ok(&mut tw, GOD, "@org/create Ironhold");
    let scope = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("org")
        .state()
        .orgs()
        .next()
        .expect("one org")
        .root_scope
        .0;
    let a = chain(&mut tw, scope, "a");
    let refusal = err(&mut tw, GOD, &format!("@force/attach {}={}", a[0], a[2]));
    assert!(
        refusal.contains("cycle") || refusal.contains("deep"),
        "{refusal}"
    );
    err(&mut tw, GOD, &format!("@force/attach {}={}", a[0], a[0]));
    assert_eq!(chain_len(&tw, a[2]), 3);
}

// ---- B7 (MEDIUM; merges C10): the outcome word ---------------------------

/// **B7a. A 4 000-byte outcome is refused at the command, and a stored one is held
/// on load** — the encoding form, built and read back.
#[test]
fn vf_b7a_a_4000_byte_outcome_is_refused_at_the_command_and_a_stored_one_is_held_on_load() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let id = declare(&mut tw, attacker, 100);
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    let huge = "z".repeat(4_000);
    err(&mut tw, GOD, &format!("@op/resolve {}={huge}", id.0));
    assert_eq!(state_of(&tw, id), OperationState::Activated);

    // …and one that reached the store some other way refuses on load and is HELD,
    // never truncated and never deleted.
    ok(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
    let mut kv = Kv::new();
    fold_into(&mut tw, &mut kv);
    edit_op(&mut kv, id, |v| {
        v["runtime"]["root"]["outcome"] = serde_json::json!("z".repeat(4_000));
    });
    reboot(&mut tw, &kv);
    assert!(
        camp(&tw).operation(id).is_none(),
        "an unbounded outcome loaded"
    );
    assert_eq!(camp(&tw).held().len(), 1);
    assert!(
        camp(&tw).held()[0].error.contains("longer than"),
        "{:?}",
        camp(&tw).held()
    );
}

/// **B7b. An outcome outside the closed vocabulary is refused**, and the refusal
/// names the whole vocabulary.
#[test]
fn vf_b7b_an_outcome_outside_the_closed_vocabulary_is_refused() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let id = declare(&mut tw, attacker, 100);
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    for word in ["glorious victory", "succes", "abort!", "forfeit_all", "0"] {
        let refusal = err(&mut tw, GOD, &format!("@op/resolve {}={word}", id.0));
        assert!(refusal.contains("Outcomes are"), "{word}: {refusal}");
    }
    assert_eq!(state_of(&tw, id), OperationState::Activated);
    // Every word in the vocabulary IS accepted, on a fresh operation each.
    for word in mush_campaign::OUTCOME_VOCABULARY {
        if word == "retry" {
            continue;
        }
        let fresh = declare(&mut tw, attacker, 10);
        ok(&mut tw, attacker, &format!("@op/activate {}", fresh.0));
        ok(&mut tw, GOD, &format!("@op/resolve {}={word}", fresh.0));
    }
}

/// **B7c. `Forfeit` in any casing forfeits, and never silently releases.**
#[test]
fn vf_b7c_forfeit_in_any_casing_forfeits_and_never_silently_releases() {
    for word in ["Forfeit", "FORFEIT", "forfeit"] {
        let mut tw = boot();
        let attacker = tw.spawn_player("Attacker");
        let defender = tw.spawn_player("Defender");
        money(&mut tw, &[(attacker, 1_000), (defender, 0)]);
        let theirs = account_of(&tw, defender);
        let id = declare(&mut tw, attacker, 300);
        ok(
            &mut tw,
            attacker,
            &format!("@op/terms {}/bond.forfeit=#{}:{theirs}", id.0, defender.0),
        );
        ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
        ok(&mut tw, GOD, &format!("@op/resolve {}={word}", id.0));
        fire_n(&mut tw, 8);
        assert_eq!(
            bal(&tw, theirs),
            300,
            "`{word}` reported a forfeit and released the bond instead"
        );
    }
}

// ---- B8 / B9 (MEDIUM; decision E5(a)) ------------------------------------

/// **B8a. One player past the per-issuer cap is refused while a second player may
/// still declare** — and resolving one frees the slot.
#[test]
fn vf_b8a_one_player_past_the_per_issuer_cap_is_refused_while_a_second_player_may_still_declare() {
    let mut tw = boot();
    let hog = tw.spawn_player("Hog");
    let other = tw.spawn_player("Other");
    money(&mut tw, &[(hog, 1_000), (other, 1_000)]);
    let mut ids = Vec::new();
    for _ in 0..mush_campaign::MAX_OPERATIONS_PER_ISSUER {
        ids.push(declare(&mut tw, hog, 0));
    }
    let refusal = err(&mut tw, hog, "@op/declare one too many");
    assert!(
        refusal.contains(&mush_campaign::MAX_OPERATIONS_PER_ISSUER.to_string()),
        "the refusal does not name the cap: {refusal}"
    );
    // The world is not exhausted — only this issuer is.
    ok(&mut tw, other, "@op/declare mine");
    ok(&mut tw, GOD, "@op/declare staff are not counted");
    // …and a terminal operation frees the slot, which cut 9's global cap never did.
    ok(&mut tw, hog, &format!("@op/resolve {}=success", ids[0].0));
    fire_n(&mut tw, 10);
    assert_eq!(state_of(&tw, ids[0]), OperationState::Resolved);
    ok(&mut tw, hog, "@op/declare room again");
}

/// **B9a. Seventy participants on an audience operation: the entity list is capped
/// deliberately and the overflow is recorded in the record's own detail.**
#[test]
fn vf_b9a_seventy_participants_record_the_entity_overflow_in_the_canon_detail() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let id = declare(&mut tw, attacker, 10);
    for n in 0..70 {
        let unit = unit_of(&mut tw, attacker, &format!("U{n}"));
        ok(&mut tw, attacker, &format!("@op/join {}=#{}", id.0, unit.0));
    }
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    ok(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
    let state = tw
        .world()
        .module_state::<mush_canon::CanonState>()
        .expect("canon");
    let record = mush_canon::records_for_source(state, &mush_campaign::canon_source(id), 8)
        .into_iter()
        .next()
        .expect("the root record");
    assert_eq!(
        record.entities.len(),
        mush_canon::MAX_ENTITIES_PER_EVENT,
        "the producer outran the consumer's cap"
    );
    assert_eq!(
        record.detail.get("entities_dropped"),
        Some(&serde_json::json!(70 - mush_canon::MAX_ENTITIES_PER_EVENT)),
        "the truncation was silent: {:?}",
        record.detail
    );
}

// ---- B12 / C19 (LOW) ------------------------------------------------------

/// **B12a. A covert operation's resolution leaves a fact condition unsatisfied**:
/// the provider answers nothing, matching the `Secret` canon scope it writes.
#[test]
fn vf_b12a_a_covert_operations_resolution_leaves_a_fact_condition_unsatisfied() {
    use mush_economy::FactProvider;
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let covert = declare(&mut tw, attacker, 10);
    let public = declare(&mut tw, attacker, 10);
    ok(
        &mut tw,
        attacker,
        &format!("@op/terms {}/visibility=covert", covert.0),
    );
    ok(
        &mut tw,
        attacker,
        &format!("@op/terms {}/visibility=public", public.0),
    );
    for id in [covert, public] {
        ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
        ok(&mut tw, attacker, &format!("@op/resolve {}=success", id.0));
    }
    fire_n(&mut tw, 12);
    let ask = |tw: &TestWorld, id: OperationId| {
        mush_campaign::CampaignFactProvider.resolve(
            tw.world(),
            mush_campaign::KIND_OPERATION_RESOLVED,
            &id.0.to_string(),
            None,
        )
    };
    assert!(
        ask(&tw, covert).is_none(),
        "a covert resolution answered a contract condition"
    );
    assert!(ask(&tw, public).is_some_and(|f| f.satisfied));
}

/// **C19a. A one-of-four lance deploys and reports understrength under all three
/// strictness scopes** — the report-never-gate rule, pinned on every arm.
#[test]
fn vf_c19a_a_one_of_four_lance_deploys_and_reports_understrength_under_all_three_scopes() {
    let mut tw = boot();
    let commander = tw.spawn_player("Commander");
    money(&mut tw, &[(commander, 1_000)]);
    ok(&mut tw, GOD, "@org/create Ironhold");
    let scope = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("org")
        .state()
        .orgs()
        .next()
        .expect("one org")
        .root_scope;
    ok(&mut tw, GOD, &format!("@force/create lance={}", scope.0));
    let node = camp(&tw).formations().next().expect("node").id;
    let unit = unit_of(&mut tw, GOD, "Solo");
    ok(&mut tw, GOD, &format!("@force/assign {node}=#{}", unit.0));
    let shown = ok(&mut tw, GOD, &format!("@formation/show {node}"));
    for scope_label in ["creation", "doctrine", "operation"] {
        assert!(shown.contains(scope_label), "{scope_label}: {shown}");
    }
    assert_eq!(
        shown.matches("understrength (1 of 4)").count(),
        3,
        "every scope must report the SAME constraint set: {shown}"
    );
    // …and it still deploys, which is the whole rule.
    let id = declare(&mut tw, commander, 10);
    ok(&mut tw, GOD, &format!("@op/join {}={node}", id.0));
    let shown = ok(&mut tw, commander, &format!("@op/show {}", id.0));
    assert!(shown.contains("Participants: 1"), "{shown}");
}

/// **C3d. A held operation's bond gets a staff campaign-side release.**
///
/// A document the boot refused is held, not deleted — so the operation is not in
/// `ops`, every `@op` verb answers the existence-hiding refusal, and cut 9 left its
/// live reservation reachable only through `@hold/release`, which belongs to the
/// placer and (since E2(a)) is refused outright. Staff get one path to it.
#[test]
fn vf_c3d_a_held_operations_bond_gets_a_staff_campaign_side_release() {
    let mut tw = boot();
    let attacker = tw.spawn_player("Attacker");
    money(&mut tw, &[(attacker, 1_000)]);
    let acct = account_of(&tw, attacker);
    let id = declare(&mut tw, attacker, 400);
    ok(&mut tw, attacker, &format!("@op/activate {}", id.0));
    assert_eq!(held(&tw, acct), 400);

    // A document the wire refuses — the shape a corrupt store or a future build's
    // record would take.
    let mut kv = Kv::new();
    fold_into(&mut tw, &mut kv);
    edit_op(&mut kv, id, |v| {
        v["runtime"]["participants"] = serde_json::json!([
            { "unit": 1, "side": "a", "command_parent": null, "joined_at": 0 },
            { "unit": 1, "side": "a", "command_parent": null, "joined_at": 0 },
        ]);
    });
    reboot(&mut tw, &kv);
    assert!(
        camp(&tw).operation(id).is_none(),
        "the fixture must be held"
    );
    assert_eq!(camp(&tw).held().len(), 1, "held, never deleted");
    assert_eq!(
        held(&tw, acct),
        400,
        "and the bond really is still reserved"
    );

    // Every ordinary route is closed, in both directions.
    assert_eq!(
        err(&mut tw, GOD, &format!("@op/show {}", id.0)),
        NO_SUCH_OPERATION
    );
    let hold = bond_hold(&tw, id).expect("the bond").id;
    for who in [attacker, GOD] {
        err(&mut tw, who, &format!("@hold/release {}", hold.0));
    }
    fire_n(&mut tw, 8);
    assert_eq!(held(&tw, acct), 400, "the tick cannot reach it either");

    // …and the one staff path releases it.
    let shown = ok(&mut tw, GOD, &format!("@op/resolve {}=abort", id.0));
    assert!(shown.contains("held"), "{shown}");
    assert_eq!(held(&tw, acct), 0, "the bond stayed stranded");
    assert_eq!(bal(&tw, acct), 1_000, "and a release must never capture");
    // A non-staff caller still gets the existence-hiding refusal, not the path.
    assert_eq!(
        err(&mut tw, attacker, &format!("@op/resolve {}=abort", id.0)),
        NO_SUCH_OPERATION
    );
}
