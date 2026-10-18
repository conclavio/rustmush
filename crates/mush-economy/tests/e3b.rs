//! E3b1 acceptance (doc 08b §4.11, scenarios 18–22): the custody/title half of the
//! live economy module.
//!
//! Everything here runs through the real `Engine` via `TestWorld`, exactly the E2b
//! rhythm — the four axes are proved *independent* (title never moves an object,
//! movement and `@chown` never move title), custody moves ride the same
//! `WriteBatch` as the journal entry that records them, every refusal path is
//! proved to mutate **nothing**, and every destruction/boot recovery path is proved
//! to fail in the safe direction (freeze, never lose).

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use mush_core::module::{HookPoint, ModError, Registrar};
use mush_core::{
    Audience, Effects, EventHook, GOD, GameModule, HookReadContext, LIMBO, ModuleBootData,
    ModuleWrite, ObjId, ObjKind, Outbox, World, WorldEvent,
};
use mush_economy::{
    AccountId, AssetLifecycle, CustodyError, CustodyProvider, CustodyRef, EconomySnapshot,
    EconomyStore, VaultId, VaultStatus,
};
use mush_proto::{OutputEvent, RichText};
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

const NS: &str = "economy";

// ---- rig ------------------------------------------------------------------

fn boot() -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(mush_economy::EconomyModule::with_custody(vec![])),
    ])
}
fn boot_with(
    providers: Vec<Arc<dyn CustodyProvider>>,
    extra: Vec<Box<dyn GameModule>>,
) -> TestWorld {
    let mut mods: Vec<Box<dyn GameModule>> = vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(mush_economy::EconomyModule::with_custody(providers)),
    ];
    mods.extend(extra);
    TestWorld::with_modules(mods)
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
fn store(tw: &TestWorld) -> &EconomyStore {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy store")
}
fn snap(tw: &TestWorld) -> &EconomySnapshot {
    store(tw).snapshot()
}
fn asset_owner(tw: &TestWorld, obj: ObjId) -> mush_org::PrincipalId {
    snap(tw).asset(obj).expect("titled").title_owner.clone()
}
fn loc(tw: &TestWorld, obj: ObjId) -> Option<ObjId> {
    tw.world().get(obj).and_then(|o| o.location)
}
fn thing(tw: &mut TestWorld, name: &str) -> ObjId {
    tw.world_mut()
        .create(ObjKind::Thing, name, GOD, Some(LIMBO))
        .expect("create")
}
fn room(tw: &mut TestWorld, name: &str) -> ObjId {
    tw.world_mut()
        .create(ObjKind::Room, name, GOD, None)
        .expect("create room")
}

fn fold_kv(tw: &mut TestWorld) -> Vec<(SmolStr, serde_json::Value)> {
    let mut kv: Vec<(SmolStr, serde_json::Value)> = Vec::new();
    for op in tw.take_module_ops() {
        if op.ns != NS {
            continue;
        }
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                kv.retain(|(k, _)| *k != key);
                kv.push((key, value));
            }
            ModuleWrite::Del { key } => kv.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    kv
}
fn merge(base: &mut Vec<(SmolStr, serde_json::Value)>, extra: Vec<(SmolStr, serde_json::Value)>) {
    for (k, v) in extra {
        base.retain(|(bk, _)| *bk != k);
        base.push((k, v));
    }
}
/// Reboot the economy **in place**, so the world (and its objects) survive — the
/// server's real shape, and the only way the liveness + custody passes have anything
/// to look at.
fn reboot(tw: &mut TestWorld, kv: Vec<(SmolStr, serde_json::Value)>) {
    tw.modules_loaded(HashMap::from([(
        SmolStr::new(NS),
        ModuleBootData {
            kv,
            streams: HashMap::new(),
            ..Default::default()
        },
    )]));
}
fn patch(kv: &mut [(SmolStr, serde_json::Value)], key: &str, f: impl Fn(&mut serde_json::Value)) {
    for (k, v) in kv.iter_mut() {
        if k == key {
            f(v);
        }
    }
}
fn journal_len(tw: &mut TestWorld) -> usize {
    store(tw).transactions().count()
}

// ---- a stub custody provider ---------------------------------------------

/// Two switches a test can flip live: which backings this provider *handles* (so it
/// wins the routing race and can then refuse the move), and which objects it
/// *claims* (so the container provider must refuse to route them at all).
#[derive(Debug, Default)]
struct StubState {
    handled: BTreeSet<ObjId>,
    claimed: BTreeSet<ObjId>,
}
#[derive(Debug, Default)]
struct Stub(Mutex<StubState>);

impl Stub {
    fn handle(&self, o: ObjId) {
        self.0.lock().expect("stub lock").handled.insert(o);
    }
    fn claim(&self, o: ObjId) {
        self.0.lock().expect("stub lock").claimed.insert(o);
    }
}
impl CustodyProvider for Stub {
    fn provider(&self) -> &str {
        "stub"
    }
    fn handles(&self, backing: &CustodyRef) -> bool {
        match backing {
            CustodyRef::Container(o) => self.0.lock().expect("stub lock").handled.contains(o),
            _ => false,
        }
    }
    fn claims(&self, _w: &World, obj: ObjId) -> bool {
        self.0.lock().expect("stub lock").claimed.contains(&obj)
    }
    fn backing_valid(&self, _w: &World, backing: &CustodyRef) -> Option<bool> {
        self.handles(backing).then_some(true)
    }
    fn contents(&self, _w: &World, backing: &CustodyRef) -> Option<Vec<ObjId>> {
        self.handles(backing).then(Vec::new)
    }
    fn holds(&self, _w: &World, backing: &CustodyRef, _obj: ObjId) -> Option<bool> {
        self.handles(backing).then_some(true)
    }
    fn move_into(
        &self,
        _c: &mut mush_core::Ctx<'_>,
        _a: ObjId,
        _o: ObjId,
        _b: &CustodyRef,
    ) -> Result<(), CustodyError> {
        Err(CustodyError::Refused("the stub provider refuses".into()))
    }
    fn move_out(
        &self,
        _c: &mut mush_core::Ctx<'_>,
        _a: ObjId,
        _o: ObjId,
        _b: &CustodyRef,
        _t: ObjId,
    ) -> Result<(), CustodyError> {
        Err(CustodyError::Refused("the stub provider refuses".into()))
    }
}

/// A module whose only job is a `CarryAttempt` veto — the hook chain a container
/// custody move MUST consult (`World::move_to` consults none).
#[derive(Debug)]
struct NoCarry;
impl EventHook for NoCarry {
    fn carry_attempt(
        &self,
        _ctx: &HookReadContext<'_>,
        _mover: ObjId,
        _obj: ObjId,
        _dest: ObjId,
    ) -> Result<(), RichText> {
        Err(RichText::tag("error", "The wards refuse that."))
    }
}
struct HookMod;
impl GameModule for HookMod {
    fn name(&self) -> &'static str {
        "carrygate"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.hook(HookPoint::CarryAttempt, Arc::new(NoCarry));
        Ok(())
    }
}

// ======================================================================
// Scenario 18 — title moves without the object; the object moves without title
// ======================================================================

