//! E6a step 1, S9 (DL-04): the **org inbox** — an organization's officers are told,
//! kinds only, what its contracts and obligations did.
//!
//! The economy tells `Object` parties itself and an organization nothing at all;
//! `mush_server::econ_inbox::OrgInbox` closes that from the composition root. What
//! the tests hold to:
//!
//! - **holders, not members**: whoever holds `economy.account.post` on the org's
//!   root compartment hears it; a member holding some other capability hears
//!   nothing;
//! - **kinds only**: the line names the contract and what happened to it, never a
//!   term, an amount, an account or a party;
//! - **nothing is lost to absence**: an officer who was away reads the lines at their
//!   next connect from a bounded, durable queue — 32 per character, 240 bytes each,
//!   no duplicates — that survives a reboot and is deleted once read.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{
    ArgShape, Audience, CmdError, CommandSpec, Ctx, GOD, GameModule, LIMBO, ModuleBootData,
    ModuleWrite, ObjId, ObjKind,
};
use mush_economy::{AccountId, ContractId, ContractState, EconomyStore};
use mush_org::{CapabilityId, OrgScopeId, OrgStore, PrincipalId};
use mush_proto::OutputEvent;
use mush_server::econ_inbox::OrgInbox;
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

const NS: &str = "economy";

/// `@inboxprobe <who>=<n> [long]`: queue `n` distinct notices for `who` through the
/// economy's own seam (`long` adds one well past the byte cap first).
struct InboxProbe;

impl GameModule for InboxProbe {
    fn name(&self) -> &'static str {
        "e6ainboxprobe"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.command(
            CommandSpec {
                name: "@inboxprobe".into(),
                shape: ArgShape::EqPair,
                help: "@inboxprobe <who>=<n> [long]   test: queue n notices".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let bad = || CmdError::msg("Usage: @inboxprobe <who>=<n> [long]");
                let who = ObjId(
                    ctx.args
                        .lhs()
                        .trim()
                        .trim_start_matches('#')
                        .parse()
                        .map_err(|_| bad())?,
                );
                let rhs = ctx.args.rhs().unwrap_or_default().to_string();
                let mut words = rhs.split_whitespace();
                let n: usize = words.next().and_then(|w| w.parse().ok()).ok_or_else(bad)?;
                if words.any(|w| w == "long") {
                    mush_economy::deliver_notice(ctx, who, "L".repeat(300));
                }
                for i in 0..n {
                    mush_economy::deliver_notice(ctx, who, format!("notice {i}"));
                }
                // A duplicate of the last one is a retry, not news.
                if n > 0 {
                    mush_economy::deliver_notice(ctx, who, format!("notice {}", n - 1));
                }
                Ok(())
            }),
        )?;
        Ok(())
    }
}

fn vocabulary() -> BTreeSet<CapabilityId> {
    mush_economy::CAPABILITIES
        .iter()
        .copied()
        .chain(["org.role.manage", "org.grant.delegate"])
        .map(|c| CapabilityId::new(c).expect("well-formed"))
        .collect()
}

