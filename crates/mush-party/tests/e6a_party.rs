//! E6a acceptance (08f §3.6, the **Party:** line): create/invite/accept/decline/
//! leave/kick/disband; the size and invite caps; idle expiry; `Locked` refuses every
//! membership change; a substitution needs the amendment and returns `Forming`;
//! reboot at every status.
//!
//! The rhythms here are the ones the axis is verified on:
//!
//! - **nothing is asserted from the message alone.** Every refusal is checked against
//!   the record — roster, status, revision — not just against the line.
//! - **existence hiding is a text equality.** A party the actor may not view answers
//!   exactly `"No such party."`, the same as one that does not exist.
//! - **fold and reboot.** Every status round-trips through the real persisted
//!   documents, and a document that breaks an invariant is dropped without taking the
//!   boot with it.
//! - **the seams are the ruleset's.** `lock`/`release`/`unlock` are called the way
//!   `ruleset-shadowrun` will call them — straight, with a `Ctx`, no command in
//!   between — and their persistence ops are folded in with the commands'.
//!
//! Everything runs through the real `Engine` via `TestWorld`, like `e4b1`/`e4b2`.

use std::collections::HashMap;

use mush_core::module::ModuleBootData;
use mush_core::{
    Audience, Ctx, Deferred, Effects, GOD, ModuleOp, ModuleWrite, ObjId, Outbox, ParsedArgs,
    SwitchSet, WorldEvent,
};
use mush_party::{
    ContractRef, DisbandCause, MAX_PARTIES_PER_LEADER, MAX_PARTY_INVITES, MAX_PARTY_SIZE,
    NO_SUCH_PARTY, NS, PARTY_IDLE_TTL, Party, PartyError, PartyId, PartyModule, PartyStatus,
    PartyStore,
};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

// ---- rig ------------------------------------------------------------------