#[test]
fn title_transfer_moves_no_object_and_movement_moves_no_title() {
    let mut tw = boot();
    let pat = tw.spawn_player("Pat");
    let quinn = tw.spawn_player("Quinn");
    let hall = room(&mut tw, "Hall");
    let van = thing(&mut tw, "van");

    let ev = tw.run(GOD, &format!("@asset/title #{}=#{}", van.0, pat.0));
    assert!(!has_error(&ev), "title refused: {}", texts(&ev));
    assert_eq!(asset_owner(&tw, van), mush_org::PrincipalId::Object(pat));
    let before_loc = loc(&tw, van);
    let before_owner = tw.world().get(van).expect("van").owner;

    // Title moves; the object does not.
    let ev = tw.run(GOD, &format!("@asset/transfer #{}=#{}", van.0, quinn.0));
    assert!(!has_error(&ev), "transfer refused: {}", texts(&ev));
    assert_eq!(asset_owner(&tw, van), mush_org::PrincipalId::Object(quinn));
    assert_eq!(loc(&tw, van), before_loc, "title transfer moved the object");
    assert_eq!(
        tw.world().get(van).expect("van").owner,
        before_owner,
        "@asset/transfer touched Obj.owner"
    );

    // The object moves; the title does not.
    let ev = tw.run(GOD, &format!("@teleport #{}=#{}", van.0, hall.0));
    assert!(!has_error(&ev), "teleport refused: {}", texts(&ev));
    assert_eq!(loc(&tw, van), Some(hall));
    assert_eq!(
        asset_owner(&tw, van),
        mush_org::PrincipalId::Object(quinn),
        "@teleport moved title"
    );

    // MUSH ownership moves; the title does not (and is not gated by it).
    let ev = tw.run(GOD, &format!("@chown #{}=Pat", van.0));
    assert!(!has_error(&ev), "chown refused: {}", texts(&ev));
    assert_eq!(tw.world().get(van).expect("van").owner, pat);
    assert_eq!(
        asset_owner(&tw, van),
        mush_org::PrincipalId::Object(quinn),
        "@chown moved title"
    );
}

// ======================================================================
// Scenario 19 — assignment is its own axis
// ======================================================================

#[test]
fn assign_and_unassign_leave_title_and_location_alone() {
    let mut tw = boot();
    let pat = tw.spawn_player("Pat");
    let pilot = tw.spawn_player("Robin");
    let van = thing(&mut tw, "van");
    tw.run(GOD, &format!("@asset/title #{}=#{}", van.0, pat.0));
    let before = loc(&tw, van);

    let ev = tw.run(
        GOD,
        &format!("@asset/assign #{}=#{} hauling", van.0, pilot.0),
    );
    assert!(!has_error(&ev), "assign refused: {}", texts(&ev));
    let a = snap(&tw).asset(van).expect("titled").clone();
    assert_eq!(
        a.assignment.as_ref().map(|x| x.assigned_to.clone()),
        Some(mush_org::PrincipalId::Object(pilot))
    );
    assert_eq!(
        a.assignment.as_ref().map(|x| x.purpose.as_str()),
        Some("hauling")
    );
    assert_eq!(
        a.title_owner,
        mush_org::PrincipalId::Object(pat),
        "assignment moved title"
    );
    assert_eq!(loc(&tw, van), before, "assignment moved the object");

    let ev = tw.run(GOD, &format!("@asset/unassign #{}", van.0));
    assert!(!has_error(&ev), "unassign refused: {}", texts(&ev));
    let a = snap(&tw).asset(van).expect("titled").clone();
    assert!(a.assignment.is_none(), "assignment not cleared");
    assert_eq!(
        a.title_owner,
        mush_org::PrincipalId::Object(pat),
        "revocation moved title"
    );
    assert_eq!(loc(&tw, van), before, "revocation moved the object");
}

// ======================================================================
// Scenario 20 — container custody
// ======================================================================

/// A box vault (vault #0) plus a titled widget carried by God, ready to deposit.
fn vault_fixture(tw: &mut TestWorld) -> (ObjId, ObjId) {
    let chest = thing(tw, "chest");
    let widget = thing(tw, "widget");
    let ev = tw.run(
        GOD,
        &format!("@vault/create strongbox=container:#{}", chest.0),
    );
    assert!(!has_error(&ev), "vault create refused: {}", texts(&ev));
    let ev = tw.run(GOD, &format!("@asset/title #{}=#{}", widget.0, GOD.0));
    assert!(!has_error(&ev), "title refused: {}", texts(&ev));
    (chest, widget)
}

#[test]
fn deposit_and_withdraw_are_atomic_with_the_journal() {
    let mut tw = boot();
    let (chest, widget) = vault_fixture(&mut tw);
    tw.drain_write_batch();

    let ev = tw.run(GOD, &format!("@vault/deposit #{}=0", widget.0));
    assert!(!has_error(&ev), "deposit refused: {}", texts(&ev));
    assert_eq!(
        loc(&tw, widget),
        Some(chest),
        "the widget did not enter the vault backing"
    );
    // The world move and the journal entry that records it are ONE batch.
    tw.drain_write_batch()
        .asserts_atomic_with_ledger(widget, NS, "journal");

    let ev = tw.run(GOD, &format!("@vault/withdraw #{}=0", widget.0));
    assert!(!has_error(&ev), "withdraw refused: {}", texts(&ev));
    assert_eq!(
        loc(&tw, widget),
        Some(GOD),
        "the widget did not land on the actor"
    );
    tw.drain_write_batch()
        .asserts_atomic_with_ledger(widget, NS, "journal");

    // Custody moved; title did not (D5).
    assert_eq!(asset_owner(&tw, widget), mush_org::PrincipalId::Object(GOD));
}

#[test]
fn a_provider_error_mutates_nothing() {
    let stub = Arc::new(Stub::default());
    let mut tw = boot_with(vec![stub.clone()], Vec::new());
    let (chest, widget) = vault_fixture(&mut tw);
    // Hand the backing to the stub AFTER creation, so it now wins routing and refuses.
    stub.handle(chest);
    let before_loc = loc(&tw, widget);
    let before_txs = journal_len(&mut tw);
    tw.take_module_ops();

    let ev = tw.run(GOD, &format!("@vault/deposit #{}=0", widget.0));
    assert!(
        has_error(&ev),
        "a refusing provider must refuse the deposit: {}",
        texts(&ev)
    );
    assert_eq!(
        loc(&tw, widget),
        before_loc,
        "world mutated after a provider refusal"
    );
    assert_eq!(
        journal_len(&mut tw),
        before_txs,
        "a transaction posted after a provider refusal"
    );
    assert!(
        tw.take_module_ops().is_empty(),
        "persists queued after a provider refusal"
    );
}

#[test]
fn a_policy_refusal_mutates_nothing_and_hides_the_vault() {
    let mut tw = boot();
    let (_chest, widget) = vault_fixture(&mut tw);
    let pat = tw.spawn_player("Pat");
    tw.world_mut().move_to(widget, pat).expect("hand it to Pat");
    let before_loc = loc(&tw, widget);
    let before_txs = journal_len(&mut tw);
    tw.take_module_ops();

    // Pat is not the vault owner, holds no capability, and is not staff.
    let ev = tw.run(pat, &format!("@vault/deposit #{}=0", widget.0));
    assert!(has_error(&ev), "an unauthorized deposit must refuse");
    assert!(
        texts(&ev).contains("No such vault."),
        "§4.5: a refusal on a vault the actor may not view must render as nonexistent, got: {}",
        texts(&ev)
    );
    assert_eq!(
        loc(&tw, widget),
        before_loc,
        "world mutated after a policy refusal"
    );
    assert_eq!(
        journal_len(&mut tw),
        before_txs,
        "a transaction posted after a policy refusal"
    );
    assert!(
        tw.take_module_ops().is_empty(),
        "persists queued after a policy refusal"
    );
    // ...and the vault is not discoverable through the list either.
    let ev = tw.run(pat, "@vault/list");
    assert!(
        texts(&ev).contains("No vaults."),
        "a hidden vault leaked through @vault/list: {}",
        texts(&ev)
    );
}

