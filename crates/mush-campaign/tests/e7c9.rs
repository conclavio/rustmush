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
use mush_core::{Audience, GOD, ModuleBootData, ModuleWrite, ObjId, World};
use mush_economy::{AccountId, EconomySnapshot, EconomyStore, HoldStatus};
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
        Box::new(mush_economy::EconomyModule::with_custody(vec![])),
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
            mush_economy::EconomyModule::with_custody(vec![]).with_config(
                mush_economy::EconomyConfig {
                    idempotency_window_secs: secs,
                    ..mush_economy::EconomyConfig::default()
                },
            ),
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
    ok(&mut tw, attacker, &format!("@op/resolve {}=forfeit", id.0));
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
    let doomed = snap(&tw)
        .holds()
        .find(|h| h.purpose == mush_campaign::bond::purpose(a))
        .expect("a's hold")
        .id;
    ok(&mut tw, attacker, &format!("@hold/release {}", doomed.0));

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
    ok(
        &mut tw,
        attacker,
        &format!("@op/terms {}/stake.outcome=1:success", id.0),
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
    // Drop the canon ledger entirely: the doomed operation's root record is gone,
    // so its canon-backed consumers can never be discharged.
    kv.remove(&SmolStr::new_static("canon"));
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
    let unit = tw.spawn_player("Unit");
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
        let u = tw.spawn_player(&format!("Unit{n}"));
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
    let unit = tw.spawn_player("Unit");
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
    for line in [
        format!("@force/create lance={}", scope.0),
        format!("@force/attach {a}={b}"),
        format!("@force/detach {a}"),
        format!("@force/assign {a}=#{}", stranger.0),
    ] {
        let refusal = err(&mut tw, stranger, &line);
        assert!(refusal.contains("Permission denied"), "{line}: {refusal}");
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

/// **P1. The crate's edge set is exactly `{core, economy, canon, org}`** — no
/// faction, no sim, no tactics, no propagation, no ruleset. `cargo tree -p
/// mush-campaign` is the gate; this is the same fact read from the manifest, so a
/// stray edge fails a unit test rather than only a human's reading.
#[test]
fn e7c9_p1_the_manifest_declares_exactly_four_edges() {
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
        BTreeSet::from(["mush-canon", "mush-core", "mush-economy", "mush-org"]),
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
    let unit = tw.spawn_player("Unit");
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
    assert_eq!(state_of(&tw, id), OperationState::Resolved);
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