fn boot() -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(
            mush_org::OrgModule::new()
                .with_capabilities(vocabulary())
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(
            mush_economy::EconomyModule::with_custody(vec![])
                .with_observers(vec![Arc::new(OrgInbox)]),
        ),
        Box::new(InboxProbe),
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
fn run(tw: &mut TestWorld, actor: ObjId, line: &str) -> Vec<(Audience, OutputEvent)> {
    let ev = tw.run(actor, line);
    assert!(!has_error(&ev), "`{line}` was refused: {}", texts(&ev));
    ev
}
fn ok(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    texts(&run(tw, actor, line))
}
fn store(tw: &TestWorld) -> &EconomyStore {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy store")
}
fn seen(tw: &TestWorld, events: &[(Audience, OutputEvent)], viewer: ObjId) -> String {
    tw.seen_by(events, viewer).join("\n")
}
fn state(tw: &TestWorld, id: ContractId) -> ContractState {
    store(tw).snapshot().contract(id).expect("contract").state
}
/// Every module's documents, per namespace — **every** module's, because
/// `modules_loaded` re-boots them all and one that is handed nothing comes back
/// empty. The E6a step 1 verify fold (D6) made that matter here: a queued notice
/// records whose business it was and re-asks `acts_for` at delivery, so an officer
/// whose org state was silently dropped by the reboot would (correctly) read nothing.
fn drain_all(tw: &mut TestWorld) -> HashMap<SmolStr, Vec<(SmolStr, serde_json::Value)>> {
    let mut all: HashMap<SmolStr, Vec<(SmolStr, serde_json::Value)>> = HashMap::new();
    for op in tw.take_module_ops() {
        let kv = all.entry(op.ns.clone()).or_default();
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                kv.retain(|(k, _)| *k != key);
                kv.push((key, value));
            }
            ModuleWrite::Del { key } => kv.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    all
}
fn reboot_all(tw: &mut TestWorld, all: HashMap<SmolStr, Vec<(SmolStr, serde_json::Value)>>) {
    tw.modules_loaded(
        all.into_iter()
            .map(|(ns, kv)| {
                (
                    ns,
                    ModuleBootData {
                        kv,
                        streams: HashMap::new(),
                    },
                )
            })
            .collect(),
    );
}

/// Ironhold, its root-compartment treasury of 1000 gold, an **officer** holding
/// `economy.account.post` (and `contract.accept`) on it, a **member** holding only
/// `contract.dispute`, and a payee with a wallet.
struct Rig {
    root: OrgScopeId,
    treasury: AccountId,
    officer: ObjId,
    member: ObjId,
    payee: ObjId,
    payee_acct: AccountId,
}

fn rig(tw: &mut TestWorld, officer_connected: bool) -> Rig {
    let officer = if officer_connected {
        tw.spawn_player("Officer")
    } else {
        tw.world_mut()
            .create(ObjKind::Player, "Officer", GOD, Some(LIMBO))
            .expect("create")
    };
    let member = tw.spawn_player("Member");
    let payee = tw.spawn_player("Payee");
    ok(tw, GOD, "@instrument/register gold=0");
    ok(tw, GOD, "@org/create Ironhold");
    let root = tw
        .world()
        .module_state::<OrgStore>()
        .expect("org store")
        .state()
        .orgs()
        .next()
        .expect("the org")
        .root_scope;
    ok(tw, GOD, "@account/system faucet=gold");
    ok(tw, GOD, "@account/open org:Ironhold=gold");
    ok(tw, GOD, &format!("@account/open #{}=gold", payee.0));
    let snap = store(tw).snapshot();
    let treasury = snap
        .accounts()
        .find(|a| a.scope == Some(root))
        .expect("treasury")
        .id;
    let payee_acct = snap
        .accounts()
        .find(|a| a.owner == PrincipalId::Object(payee))
        .expect("payee wallet")
        .id;
    let faucet = snap
        .accounts()
        .find(|a| matches!(a.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id;
    ok(tw, GOD, &format!("@econ/mint {faucet}={treasury} 1000"));
    for cap in ["economy.account.post", "contract.accept"] {
        ok(
            tw,
            GOD,
            &format!("@org/grant #{}=Ironhold {cap}", officer.0),
        );
    }
    ok(
        tw,
        GOD,
        &format!("@org/grant #{}=Ironhold contract.dispute", member.0),
    );
    Rig {
        root,
        treasury,
        officer,
        member,
        payee,
        payee_acct,
    }
}

/// An org-issued contract paying the payee 100 on a staff attestation, offered.
fn org_offer(
    tw: &mut TestWorld,
    r: &Rig,
    name: &str,
) -> (ContractId, Vec<(Audience, OutputEvent)>) {
    ok(
        tw,
        GOD,
        &format!("@contract/draft {name} scope=#{}", r.root.0),
    );
    let id = store(tw)
        .snapshot()
        .contracts()
        .map(|c| c.id)
        .max()
        .expect("the draft");
    ok(tw, GOD, &format!("@contract/party {id}=#{}", r.payee.0));
    ok(tw, GOD, &format!("@contract/pay {id}={} 100", r.treasury));
    ok(
        tw,
        GOD,
        &format!("@contract/split {id}={} 1000‰", r.payee_acct),
    );
    ok(tw, GOD, &format!("@contract/remainder {id}={}", r.treasury));
    ok(tw, GOD, &format!("@contract/condition {id}=attest staff"));
    let ev = run(tw, GOD, &format!("@contract/offer {id}"));
    (id, ev)
}

/// Every kinds-only line an event batch carries for `viewer`.
fn inbox_lines(tw: &TestWorld, ev: &[(Audience, OutputEvent)], viewer: ObjId) -> Vec<String> {
    tw.seen_by(ev, viewer)
        .into_iter()
        .filter(|l| l.starts_with("Contract #") || l.starts_with("Obligation #"))
        .collect()
}

#[test]
fn e6s1_s9_an_org_payers_officer_hears_the_crewed_settled_disputed_and_refunded_lines() {
    let mut tw = boot();
    let r = rig(&mut tw, true);

    // Offered: the officer is told; the member, who holds no spending right, is not.
    let (id, ev) = org_offer(&mut tw, &r, "Supply");
    let officer = inbox_lines(&tw, &ev, r.officer);
    assert_eq!(
        officer,
        vec![format!(
            "Contract #{id} has been offered to your organization (@contract/show #{id})."
        )]
    );
    assert!(
        inbox_lines(&tw, &ev, r.member).is_empty(),
        "a non-holder was told"
    );
    // The payee hears the economy's OWN party notice, and not the org's line.
    assert!(
        !seen(&tw, &ev, r.payee).contains("your organization"),
        "the observer repeated an Object party"
    );

    // Crewed: the org's own accept (by the officer) and the payee's activate it.
    ok(&mut tw, r.officer, &format!("@contract/accept {id}"));
    let ev = run(&mut tw, r.payee, &format!("@contract/accept {id}"));
    assert_eq!(state(&tw, id), ContractState::Active);
    let officer = inbox_lines(&tw, &ev, r.officer);
    assert_eq!(
        officer,
        vec![format!(
            "Contract #{id} is active: every party has accepted."
        )]
    );
    assert!(inbox_lines(&tw, &ev, r.member).is_empty());

    // Settled — and the line carries no term.
    let ev = run(&mut tw, GOD, &format!("@contract/attest {id}=0"));
    assert!(matches!(state(&tw, id), ContractState::Settled { .. }));
    let officer = inbox_lines(&tw, &ev, r.officer);
    assert_eq!(officer, vec![format!("Contract #{id} has settled.")]);
    let everything = seen(&tw, &ev, r.officer);
    for forbidden in ["100", "account #", "Payee", "escrow", "gold"] {
        assert!(
            !everything.contains(forbidden),
            "the officer's line carries {forbidden:?}: {everything}"
        );
    }

    // Disputed.
    let (id2, _) = org_offer(&mut tw, &r, "Disputed");
    ok(&mut tw, r.officer, &format!("@contract/accept {id2}"));
    ok(&mut tw, r.payee, &format!("@contract/accept {id2}"));
    let ev = run(&mut tw, r.payee, &format!("@contract/dispute {id2}"));
    assert_eq!(
        inbox_lines(&tw, &ev, r.officer),
        vec![format!(
            "Contract #{id2} is disputed: its escrow is frozen until it is resolved."
        )]
    );

    // Refunded.
    let (id3, _) = org_offer(&mut tw, &r, "Withdrawn");
    ok(&mut tw, r.officer, &format!("@contract/accept {id3}"));
    let ev = run(&mut tw, GOD, &format!("@contract/cancel {id3}"));
    assert_eq!(state(&tw, id3), ContractState::Cancelled);
    assert_eq!(
        inbox_lines(&tw, &ev, r.officer),
        vec![format!("Contract #{id3} ended; its escrow was released.")]
    );
    assert!(inbox_lines(&tw, &ev, r.member).is_empty());
}

#[test]
fn e6s1_s9_an_offline_officer_reads_the_lines_at_connect_and_they_survive_a_reboot() {
    let mut tw = boot();
    let r = rig(&mut tw, false);
    assert!(!tw.world().is_connected(r.officer));
    let (id, ev) = org_offer(&mut tw, &r, "Supply");
    assert!(
        inbox_lines(&tw, &ev, r.officer).is_empty(),
        "an absent officer was written to"
    );
    ok(&mut tw, r.payee, &format!("@contract/accept {id}"));
    // Nothing was lost: it is queued, durably.
    let queued = store(&tw).pending_notices(r.officer);
    assert_eq!(queued.len(), 1, "the offer was not queued: {queued:?}");
    let all = drain_all(&mut tw);
    assert!(
        all.get(&SmolStr::new(NS)).is_some_and(|kv| kv
            .iter()
            .any(|(k, _)| *k == mush_economy::notice_key(r.officer))),
        "the queue was not persisted"
    );
    // A restart keeps it.
    reboot_all(&mut tw, all);
    assert_eq!(store(&tw).pending_notices(r.officer).len(), 1);
    // The officer's own accept happens after the restart, from a connected session —
    // and the first thing they read is what they missed.
    let ev = tw.connect(r.officer);
    let lines = seen(&tw, &ev, r.officer);
    assert!(
        lines.contains("ECONOMY: 1 notice while you were away."),
        "no reconnect header: {lines}"
    );
    assert!(
        lines.contains(&format!(
            "Contract #{id} has been offered to your organization"
        )),
        "the queued line was not rendered: {lines}"
    );
    assert!(
        store(&tw).pending_notices(r.officer).is_empty(),
        "the queue was not cleared once read"
    );
    let cleared = tw
        .take_module_ops()
        .into_iter()
        .any(|op| matches!(op.op, ModuleWrite::Del { key } if key == mush_economy::notice_key(r.officer)));
    assert!(cleared, "the notice doc was not deleted once read");
    // A second connect renders nothing twice.
    let ev = tw.connect(r.officer);
    assert!(!seen(&tw, &ev, r.officer).contains("ECONOMY:"));
}

#[test]
fn e6s1_s9_the_queue_caps_hold() {
    let mut tw = boot();
    let r = rig(&mut tw, false);
    ok(
        &mut tw,
        GOD,
        &format!("@inboxprobe #{}=40 long", r.officer.0),
    );
    let queued = store(&tw).pending_notices(r.officer);
    assert_eq!(
        queued.len(),
        mush_economy::MAX_PENDING_NOTICES,
        "the per-character cap did not hold"
    );
    assert!(
        queued
            .iter()
            .all(|n| n.text.len() <= mush_economy::MAX_NOTICE_BYTES),
        "a line over the byte cap was queued"
    );
    // Oldest dropped, newest kept, the duplicate queued once.
    assert!(!queued.iter().any(|n| n.text == "notice 0"));
    assert!(!queued.iter().any(|n| n.text.starts_with("LLLL")));
    assert_eq!(
        queued.iter().filter(|n| n.text == "notice 39").count(),
        1,
        "a duplicate was queued twice"
    );
    // Loading re-checks both caps: an oversized entry is dropped, an over-cap list
    // keeps its newest entries.
    let mut all = drain_all(&mut tw);
    let key = mush_economy::notice_key(r.officer);
    for (k, v) in all.entry(SmolStr::new(NS)).or_default() {
        if *k == key {
            let mut list = v.as_array().cloned().unwrap_or_default();
            list.insert(
                0,
                serde_json::json!({ "at": 1, "text": "X".repeat(mush_economy::MAX_NOTICE_BYTES + 1) }),
            );
            for i in 0..10 {
                list.insert(
                    0,
                    serde_json::json!({ "at": 1, "text": format!("stale {i}") }),
                );
            }
            *v = serde_json::Value::Array(list);
        }
    }
    reboot_all(&mut tw, all);
    let loaded = store(&tw).pending_notices(r.officer);
    assert_eq!(loaded.len(), mush_economy::MAX_PENDING_NOTICES);
    assert!(!loaded.iter().any(|n| n.text.starts_with("XXXX")));
    assert!(!loaded.iter().any(|n| n.text.starts_with("stale")));
    assert!(loaded.iter().any(|n| n.text == "notice 39"));
    // Rendered in one header plus the lines, then gone.
    let ev = tw.connect(r.officer);
    let lines = seen(&tw, &ev, r.officer);
    assert!(
        lines.contains("ECONOMY: 32 notices while you were away."),
        "{lines}"
    );
    assert!(store(&tw).pending_notices(r.officer).is_empty());
}

// ======================================================================
// ---- E6a step 1 verify fold ----
//
// Lens D, the disclosure-and-bounds half of the org inbox: being told and being
// able to read must be one question, the fan-out must be bounded, a queued line
// must be re-gated when it is finally read, and a hidden organization stays
// hidden. Plus R6: a session held across a restart still reads its queue.
// ======================================================================

/// The one organization these rigs make.
fn the_org(tw: &TestWorld) -> mush_org::OrgId {
    tw.world()
        .module_state::<OrgStore>()
        .expect("org store")
        .state()
        .orgs()
        .next()
        .expect("the org")
        .id
}

#[test]
fn vf_d1_the_obligation_inbox_never_outruns_the_obligation_view_gate() {
    let mut tw = boot();
    let r = rig(&mut tw, true);
    ok(
        &mut tw,
        GOD,
        &format!(
            "@obligation/create {}={} 10 every=100 start=100",
            r.treasury, r.payee_acct
        ),
    );
    let id = store(&tw)
        .snapshot()
        .obligations()
        .map(|o| o.id)
        .max()
        .expect("the obligation");

    // The officer holds `economy.account.post` on the org root and nothing else:
    // exactly who `principal_holders` fans an obligation line out to. The gate
    // compared the debtor to the actor by IDENTITY, so an organization was "a party"
    // no person matched — the officer was told "Obligation #N is delinquent." and was
    // then answered "No such obligation." by the very command the line points at.
    assert!(
        mush_economy::may_view_obligation(tw.world(), r.officer, id),
        "the holder the inbox writes to cannot read the record it names"
    );
    let seen = ok(&mut tw, r.officer, &format!("@obligation/show {id}"));
    assert!(seen.contains(&format!("obligation #{id}")), "{seen}");
    // A member holding no spending right is told nothing and sees nothing: the two
    // answers agree in this direction too.
    assert!(!mush_economy::may_view_obligation(tw.world(), r.member, id));
    let refused = tw.run(r.member, &format!("@obligation/show {id}"));
    assert!(
        texts(&refused).contains("No such obligation."),
        "{}",
        texts(&refused)
    );

    // The `ObligationChanged` branch gets its first tests: a line to a holder, and
    // silence to a member of the same organization who may not read the record.
    let ev = run(&mut tw, GOD, &format!("@obligation/cancel {id}"));
    assert_eq!(
        inbox_lines(&tw, &ev, r.officer),
        vec![format!("Obligation #{id} is cancelled.")]
    );
    assert!(
        inbox_lines(&tw, &ev, r.member).is_empty(),
        "a non-holder was told about the org's debt"
    );
}

#[test]
fn vf_d9_a_created_obligation_is_not_reported_as_back_on_schedule() {
    let mut tw = boot();
    let r = rig(&mut tw, true);
    // `@obligation/create` emits `ObligationChanged { Active }`, and the inbox had
    // exactly one line for `Active`: "is back on schedule" — about a record that had
    // never been anywhere.
    let ev = run(
        &mut tw,
        GOD,
        &format!(
            "@obligation/create {}={} 10 every=100 start=100",
            r.treasury, r.payee_acct
        ),
    );
    let id = store(&tw)
        .snapshot()
        .obligations()
        .map(|o| o.id)
        .max()
        .expect("the obligation");
    assert_eq!(
        inbox_lines(&tw, &ev, r.officer),
        vec![format!(
            "Obligation #{id} has been created (@obligation/show #{id})."
        )]
    );

    // A record that really has been somewhere still says so: suspended, then resumed.
    let ev = run(&mut tw, GOD, &format!("@obligation/suspend {id}=probe"));
    assert_eq!(
        inbox_lines(&tw, &ev, r.officer),
        vec![format!("Obligation #{id} is suspended pending a decision.")]
    );
    let ev = run(&mut tw, GOD, &format!("@obligation/resume {id}"));
    assert_eq!(
        inbox_lines(&tw, &ev, r.officer),
        vec![format!("Obligation #{id} is back on schedule.")]
    );
}

#[test]
fn vf_d3_one_event_fans_out_to_at_most_the_holder_cap() {
    let mut tw = boot();
    let r = rig(&mut tw, true);
    // Forty more officers, all holding the spending right, none of them connected:
    // one `ContractChanged` used to be one durable `econ:notice:{obj}` write each,
    // inside the emitting post's own batch.
    let mut crowd: Vec<ObjId> = Vec::new();
    for i in 0..40 {
        let who = tw
            .world_mut()
            .create(ObjKind::Player, &format!("Clerk{i}"), GOD, Some(LIMBO))
            .expect("create");
        ok(
            &mut tw,
            GOD,
            &format!("@org/grant #{}=Ironhold economy.account.post", who.0),
        );
        crowd.push(who);
    }
    let org = the_org(&tw);
    let holders = mush_economy::principal_holders(tw.world(), &PrincipalId::Organization(org));
    assert_eq!(
        holders.len(),
        mush_economy::MAX_NOTICE_HOLDERS,
        "the fan-out is unbounded: {} holders",
        holders.len()
    );

    let (_id, _ev) = org_offer(&mut tw, &r, "Crowded");
    let queued = crowd
        .iter()
        .filter(|w| !store(&tw).pending_notices(**w).is_empty())
        .count();
    assert!(
        queued <= mush_economy::MAX_NOTICE_HOLDERS,
        "one event queued {queued} documents"
    );
}

#[test]
fn vf_d6_a_queued_line_is_re_gated_against_live_standing_at_delivery() {
    let mut tw = boot();
    let r = rig(&mut tw, false);
    assert!(!tw.world().is_connected(r.officer));
    let (_id, _) = org_offer(&mut tw, &r, "Supply");
    assert_eq!(
        store(&tw).pending_notices(r.officer).len(),
        1,
        "the absent officer's line was not queued"
    );

    // Standing was checked when the line was EMITTED. A queue is a delay, and an
    // officer whose grant went away while they were gone used to connect three weeks
    // later and read every contract transition the organization had had since.
    let mut all = drain_all(&mut tw);
    let org_ns = SmolStr::new(mush_org::NS);
    let officer = r.officer;
    all.entry(org_ns).or_default().retain(|(k, v)| {
        !(k.starts_with("grant:")
            && v["grantee"] == serde_json::json!(officer.0)
            && v["capability"] == serde_json::json!("economy.account.post"))
    });
    reboot_all(&mut tw, all);
    assert!(
        !mush_economy::acts_for(
            tw.world(),
            officer,
            &PrincipalId::Organization(the_org(&tw)),
        ),
        "the grant survived the rewrite; the test proves nothing"
    );
    assert_eq!(
        store(&tw).pending_notices(officer).len(),
        1,
        "the queue itself did not survive"
    );

    let ev = tw.connect(officer);
    let lines = seen(&tw, &ev, officer);
    assert!(
        !lines.contains("ECONOMY:"),
        "a revoked officer read the organization's backlog: {lines}"
    );
    assert!(
        store(&tw).pending_notices(officer).is_empty(),
        "the dropped queue was not cleared"
    );
}

#[test]
fn vf_d7_a_hidden_organization_is_never_named_to_an_outsider() {
    let mut tw = boot();
    let r = rig(&mut tw, true);
    let stranger = tw.spawn_player("Stranger");
    let org = the_org(&tw);
    // A covert backer. `render_principal` named it unconditionally, and
    // `contract_notice`'s `announce=yes` line renders the issuer to an AUDIENCE —
    // people outside the terms by definition (08f G3).
    let mut all = drain_all(&mut tw);
    let org_ns = SmolStr::new(mush_org::NS);
    for (k, v) in all.entry(org_ns).or_default() {
        if k.starts_with("org:") {
            v["hidden"] = serde_json::json!(true);
        }
    }
    reboot_all(&mut tw, all);
    assert!(
        tw.world()
            .module_state::<OrgStore>()
            .expect("org store")
            .state()
            .org(org)
            .is_some_and(|o| o.hidden),
        "the org did not come back hidden"
    );

    // A public contract that announces its issuer — the shape the fold is about.
    ok(
        &mut tw,
        GOD,
        &format!("@contract/draft Covert scope=#{}", r.root.0),
    );
    let id = store(&tw)
        .snapshot()
        .contracts()
        .map(|c| c.id)
        .max()
        .expect("the draft");
    ok(
        &mut tw,
        GOD,
        &format!("@contract/party {id}=#{}", r.payee.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/pay {id}={} 100", r.treasury),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/split {id}={} 1000‰", r.payee_acct),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/remainder {id}={}", r.treasury),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/condition {id}=attest staff"),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/visibility {id}=public announce=yes"),
    );
    ok(&mut tw, GOD, &format!("@contract/offer {id}"));

    // A member reads the name: hiding is about non-members (org §12.2), and an
    // officer is who the inbox writes to in the first place.
    let inside = ok(&mut tw, r.officer, &format!("@contract/show {id}"));
    assert!(
        inside.contains("Ironhold"),
        "a member lost their own organization: {inside}"
    );
    // Everybody else sees a shape, never a name — the payee included. A hidden
    // organization that contracts with you has chosen to be covert, and the surface
    // it issued is what the counterparty consented to, not its name.
    for outsider in [stranger, r.payee] {
        let outside = ok(&mut tw, outsider, &format!("@contract/show {id}"));
        assert!(
            !outside.contains("Ironhold"),
            "a hidden organization was named to a non-member: {outside}"
        );
        assert!(
            outside.contains("an organization"),
            "the hidden issuer did not render as a shape: {outside}"
        );
    }
}

#[test]
fn vf_r6_a_reattached_session_reads_its_queue_on_the_next_sweep() {
    let mut tw = boot();
    let r = rig(&mut tw, false);
    tw.set_now(1_000);
    ok(&mut tw, GOD, &format!("@inboxprobe #{}=1", r.officer.0));
    assert_eq!(store(&tw).pending_notices(r.officer).len(), 1);

    // `Engine::reattach` fires no `Connected` hook, so a session held across a
    // restart never triggered `render_pending`: the queue sat in its document until
    // the player quit and logged in again. The economy's own sweep is the catch-up.
    tw.world_mut().set_connected(r.officer, true);
    tw.set_now(1_100);
    let ev = tw.fire_due(1_100 * 1_000);
    let lines = seen(&tw, &ev, r.officer);
    assert!(
        lines.contains("ECONOMY: 1 notice while you were away."),
        "the sweep did not deliver a standing queue: {lines}"
    );
    assert!(store(&tw).pending_notices(r.officer).is_empty());
    // ...and it says it once.
    tw.set_now(1_200);
    let ev = tw.fire_due(1_200 * 1_000);
    assert!(!seen(&tw, &ev, r.officer).contains("ECONOMY:"));
}