#[test]
fn a_carry_hook_refusal_mutates_nothing() {
    let mut tw = boot_with(Vec::new(), vec![Box::new(HookMod)]);
    let (_chest, widget) = vault_fixture(&mut tw);
    let before_loc = loc(&tw, widget);
    let before_txs = journal_len(&mut tw);
    tw.take_module_ops();

    let ev = tw.run(GOD, &format!("@vault/deposit #{}=0", widget.0));
    assert!(
        has_error(&ev),
        "the CarryAttempt chain must be consulted: {}",
        texts(&ev)
    );
    assert!(
        texts(&ev).contains("wards refuse"),
        "the hook's own reason should surface for staff: {}",
        texts(&ev)
    );
    assert_eq!(
        loc(&tw, widget),
        before_loc,
        "world mutated after a carry-hook veto"
    );
    assert_eq!(
        journal_len(&mut tw),
        before_txs,
        "a transaction posted after a carry-hook veto"
    );
    assert!(
        tw.take_module_ops().is_empty(),
        "persists queued after a carry-hook veto"
    );
}

#[test]
fn an_object_another_provider_claims_is_never_routed_to_the_container_provider() {
    let stub = Arc::new(Stub::default());
    let mut tw = boot_with(vec![stub.clone()], Vec::new());
    let (_chest, widget) = vault_fixture(&mut tw);
    // The stub handles NO backing (so the container provider resolves the vault) but
    // claims the object — a sim-placed hull is not a thing you drop in a footlocker.
    stub.claim(widget);
    let before_loc = loc(&tw, widget);
    let before_txs = journal_len(&mut tw);
    tw.take_module_ops();

    let ev = tw.run(GOD, &format!("@vault/deposit #{}=0", widget.0));
    assert!(
        has_error(&ev),
        "a claimed object must not route to the container provider: {}",
        texts(&ev)
    );
    assert_eq!(loc(&tw, widget), before_loc);
    assert_eq!(journal_len(&mut tw), before_txs);
    assert!(tw.take_module_ops().is_empty());
    // ...and it may not back a container vault either.
    let ev = tw.run(
        GOD,
        &format!("@vault/create satchel=container:#{}", widget.0),
    );
    assert!(
        has_error(&ev),
        "a claimed object must not back a container vault: {}",
        texts(&ev)
    );
}

#[test]
fn the_containment_depth_cap_refuses_a_deposit() {
    let mut tw = boot();
    // LIMBO + seven nested crates = a chain already at MAX_CONTAINMENT_DEPTH.
    let mut chain: Vec<ObjId> = Vec::new();
    for i in 0..7 {
        let c = thing(&mut tw, &format!("crate{i}"));
        if let Some(prev) = chain.last() {
            tw.world_mut().move_to(c, *prev).expect("nest");
        }
        chain.push(c);
    }
    let deep = *chain.last().expect("chain");
    let widget = thing(&mut tw, "widget");
    let ev = tw.run(GOD, &format!("@vault/create deepbox=container:#{}", deep.0));
    assert!(!has_error(&ev), "vault create refused: {}", texts(&ev));
    let before = loc(&tw, widget);

    let ev = tw.run(GOD, &format!("@vault/deposit #{}=0", widget.0));
    assert!(
        has_error(&ev),
        "a deposit past MAX_CONTAINMENT_DEPTH must refuse: {}",
        texts(&ev)
    );
    assert!(
        texts(&ev).contains("too deeply"),
        "expected the depth message, got: {}",
        texts(&ev)
    );
    assert_eq!(
        loc(&tw, widget),
        before,
        "the over-deep deposit still moved the object"
    );
}

#[test]
fn a_withdrawal_may_not_land_in_another_vaults_backing() {
    let mut tw = boot();
    let (chest, widget) = vault_fixture(&mut tw);
    let other = thing(&mut tw, "safe");
    let ev = tw.run(GOD, &format!("@vault/create safe=container:#{}", other.0));
    assert!(!has_error(&ev), "second vault refused: {}", texts(&ev));
    tw.run(GOD, &format!("@vault/deposit #{}=0", widget.0));
    assert_eq!(loc(&tw, widget), Some(chest));

    // Stand inside the other vault's backing, so `to=here` is otherwise a perfectly
    // legal destination — the ONLY thing refusing is "entering a vault is a Deposit".
    tw.world_mut()
        .move_to(GOD, other)
        .expect("step into the safe");
    let ev = tw.run(GOD, &format!("@vault/withdraw #{}=0 to=here", widget.0));
    assert!(
        has_error(&ev),
        "withdrawing into a vault backing must refuse: {}",
        texts(&ev)
    );
    assert_eq!(
        loc(&tw, widget),
        Some(chest),
        "the widget moved despite the refusal"
    );
}

// ---- E3 verify: what custody may hold, and where a withdrawal may land ----

#[test]
fn an_exit_is_topology_and_can_never_be_deposited() {
    // `World::move_to` files an `Exit` into the destination's `exits`, which NO
    // custody query walks: a deposited exit would be journaled and then be invisible
    // to `holds`/`contents`, unwithdrawable, let `@vault/close` close a vault that
    // still held it, and boot as a custody mismatch.
    let mut tw = boot();
    let (chest, _widget) = vault_fixture(&mut tw);
    let hall = room(&mut tw, "Hall");
    let north = tw
        .world_mut()
        .create(ObjKind::Exit, "north", GOD, Some(hall))
        .expect("create exit");
    // Stand in the hall, so co-location standing (staff, co-located) is satisfied and
    // the ONLY thing left to refuse is "an exit is not a thing custody can hold".
    tw.world_mut()
        .move_to(GOD, hall)
        .expect("step into the hall");
    let before_txs = journal_len(&mut tw);

    let ev = tw.run(GOD, &format!("@vault/deposit #{}=0", north.0));
    assert!(
        has_error(&ev),
        "an exit must never be depositable: {}",
        texts(&ev)
    );
    assert_eq!(loc(&tw, north), Some(hall), "the exit moved");
    assert!(
        tw.world().get(chest).expect("chest").exits.is_empty(),
        "the exit was filed into the vault backing's `exits`, where no custody query looks"
    );
    assert_eq!(
        journal_len(&mut tw),
        before_txs,
        "a custody entry was journaled for an exit"
    );
}

#[test]
fn a_vault_never_takes_an_actor_as_contents() {
    // `obj == actor` passes every standing clause trivially — co-located with itself,
    // and `perm::controls(a, a)` — so a player could file THEMSELF into a vault.
    let mut tw = boot();
    let (chest, _widget) = vault_fixture(&mut tw);
    let pat = tw.spawn_player("Pat");
    tw.world_mut()
        .move_to(pat, LIMBO)
        .expect("co-locate Pat with God");
    let before_txs = journal_len(&mut tw);

    let ev = tw.run(GOD, &format!("@vault/deposit #{}=0", GOD.0));
    assert!(
        has_error(&ev),
        "depositing yourself must refuse: {}",
        texts(&ev)
    );
    assert_ne!(loc(&tw, GOD), Some(chest), "the actor deposited themself");
    // ...and an actor is not cargo when someone else names them, either.
    let ev = tw.run(GOD, &format!("@vault/deposit #{}=0", pat.0));
    assert!(
        has_error(&ev),
        "depositing a player must refuse: {}",
        texts(&ev)
    );
    assert_eq!(
        loc(&tw, pat),
        Some(LIMBO),
        "a player was moved into a vault"
    );
    assert_eq!(
        journal_len(&mut tw),
        before_txs,
        "a custody entry was journaled for an actor"
    );
}