fn boot() -> TestWorld {
    TestWorld::with_modules(vec![Box::new(PartyModule::new())])
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

/// A refusal that must be indistinguishable from "there is no such party".
fn hidden(tw: &mut TestWorld, actor: ObjId, line: &str) {
    let t = err(tw, actor, line);
    assert!(
        t.contains(NO_SUCH_PARTY),
        "`{line}` must hide behind {NO_SUCH_PARTY:?}, got: {t}"
    );
}

fn store(tw: &TestWorld) -> &PartyStore {
    tw.world()
        .module_state::<PartyStore>()
        .expect("party store slot")
}

fn party(tw: &TestWorld, id: PartyId) -> Party {
    store(tw).party(id).expect("party exists").clone()
}

fn status(tw: &TestWorld, id: PartyId) -> PartyStatus {
    party(tw, id).status
}

fn members(tw: &TestWorld, id: PartyId) -> Vec<ObjId> {
    party(tw, id).members
}

/// The persisted namespace, folded exactly as the world task folds a `WriteBatch`.
#[derive(Default)]
struct Persisted(Vec<(SmolStr, serde_json::Value)>);

impl Persisted {
    fn apply(&mut self, ops: Vec<ModuleOp>) {
        for op in ops {
            if op.ns != NS {
                continue;
            }
            match op.op {
                ModuleWrite::JsonSet { key, value } => {
                    self.0.retain(|(k, _)| *k != key);
                    self.0.push((key, value));
                }
                ModuleWrite::Del { key } => self.0.retain(|(k, _)| *k != key),
                ModuleWrite::StreamAppend { .. } => {}
            }
        }
    }

    /// Fold everything the engine has queued since the last call.
    fn sync(&mut self, tw: &mut TestWorld) {
        let ops = tw.take_module_ops();
        self.apply(ops);
    }

    fn kv(&self) -> Vec<(SmolStr, serde_json::Value)> {
        self.0.clone()
    }
}

/// Run one closure against a real `Ctx`, the way the ruleset calls the seams, and
/// hand back what it deferred so the caller can fold it.
fn with_ctx<T>(tw: &mut TestWorld, f: impl FnOnce(&mut Ctx<'_>) -> T) -> (T, Vec<ModuleOp>) {
    let engine = tw.engine_mut();
    let mut out = Outbox::new();
    let mut ctx = Ctx {
        w: &mut engine.world,
        actor: GOD,
        switches: SwitchSet::default(),
        args: ParsedArgs::default(),
        out: &mut out,
        reg: &engine.registry,
        module: None,
        typed_state: mush_core::ModuleActionState::unavailable(),
    };
    let r = f(&mut ctx);
    drop(ctx);
    let ops = out
        .take_deferred()
        .into_iter()
        .filter_map(|d| match d {
            Deferred::Persist(op) => Some(op),
            _ => None,
        })
        .collect();
    (r, ops)
}

fn lock_at(tw: &mut TestWorld, st: &mut Persisted, id: PartyId, by: u64) {
    st.sync(tw);
    let (r, ops) = with_ctx(tw, |ctx| mush_party::lock(ctx, id, ContractRef(by)));
    st.apply(ops);
    assert_eq!(r, Ok(()), "lock({id}) must be accepted");
}

fn release_at(tw: &mut TestWorld, st: &mut Persisted, id: PartyId, how: DisbandCause) {
    st.sync(tw);
    let (r, ops) = with_ctx(tw, |ctx| mush_party::release(ctx, id, how));
    st.apply(ops);
    assert_eq!(r, Ok(()), "release({id}) must be accepted");
}

fn unlock_at(tw: &mut TestWorld, st: &mut Persisted, id: PartyId) {
    st.sync(tw);
    let (r, ops) = with_ctx(tw, |ctx| mush_party::unlock(ctx, id));
    st.apply(ops);
    assert_eq!(r, Ok(()), "unlock({id}) must be accepted");
}

fn reboot(tw: &mut TestWorld, kv: Vec<(SmolStr, serde_json::Value)>) {
    tw.modules_loaded(HashMap::from([(
        SmolStr::new(NS),
        ModuleBootData {
            kv,
            streams: HashMap::new(),
        },
    )]));
}

fn patch(kv: &mut [(SmolStr, serde_json::Value)], key: &str, f: impl Fn(&mut serde_json::Value)) {
    let mut seen = false;
    for (k, v) in kv.iter_mut() {
        if k == key {
            f(v);
            seen = true;
        }
    }
    assert!(seen, "no such persisted key to patch: {key}");
}

/// `n` connected players named `<prefix><i>`.
fn crowd(tw: &mut TestWorld, prefix: &str, n: usize) -> Vec<ObjId> {
    (0..n)
        .map(|i| tw.spawn_player(&format!("{prefix}{i}")))
        .collect()
}

/// A leader, a second member and an invitee on party 0, at wall time `now`.
struct Crew {
    ann: ObjId,
    bob: ObjId,
    cid: ObjId,
    outsider: ObjId,
    id: PartyId,
}

fn crew(tw: &mut TestWorld, now: i64) -> Crew {
    tw.set_now(now);
    let ann = tw.spawn_player("Ann");
    let bob = tw.spawn_player("Bob");
    let cid = tw.spawn_player("Cid");
    let outsider = tw.spawn_player("Zed");
    ok(tw, ann, "+party/create Wrecking Crew");
    ok(tw, ann, "+party/invite Bob");
    ok(tw, bob, "+party/accept 0");
    ok(tw, ann, "+party/invite Cid");
    Crew {
        ann,
        bob,
        cid,
        outsider,
        id: PartyId(0),
    }
}

// ---------------------------------------------------------------------------
// 1. The verbs
// ---------------------------------------------------------------------------

/// §3.6: create / invite / accept work, and the record says so — not the message.
#[test]
fn create_invite_and_accept_build_a_roster() {
    let mut tw = boot();
    let c = crew(&mut tw, 1_000);
    let p = party(&tw, c.id);
    assert_eq!(p.name, "Wrecking Crew");
    assert_eq!(p.leader, c.ann);
    assert_eq!(p.members, vec![c.ann, c.bob]);
    assert!(p.is_invited(c.cid), "Cid's invitation stands");
    assert!(!p.is_member(c.cid), "an invitation is not membership");
    assert_eq!(p.status, PartyStatus::Forming);
    assert_eq!(p.expires_at, 1_000 + PARTY_IDLE_TTL);
    assert_eq!(mush_party::parties_of(tw.world(), c.bob), vec![c.id]);
    assert_eq!(mush_party::parties_of(tw.world(), c.cid), Vec::new());
}

/// §3.6: decline consumes the invitation and touches nothing else; uninvite is the
/// leader's half of the same act.
#[test]
fn decline_and_uninvite_consume_only_the_invitation() {
    let mut tw = boot();
    let c = crew(&mut tw, 1_000);
    ok(&mut tw, c.cid, "+party/decline 0");
    let p = party(&tw, c.id);
    assert!(!p.is_invited(c.cid));
    assert_eq!(p.members, vec![c.ann, c.bob]);

    ok(&mut tw, c.ann, "+party/invite Cid");
    ok(&mut tw, c.ann, "+party/uninvite Cid");
    let p = party(&tw, c.id);
    assert!(!p.is_invited(c.cid));
    assert_eq!(p.members, vec![c.ann, c.bob]);
    // A second withdrawal is refused, and says so without hiding: Ann may view it.
    let t = err(&mut tw, c.ann, "+party/uninvite Cid");
    assert!(t.contains("not invited"), "{t}");
}

/// §3.6: kick removes a member, and never the leader — that is what `/disband` is
/// for. A non-leader cannot kick at all.
#[test]
fn kick_removes_a_member_and_never_the_leader() {
    let mut tw = boot();
    let c = crew(&mut tw, 1_000);
    let t = err(&mut tw, c.ann, "+party/kick Ann");
    assert!(t.contains("+party/disband"), "{t}");

    // Bob leads nothing, so the leader-verb pool is empty and the refusal HIDES.
    hidden(&mut tw, c.bob, "+party/kick Ann");

    ok(&mut tw, c.ann, "+party/kick Bob");
    assert_eq!(members(&tw, c.id), vec![c.ann]);
    assert_eq!(mush_party::parties_of(tw.world(), c.bob), Vec::new());
}

/// §3.6: disband is terminal — every verb afterwards is refused, and the record is
/// kept as the account of who ran with whom.
#[test]
fn disband_is_terminal_and_keeps_the_record() {
    let mut tw = boot();
    let c = crew(&mut tw, 1_000);
    ok(&mut tw, c.ann, "+party/disband");
    assert_eq!(
        status(&tw, c.id),
        PartyStatus::Disbanded {
            how: DisbandCause::Disbanded
        }
    );
    // The invitation went with it (a disbanded party holds none), so Cid can no
    // longer see it at all.
    assert!(party(&tw, c.id).invited.is_empty());
    hidden(&mut tw, c.cid, "+party/accept 0");
    // Ann is still a member, so she sees it — and every verb refuses.
    let t = err(&mut tw, c.ann, "+party/leave 0");
    assert!(t.contains("disbanded"), "{t}");
    // Named explicitly, because the leader-verb pool is empty once it is terminal:
    // the bare form would (correctly) hide instead.
    hidden(&mut tw, c.ann, "+party/invite Zed");
    let t = err(&mut tw, c.ann, "+party/invite 0 Zed");
    assert!(t.contains("disbanded"), "{t}");
    assert_eq!(members(&tw, c.id), vec![c.ann, c.bob], "the record is kept");
    assert_eq!(store(&tw).live_count(), 0);
}

/// §3.6: the last member leaving disbands `Empty`; a leaving leader hands the lead to
/// the next member in roster order.
#[test]
fn the_last_member_leaving_disbands_empty_and_a_leader_hands_on() {
    let mut tw = boot();
    let c = crew(&mut tw, 1_000);
    ok(&mut tw, c.ann, "+party/role Bob=decker");
    ok(&mut tw, c.ann, "+party/leave");
    let p = party(&tw, c.id);
    assert_eq!(p.leader, c.bob, "the lead passed to the next member");
    assert_eq!(p.members, vec![c.bob]);
    assert!(p.is_forming());

    ok(&mut tw, c.bob, "+party/leave");
    let p = party(&tw, c.id);
    assert_eq!(
        p.status,
        PartyStatus::Disbanded {
            how: DisbandCause::Empty
        }
    );
    assert!(p.members.is_empty(), "Disbanded{{Empty}} has no roster");
    assert!(p.roles.is_empty(), "a leaving member takes their label");
    assert!(
        p.invited.is_empty(),
        "and a disbanded party holds no invites"
    );
}

/// A role is a **label**, and it counts as activity (the idle clock moves).
#[test]
fn a_role_labels_a_member_and_counts_as_activity() {
    let mut tw = boot();
    let c = crew(&mut tw, 1_000);
    tw.set_now(2_000);
    ok(&mut tw, c.ann, "+party/role Bob=decker");
    let p = party(&tw, c.id);
    assert_eq!(p.role_of(c.bob).map(SmolStr::as_str), Some("decker"));
    assert_eq!(p.expires_at, 2_000 + PARTY_IDLE_TTL, "role is activity");

    ok(&mut tw, c.ann, "+party/role Bob=");
    assert!(
        party(&tw, c.id).role_of(c.bob).is_none(),
        "an empty value clears"
    );

    // A label on somebody who is not a member is refused, and an oversized one too.
    let t = err(&mut tw, c.ann, "+party/role Zed=face");
    assert!(t.contains("not in that party"), "{t}");
    let long = "x".repeat(mush_party::MAX_PARTY_ROLE_BYTES + 1);
    err(&mut tw, c.ann, &format!("+party/role Bob={long}"));
    assert!(party(&tw, c.id).role_of(c.bob).is_none());
}

// ---------------------------------------------------------------------------
// 2. Existence hiding
// ---------------------------------------------------------------------------

/// §3.6 / §3.4: a non-member, non-invitee, non-staff viewer gets `"No such party."`
/// for a party they were not invited to — identical to a party that never existed.
#[test]
fn a_non_viewer_cannot_tell_a_hidden_party_from_a_missing_one() {
    let mut tw = boot();
    let c = crew(&mut tw, 1_000);

    hidden(&mut tw, c.outsider, "+party/accept 0");
    hidden(&mut tw, c.outsider, "+party/decline 0");
    hidden(&mut tw, c.outsider, "+party 0");
    hidden(&mut tw, c.outsider, "+party/leave 0");
    hidden(&mut tw, c.outsider, "+party/disband 0");
    // The party that does not exist answers with the very same words.
    hidden(&mut tw, c.outsider, "+party 99");
    hidden(&mut tw, c.ann, "+party 99");
    // A malformed id hides too — it must not be distinguishable from a real one.
    hidden(&mut tw, c.outsider, "+party/accept nonsense");

    assert!(!mush_party::may_view(tw.world(), c.outsider, c.id));
    assert!(mush_party::may_view(tw.world(), c.bob, c.id), "a member");
    assert!(mush_party::may_view(tw.world(), c.cid, c.id), "an invitee");
    assert!(mush_party::may_view(tw.world(), GOD, c.id), "staff");
    assert!(!mush_party::may_view(tw.world(), GOD, PartyId(99)));

    // Staff may LOOK and still may not join: viewing is not standing.
    let t = ok(&mut tw, GOD, "+party 0");
    assert!(t.contains("Wrecking Crew"), "{t}");
    let t = err(&mut tw, GOD, "+party/accept 0");
    assert!(t.contains("not invited"), "{t}");
}

// ---------------------------------------------------------------------------
// 3. Caps
// ---------------------------------------------------------------------------

/// §3.6: `MAX_PARTY_SIZE` refuses the ninth member.
#[test]
fn the_ninth_member_is_refused() {
    let mut tw = boot();
    tw.set_now(1_000);
    let ann = tw.spawn_player("Ann");
    let guests = crowd(&mut tw, "Guest", MAX_PARTY_SIZE);
    ok(&mut tw, ann, "+party/create Overfull");
    for i in 0..MAX_PARTY_SIZE {
        ok(&mut tw, ann, &format!("+party/invite Guest{i}"));
    }
    // Leader + seven accepts = eight members, the cap exactly.
    for guest in guests.iter().take(MAX_PARTY_SIZE - 1) {
        ok(&mut tw, *guest, "+party/accept 0");
    }
    assert_eq!(members(&tw, PartyId(0)).len(), MAX_PARTY_SIZE);

    let ninth = guests[MAX_PARTY_SIZE - 1];
    let t = err(&mut tw, ninth, "+party/accept 0");
    assert!(t.contains("full"), "{t}");
    assert_eq!(members(&tw, PartyId(0)).len(), MAX_PARTY_SIZE);
    assert!(
        party(&tw, PartyId(0)).is_invited(ninth),
        "a refused accept consumes nothing"
    );
}

/// §3.6: `MAX_PARTY_INVITES` refuses the seventeenth invitation.
#[test]
fn the_seventeenth_invitation_is_refused() {
    let mut tw = boot();
    tw.set_now(1_000);
    let ann = tw.spawn_player("Ann");
    let _guests = crowd(&mut tw, "Guest", MAX_PARTY_INVITES + 1);
    ok(&mut tw, ann, "+party/create Popular");
    for i in 0..MAX_PARTY_INVITES {
        ok(&mut tw, ann, &format!("+party/invite Guest{i}"));
    }
    assert_eq!(party(&tw, PartyId(0)).invited.len(), MAX_PARTY_INVITES);
    let t = err(
        &mut tw,
        ann,
        &format!("+party/invite Guest{MAX_PARTY_INVITES}"),
    );
    assert!(t.contains("invitations outstanding"), "{t}");
    assert_eq!(party(&tw, PartyId(0)).invited.len(), MAX_PARTY_INVITES);
}

/// `MAX_PARTIES_PER_LEADER` bounds `+party/create`, and a disbanded party frees its
/// slot (terminal records do not count against the live cap).
#[test]
fn a_leader_is_bounded_and_a_disband_frees_the_slot() {
    let mut tw = boot();
    tw.set_now(1_000);
    let ann = tw.spawn_player("Ann");
    for i in 0..MAX_PARTIES_PER_LEADER {
        ok(&mut tw, ann, &format!("+party/create Crew{i}"));
    }
    let t = err(&mut tw, ann, "+party/create One Too Many");
    assert!(t.contains("Disband one first"), "{t}");
    assert_eq!(store(&tw).live_count(), MAX_PARTIES_PER_LEADER);

    ok(&mut tw, ann, "+party/disband 0");
    ok(&mut tw, ann, "+party/create One More");
    assert_eq!(store(&tw).live_count(), MAX_PARTIES_PER_LEADER);
    assert_eq!(store(&tw).parties().count(), MAX_PARTIES_PER_LEADER + 1);
}

// ---------------------------------------------------------------------------
// 4. Idle expiry
// ---------------------------------------------------------------------------

/// §3.6: idle expiry disbands a `Forming` party past `expires_at`, and a membership
/// change refreshes the clock.
#[test]
fn idle_expiry_disbands_a_forming_party_and_activity_refreshes_it() {
    let mut tw = boot();
    let c = crew(&mut tw, 1_000);
    ok(&mut tw, c.ann, "+party/create Second Crew");
    let stale = PartyId(1);

    // Party 0 sees activity just before its deadline; party 1 does not.
    let late = 1_000 + PARTY_IDLE_TTL - 10;
    tw.set_now(late);
    ok(&mut tw, c.ann, "+party/role 0 Bob=face");
    assert_eq!(party(&tw, c.id).expires_at, late + PARTY_IDLE_TTL);

    let after = 1_000 + PARTY_IDLE_TTL + 1;
    tw.set_now(after);
    tw.fire_due(after.saturating_mul(1_000));

    assert_eq!(status(&tw, c.id), PartyStatus::Forming, "activity saved it");
    assert_eq!(
        status(&tw, stale),
        PartyStatus::Disbanded {
            how: DisbandCause::Expired
        }
    );
    assert_eq!(store(&tw).live_count(), 1);
}

/// A lapsed **invitation** is pruned by the same fire, and pruning is housekeeping —
/// it does not refresh the party's own idle clock.
#[test]
fn the_tick_prunes_lapsed_invitations_without_calling_it_activity() {
    let mut tw = boot();
    let c = crew(&mut tw, 1_000);
    // Cid was invited at t=1000, so his invitation lapses at 1000 + TTL. Keep the
    // party itself alive past that point with a later membership change.
    let late = 1_000 + PARTY_IDLE_TTL - 10;
    tw.set_now(late);
    ok(&mut tw, c.ann, "+party/role Bob=rigger");
    let deadline = party(&tw, c.id).expires_at;

    let after = 1_000 + PARTY_IDLE_TTL + 1;
    tw.set_now(after);
    tw.fire_due(after.saturating_mul(1_000));

    let p = party(&tw, c.id);
    assert!(p.is_forming());
    assert!(!p.is_invited(c.cid), "the lapsed invitation was pruned");
    assert_eq!(p.expires_at, deadline, "pruning is not activity");
    hidden(&mut tw, c.cid, "+party/accept 0");
}

/// §3.6: a `Locked` party never expires — its contract's own deadline governs it, and
/// this crate cannot read that deadline.
#[test]
fn a_locked_party_never_expires() {
    let mut tw = boot();
    let mut st = Persisted::default();
    let c = crew(&mut tw, 1_000);
    lock_at(&mut tw, &mut st, c.id, 77);

    let after = 1_000 + PARTY_IDLE_TTL * 3;
    tw.set_now(after);
    tw.fire_due(after.saturating_mul(1_000));

    assert_eq!(
        status(&tw, c.id),
        PartyStatus::Locked {
            by: ContractRef(77)
        }
    );
    assert_eq!(store(&tw).live_count(), 1);
}

// ---------------------------------------------------------------------------
// 5. Locked
// ---------------------------------------------------------------------------

/// §3.6: `Locked` refuses **every** membership change. Asserted verb by verb.
///
/// Since the E6a verify fold (D6) the lock also **prunes** the outstanding
/// invitations, so `/accept` and `/decline` from a former invitee no longer reach the
/// locked refusal at all — they reach `NO_SUCH_PARTY`, which is the stronger answer
/// and the one `vf_d6_…` asserts. Everything a *member* can try still refuses as
/// locked.
#[test]
fn locked_refuses_every_membership_change() {
    let mut tw = boot();
    let mut st = Persisted::default();
    let c = crew(&mut tw, 1_000);
    let dee = tw.spawn_player("Dee");
    let before = party(&tw, c.id);
    lock_at(&mut tw, &mut st, c.id, 77);

    for (actor, line) in [
        (c.ann, "+party/invite Dee"),
        (c.ann, "+party/uninvite Cid"),
        (c.bob, "+party/leave"),
        (c.ann, "+party/kick Bob"),
        (c.ann, "+party/role Bob=face"),
        (c.ann, "+party/disband"),
    ] {
        let t = err(&mut tw, actor, line);
        assert!(
            t.contains("locked to a job"),
            "`{line}` must refuse as locked, got: {t}"
        );
    }
    for line in ["+party/accept 0", "+party/decline 0"] {
        let t = err(&mut tw, c.cid, line);
        assert!(
            t.contains(mush_party::NO_SUCH_PARTY),
            "`{line}` from a pruned invitee must hide the party, got: {t}"
        );
    }

    let after = party(&tw, c.id);
    assert_eq!(after.members, before.members, "the roster did not move");
    assert!(
        after.invited.is_empty(),
        "the lock did not prune the outstanding invitations (D6)"
    );
    assert_eq!(after.roles, before.roles);
    assert!(!after.is_member(dee));
}

/// §3.6: a substitution needs the amendment. `lock` → the change is refused →
/// `unlock` → `Forming` → the change works.
#[test]
fn a_substitution_needs_the_amendment_and_returns_to_forming() {
    let mut tw = boot();
    let mut st = Persisted::default();
    let c = crew(&mut tw, 1_000);
    lock_at(&mut tw, &mut st, c.id, 77);
    err(&mut tw, c.ann, "+party/kick Bob");
    assert_eq!(members(&tw, c.id), vec![c.ann, c.bob]);

    // The ruleset amends its contract, then calls unlock.
    unlock_at(&mut tw, &mut st, c.id);
    assert_eq!(status(&tw, c.id), PartyStatus::Forming);

    ok(&mut tw, c.ann, "+party/kick Bob");
    assert_eq!(members(&tw, c.id), vec![c.ann]);
    ok(&mut tw, c.ann, "+party/invite Zed");
}

/// `unlock` refreshes the idle clock: a party locked for longer than the TTL must not
/// come back to `Forming` already past its deadline.
#[test]
fn unlock_refreshes_the_idle_clock() {
    let mut tw = boot();
    let mut st = Persisted::default();
    let c = crew(&mut tw, 1_000);
    lock_at(&mut tw, &mut st, c.id, 77);

    let much_later = 1_000 + PARTY_IDLE_TTL * 2;
    tw.set_now(much_later);
    unlock_at(&mut tw, &mut st, c.id);
    assert_eq!(party(&tw, c.id).expires_at, much_later + PARTY_IDLE_TTL);

    tw.fire_due(much_later.saturating_mul(1_000));
    assert_eq!(status(&tw, c.id), PartyStatus::Forming);
}

/// §3.6: `lock`/`release`/`unlock` refuse from the wrong state, with the right
/// [`PartyError`], and change nothing when they do.
#[test]
fn the_seams_refuse_from_the_wrong_state() {
    let mut tw = boot();
    let mut st = Persisted::default();
    let c = crew(&mut tw, 1_000);

    // No such party.
    let (r, _) = with_ctx(&mut tw, |ctx| {
        mush_party::lock(ctx, PartyId(99), ContractRef(1))
    });
    assert_eq!(r, Err(PartyError::NoStanding));
    let (r, _) = with_ctx(&mut tw, |ctx| {
        mush_party::release(ctx, PartyId(99), DisbandCause::Settled)
    });
    assert_eq!(r, Err(PartyError::NoStanding));
    let (r, _) = with_ctx(&mut tw, |ctx| mush_party::unlock(ctx, PartyId(99)));
    assert_eq!(r, Err(PartyError::NoStanding));

    // Forming: release and unlock are both wrong-state.
    let (r, _) = with_ctx(&mut tw, |ctx| {
        mush_party::release(ctx, PartyId(0), DisbandCause::Settled)
    });
    assert_eq!(r, Err(PartyError::State));
    let (r, _) = with_ctx(&mut tw, |ctx| mush_party::unlock(ctx, PartyId(0)));
    assert_eq!(r, Err(PartyError::State));
    assert_eq!(status(&tw, c.id), PartyStatus::Forming);

    // Locked: a second lock is wrong-state.
    lock_at(&mut tw, &mut st, c.id, 77);
    let (r, _) = with_ctx(&mut tw, |ctx| {
        mush_party::lock(ctx, PartyId(0), ContractRef(2))
    });
    assert_eq!(r, Err(PartyError::State));
    assert_eq!(
        status(&tw, c.id),
        PartyStatus::Locked {
            by: ContractRef(77)
        },
        "a refused lock did not re-point the contract"
    );

    // Disbanded: everything is wrong-state, forever.
    release_at(&mut tw, &mut st, c.id, DisbandCause::Settled);
    let (r, _) = with_ctx(&mut tw, |ctx| {
        mush_party::lock(ctx, PartyId(0), ContractRef(3))
    });
    assert_eq!(r, Err(PartyError::State));
    let (r, _) = with_ctx(&mut tw, |ctx| mush_party::unlock(ctx, PartyId(0)));
    assert_eq!(r, Err(PartyError::State));
    let (r, _) = with_ctx(&mut tw, |ctx| {
        mush_party::release(ctx, PartyId(0), DisbandCause::Released)
    });
    assert_eq!(r, Err(PartyError::State));
    assert_eq!(
        status(&tw, c.id),
        PartyStatus::Disbanded {
            how: DisbandCause::Settled
        }
    );
}

// ---------------------------------------------------------------------------
// 6. Which party? — the disambiguation rule
// ---------------------------------------------------------------------------

/// With more than one candidate the party-less verbs refuse and list the ids; the
/// explicit `<id>`-first form then works. A lone dbref is never read as a party id.
#[test]
fn ambiguity_refuses_with_the_ids_and_the_explicit_form_works() {
    let mut tw = boot();
    tw.set_now(1_000);
    let ann = tw.spawn_player("Ann");
    let bob = tw.spawn_player("Bob");
    ok(&mut tw, ann, "+party/create First");
    ok(&mut tw, ann, "+party/create Second");
    ok(&mut tw, ann, "+party/invite 1 Bob");
    ok(&mut tw, bob, "+party/accept 1");

    let t = err(&mut tw, ann, "+party/disband");
    assert!(t.contains("You lead more than one party"), "{t}");
    assert!(t.contains("0, 1"), "the candidate ids are named: {t}");

    let t = err(&mut tw, ann, "+party/kick Bob");
    assert!(t.contains("+party/kick <id> <who>"), "{t}");

    // A lone dbref is a target, never a party id: Bob is not in party #<bob>.
    let t = err(&mut tw, ann, &format!("+party/kick #{}", bob.0));
    assert!(t.contains("You lead more than one party"), "{t}");

    ok(&mut tw, ann, &format!("+party/kick 1 #{}", bob.0));
    assert_eq!(members(&tw, PartyId(1)), vec![ann]);
    ok(&mut tw, ann, "+party/disband 0");
    assert_eq!(
        status(&tw, PartyId(0)),
        PartyStatus::Disbanded {
            how: DisbandCause::Disbanded
        }
    );
    // One candidate left: the bare form resolves again.
    ok(&mut tw, ann, "+party/disband");
    assert!(party(&tw, PartyId(1)).is_terminal());
    // None left: the refusal hides.
    hidden(&mut tw, ann, "+party/disband");
}

// ---------------------------------------------------------------------------
// 7. Reboot
// ---------------------------------------------------------------------------

/// §3.6: reboot at **every** status — `Forming`, `Locked`, and each `DisbandCause` —
/// round-trips through the wire.
#[test]
fn reboot_round_trips_every_status() {
    let mut tw = boot();
    let mut st = Persisted::default();
    tw.set_now(1_000);
    let ann = tw.spawn_player("Ann");
    let bob = tw.spawn_player("Bob");

    // 0: expires on the tick.
    ok(&mut tw, ann, "+party/create Ghosts");
    let after = 1_000 + PARTY_IDLE_TTL + 1;
    tw.set_now(after);
    tw.fire_due(after.saturating_mul(1_000));
    assert_eq!(
        status(&tw, PartyId(0)),
        PartyStatus::Disbanded {
            how: DisbandCause::Expired
        }
    );

    // 1: the leader disbands it.
    ok(&mut tw, ann, "+party/create Quitters");
    ok(&mut tw, ann, "+party/disband 1");
    // 2: the last member leaves it.
    ok(&mut tw, ann, "+party/create Solo");
    ok(&mut tw, ann, "+party/leave 2");
    // 3: locked, then settled.
    ok(&mut tw, ann, "+party/create Paid");
    lock_at(&mut tw, &mut st, PartyId(3), 300);
    release_at(&mut tw, &mut st, PartyId(3), DisbandCause::Settled);
    // 4: locked, then released.
    ok(&mut tw, ann, "+party/create Refunded");
    lock_at(&mut tw, &mut st, PartyId(4), 400);
    release_at(&mut tw, &mut st, PartyId(4), DisbandCause::Released);
    // 5: still forming, with a roster, a role and an invitation.
    ok(&mut tw, ann, "+party/create Live");
    ok(&mut tw, ann, "+party/invite 5 Bob");
    ok(&mut tw, bob, "+party/accept 5");
    ok(&mut tw, ann, "+party/role 5 Bob=decker");
    // 6: locked and staying that way.
    ok(&mut tw, ann, "+party/create Frozen");
    lock_at(&mut tw, &mut st, PartyId(6), 600);

    st.sync(&mut tw);
    let before: Vec<Party> = store(&tw).parties().cloned().collect();
    assert_eq!(before.len(), 7);
    let next_id = store(&tw).meta_doc();

    reboot(&mut tw, st.kv());

    let after: Vec<Party> = store(&tw).parties().cloned().collect();
    assert_eq!(after, before, "every status round-trips byte for byte");
    assert_eq!(
        store(&tw).meta_doc(),
        next_id,
        "the allocator floor survives"
    );
    assert_eq!(store(&tw).live_count(), 2, "Live and Frozen");
    assert_eq!(
        status(&tw, PartyId(6)),
        PartyStatus::Locked {
            by: ContractRef(600)
        }
    );
    assert_eq!(
        party(&tw, PartyId(5)).role_of(bob).map(SmolStr::as_str),
        Some("decker")
    );

    // And the world still works afterwards.
    ok(&mut tw, ann, "+party/kick 5 Bob");
    assert_eq!(members(&tw, PartyId(5)), vec![ann]);
}

/// One way to corrupt a persisted `party:{id}` document.
type Breakage = Box<dyn Fn(&mut serde_json::Value)>;

/// §3.6 / the E5a fold rule: a document that violates an invariant is dropped
/// **loudly** and the world still boots — the other parties load, the allocator floor
/// still comes off the key, and a `Del` fixup keeps the store matching RAM.
#[test]
fn a_document_that_breaks_an_invariant_is_dropped_and_the_boot_survives() {
    let mut tw = boot();
    let mut st = Persisted::default();
    tw.set_now(1_000);
    let ann = tw.spawn_player("Ann");
    let bob = tw.spawn_player("Bob");
    ok(&mut tw, ann, "+party/create Keeper");
    ok(&mut tw, ann, "+party/create Doomed");
    ok(&mut tw, ann, "+party/invite 1 Bob");
    ok(&mut tw, bob, "+party/accept 1");
    st.sync(&mut tw);
    let clean = st.kv();

    let breakages: Vec<(&str, Breakage)> = vec![
        (
            "nine members",
            Box::new(|v: &mut serde_json::Value| {
                v["members"] = serde_json::json!([2, 3, 4, 5, 6, 7, 8, 9, 10]);
                v["leader"] = serde_json::json!(2);
                v["roles"] = serde_json::json!({});
            }),
        ),
        (
            "a role for a non-member",
            Box::new(|v: &mut serde_json::Value| {
                v["roles"] = serde_json::json!({ "4242": "face" });
            }),
        ),
        (
            "a duplicate member",
            Box::new(|v: &mut serde_json::Value| {
                let leader = v["leader"].clone();
                v["members"] = serde_json::json!([leader.clone(), leader]);
            }),
        ),
        (
            "an id that disagrees with its key",
            Box::new(|v: &mut serde_json::Value| {
                v["id"] = serde_json::json!(4242);
            }),
        ),
        (
            "an invitation for a member",
            Box::new(|v: &mut serde_json::Value| {
                let leader = v["leader"].clone();
                v["invited"] = serde_json::json!([{
                    "who": leader.clone(),
                    "by": leader,
                    "at": 1_000,
                    "expires_at": 1_000 + PARTY_IDLE_TTL,
                }]);
            }),
        ),
        (
            "an oversized name",
            Box::new(|v: &mut serde_json::Value| {
                v["name"] = serde_json::json!("x".repeat(mush_party::MAX_PARTY_NAME_BYTES + 1));
            }),
        ),
    ];

    for (what, break_it) in breakages {
        let mut kv = clean.clone();
        patch(&mut kv, "party:1", |v| break_it(v));
        reboot(&mut tw, kv);

        assert!(
            store(&tw).party(PartyId(1)).is_none(),
            "{what}: the bad document must not load"
        );
        assert!(
            store(&tw).party(PartyId(0)).is_some(),
            "{what}: the good document still loads"
        );
        assert_eq!(
            store(&tw).meta_doc(),
            serde_json::json!({ "next_party_id": 2 }),
            "{what}: the key still floors the allocator"
        );

        // The fixup is queued, so the store stops disagreeing with RAM.
        let mut fixups = Persisted::default();
        fixups.sync(&mut tw);
        let deleted = tw.take_module_ops();
        drop(deleted);

        // And the world is usable: a fresh party gets a fresh id, never id 1's.
        let created = ok(&mut tw, ann, "+party/create Recovery");
        assert!(created.contains("Party 2 created"), "{what}: {created}");
        ok(&mut tw, ann, "+party/disband 2");
    }
}

/// The `Del` fixup for a dropped document is actually queued (so a re-fold of the
/// namespace no longer carries it).
#[test]
fn a_dropped_document_is_deleted_from_the_store() {
    let mut tw = boot();
    let mut st = Persisted::default();
    tw.set_now(1_000);
    let ann = tw.spawn_player("Ann");
    ok(&mut tw, ann, "+party/create Doomed");
    st.sync(&mut tw);
    let mut kv = st.kv();
    patch(&mut kv, "party:0", |v| {
        v["members"] = serde_json::json!([]);
    });

    reboot(&mut tw, kv.clone());
    assert!(store(&tw).party(PartyId(0)).is_none());

    let mut folded = Persisted::default();
    folded.apply(
        kv.into_iter()
            .map(|(key, value)| ModuleOp {
                ns: SmolStr::new(NS),
                op: ModuleWrite::JsonSet { key, value },
            })
            .collect(),
    );
    folded.sync(&mut tw);
    assert!(
        !folded.kv().iter().any(|(k, _)| k == "party:0"),
        "the boot queued a Del for the document it refused"
    );
}

// ---------------------------------------------------------------------------
// 8. Destroyed objects
// ---------------------------------------------------------------------------

/// A destroyed member leaves every live, **unlocked** roster — and a `Locked` roster
/// is left exactly as it is, because it is frozen against a contract this crate
/// cannot read (D-E6-5).
#[test]
fn a_destroyed_member_leaves_an_unlocked_roster_only() {
    let mut tw = boot();
    let mut st = Persisted::default();
    let c = crew(&mut tw, 1_000);
    ok(&mut tw, c.ann, "+party/create Other Crew");
    ok(&mut tw, c.ann, "+party/invite 1 Bob");
    lock_at(&mut tw, &mut st, PartyId(1), 77);

    // The observer is the unit under test, so the event is emitted the way core
    // emits it, through the engine — not through whatever policy guards `@destroy`
    // for players.
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    tw.engine_mut()
        .emit_world_event(&WorldEvent::ObjDestroyed { obj: c.bob }, &mut out, &mut fx);

    let unlocked = party(&tw, c.id);
    assert!(
        !unlocked.is_member(c.bob),
        "an unlocked roster drops a destroyed member"
    );
    assert_eq!(unlocked.members, vec![c.ann]);
    assert!(unlocked.is_forming());

    // The lock pruned Bob's invitation to the second party (D6), so what is asserted
    // here is what remains true: the destroyed member is not on the *roster*, and the
    // observer left the locked party's status and members exactly as they were.
    let locked = party(&tw, PartyId(1));
    assert!(
        !locked.is_invited(c.bob),
        "the lock did not prune the outstanding invitation (D6)"
    );
    assert_eq!(
        locked.members,
        vec![c.ann],
        "a locked party's roster is never edited behind the contract"
    );
    assert_eq!(
        locked.status,
        PartyStatus::Locked {
            by: ContractRef(77)
        }
    );
}

// ---- E6a verify fold ----

/// D6: a stale invitee could watch the crew for the life of the run.
///
/// `may_view` admits an invitee, `Locked` refuses `/uninvite`, and the lock left the
/// invitation standing — so somebody the leader had invited and never removed kept
/// reading the roster of a party that had gone on a job, and nobody could take it away
/// from them. The lock prunes the invitations, because a locked party can neither
/// accept one nor withdraw one.
#[test]
fn vf_d6_the_lock_prunes_outstanding_invitations_and_hides_the_party() {
    let mut tw = boot();
    let mut st = Persisted::default();
    let c = crew(&mut tw, 1_000);
    // Cid holds an outstanding invitation and can see the party.
    assert!(party(&tw, c.id).is_invited(c.cid));
    ok(&mut tw, c.cid, &format!("+party {}", c.id));

    lock_at(&mut tw, &mut st, c.id, 77);

    let p = party(&tw, c.id);
    assert!(p.invited.is_empty(), "the lock left an invitation standing");
    assert!(!p.is_invited(c.cid));
    assert_eq!(p.members, vec![c.ann, c.bob], "the roster moved");
    // And the un-accepted invitee is now told exactly what a stranger is told.
    hidden(&mut tw, c.cid, &format!("+party {}", c.id));
    // A member still reads it.
    ok(&mut tw, c.bob, &format!("+party {}", c.id));
}

/// D6: the `ContractRef` a locked party carries is the ruleset's handle, and a member
/// render never names it. It is an economy-side contract id the party cannot itself
/// interpret and whose own view gate never admitted the crew.
#[test]
fn vf_d6_a_members_view_names_no_contract_id() {
    let mut tw = boot();
    let mut st = Persisted::default();
    let c = crew(&mut tw, 1_000);
    lock_at(&mut tw, &mut st, c.id, 4_242);

    let mine = ok(&mut tw, c.bob, &format!("+party {}", c.id));
    assert!(
        mine.contains("locked to a job"),
        "a member is not told the party is on a job: {mine}"
    );
    assert!(
        !mine.contains("4242"),
        "the contract id leaked to a member: {mine}"
    );
    // Staff, who may already read the contract through the economy's own surface,
    // keep the handle for support.
    let theirs = ok(&mut tw, GOD, &format!("+party {}", c.id));
    assert!(theirs.contains("4242"), "staff lost the handle: {theirs}");
}
