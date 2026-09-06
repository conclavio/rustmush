//! E3d acceptance (doc 08b §6, D-E3-8 / D-E3-9 / D-E3-11): the faction →
//! backing-`OrgId` link and the one-way `Faction.resources` migration, driven
//! through the real `Engine` with the server's own module order
//! (org → faction → economy, since the post-E3 owner decision).
//!
//! The verify axis of this cut is **attribution + idempotent migration**, so the
//! tests cluster around three questions:
//!
//! - *whose money is this?* — the link resolves live, and every way of losing that
//!   answer (unbound, missing, two factions naming one org) fails closed rather
//!   than guessing;
//! - *did the migration happen exactly once?* — post-then-mark converges after a
//!   dropped faction-doc write, and a re-run is an idempotency HIT that mints
//!   nothing;
//! - *who may look?* — a faction officer is not an org member (D-E3-11), so the
//!   officer gate shows ratings and the economy's own §4.5 view gate shows money.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use mush_core::module::GameModule;
use mush_core::{Audience, GOD, ModuleBootData, ModuleWrite, ObjId};
use mush_economy::{AccountId, EconomyModule, EconomyStore};
use mush_faction::{
    Faction, FactionId, FactionModule, FactionStandingProvider, FactionState, LinkError,
    backing_org,
};
use mush_org::{OrgId, OrgModule, OrgStore};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

// ---- rig ------------------------------------------------------------------

/// The three namespaces this cut spans. `fold_kv` keeps all of them so a reboot
/// re-derives the faction link, the organization it names, and the journal that
/// holds its money together — dropping any one of them would make the reboot
/// tests prove nothing.
const NAMESPACES: [&str; 3] = ["faction", "org", "economy"];

/// Exactly `mush-server/src/modules.rs`'s order since the post-E3 owner decision:
/// org registers BEFORE faction, so faction boot can validate every backing link
/// against loaded org state — reporting a missing org or a conflict, never
/// unbinding. [`backing_org`] still resolves at every use; boot only adds the
/// early, loud report.
fn modules() -> Vec<Box<dyn GameModule>> {
    vec![
        Box::new(OrgModule::with_predicates(vec![Arc::new(
            FactionStandingProvider,
        )])),
        Box::new(FactionModule::new()),
        Box::new(EconomyModule::with_custody(vec![])),
    ]
}

/// Rewrite one persisted faction doc inside a folded kv (a restored backup or a
/// hand-edited doc — the states boot-time validation exists to report).
fn patch_faction_doc(kv: &mut Kv, fid: FactionId, patch: impl FnOnce(&mut serde_json::Value)) {
    let key = format!("fac:{}", fid.0);
    let bucket = kv.get_mut("faction").expect("faction namespace present");
    let (_, doc) = bucket
        .iter_mut()
        .find(|(k, _)| k.as_str() == key.as_str())
        .expect("faction doc present");
    patch(doc);
}