#[test]
fn a_withdrawal_may_not_land_inside_another_vaults_backing() {
    // The engine's own check is `is_vault_backing(to)` — direct only. Standing INSIDE
    // vault #1's backing, `to = me` is not a backing, so the object lands in vault #1
    // with no Deposit authorization, no standing check, and no custody effect.
    let mut tw = boot();
    let (chest, widget) = vault_fixture(&mut tw);
    let safe = thing(&mut tw, "safe");
    assert!(!has_error(
        &tw.run(GOD, &format!("@vault/create safe=container:#{}", safe.0))
    ));
    assert!(!has_error(
        &tw.run(GOD, &format!("@vault/deposit #{}=0", widget.0))
    ));
    assert_eq!(loc(&tw, widget), Some(chest));
    tw.world_mut()
        .move_to(GOD, safe)
        .expect("step into the safe");
    let before_txs = journal_len(&mut tw);

    let ev = tw.run(GOD, &format!("@vault/withdraw #{}=0", widget.0));
    assert!(
        has_error(&ev),
        "a withdrawal into a vault by geography must refuse: {}",
        texts(&ev)
    );
    assert_eq!(
        loc(&tw, widget),
        Some(chest),
        "the widget moved into the other vault"
    );
    assert_eq!(
        journal_len(&mut tw),
        before_txs,
        "the refused withdrawal posted a transaction"
    );
    // Standing outside it again, the same withdrawal is fine.
    tw.world_mut().move_to(GOD, LIMBO).expect("step back out");
    assert!(!has_error(
        &tw.run(GOD, &format!("@vault/withdraw #{}=0", widget.0))
    ));
    assert_eq!(loc(&tw, widget), Some(GOD));
}

#[test]
fn a_vaults_backing_may_not_be_deposited_into_another_vault() {
    // Nesting backing #0 inside vault #1 puts everything vault #0 holds under vault
    // #1's withdraw policy — its holders could extract vault #0's contents without
    // ever touching vault #0.
    let mut tw = boot();
    let (chest, widget) = vault_fixture(&mut tw);
    let strongroom = thing(&mut tw, "strongroom");
    assert!(!has_error(&tw.run(
        GOD,
        &format!("@vault/create strongroom=container:#{}", strongroom.0)
    )));
    assert!(!has_error(
        &tw.run(GOD, &format!("@vault/deposit #{}=0", widget.0))
    ));
    let before_loc = loc(&tw, chest);
    let before_txs = journal_len(&mut tw);

    let ev = tw.run(GOD, &format!("@vault/deposit #{}=1", chest.0));
    assert!(
        has_error(&ev),
        "a vault backing must not be depositable: {}",
        texts(&ev)
    );
    assert!(
        texts(&ev).contains("backs a vault"),
        "expected the backing message, got: {}",
        texts(&ev)
    );
    assert_eq!(
        loc(&tw, chest),
        before_loc,
        "the backing moved into the other vault"
    );
    assert_eq!(
        journal_len(&mut tw),
        before_txs,
        "the refused deposit posted a transaction"
    );
    assert_eq!(
        loc(&tw, widget),
        Some(chest),
        "the inner vault's contents moved"
    );
}

#[test]
fn a_deposited_container_still_exposes_its_contents() {
    let mut tw = boot();
    let (chest, _widget) = vault_fixture(&mut tw);
    let satchel = thing(&mut tw, "satchel");
    let coin = thing(&mut tw, "coin");
    tw.world_mut()
        .move_to(coin, satchel)
        .expect("pack the satchel");

    let ev = tw.run(GOD, &format!("@vault/deposit #{}=0", satchel.0));
    assert!(!has_error(&ev), "deposit refused: {}", texts(&ev));
    assert_eq!(loc(&tw, satchel), Some(chest));

    // A container deposited whole is not a laundering primitive: its contents are
    // visible and individually withdrawable.
    let ev = tw.run(GOD, "@vault/show 0");
    let shown = texts(&ev);
    assert!(
        shown.contains(&format!("holds #{}", satchel.0)),
        "satchel missing: {shown}"
    );
    assert!(
        shown.contains(&format!("holds #{}", coin.0)),
        "transitive content missing: {shown}"
    );
}

// ======================================================================
// Scenario 21 — destruction: the gate, the event backstop, the boot passes
// ======================================================================

#[test]
fn destroy_is_refused_while_titled_and_while_containing_a_titled_object() {
    let mut tw = boot();
    let pat = tw.spawn_player("Pat");
    let crate_ = thing(&mut tw, "crate");
    let widget = thing(&mut tw, "widget");
    tw.run(GOD, &format!("@asset/title #{}=#{}", widget.0, pat.0));

    let ev = tw.run(GOD, &format!("@destroy #{}", widget.0));
    assert!(
        has_error(&ev),
        "destroying a titled object must be refused: {}",
        texts(&ev)
    );
    assert!(
        tw.world()
            .get(widget)
            .is_some_and(|o| !o.flags.contains(mush_core::Flags::GOING))
    );

    // ...and so must destroying anything that transitively contains it.
    tw.world_mut()
        .move_to(widget, crate_)
        .expect("pack the crate");
    let ev = tw.run(GOD, &format!("@destroy #{}", crate_.0));
    assert!(
        has_error(&ev),
        "destroying a container of titled goods must be refused: {}",
        texts(&ev)
    );
    assert!(
        tw.world()
            .get(crate_)
            .is_some_and(|o| !o.flags.contains(mush_core::Flags::GOING))
    );

    // Non-staff get the neutral text; staff get the detail (§4.5).
    let joe = tw.spawn_player("Joe");
    assert!(
        !has_error(&tw.run(GOD, &format!("@chown #{}=Joe", crate_.0))),
        "chown the crate to Joe"
    );
    tw.world_mut()
        .move_to(crate_, joe)
        .expect("hand the crate over");
    let ev = tw.run(joe, &format!("@destroy #{}", crate_.0));
    let seen = texts(&ev);
    assert!(
        seen.contains("You cannot destroy that right now."),
        "expected the neutral refusal, got: {seen}"
    );
    assert!(
        !seen.contains("title"),
        "the neutral refusal leaked detail: {seen}"
    );
}

#[test]
fn destroying_an_owner_freezes_their_titles_via_the_event() {
    let mut tw = boot();
    let pat = tw.spawn_player("Pat");
    let widget = thing(&mut tw, "widget");
    tw.run(GOD, &format!("@asset/title #{}=#{}", widget.0, pat.0));
    tw.take_module_ops();

    tw.world_mut().destroy(pat).expect("destroy");
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    tw.engine_mut()
        .emit_world_event(&WorldEvent::ObjDestroyed { obj: pat }, &mut out, &mut fx);

    let a = snap(&tw).asset(widget).expect("titled").clone();
    assert!(a.frozen, "the title of a destroyed owner must latch frozen");
    assert_eq!(
        a.title_owner,
        mush_org::PrincipalId::Object(pat),
        "the backstop transferred title"
    );
    assert_eq!(
        a.lifecycle,
        AssetLifecycle::Active,
        "the backstop wrote the asset off"
    );
    assert!(
        tw.take_module_ops().iter().any(|o| matches!(&o.op, ModuleWrite::JsonSet { key, .. } if key == &format!("asset:{}", widget.0))),
        "the frozen asset was not re-persisted"
    );
}

#[test]
fn a_destruction_that_emits_no_event_is_caught_at_boot() {
    let mut tw = boot();
    let pat = tw.spawn_player("Pat");
    let widget = thing(&mut tw, "widget");
    tw.run(GOD, &format!("@asset/title #{}=#{}", widget.0, pat.0));
    let kv = fold_kv(&mut tw);
    assert!(!snap(&tw).asset(widget).expect("titled").frozen);

    // Programmatic destruction with NO ObjDestroyed emitted — the backstop the event
    // observer cannot see.
    tw.world_mut().destroy(widget).expect("destroy");
    reboot(&mut tw, kv);

    let a = snap(&tw).asset(widget).expect("titled").clone();
    assert!(
        a.frozen,
        "boot liveness must freeze a title whose object is gone"
    );
    assert_eq!(a.frozen_reason.as_deref(), Some("principal-gone"));
    assert!(
        !store(&tw).is_frozen(),
        "one dead object must not freeze the whole economy"
    );
}

#[test]
fn destroying_a_vault_backing_freezes_the_vault_and_leaves_titles_intact() {
    let mut tw = boot();
    let (chest, widget) = vault_fixture(&mut tw);
    tw.run(GOD, &format!("@vault/deposit #{}=0", widget.0));
    tw.take_module_ops();

    tw.world_mut().destroy(chest).expect("destroy the chest");
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    tw.engine_mut()
        .emit_world_event(&WorldEvent::ObjDestroyed { obj: chest }, &mut out, &mut fx);

    assert_eq!(
        snap(&tw).vault(VaultId(0)).expect("vault").status,
        VaultStatus::Frozen
    );
    let a = snap(&tw).asset(widget).expect("titled");
    assert_eq!(
        a.title_owner,
        mush_org::PrincipalId::Object(GOD),
        "destroying a backing moved title"
    );
    assert_eq!(
        a.lifecycle,
        AssetLifecycle::Active,
        "destroying a backing wrote a title off"
    );
    assert!(
        tw.take_module_ops()
            .iter()
            .any(|o| matches!(&o.op, ModuleWrite::JsonSet { key, .. } if key == "vault:0")),
        "the frozen vault was not re-persisted"
    );
}

#[test]
fn an_event_freeze_survives_reboot_without_freezing_the_economy() {
    // The C2 regression: the `frozen` latch is doc authority and is NEVER compared at
    // boot, so a doc that carries it must not read as drift against a journal that
    // (correctly) never carries it.
    let mut tw = boot();
    let pat = tw.spawn_player("Pat");
    let robin = tw.spawn_player("Robin");
    let van = thing(&mut tw, "van");
    tw.run(GOD, &format!("@asset/title #{}=#{}", van.0, pat.0));
    tw.run(
        GOD,
        &format!("@asset/assign #{}=#{} hauling", van.0, robin.0),
    );
    let mut kv = fold_kv(&mut tw);

    tw.world_mut().destroy(robin).expect("destroy the assignee");
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    tw.engine_mut()
        .emit_world_event(&WorldEvent::ObjDestroyed { obj: robin }, &mut out, &mut fx);
    assert!(
        snap(&tw).asset(van).expect("titled").frozen,
        "the event did not latch the asset"
    );
    merge(&mut kv, fold_kv(&mut tw));

    reboot(&mut tw, kv);
    assert!(
        !store(&tw).is_frozen(),
        "a frozen ASSET must never freeze the whole economy (C2)"
    );
    assert!(
        snap(&tw).asset(van).expect("titled").frozen,
        "the latch was silently healed away"
    );
    assert_eq!(
        asset_owner(&tw, van),
        mush_org::PrincipalId::Object(pat),
        "replay lost the title"
    );
}

#[test]
fn teleporting_out_of_a_vault_is_caught_as_a_custody_mismatch() {
    let mut tw = boot();
    let (chest, widget) = vault_fixture(&mut tw);
    let ev = tw.run(GOD, &format!("@vault/deposit #{}=0", widget.0));
    assert!(!has_error(&ev), "deposit refused: {}", texts(&ev));
    assert_eq!(loc(&tw, widget), Some(chest));
    let kv = fold_kv(&mut tw);

    // Something moved it out without a Withdraw: the journal still says "deposited".
    tw.world_mut()
        .move_to(widget, LIMBO)
        .expect("teleport it out");
    reboot(&mut tw, kv);

    let a = snap(&tw).asset(widget).expect("titled").clone();
    assert!(
        a.frozen,
        "boot custody reconciliation must freeze a mismatched title"
    );
    assert_eq!(a.frozen_reason.as_deref(), Some("custody-mismatch"));
    assert!(
        !store(&tw).is_frozen(),
        "a custody mismatch is not a whole-economy freeze"
    );
}

// ======================================================================
// Scenario 22 — persistence, reboot identity, and the projection quartet
// ======================================================================

#[test]
fn every_step_of_the_title_custody_sequence_survives_a_reboot_identically() {
    let mut tw = boot();
    let pat = tw.spawn_player("Pat");
    let (_chest, widget) = vault_fixture(&mut tw);
    let mut kv = fold_kv(&mut tw);

    let steps = [
        format!("@asset/assign #{}=#{} survey", widget.0, pat.0),
        format!("@vault/deposit #{}=0", widget.0),
        format!("@vault/withdraw #{}=0", widget.0),
        format!("@asset/transfer #{}=#{}", widget.0, pat.0),
    ];
    for step in steps {
        let ev = tw.run(GOD, &step);
        assert!(!has_error(&ev), "{step} refused: {}", texts(&ev));
        merge(&mut kv, fold_kv(&mut tw));
        let before = snap(&tw).clone();
        reboot(&mut tw, kv.clone());
        assert_eq!(
            snap(&tw),
            &before,
            "reboot after {step:?} did not reproduce the snapshot"
        );
        assert!(
            !store(&tw).is_frozen(),
            "reboot after {step:?} froze the economy"
        );
        merge(&mut kv, fold_kv(&mut tw));
    }
    assert_eq!(asset_owner(&tw, widget), mush_org::PrincipalId::Object(pat));
}

#[test]
fn boot_heals_a_lagging_asset_projection_without_freezing() {
    let mut tw = boot();
    let pat = tw.spawn_player("Pat");
    let widget = thing(&mut tw, "widget");
    tw.run(GOD, &format!("@asset/title #{}=#{}", widget.0, pat.0));
    let mut kv = fold_kv(&mut tw); // asset doc at the Created effect (seq 0)
    let lagging = kv
        .iter()
        .find(|(k, _)| k == &format!("asset:{}", widget.0))
        .map(|(_, v)| v.clone())
        .expect("asset doc");
    tw.run(
        GOD,
        &format!("@asset/assign #{}=#{} survey", widget.0, GOD.0),
    );
    merge(&mut kv, fold_kv(&mut tw));
    // A dropped companion write: the doc still shows the pre-assignment state.
    let key = format!("asset:{}", widget.0);
    patch(&mut kv, &key, |v| *v = lagging.clone());

    reboot(&mut tw, kv);
    assert!(
        !store(&tw).is_frozen(),
        "a benign asset lag must NOT freeze"
    );
    let a = snap(&tw).asset(widget).expect("titled");
    assert!(
        a.assignment.is_some(),
        "the lagging projection was not re-derived to the journal head"
    );
}