fn boot() -> TestWorld {
    TestWorld::with_modules(modules())
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
/// Run a command that must succeed, returning what the actor saw.
fn ok(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let ev = tw.run(actor, line);
    assert!(!has_error(&ev), "`{line}` was refused: {}", texts(&ev));
    texts(&ev)
}
/// Run a command that must be refused, returning what the actor saw.
fn err(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let ev = tw.run(actor, line);
    assert!(
        has_error(&ev),
        "`{line}` should have been refused, got: {}",
        texts(&ev)
    );
    texts(&ev)
}

/// The folded per-namespace kv a reboot is fed.
type Kv = BTreeMap<SmolStr, Vec<(SmolStr, serde_json::Value)>>;

/// Fold every module op queued since the last drain into `kv` — the `e2b.rs`
/// helper widened to all three namespaces, and *accumulating* so a test can
/// snapshot a doc mid-run and put the earlier version back (a dropped write).
fn fold_into(tw: &mut TestWorld, kv: &mut Kv) {
    for op in tw.take_module_ops() {
        if !NAMESPACES.contains(&op.ns.as_str()) {
            continue;
        }
        let bucket = kv.entry(op.ns.clone()).or_default();
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                bucket.retain(|(k, _)| *k != key);
                bucket.push((key, value));
            }
            ModuleWrite::Del { key } => bucket.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
}
fn fold_kv(tw: &mut TestWorld) -> Kv {
    let mut kv = Kv::new();
    fold_into(tw, &mut kv);
    kv
}
fn reboot(kv: Kv) -> TestWorld {
    let mut tw = boot();
    let boot_map: HashMap<SmolStr, ModuleBootData> = kv
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
        .collect();
    tw.modules_loaded(boot_map);
    tw
}

fn fac_state(tw: &TestWorld) -> &FactionState {
    tw.world()
        .module_state::<FactionState>()
        .expect("faction state")
}
fn faction(tw: &TestWorld, name: &str) -> Faction {
    fac_state(tw)
        .faction_by_name(name)
        .expect("faction exists")
        .clone()
}
fn store(tw: &TestWorld) -> &EconomyStore {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy store")
}
fn bal(tw: &TestWorld, id: AccountId) -> i64 {
    store(tw)
        .snapshot()
        .account(id)
        .expect("account")
        .posted_balance
        .minor()
}
fn tx_count(tw: &TestWorld) -> usize {
    store(tw).transactions().count()
}
/// The org's root-scope account (the migration destination), by construction the
/// only scoped account in these fixtures.
fn org_account(tw: &TestWorld) -> Option<AccountId> {
    store(tw)
        .snapshot()
        .accounts()
        .find(|a| a.scope.is_some())
        .map(|a| a.id)
}
fn root_scope(tw: &TestWorld, org: OrgId) -> mush_org::OrgScopeId {
    tw.world()
        .module_state::<OrgStore>()
        .and_then(|s| s.state().org(org).map(|o| o.root_scope))
        .expect("org exists")
}

/// Gold (scale 2), a system faucet (account #0), a Camarilla with one rating pool
/// (`influence.police`) and one pool destined to become money (`gold = 250`), and
/// a freshly minted backing org. Returns nothing — every id in these tests is
/// looked up, never assumed.
fn setup(tw: &mut TestWorld) {
    ok(tw, GOD, "@instrument/register gold=2");
    ok(tw, GOD, "@account/system faucet=gold");
    ok(tw, GOD, "@faction/create Camarilla");
    ok(
        tw,
        GOD,
        "@faction/resource Camarilla=influence.police/3/precinct captains",
    );
    ok(tw, GOD, "@faction/resource Camarilla=gold/250/turf income");
    ok(tw, GOD, "@faction/org Camarilla=new");
}

// ----------------------------------------------------------------------
// The link
// ----------------------------------------------------------------------

#[test]
fn bind_new_unbind_and_rebind_an_existing_org() {
    let mut tw = boot();
    ok(&mut tw, GOD, "@faction/create Camarilla");
    assert_eq!(
        backing_org(tw.world(), &faction(&tw, "Camarilla")),
        Err(LinkError::Unbound),
        "@faction/create never auto-binds — attribution is a deliberate staff act"
    );
    assert!(ok(&mut tw, GOD, "@faction/show Camarilla").contains("no backing org"));

    // `new` mints an org + root scope through `submit`, exactly as @org/create.
    ok(&mut tw, GOD, "@faction/org Camarilla=new");
    let org = backing_org(tw.world(), &faction(&tw, "Camarilla")).expect("bound");
    assert_eq!(org, OrgId(0));
    assert!(ok(&mut tw, GOD, "@faction/show Camarilla").contains("org #0"));
    // The org is real: it has a root scope and answers to @org/show.
    assert!(ok(&mut tw, GOD, "@org/show Camarilla").contains("root scope #0"));
    // Minting twice is refused; the link is not a stack.
    assert!(err(&mut tw, GOD, "@faction/org Camarilla=new").contains("unbind it first"));

    // `none` unbinds and leaves the org standing.
    ok(&mut tw, GOD, "@faction/org Camarilla=none");
    assert_eq!(
        backing_org(tw.world(), &faction(&tw, "Camarilla")),
        Err(LinkError::Unbound)
    );
    assert!(
        tw.world()
            .module_state::<OrgStore>()
            .unwrap()
            .state()
            .org(org)
            .is_some(),
        "unbinding is not a deletion"
    );

    // Re-binding the same org by name works; so does by #id.
    ok(&mut tw, GOD, "@faction/org Camarilla=Camarilla");
    assert_eq!(backing_org(tw.world(), &faction(&tw, "Camarilla")), Ok(org));
    ok(&mut tw, GOD, "@faction/org Camarilla=none");
    ok(&mut tw, GOD, "@faction/org Camarilla=#0");
    assert_eq!(backing_org(tw.world(), &faction(&tw, "Camarilla")), Ok(org));
    // A nonexistent org is refused.
    err(&mut tw, GOD, "@faction/org Camarilla=#77");
}

#[test]
fn one_org_backs_one_faction_and_a_duplicate_makes_both_unusable() {
    let mut tw = boot();
    setup(&mut tw);
    ok(&mut tw, GOD, "@faction/create Sabbat");
    let org = OrgId(0);

    // The bind-time check refuses the second binding outright.
    let seen = err(&mut tw, GOD, "@faction/org Sabbat=#0");
    assert!(seen.contains("already backs faction Camarilla"), "{seen}");
    assert_eq!(faction(&tw, "Sabbat").org, None);

    // Force the duplicate the way a restored backup or a hand-edited doc would,
    // and BOTH links must now fail closed — there is no safe way to guess which
    // faction the org's money belongs to.
    let sabbat = faction(&tw, "Sabbat").id;
    tw.world_mut()
        .module_state_mut::<FactionState>()
        .unwrap()
        .factions
        .get_mut(&sabbat)
        .unwrap()
        .org = Some(org);
    assert_eq!(
        backing_org(tw.world(), &faction(&tw, "Camarilla")),
        Err(LinkError::Conflict(org))
    );
    assert_eq!(
        backing_org(tw.world(), &faction(&tw, "Sabbat")),
        Err(LinkError::Conflict(org))
    );
    assert!(ok(&mut tw, GOD, "@faction/show Camarilla").contains("org #0 (CONFLICT)"));
    assert!(ok(&mut tw, GOD, "@faction/show Sabbat").contains("org #0 (CONFLICT)"));
    // And the money command refuses on either side.
    for f in ["Camarilla", "Sabbat"] {
        let seen = err(
            &mut tw,
            GOD,
            &format!("@faction/migrate/confirm {f}=gold #0"),
        );
        assert!(seen.contains("more than one faction"), "{seen}");
    }
    assert_eq!(tx_count(&tw), 0, "nothing posted under a conflicted link");

    // Unbinding one side restores the other.
    ok(&mut tw, GOD, "@faction/org Sabbat=none");
    assert_eq!(backing_org(tw.world(), &faction(&tw, "Camarilla")), Ok(org));
}

#[test]
fn a_missing_org_makes_the_link_unresolvable_not_absent() {
    let mut tw = boot();
    ok(&mut tw, GOD, "@faction/create Camarilla");
    // Bind to an org that does not exist (the state a lost org doc leaves behind).
    let fid = faction(&tw, "Camarilla").id;
    tw.world_mut()
        .module_state_mut::<FactionState>()
        .unwrap()
        .factions
        .get_mut(&fid)
        .unwrap()
        .org = Some(OrgId(9));
    assert_eq!(
        backing_org(tw.world(), &faction(&tw, "Camarilla")),
        Err(LinkError::Missing(OrgId(9)))
    );
    assert!(ok(&mut tw, GOD, "@faction/show Camarilla").contains("org #9 (MISSING)"));
}

#[test]
fn boot_reports_a_missing_backing_org_and_preserves_the_link() {
    let mut tw = boot();
    setup(&mut tw);
    let fid = faction(&tw, "Camarilla").id;
    let mut kv = fold_kv(&mut tw);
    // The persisted doc names an org that does not exist (a lost org doc). Org
    // loads BEFORE faction now, so faction boot sees the loaded org state and
    // reports the gap — and must NOT repair it by unbinding (doc 08 §15.2).
    patch_faction_doc(&mut kv, fid, |doc| doc["org"] = serde_json::json!(9));
    let mut tw2 = reboot(kv);
    assert_eq!(
        faction(&tw2, "Camarilla").org,
        Some(OrgId(9)),
        "boot reports, never unbinds"
    );
    assert_eq!(
        backing_org(tw2.world(), &faction(&tw2, "Camarilla")),
        Err(LinkError::Missing(OrgId(9)))
    );
    assert!(ok(&mut tw2, GOD, "@faction/show Camarilla").contains("org #9 (MISSING)"));
}

#[test]
fn boot_reports_a_two_factions_one_org_conflict_and_preserves_both_links() {
    let mut tw = boot();
    setup(&mut tw); // Camarilla is bound to org #0
    ok(&mut tw, GOD, "@faction/create Sabbat");
    let sabbat = faction(&tw, "Sabbat").id;
    let mut kv = fold_kv(&mut tw);
    // A restored backup binds the second faction to the same org.
    patch_faction_doc(&mut kv, sabbat, |doc| doc["org"] = serde_json::json!(0));
    let mut tw2 = reboot(kv);
    assert_eq!(faction(&tw2, "Camarilla").org, Some(OrgId(0)));
    assert_eq!(faction(&tw2, "Sabbat").org, Some(OrgId(0)));
    assert!(ok(&mut tw2, GOD, "@faction/show Camarilla").contains("(CONFLICT)"));
    assert!(ok(&mut tw2, GOD, "@faction/show Sabbat").contains("(CONFLICT)"));
    // Both links fail closed at use until staff re-bind one of them.
    err(&mut tw2, GOD, "@faction/migrate/confirm Camarilla=gold #0");
    assert_eq!(tx_count(&tw2), 0);
}

#[test]
fn destroy_is_refused_while_bound_and_allowed_after_unbinding() {
    let mut tw = boot();
    setup(&mut tw);
    let seen = err(&mut tw, GOD, "@faction/destroy Camarilla");
    assert!(seen.contains("unbind it first"), "{seen}");
    assert!(
        fac_state(&tw).faction_by_name("Camarilla").is_some(),
        "still there"
    );

    // A MISSING link blocks destruction too: it is exactly the state that needs
    // staff attention, not a licence to delete the attribution.
    let fid = faction(&tw, "Camarilla").id;
    tw.world_mut()
        .module_state_mut::<FactionState>()
        .unwrap()
        .factions
        .get_mut(&fid)
        .unwrap()
        .org = Some(OrgId(9));
    err(&mut tw, GOD, "@faction/destroy Camarilla");

    ok(&mut tw, GOD, "@faction/org Camarilla=none");
    ok(&mut tw, GOD, "@faction/destroy Camarilla");
    assert!(fac_state(&tw).faction_by_name("Camarilla").is_none());
    // The org outlives the faction.
    assert!(
        tw.world()
            .module_state::<OrgStore>()
            .unwrap()
            .state()
            .org(OrgId(0))
            .is_some()
    );
}

// ----------------------------------------------------------------------
// Ratings vs money
// ----------------------------------------------------------------------

#[test]
fn a_rating_key_still_applies_and_a_migrated_key_is_refused() {
    let mut tw = boot();
    setup(&mut tw);
    ok(&mut tw, GOD, "@faction/migrate/confirm Camarilla=gold #0");

    // The untouched rating pool keeps its saturating, unaudited arithmetic.
    ok(
        &mut tw,
        GOD,
        "@faction/resource Camarilla=influence.police/-1/lost a captain",
    );
    assert_eq!(
        faction(&tw, "Camarilla").resources.get("influence.police"),
        Some(&2)
    );

    // The migrated key is money now and this verb refuses it, with the pointer.
    let seen = err(&mut tw, GOD, "@faction/resource Camarilla=gold/100");
    assert!(seen.contains("@econ/pay"), "{seen}");
    assert!(seen.contains("@econ/mint"), "{seen}");
    // Case and spacing do not smuggle it past the check.
    err(&mut tw, GOD, "@faction/resource Camarilla= GOLD /100");
    let f = faction(&tw, "Camarilla");
    assert!(
        !f.resources.contains_key("gold"),
        "no zeroed rating shadowing the money"
    );
    assert!(f.migrated.contains("gold"));
}

// ----------------------------------------------------------------------
// Migration refusals
// ----------------------------------------------------------------------

#[test]
fn migrate_without_confirm_shows_the_scale_and_moves_nothing() {
    let mut tw = boot();
    setup(&mut tw);
    let seen = ok(&mut tw, GOD, "@faction/migrate Camarilla=gold #0");
    // Verbatim minor units: 250 on a scale-2 instrument reads as 2.50, and staff
    // see that interpretation BEFORE anything mints (F2).
    assert!(seen.contains("2.50"), "{seen}");
    assert!(seen.contains("Nothing has moved"), "{seen}");
    assert!(seen.contains("/confirm"), "{seen}");
    assert_eq!(tx_count(&tw), 0);
    assert_eq!(bal(&tw, AccountId(0)), 0);
    assert_eq!(faction(&tw, "Camarilla").resources.get("gold"), Some(&250));
    assert!(faction(&tw, "Camarilla").migrated.is_empty());
}

#[test]
fn migrate_refuses_an_unregistered_or_inactive_instrument() {
    let mut tw = boot();
    setup(&mut tw);
    // Not a registered instrument at all.
    let seen = err(
        &mut tw,
        GOD,
        "@faction/migrate/confirm Camarilla=influence.police #0",
    );
    assert!(seen.contains("No instrument"), "{seen}");
    // Not even a usable instrument code.
    err(
        &mut tw,
        GOD,
        "@faction/migrate/confirm Camarilla=boon credit #0",
    );

    // Suspend gold the way a lifecycle change would, through a reboot, and the
    // migration refuses rather than marking a key it cannot post.
    let mut kv = fold_kv(&mut tw);
    for (k, v) in kv.get_mut("economy").expect("economy kv") {
        if k == "instrument:gold" {
            v["status"] = serde_json::json!("Suspended");
        }
    }
    let mut tw2 = reboot(kv);
    let seen = err(&mut tw2, GOD, "@faction/migrate/confirm Camarilla=gold #0");
    assert!(seen.contains("Suspended"), "{seen}");
    assert_eq!(tx_count(&tw2), 0);
    assert!(faction(&tw2, "Camarilla").migrated.is_empty());
}

#[test]
fn migrate_refuses_a_non_system_source_and_an_unbacked_faction() {
    let mut tw = boot();
    setup(&mut tw);
    // A player wallet is not a faucet: a migration mints value that never had a
    // counterparty, so the source must be a registered system source/sink.
    let pat = tw.spawn_player("Pat");
    ok(&mut tw, GOD, &format!("@account/open #{}=gold", pat.0));
    let seen = err(&mut tw, GOD, "@faction/migrate/confirm Camarilla=gold #1");
    assert!(seen.contains("system source/sink"), "{seen}");
    assert_eq!(tx_count(&tw), 0);
    assert!(faction(&tw, "Camarilla").migrated.is_empty());

    // No backing org, no attribution, no migration.
    ok(&mut tw, GOD, "@faction/org Camarilla=none");
    let seen = err(&mut tw, GOD, "@faction/migrate/confirm Camarilla=gold #0");
    assert!(seen.contains("no backing organization"), "{seen}");
    assert_eq!(tx_count(&tw), 0);
}

#[test]
fn migrate_refuses_a_zero_or_negative_pool() {
    let mut tw = boot();
    setup(&mut tw);
    ok(&mut tw, GOD, "@instrument/register silver=0");
    ok(&mut tw, GOD, "@account/system sink=silver");

    // A key with no pool: nothing to mint.
    let seen = err(&mut tw, GOD, "@faction/migrate/confirm Camarilla=silver #1");
    assert!(seen.contains("nothing to migrate"), "{seen}");

    // A negative pool: the journal cannot express minted debt.
    ok(
        &mut tw,
        GOD,
        "@faction/resource Camarilla=silver/-40/overspent",
    );
    let seen = err(&mut tw, GOD, "@faction/migrate/confirm Camarilla=silver #1");
    assert!(seen.contains("NEGATIVE"), "{seen}");
    assert_eq!(tx_count(&tw), 0);
    assert!(faction(&tw, "Camarilla").migrated.is_empty());
}

// ----------------------------------------------------------------------
// Refusals that must land ABOVE the wallet, not below it
// ----------------------------------------------------------------------
//
// `open_root_account` is the one store mutation `@faction/migrate` makes before
// `post`: it advances the account allocator and puts a record in RAM under a
// DEFERRED persist. Core discards a failed handler's deferred writes but not its
// world mutations, so a refusal that arrives after that point and returns `Err`
// leaves a phantom account — no doc, but real in RAM, holding the allocator's
// advance and binding the org's root scope in `EconomyScopeUse` until the next
// reboot. Every refusal decidable from facts already in hand therefore has to be
// decided first, and these two are the ones the economy would otherwise raise from
// inside `post`.

/// Count every account, phantom or not: the RAM snapshot is exactly where a
/// discarded-persist account would still be visible.
fn account_count(tw: &TestWorld) -> usize {
    store(tw).snapshot().accounts().count()
}
/// Whether anything was queued to the economy namespace since the last drain.
fn wrote_economy(tw: &mut TestWorld) -> bool {
    tw.take_module_ops()
        .iter()
        .any(|op| op.ns.as_str() == "economy")
}
/// Rewrite an org's persisted status. The only route to a wound-down org — the org
/// mutation alphabet has no status op — and the same trick `mush-economy`'s e3b2
/// suite uses.
fn with_org_status(kv: &Kv, org: OrgId, status: &str) -> Kv {
    let mut kv = kv.clone();
    let key = mush_org::org_key(org);
    for (k, v) in kv.get_mut("org").expect("org kv") {
        if *k == key {
            v["status"] = serde_json::json!(status);
        }
    }
    kv
}

#[test]
fn a_frozen_economy_refuses_the_migration_before_a_wallet_is_opened() {
    let mut tw = boot();
    setup(&mut tw);
    // Freeze the way boot freezes: a stored balance that disagrees with the journal
    // at its own claimed sequence is drift, and drift latches the whole economy.
    let mut kv = fold_kv(&mut tw);
    for (k, v) in kv.get_mut("economy").expect("economy kv") {
        if k == "account:0" {
            v["posted_balance"] = serde_json::json!(5);
        }
    }
    let mut tw2 = reboot(kv);
    assert!(store(&tw2).is_frozen(), "the drift must latch the freeze");
    let before = account_count(&tw2);
    let _ = wrote_economy(&mut tw2); // drain boot's own healing writes

    let seen = err(&mut tw2, GOD, "@faction/migrate/confirm Camarilla=gold #0");
    assert!(seen.contains("frozen"), "{seen}");

    // The refusal landed above `open_root_account`: no wallet in RAM, none on the
    // scope, and not one economy write queued (so the allocator never moved either).
    assert_eq!(
        account_count(&tw2),
        before,
        "a phantom wallet was opened under a freeze"
    );
    assert!(
        org_account(&tw2).is_none(),
        "no org root-scope wallet exists"
    );
    assert_eq!(tx_count(&tw2), 0);
    assert!(
        !wrote_economy(&mut tw2),
        "a refused migration wrote to the economy"
    );
    // And the rating is untouched: still a rating, still unmarked.
    let f = faction(&tw2, "Camarilla");
    assert_eq!(f.resources.get("gold"), Some(&250));
    assert!(f.migrated.is_empty());
}

#[test]
fn a_suspended_backing_org_refuses_the_migration_before_a_wallet_is_opened() {
    let mut tw = boot();
    setup(&mut tw);
    let kv = fold_kv(&mut tw);

    // A `Suspended` org may not RECEIVE value (§4.4), so the credit half of the plan
    // could never be authorized — and the wallet meant to receive it is never opened.
    let mut tw2 = reboot(with_org_status(&kv, OrgId(0), "Suspended"));
    let before = account_count(&tw2);
    let _ = wrote_economy(&mut tw2);
    let seen = err(&mut tw2, GOD, "@faction/migrate/confirm Camarilla=gold #0");
    assert!(seen.contains("Suspended"), "{seen}");
    assert_eq!(
        account_count(&tw2),
        before,
        "a phantom wallet was opened for an uncreditable org"
    );
    assert!(
        org_account(&tw2).is_none(),
        "no org root-scope wallet exists"
    );
    assert_eq!(tx_count(&tw2), 0);
    assert!(
        !wrote_economy(&mut tw2),
        "a refused migration wrote to the economy"
    );
    let f = faction(&tw2, "Camarilla");
    assert_eq!(f.resources.get("gold"), Some(&250));
    assert!(f.migrated.is_empty());

    // The hoisted gate MIRRORS the economy's own rule rather than inventing a
    // stricter one: a `Dissolving` org is exactly the case that must still settle.
    let mut tw3 = reboot(with_org_status(&kv, OrgId(0), "Dissolving"));
    ok(&mut tw3, GOD, "@faction/migrate/confirm Camarilla=gold #0");
    let dest = org_account(&tw3).expect("a dissolving org still takes final settlement");
    assert_eq!(bal(&tw3, dest), 250);
    assert_eq!(tx_count(&tw3), 1);
    assert!(faction(&tw3, "Camarilla").migrated.contains("gold"));
}

// ----------------------------------------------------------------------
// The migration itself
// ----------------------------------------------------------------------

#[test]
fn a_confirmed_migrate_posts_once_and_a_second_run_changes_nothing() {
    let mut tw = boot();
    setup(&mut tw);
    let seen = ok(&mut tw, GOD, "@faction/migrate/confirm Camarilla=gold #0");
    assert!(seen.contains("2.50"), "{seen}");
    let dest = org_account(&tw).expect("org wallet opened");
    assert_eq!(bal(&tw, AccountId(0)), -250, "the faucet drives negative");
    assert_eq!(bal(&tw, dest), 250, "the org's root-scope wallet holds it");
    assert_eq!(tx_count(&tw), 1);

    // The wallet is the org's, on its root scope, and cannot go negative.
    let org = backing_org(tw.world(), &faction(&tw, "Camarilla")).expect("bound");
    let acct = store(&tw)
        .snapshot()
        .account(dest)
        .expect("account")
        .clone();
    assert_eq!(acct.owner, mush_org::PrincipalId::Organization(org));
    assert_eq!(acct.scope, Some(root_scope(&tw, org)));
    assert!(!acct.credit.allow_negative);

    // The faction doc: key gone, key marked, and the ResourceChange appended
    // DIRECTLY (going through `apply_resource` would resurrect `gold` at 0).
    let f = faction(&tw, "Camarilla");
    assert!(!f.resources.contains_key("gold"));
    assert!(f.migrated.contains("gold"));
    let last = f.resource_log.last().expect("a log entry");
    assert_eq!(last.kind, "gold");
    assert_eq!(last.delta, -250);
    assert!(
        last.reason
            .contains(&format!("migrated to account #{dest}")),
        "{}",
        last.reason
    );
    assert_eq!(last.by, GOD);

    // A second run is an idempotency HIT: no second transaction, no second wallet,
    // nothing minted.
    let seen = ok(&mut tw, GOD, "@faction/migrate/confirm Camarilla=gold #0");
    assert!(seen.contains("already migrated"), "{seen}");
    assert_eq!(tx_count(&tw), 1);
    assert_eq!(bal(&tw, AccountId(0)), -250);
    assert_eq!(bal(&tw, dest), 250);
    assert_eq!(
        store(&tw).snapshot().accounts().count(),
        2,
        "no duplicate wallet"
    );
    assert_eq!(
        faction(&tw, "Camarilla").resource_log.len(),
        f.resource_log.len()
    );
}

#[test]
fn a_dropped_faction_doc_write_converges_on_a_re_run() {
    let mut tw = boot();
    setup(&mut tw);
    let mut kv = fold_kv(&mut tw);
    // The pre-migration faction doc — the version that survives if the post-migrate
    // write is the one that drops.
    let stale = kv["faction"]
        .iter()
        .find(|(k, _)| k == "fac:0")
        .expect("faction doc")
        .1
        .clone();

    ok(&mut tw, GOD, "@faction/migrate/confirm Camarilla=gold #0");
    let dest = org_account(&tw).expect("org wallet");
    fold_into(&mut tw, &mut kv);
    // Drop the faction-doc write: the money is durable, the marking is not.
    for (k, v) in kv.get_mut("faction").expect("faction kv") {
        if k == "fac:0" {
            *v = stale.clone();
        }
    }

    let mut tw2 = reboot(kv);
    // The journal is the authority: the balances survive the lost companion write.
    assert_eq!(bal(&tw2, AccountId(0)), -250);
    assert_eq!(bal(&tw2, dest), 250);
    // The faction, however, reboots believing `gold` is still a rating.
    let f = faction(&tw2, "Camarilla");
    assert_eq!(f.resources.get("gold"), Some(&250));
    assert!(f.migrated.is_empty());

    // Re-running the identical command converges: the idempotency key resolves to
    // the SAME transaction (nothing mints), and the marking finally lands.
    ok(&mut tw2, GOD, "@faction/migrate/confirm Camarilla=gold #0");
    assert_eq!(tx_count(&tw2), 1, "the HIT posted nothing new");
    assert_eq!(bal(&tw2, AccountId(0)), -250, "balances unchanged");
    assert_eq!(bal(&tw2, dest), 250);
    let f = faction(&tw2, "Camarilla");
    assert!(!f.resources.contains_key("gold"), "the key is gone");
    assert!(f.migrated.contains("gold"));
}

/// Post, lose the faction-doc write, reboot — and hand the world back to staff who
/// do not know the money already moved. Whatever they do to the rating in the
/// meantime, the re-run has to converge.
///
/// It could not before: the re-run rebuilds its plan from the CURRENT pool, so an
/// edited pool hashes differently under the same `faction.migrate/{fid}/{code}` key
/// and `post` refuses `IdempotencyConflict` — permanently, with 250 in the wallet
/// and 300 live as a rating, and the key never marked. Asking the idempotency index
/// before judging the pool is what makes the outcome reachable.
///
/// Returns `(TestWorld, dest, tx)` positioned just after the reboot.
fn posted_but_unmarked(tw: &mut TestWorld) -> (TestWorld, AccountId, mush_economy::TransactionId) {
    setup(tw);
    let mut kv = fold_kv(tw);
    let stale = kv["faction"]
        .iter()
        .find(|(k, _)| k == "fac:0")
        .expect("faction doc")
        .1
        .clone();

    ok(tw, GOD, "@faction/migrate/confirm Camarilla=gold #0");
    let dest = org_account(tw).expect("org wallet");
    let tx = store(tw)
        .transactions()
        .next()
        .expect("the migration transaction")
        .id;
    fold_into(tw, &mut kv);
    for (k, v) in kv.get_mut("faction").expect("faction kv") {
        if k == "fac:0" {
            *v = stale.clone();
        }
    }
    let tw2 = reboot(kv);
    // The money is durable; the marking is not, so `gold` reboots as a live rating.
    assert_eq!(faction(&tw2, "Camarilla").resources.get("gold"), Some(&250));
    assert!(faction(&tw2, "Camarilla").migrated.is_empty());
    (tw2, dest, tx)
}

#[test]
fn a_pool_edited_between_the_post_and_the_re_run_still_converges() {
    let mut tw = boot();
    let (mut tw2, dest, tx) = posted_but_unmarked(&mut tw);

    // Staff, not knowing the money moved, top the rating up.
    ok(
        &mut tw2,
        GOD,
        "@faction/resource Camarilla=gold/50/more turf",
    );
    assert_eq!(faction(&tw2, "Camarilla").resources.get("gold"), Some(&300));
    let before = faction(&tw2, "Camarilla").resource_log.len();

    let seen = ok(&mut tw2, GOD, "@faction/migrate/confirm Camarilla=gold #0");
    assert!(seen.contains("already migrated"), "{seen}");
    assert!(
        seen.contains(&format!("transaction #{tx}")),
        "the ORIGINAL transaction: {seen}"
    );

    // Converged: the key is retired, marked, and the residual is logged as DROPPED.
    let f = faction(&tw2, "Camarilla");
    assert!(!f.resources.contains_key("gold"), "the key is gone");
    assert!(f.migrated.contains("gold"));
    let last = f.resource_log.last().expect("a log entry");
    assert_eq!(last.kind, "gold");
    assert_eq!(
        last.delta, -300,
        "the whole residual is dropped, not just the migrated 250"
    );
    assert!(last.reason.contains("already posted"), "{}", last.reason);
    assert_eq!(last.by, GOD);
    assert_eq!(f.resource_log.len(), before + 1);

    // Not a minor unit moved: a rating edited after the post was never money.
    assert_eq!(
        tx_count(&tw2),
        1,
        "the residual must not mint a second transaction"
    );
    assert_eq!(bal(&tw2, AccountId(0)), -250, "balances unchanged");
    assert_eq!(bal(&tw2, dest), 250);
    assert_eq!(
        store(&tw2).snapshot().accounts().count(),
        2,
        "no duplicate wallet"
    );

    // And it is idempotent in its own right: a third run reports the same
    // transaction, writes no doc, and appends no log line.
    let len = f.resource_log.len();
    let seen = ok(&mut tw2, GOD, "@faction/migrate/confirm Camarilla=gold #0");
    assert!(seen.contains("Nothing moved"), "{seen}");
    assert_eq!(faction(&tw2, "Camarilla").resource_log.len(), len);
    assert_eq!(tx_count(&tw2), 1);
}

#[test]
fn a_pool_zeroed_between_the_post_and_the_re_run_still_converges() {
    let mut tw = boot();
    let (mut tw2, dest, tx) = posted_but_unmarked(&mut tw);

    // The other half of the same trap: spending the rating down to 0 used to make the
    // re-run die on "holds no pool" instead, so the key was never marked either way.
    ok(
        &mut tw2,
        GOD,
        "@faction/resource Camarilla=gold/-250/spent it",
    );
    assert_eq!(faction(&tw2, "Camarilla").resources.get("gold"), Some(&0));
    let before = faction(&tw2, "Camarilla").resource_log.len();

    let seen = ok(&mut tw2, GOD, "@faction/migrate/confirm Camarilla=gold #0");
    assert!(seen.contains("already migrated"), "{seen}");
    assert!(seen.contains(&format!("transaction #{tx}")), "{seen}");

    let f = faction(&tw2, "Camarilla");
    assert!(
        !f.resources.contains_key("gold"),
        "the zeroed key is gone, not left shadowing"
    );
    assert!(f.migrated.contains("gold"));
    // A zero residual is nothing to log — the drop entry would be noise.
    assert_eq!(f.resource_log.len(), before);
    assert_eq!(tx_count(&tw2), 1);
    assert_eq!(bal(&tw2, AccountId(0)), -250);
    assert_eq!(bal(&tw2, dest), 250);
    // The key is money now, and the rating verb says so.
    let seen = err(&mut tw2, GOD, "@faction/resource Camarilla=gold/10");
    assert!(seen.contains("@econ/pay"), "{seen}");
}

#[test]
fn a_reboot_after_migrate_reproduces_the_balances_from_the_journal() {
    let mut tw = boot();
    setup(&mut tw);
    ok(&mut tw, GOD, "@faction/migrate/confirm Camarilla=gold #0");
    let dest = org_account(&tw).expect("org wallet");
    let kv = fold_kv(&mut tw);

    let tw2 = reboot(kv);
    assert!(!store(&tw2).is_frozen(), "a clean reboot does not freeze");
    assert_eq!(bal(&tw2, AccountId(0)), -250);
    assert_eq!(bal(&tw2, dest), 250);
    // Conservation across every gold account is zero (the E2 tripwire).
    let sum: i128 = store(&tw2)
        .snapshot()
        .accounts()
        .map(|a| a.posted_balance.widen())
        .sum();
    assert_eq!(sum, 0);
    // The link, the marking, and the log tail all round-trip.
    let f = faction(&tw2, "Camarilla");
    assert_eq!(backing_org(tw2.world(), &f), Ok(OrgId(0)));
    assert!(f.migrated.contains("gold"));
    assert!(!f.resources.contains_key("gold"));
    assert_eq!(f.resources.get("influence.police"), Some(&3));
    assert_eq!(f.resource_log.last().expect("log tail").delta, -250);
}

// ----------------------------------------------------------------------
// The disclosure boundary (§4.5 / D-E3-11)
// ----------------------------------------------------------------------

#[test]
fn a_faction_officer_sees_ratings_but_not_money_and_cannot_spend() {
    let mut tw = boot();
    setup(&mut tw);
    ok(&mut tw, GOD, "@faction/migrate/confirm Camarilla=gold #0");
    let dest = org_account(&tw).expect("org wallet");

    // A faction officer with NO OrgMembership and no grant — the E1c membership
    // adapter is struck (D-E3-11), so faction rank confers nothing org-side.
    let ophelia = tw.spawn_player("Ophelia");
    ok(&mut tw, GOD, "@faction/add Camarilla=Ophelia/level:3");
    assert!(fac_state(&tw).is_officer(ophelia, faction(&tw, "Camarilla").id));
    assert!(
        tw.world()
            .module_state::<OrgStore>()
            .unwrap()
            .state()
            .membership(OrgId(0), ophelia)
            .is_none(),
        "faction rank is not an OrgMembership"
    );

    // Ratings: yes. Money: nothing at all — not a balance, not a "hidden" marker,
    // not even the fact that a wallet exists.
    let seen = ok(&mut tw, ophelia, "+faction Camarilla");
    assert!(seen.contains("influence.police = 3"), "{seen}");
    assert!(
        !seen.contains("Account #"),
        "money leaked to a faction officer: {seen}"
    );
    assert!(
        !seen.contains("2.50"),
        "balance leaked to a faction officer: {seen}"
    );

    // Staff see it in the same render.
    let seen = ok(&mut tw, GOD, "+faction Camarilla");
    assert!(seen.contains(&format!("Account #{dest}")), "{seen}");
    assert!(seen.contains("2.50"), "{seen}");

    // The documented deny: the officer cannot post from the org's wallet.
    ok(&mut tw, GOD, &format!("@account/open #{}=gold", ophelia.0));
    let mine = store(&tw)
        .snapshot()
        .accounts()
        .find(|a| a.owner == mush_org::PrincipalId::Object(ophelia))
        .expect("personal account")
        .id;
    err(&mut tw, ophelia, &format!("@econ/pay {dest}={mine} 1.00"));
    assert_eq!(bal(&tw, dest), 250, "nothing moved");
    assert_eq!(bal(&tw, mine), 0);
    assert_eq!(tx_count(&tw), 1);

    // A real grant of `economy.account.view` on the root scope — and only that —
    // opens the read. The gate is the economy's, not the faction's.
    let root = root_scope(&tw, OrgId(0));
    ok(
        &mut tw,
        GOD,
        &format!(
            "@org/grant #{}=Camarilla economy.account.view scope=#{root}",
            ophelia.0
        ),
    );
    let seen = ok(&mut tw, ophelia, "+faction Camarilla");
    assert!(seen.contains(&format!("Account #{dest}")), "{seen}");
    assert!(seen.contains("2.50"), "{seen}");
    // Viewing is not spending.
    err(&mut tw, ophelia, &format!("@econ/pay {dest}={mine} 1.00"));
    assert_eq!(bal(&tw, dest), 250);
}

#[test]
fn an_outsider_sees_neither_ratings_nor_money() {
    let mut tw = boot();
    setup(&mut tw);
    ok(&mut tw, GOD, "@faction/migrate/confirm Camarilla=gold #0");
    let xena = tw.spawn_player("Xena");
    let seen = ok(&mut tw, xena, "+faction Camarilla");
    assert!(!seen.contains("influence.police"), "{seen}");
    assert!(!seen.contains("Account #"), "{seen}");
    assert!(!seen.contains("2.50"), "{seen}");
}

#[test]
fn faction_ids_scope_the_idempotency_key_so_two_factions_both_migrate() {
    // The key is `faction.migrate/{fid}/{code}`: two factions migrating the same
    // instrument must both post, or the second would silently resolve to the
    // first's transaction and mint nothing while marking its key.
    let mut tw = boot();
    setup(&mut tw);
    ok(&mut tw, GOD, "@faction/create Sabbat");
    ok(&mut tw, GOD, "@faction/resource Sabbat=gold/40/tithes");
    ok(&mut tw, GOD, "@faction/org Sabbat=new");
    ok(&mut tw, GOD, "@faction/migrate/confirm Camarilla=gold #0");
    ok(&mut tw, GOD, "@faction/migrate/confirm Sabbat=gold #0");
    assert_eq!(tx_count(&tw), 2, "two distinct migrations");
    assert_eq!(bal(&tw, AccountId(0)), -290);
    let wallets: Vec<i64> = store(&tw)
        .snapshot()
        .accounts()
        .filter(|a| a.scope.is_some())
        .map(|a| a.posted_balance.minor())
        .collect();
    assert_eq!(wallets, vec![250, 40]);
    assert!(faction(&tw, "Camarilla").migrated.contains("gold"));
    assert!(faction(&tw, "Sabbat").migrated.contains("gold"));
    assert_eq!(faction(&tw, "Camarilla").id, FactionId(0));
}