#[test]
fn boot_freezes_on_a_drifted_asset_projection() {
    let mut tw = boot();
    let pat = tw.spawn_player("Pat");
    let widget = thing(&mut tw, "widget");
    tw.run(GOD, &format!("@asset/title #{}=#{}", widget.0, pat.0));
    let mut kv = fold_kv(&mut tw);
    // DRIFT: the doc claims the journal's own sequence but names a different owner.
    let key = format!("asset:{}", widget.0);
    patch(&mut kv, &key, |v| {
        v["title_owner"] = serde_json::json!({ "Object": GOD.0 })
    });

    reboot(&mut tw, kv);
    assert!(
        store(&tw).is_frozen(),
        "real title drift must freeze the economy"
    );
}

#[test]
fn boot_freezes_on_an_asset_projection_ahead_of_the_journal() {
    let mut tw = boot();
    let pat = tw.spawn_player("Pat");
    let widget = thing(&mut tw, "widget");
    tw.run(GOD, &format!("@asset/title #{}=#{}", widget.0, pat.0));
    let mut kv = fold_kv(&mut tw);
    // AHEAD: the projection remembers an effect the journal no longer has.
    let key = format!("asset:{}", widget.0);
    patch(&mut kv, &key, |v| {
        v["last_journal_seq"] = serde_json::json!(99)
    });

    reboot(&mut tw, kv);
    assert!(
        store(&tw).is_frozen(),
        "an asset projection ahead of the journal (lost tail) must freeze"
    );
}

#[test]
fn boot_rebuilds_a_missing_asset_doc_frozen() {
    let mut tw = boot();
    let pat = tw.spawn_player("Pat");
    let widget = thing(&mut tw, "widget");
    tw.run(GOD, &format!("@asset/title #{}=#{}", widget.0, pat.0));
    let mut kv = fold_kv(&mut tw);
    let key = format!("asset:{}", widget.0);
    kv.retain(|(k, _)| k != &key);

    reboot(&mut tw, kv);
    assert!(
        !store(&tw).is_frozen(),
        "a lost projection doc is not a whole-economy freeze"
    );
    let a = snap(&tw)
        .asset(widget)
        .expect("rebuilt from the journal")
        .clone();
    assert_eq!(
        a.title_owner,
        mush_org::PrincipalId::Object(pat),
        "the journal did not rebuild the title"
    );
    assert!(
        a.frozen,
        "a lost doc is a lost latch: the rebuild must be frozen"
    );
    assert_eq!(a.frozen_reason.as_deref(), Some("doc-lost"));
}

#[test]
fn an_asset_doc_with_no_journal_history_is_not_loaded() {
    // C6: an unjournaled doc is not a title — loading one would let a hand-written
    // document mint ownership out of nothing.
    let mut tw = boot();
    let pat = tw.spawn_player("Pat");
    let widget = thing(&mut tw, "widget");
    let forged = serde_json::json!({
        "object": widget.0, "title_owner": { "Object": pat.0 }, "scope": null,
        "assignment": null, "maintenance_account": null, "catalog": null,
        "encumbrances": [], "acquisition": null, "lifecycle": "Active",
        "last_journal_seq": 0, "revision": 1
    });
    let kv = vec![(SmolStr::new(format!("asset:{}", widget.0)), forged)];

    reboot(&mut tw, kv);
    assert!(
        snap(&tw).asset(widget).is_none(),
        "an unjournaled asset doc was loaded as a title"
    );
    assert!(
        !store(&tw).is_frozen(),
        "an unjournaled doc must not brick the economy"
    );
}

#[test]
fn reversing_an_effect_carrying_transaction_is_refused_at_the_command() {
    let mut tw = boot();
    let pat = tw.spawn_player("Pat");
    let widget = thing(&mut tw, "widget");
    let ev = tw.run(GOD, &format!("@asset/title #{}=#{}", widget.0, pat.0));
    assert!(!has_error(&ev), "title refused: {}", texts(&ev));

    let ev = tw.run(GOD, "@econ/reverse 0");
    assert!(
        has_error(&ev),
        "an effect-carrying transaction must not be money-reversed"
    );
    assert_eq!(
        asset_owner(&tw, widget),
        mush_org::PrincipalId::Object(pat),
        "the refused reversal moved title"
    );
}

#[test]
fn a_frozen_economy_refuses_asset_and_custody_ops() {
    let mut tw = boot();
    let pat = tw.spawn_player("Pat");
    let widget = thing(&mut tw, "widget");
    let chest = thing(&mut tw, "chest");
    assert!(!has_error(&tw.run(GOD, "@instrument/register gold=0")));
    assert!(!has_error(&tw.run(GOD, "@account/system faucet=gold")));
    assert!(!has_error(
        &tw.run(GOD, &format!("@account/open #{}=gold", pat.0))
    ));
    assert!(!has_error(&tw.run(GOD, "@econ/mint 0=1 100")));
    // The vault exists BEFORE the freeze — a frozen economy binds no new ones.
    assert!(!has_error(&tw.run(
        GOD,
        &format!("@vault/create strongbox=container:#{}", chest.0)
    )));
    let mut kv = fold_kv(&mut tw);
    // Break conservation so boot latches the whole-economy freeze.
    patch(&mut kv, "account:1", |v| {
        v["posted_balance"] = serde_json::json!(999)
    });
    reboot(&mut tw, kv);
    assert!(store(&tw).is_frozen());

    let ev = tw.run(GOD, &format!("@asset/title #{}=#{}", widget.0, pat.0));
    assert!(has_error(&ev), "a frozen economy must refuse a title op");
    assert!(
        snap(&tw).asset(widget).is_none(),
        "a title posted into a frozen economy"
    );
    // E3-verify L: the non-recovery control writes are refused too — a freeze that
    // still lets staff bind a fresh vault (or open a fresh wallet) is a freeze with
    // a door in it.
    let ev = tw.run(GOD, &format!("@vault/create box=container:#{}", widget.0));
    assert!(
        has_error(&ev),
        "a frozen economy must refuse a new vault binding: {}",
        texts(&ev)
    );
    assert_eq!(
        snap(&tw).vaults().count(),
        1,
        "a vault was bound into a frozen economy"
    );
    let ev = tw.run(GOD, &format!("@vault/deposit #{}=0", widget.0));
    assert!(has_error(&ev), "a frozen economy must refuse a custody op");
    // ...while the recovery verbs stay available.
    assert!(
        !has_error(&tw.run(GOD, "@vault/freeze 0")),
        "vault freeze is a recovery verb"
    );
}

#[test]
fn interior_vaults_are_refused_until_the_provider_is_wired() {
    let mut tw = boot();
    let hangar = thing(&mut tw, "hangar");
    let ev = tw.run(GOD, &format!("@vault/create bay=interior:#{}", hangar.0));
    assert!(has_error(&ev), "interior backings have no provider in E3b1");
    assert!(
        texts(&ev).contains("interior custody provider"),
        "expected the E3c pointer, got: {}",
        texts(&ev)
    );
    assert_eq!(snap(&tw).vaults().count(), 0);
}

#[test]
fn the_vault_allocator_never_re_mints_a_live_id() {
    let mut tw = boot();
    let (_chest, widget) = vault_fixture(&mut tw);
    tw.run(GOD, &format!("@vault/deposit #{}=0", widget.0));
    let mut kv = fold_kv(&mut tw);
    // A stale meta that would hand out vault #0 again.
    patch(&mut kv, "meta", |v| {
        v["next_vault_id"] = serde_json::json!(0)
    });
    reboot(&mut tw, kv);

    let chest2 = thing(&mut tw, "chest2");
    let ev = tw.run(
        GOD,
        &format!("@vault/create second=container:#{}", chest2.0),
    );
    assert!(!has_error(&ev), "second vault refused: {}", texts(&ev));
    assert!(
        snap(&tw).vault(VaultId(1)).is_some(),
        "the allocator floor did not hold: {}",
        texts(&ev)
    );
    assert_eq!(
        snap(&tw)
            .vault(VaultId(0))
            .expect("first vault")
            .name
            .as_str(),
        "strongbox"
    );
}

#[test]
fn a_vault_whose_backing_vanished_boots_frozen() {
    let mut tw = boot();
    let (chest, _widget) = vault_fixture(&mut tw);
    let kv = fold_kv(&mut tw);
    tw.world_mut().destroy(chest).expect("destroy the backing");

    reboot(&mut tw, kv);
    assert_eq!(
        snap(&tw).vault(VaultId(0)).expect("vault").status,
        VaultStatus::Frozen,
        "a vault whose backing is gone must boot frozen, never deleted"
    );
    assert!(!store(&tw).is_frozen());
}

#[test]
fn pay_with_an_asset_leg_refuses_a_two_party_exchange() {
    let mut tw = boot();
    let pat = tw.spawn_player("Pat");
    let quinn = tw.spawn_player("Quinn");
    let widget = thing(&mut tw, "widget");
    assert!(!has_error(&tw.run(GOD, "@instrument/register gold=0")));
    assert!(!has_error(&tw.run(GOD, "@account/system faucet=gold")));
    assert!(!has_error(
        &tw.run(GOD, &format!("@account/open #{}=gold", pat.0))
    ));
    assert!(!has_error(
        &tw.run(GOD, &format!("@account/open #{}=gold", quinn.0))
    ));
    assert!(!has_error(&tw.run(GOD, "@econ/mint 0=1 500")));
    // Quinn holds title; Pat has the money. That is a sale, and E3 plans carry ONE
    // authority — counterparty consent is E4.
    assert!(!has_error(
        &tw.run(GOD, &format!("@asset/title #{}=#{}", widget.0, quinn.0))
    ));

    let ev = tw.run(pat, &format!("@econ/pay 1=2 100 asset=#{}", widget.0));
    assert!(has_error(&ev), "a two-party exchange must refuse");
    assert_eq!(
        snap(&tw)
            .account(AccountId(1))
            .expect("account")
            .posted_balance
            .minor(),
        500,
        "the refused sale moved money"
    );
    assert_eq!(
        asset_owner(&tw, widget),
        mush_org::PrincipalId::Object(quinn),
        "the refused sale moved title"
    );

    // Staff authorize both sides, and money + title ride ONE transaction.
    let ev = tw.run(GOD, &format!("@econ/pay 1=2 100 asset=#{}", widget.0));
    assert!(
        !has_error(&ev),
        "staff pay+transfer refused: {}",
        texts(&ev)
    );
    assert_eq!(
        snap(&tw)
            .account(AccountId(1))
            .expect("account")
            .posted_balance
            .minor(),
        400
    );
    assert_eq!(
        asset_owner(&tw, widget),
        mush_org::PrincipalId::Object(quinn)
    );
    let a = snap(&tw).asset(widget).expect("titled");
    assert!(
        a.acquisition.is_some(),
        "invariant 22: money moved, so acquisition is stamped"
    );
}

#[test]
fn asset_reads_are_hidden_and_field_limited_for_non_staff() {
    let mut tw = boot();
    let pat = tw.spawn_player("Pat");
    let joe = tw.spawn_player("Joe");
    let van = thing(&mut tw, "van");
    assert!(!has_error(
        &tw.run(GOD, &format!("@asset/title #{}=#{}", van.0, pat.0))
    ));

    // The title owner reads the §4.5 field set — and nothing beyond it.
    let ev = tw.run(pat, &format!("@asset/show #{}", van.0));
    let seen = texts(&ev);
    assert!(
        seen.contains("title"),
        "the owner should see the title line: {seen}"
    );
    assert!(
        seen.contains("encumbrances: 0"),
        "the owner should see the encumbrance COUNT: {seen}"
    );
    assert!(
        !seen.contains("last journal seq"),
        "a non-staff read leaked journal internals: {seen}"
    );

    // A stranger cannot tell "denied" from "there is no such thing" (§4.5).
    let ev = tw.run(joe, &format!("@asset/show #{}", van.0));
    assert!(
        texts(&ev).contains("No such asset."),
        "existence leaked to a stranger: {}",
        texts(&ev)
    );
    let ev = tw.run(joe, "@asset/list");
    assert!(
        texts(&ev).contains("No assets."),
        "a hidden asset leaked through @asset/list: {}",
        texts(&ev)
    );
    // A list filter naming a principal that does not exist is an EMPTY list, not an error.
    let ev = tw.run(pat, "@asset/list owner=org:nonesuch");
    assert!(
        texts(&ev).contains("No assets."),
        "an unresolvable filter should list nothing: {}",
        texts(&ev)
    );

    // D-E3-12: receiving title is not free — Pat may not push it onto Joe.
    let ev = tw.run(pat, &format!("@asset/transfer #{}=#{}", van.0, joe.0));
    assert!(has_error(&ev), "an unsolicited title push must be refused");
    assert_eq!(asset_owner(&tw, van), mush_org::PrincipalId::Object(pat));
}

#[test]
fn a_lien_is_staff_only_and_blocks_a_transfer_until_released() {
    let mut tw = boot();
    let pat = tw.spawn_player("Pat");
    let widget = thing(&mut tw, "widget");
    assert!(!has_error(&tw.run(GOD, "@instrument/register gold=0")));
    assert!(!has_error(
        &tw.run(GOD, &format!("@asset/title #{}=#{}", widget.0, GOD.0))
    ));

    // Invariant 19: an unpriced lien is a hostage primitive, so creating one is staff.
    let ev = tw.run(
        pat,
        &format!("@asset/lien #{}=#{} 50 gold", widget.0, pat.0),
    );
    assert!(has_error(&ev), "a non-staff lien must refuse");
    assert!(
        snap(&tw)
            .asset(widget)
            .expect("titled")
            .encumbrances
            .is_empty()
    );

    let ev = tw.run(
        GOD,
        &format!("@asset/lien #{}=#{} 50 gold", widget.0, pat.0),
    );
    assert!(!has_error(&ev), "staff lien refused: {}", texts(&ev));
    assert_eq!(
        snap(&tw).asset(widget).expect("titled").encumbrances.len(),
        1
    );

    // Invariant 12: a claim may not ride along on a title change.
    let ev = tw.run(GOD, &format!("@asset/transfer #{}=#{}", widget.0, pat.0));
    assert!(
        has_error(&ev),
        "transferring an encumbered asset must refuse: {}",
        texts(&ev)
    );
    let ev = tw.run(GOD, &format!("@asset/writeoff #{}=scrapped", widget.0));
    assert!(
        has_error(&ev),
        "writing off an encumbered asset must refuse: {}",
        texts(&ev)
    );

    // The creditor may release their own claim (invariant 19), and then title moves.
    let ev = tw.run(pat, &format!("@asset/release #{}=1", widget.0));
    assert!(
        !has_error(&ev),
        "the creditor's own release refused: {}",
        texts(&ev)
    );
    assert!(
        snap(&tw)
            .asset(widget)
            .expect("titled")
            .encumbrances
            .is_empty()
    );
    let ev = tw.run(GOD, &format!("@asset/transfer #{}=#{}", widget.0, pat.0));
    assert!(
        !has_error(&ev),
        "transfer after release refused: {}",
        texts(&ev)
    );
    assert_eq!(asset_owner(&tw, widget), mush_org::PrincipalId::Object(pat));
}

#[test]
fn maintenance_points_at_a_related_account_and_clears() {
    let mut tw = boot();
    let widget = thing(&mut tw, "widget");
    assert!(!has_error(&tw.run(GOD, "@instrument/register gold=0")));
    assert!(!has_error(
        &tw.run(GOD, &format!("@account/open #{}=gold", GOD.0))
    ));
    assert!(!has_error(
        &tw.run(GOD, &format!("@asset/title #{}=#{}", widget.0, GOD.0))
    ));

    let ev = tw.run(GOD, &format!("@asset/maintenance #{}=0", widget.0));
    assert!(!has_error(&ev), "maintenance refused: {}", texts(&ev));
    assert_eq!(
        snap(&tw).asset(widget).expect("titled").maintenance_account,
        Some(AccountId(0))
    );

    let ev = tw.run(GOD, &format!("@asset/maintenance #{}=none", widget.0));
    assert!(
        !has_error(&ev),
        "clearing maintenance refused: {}",
        texts(&ev)
    );
    assert!(
        snap(&tw)
            .asset(widget)
            .expect("titled")
            .maintenance_account
            .is_none()
    );

    // Transfer clears the payer relationship with the old owner (invariant 13).
    let pat = tw.spawn_player("Pat");
    assert!(!has_error(
        &tw.run(GOD, &format!("@asset/maintenance #{}=0", widget.0))
    ));
    assert!(!has_error(
        &tw.run(GOD, &format!("@asset/transfer #{}=#{}", widget.0, pat.0))
    ));
    let a = snap(&tw).asset(widget).expect("titled");
    assert!(
        a.maintenance_account.is_none(),
        "transfer kept the old owner's maintenance payer"
    );
    assert!(a.assignment.is_none());
}

// ======================================================================
// Doc 18 cut IC13 — the `rehome_vault` seam
// ======================================================================

/// A test-only module with one command, `@testrehome <vault>=<#obj|system>`: it
/// calls [`mush_economy::rehome_vault`] (the test kit gives tests no `Ctx`, so the
/// seam is reached through a command, e2b's `TestPostMod` pattern) and prints
/// `true` or `false`.
struct RehomeMod;

impl GameModule for RehomeMod {
    fn name(&self) -> &'static str {
        "e3btestrehome"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::EconomyCarveOut);
        reg.command(
            mush_core::CommandSpec {
                name: "@testrehome".into(),
                shape: mush_core::ArgShape::Raw,
                help: "@testrehome <vault>=<#obj|system>   test: re-own a vault".into(),
                ..mush_core::CommandSpec::default()
            },
            Arc::new(
                |ctx: &mut mush_core::Ctx<'_>| -> Result<(), mush_core::CmdError> {
                    let raw = ctx.args.raw().to_string();
                    let (vault, owner) = raw
                        .split_once('=')
                        .ok_or_else(|| mush_core::CmdError::msg("Usage: @testrehome <v>=<o>"))?;
                    let vault: u64 = vault
                        .trim()
                        .parse()
                        .map_err(|_| mush_core::CmdError::msg("bad vault"))?;
                    let owner = match owner.trim() {
                        "system" => mush_org::PrincipalId::System(
                            mush_org::SystemPrincipalId::new("economy.escrow")
                                .ok_or_else(|| mush_core::CmdError::msg("bad system id"))?,
                        ),
                        other => mush_org::PrincipalId::Object(ObjId(
                            other
                                .trim_start_matches('#')
                                .parse()
                                .map_err(|_| mush_core::CmdError::msg("bad owner"))?,
                        )),
                    };
                    let done = mush_economy::rehome_vault(
                        ctx,
                        VaultId(vault),
                        owner,
                        None,
                        SmolStr::new_static("rehomed"),
                    );
                    let actor = ctx.actor;
                    ctx.out.sys_to_actor(actor, "test", done.to_string());
                    Ok(())
                },
            ),
        )?;
        Ok(())
    }
}

/// Doc 18 cut IC13 item 1: a re-home moves the owner index and nothing else, bumps
/// the revision by exactly one, persists the vault document (and only that), and
/// survives a reboot; a `System` owner and an unknown vault write nothing.
#[test]
fn ic13_rehome_vault_moves_the_owner_and_keeps_the_backing() {
    let mut tw = boot_with(vec![], vec![Box::new(RehomeMod)]);
    let (chest, _widget) = vault_fixture(&mut tw);
    let mut kv = fold_kv(&mut tw);
    let pat = tw.spawn_player("Pat");
    let before = snap(&tw).vault(VaultId(0)).expect("the vault").clone();
    let old = before.owner.clone();
    let new = mush_org::PrincipalId::Object(pat);
    assert_ne!(old, new);

    let ev = tw.run(GOD, &format!("@testrehome 0=#{}", pat.0));
    assert_eq!(texts(&ev), "true");
    let ops = fold_kv(&mut tw);
    assert_eq!(
        ops.iter().map(|(k, _)| k.to_string()).collect::<Vec<_>>(),
        vec![mush_economy::vault_key(VaultId(0)).to_string()],
        "only the vault document is written (no allocator move)"
    );
    merge(&mut kv, ops);

    // 1. The owner index moved.
    assert!(snap(&tw).vaults_of(&old).all(|v| v.id != VaultId(0)));
    assert!(snap(&tw).vaults_of(&new).any(|v| v.id == VaultId(0)));
    // 2. The backing index still names it; status and backing are unchanged.
    let after = snap(&tw).vault(VaultId(0)).expect("the vault").clone();
    assert_eq!(
        snap(&tw).vault_by_backing(&before.backing).map(|v| v.id),
        Some(VaultId(0))
    );
    assert_eq!(after.backing, before.backing);
    assert_eq!(after.backing, CustodyRef::Container(chest));
    assert_eq!(after.status, before.status);
    assert_eq!(after.owner, new);
    assert_eq!(after.name.as_str(), "rehomed");
    // 3. The revision went up by exactly one.
    assert_eq!(after.revision, before.revision + 1);

    // 4. A System owner and an unknown vault print false and write nothing.
    let ev = tw.run(GOD, "@testrehome 0=system");
    assert_eq!(texts(&ev), "false");
    assert!(fold_kv(&mut tw).is_empty(), "a System re-home wrote");
    let ev = tw.run(GOD, &format!("@testrehome 99=#{}", pat.0));
    assert_eq!(texts(&ev), "false");
    assert!(
        fold_kv(&mut tw).is_empty(),
        "an unknown vault's re-home wrote"
    );
    assert_eq!(snap(&tw).vault(VaultId(0)), Some(&after));

    // 5. A reboot reads the new owner back.
    reboot(&mut tw, kv);
    let back = snap(&tw).vault(VaultId(0)).expect("the vault after reboot");
    assert_eq!(back.owner, new);
    assert_eq!(back.revision, after.revision);
    assert!(snap(&tw).vaults_of(&new).any(|v| v.id == VaultId(0)));
    assert!(snap(&tw).vaults_of(&old).all(|v| v.id != VaultId(0)));
}
