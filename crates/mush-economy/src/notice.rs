//! Notices for the people who act for a principal the economy has something to
//! tell, and the bounded queue for the ones who were not there to read it (E6a
//! step 1, DL-04 — the housing `notice.rs` pattern, economy-owned and game-neutral).
//!
//! The economy's own notices reach `PrincipalId::Object` parties only
//! (`contract_live::notify`, `sweep::notify_principal`): an organization has no
//! inbox, so an org payer was told nothing when its run was crewed, settled,
//! disputed or refunded (08f review G5). The server's `OrgInbox` observer turns the
//! economy's kinds-only events into one line per **holder of `acts_for`** on the
//! org — `economy.account.post` on its root compartment, the same predicate that
//! decides who may spend its money and see its escrow — and delivers it through
//! [`deliver_notice`]: straight to a connected holder, queued durably for one who is
//! away and rendered at their next connect.
//!
//! Three bounds, all of them the difference between a notice and a nuisance
//! (D-LBK-3's discipline):
//!
//! - **at most [`MAX_PENDING_NOTICES`] queued per object**, oldest dropped;
//! - **at most [`MAX_NOTICE_BYTES`] per line**, re-checked on load through a
//!   `TryFrom` wire, so a hand-written document cannot greet a player with a
//!   paragraph;
//! - **no duplicates**: an identical line still queued is a retry, not news.
//!
//! Nobody acts for a `System` principal, staff included ([`crate::api::WorldAuth`]),
//! so a system party is told nothing here either — its notices are the staff
//! channel's (DL-05).

use std::sync::Arc;

use mush_core::{Ctx, EventHook, ModuleWrite, ObjId, World};
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::api;
use crate::module::EconomyStore;

/// The most notices queued for one object while they are away.
pub const MAX_PENDING_NOTICES: usize = 32;
/// The most bytes one queued notice carries.
pub const MAX_NOTICE_BYTES: usize = 240;
/// The most holders **one event** may fan out to (E6a step 1 verify fold, D3).
///
/// One `ContractChanged` on a contract naming a large organization was one durable
/// `econ:notice:{obj}` write per absent holder, and a contract may name up to
/// [`crate::MAX_PARTIES`] org principals: a single amendment of a big guild's
/// contract could mean hundreds of document writes inside one post's batch. Thirty-two
/// is the same number as the per-object queue, taken in id order so the audience is
/// stable across events, and the truncation is logged once per event.
pub const MAX_NOTICE_HOLDERS: usize = 32;
/// The most objects one sweep fire delivers a standing queue to (R6).
pub const MAX_NOTICE_DELIVERIES_PER_SWEEP: usize = 32;

/// `econ:notice:{obj}` — a **leaf** doc (the offline queue): a corrupt one is
/// skipped loudly, which costs a player a notice about a fact they can still read
/// off `@contract/show`.
#[must_use]
pub fn notice_key(who: ObjId) -> SmolStr {
    SmolStr::new(format!("econ:notice:{}", who.0))
}

/// One queued notice for an object that was not connected when it was sent.
///
/// Loading re-checks the byte cap: an oversized entry refuses to deserialize and
/// the loader drops it with a log line (notices are ephemeral) and keeps the rest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "PendingNoticeWire")]
pub struct PendingNotice {
    /// Wall clock, for the render.
    pub at: i64,
    /// The text (≤ [`MAX_NOTICE_BYTES`]).
    pub text: SmolStr,
    /// **Whose business this line was** (E6a step 1 verify fold, D6), when it was
    /// queued on behalf of a principal rather than said to the object about itself.
    ///
    /// Standing was checked when the line was *emitted*; a queue is a delay, and an
    /// officer removed from an organization while they were away used to connect
    /// three weeks later and read every contract transition the org had had since.
    /// [`render_pending`] re-asks [`acts_for`] at delivery and drops what no longer
    /// holds. `None` — every notice written before this field, and every notice an
    /// object is told about itself — is always delivered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<PrincipalId>,
}

#[derive(Deserialize)]
struct PendingNoticeWire {
    at: i64,
    text: SmolStr,
    #[serde(default)]
    about: Option<PrincipalId>,
}

impl TryFrom<PendingNoticeWire> for PendingNotice {
    type Error = String;

    fn try_from(w: PendingNoticeWire) -> Result<PendingNotice, String> {
        if w.text.len() > MAX_NOTICE_BYTES {
            return Err(format!(
                "a pending economy notice is at most {MAX_NOTICE_BYTES} bytes ({} given)",
                w.text.len()
            ));
        }
        Ok(PendingNotice {
            at: w.at,
            text: w.text,
            about: w.about,
        })
    }
}

/// Bound a notice on a char boundary, so a stored line always round-trips.
fn clamp(mut msg: String) -> String {
    if msg.len() > MAX_NOTICE_BYTES {
        let mut cut = MAX_NOTICE_BYTES;
        while cut > 0 && !msg.is_char_boundary(cut) {
            cut -= 1;
        }
        msg.truncate(cut);
    }
    msg
}

/// Tell one person something the economy did.
///
/// Connected → straight to them. Not connected → queued, durably, for the next
/// connect. There is deliberately no third case: a notice that is neither shown nor
/// kept is a notice that did not happen.
pub fn deliver_notice(ctx: &mut Ctx<'_>, who: ObjId, msg: impl Into<String>) {
    deliver_notice_about(ctx, who, None, msg);
}

/// [`deliver_notice`] for a line `who` is being told **on behalf of `about`** (D6):
/// the standing that earned them the line is re-checked when a queued copy is
/// finally read.
pub fn deliver_notice_for(
    ctx: &mut Ctx<'_>,
    who: ObjId,
    about: &PrincipalId,
    msg: impl Into<String>,
) {
    deliver_notice_about(ctx, who, Some(about.clone()), msg);
}

fn deliver_notice_about(
    ctx: &mut Ctx<'_>,
    who: ObjId,
    about: Option<PrincipalId>,
    msg: impl Into<String>,
) {
    let msg = clamp(msg.into());
    if ctx.w.is_connected(who) {
        ctx.out.sys_to_actor(who, "economy", msg);
        return;
    }
    queue_notice_about(ctx, who, about, msg);
}

/// Queue a notice without trying to deliver it.
pub fn queue_notice(ctx: &mut Ctx<'_>, who: ObjId, msg: impl Into<String>) {
    queue_notice_about(ctx, who, None, msg);
}

fn queue_notice_about(
    ctx: &mut Ctx<'_>,
    who: ObjId,
    about: Option<PrincipalId>,
    msg: impl Into<String>,
) {
    let text = SmolStr::new(clamp(msg.into()));
    let at = ctx.w.now();
    let doc = {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else {
            return;
        };
        let list = store.notices.entry(who).or_default();
        if list.iter().any(|n| n.text == text) {
            return;
        }
        while list.len() >= MAX_PENDING_NOTICES {
            list.remove(0);
        }
        list.push(PendingNotice { at, text, about });
        list.clone()
    };
    write_queue(ctx, who, &doc);
}

/// Persist one object's queue. R9: the serialization is fallible in principle and
/// this runs on observer and tick paths, so a failure is a log line and a dropped
/// notice — never an `expect` that would abort the world task mid-sweep.
fn write_queue(ctx: &mut Ctx<'_>, who: ObjId, doc: &[PendingNotice]) {
    let Ok(value) = serde_json::to_value(doc) else {
        tracing::error!(
            who = who.0,
            "economy: a pending-notice queue would not serialize; the notice is dropped"
        );
        return;
    };
    api::push_op(
        ctx.out,
        ModuleWrite::JsonSet {
            key: notice_key(who),
            value,
        },
    );
}

/// Deliver and clear one object's queued notices — the `Connected` hook's whole job.
///
/// Clearing is a **delete**, not an empty list: a player with nothing pending owns no
/// document at all.
pub(crate) fn render_pending(ctx: &mut Ctx<'_>, who: ObjId) {
    let queued = {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else {
            return;
        };
        match store.notices.remove(&who) {
            Some(list) if !list.is_empty() => list,
            _ => return,
        }
    };
    // D6: standing was checked when the line was emitted; the queue is a delay, so
    // ask again now. An officer removed from an organization while they were away
    // reads nothing about it, however long the queue sat.
    let queued_len = queued.len();
    let list: Vec<PendingNotice> = queued
        .into_iter()
        .filter(|n| {
            n.about
                .as_ref()
                .is_none_or(|about| acts_for(ctx.w, who, about))
        })
        .collect();
    let dropped = queued_len - list.len();
    if dropped > 0 {
        tracing::info!(
            who = who.0,
            dropped,
            "economy: dropped queued notices whose standing no longer holds"
        );
    }
    if list.is_empty() {
        api::delete_key(ctx.out, notice_key(who));
        return;
    }
    let n = list.len();
    ctx.out.sys_to_actor(
        who,
        "economy",
        format!(
            "ECONOMY: {n} notice{} while you were away.",
            if n == 1 { "" } else { "s" }
        ),
    );
    for note in &list {
        ctx.out.sys_to_actor(who, "economy", note.text.to_string());
    }
    api::delete_key(ctx.out, notice_key(who));
}

/// Deliver every standing queue whose owner is **already connected** (E6a step 1
/// verify fold, R6) — the sweep's own catch-up pass.
///
/// `Engine::reattach` does not fire `Connected`: a session held across a restart is
/// live from the world's point of view and never triggers [`ReconnectNotice`], so a
/// notice queued before the restart sat in its document until the player quit and
/// came back. The sweep is the economy's own periodic reach, so it closes it here
/// rather than asking the server for a second reconnect hook. Bounded by
/// [`MAX_NOTICE_DELIVERIES_PER_SWEEP`] per fire, oldest object id first; a
/// disconnected owner is left exactly where they were.
pub(crate) fn deliver_connected(ctx: &mut Ctx<'_>) {
    let waiting: Vec<ObjId> = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else {
            return;
        };
        store
            .notices
            .iter()
            .filter(|(_, list)| !list.is_empty())
            .map(|(who, _)| *who)
            .collect()
    };
    let mut delivered = 0usize;
    for who in waiting {
        if delivered >= MAX_NOTICE_DELIVERIES_PER_SWEEP {
            break;
        }
        if !ctx.w.is_connected(who) {
            continue;
        }
        delivered = delivered.saturating_add(1);
        render_pending(ctx, who);
    }
}

/// Renders whatever the actor missed while they were away — and, for a
/// Wizard-or-better, the ladder's open pages nobody has been told about (R4 + R5).
#[derive(Debug)]
pub(crate) struct ReconnectNotice;

impl EventHook for ReconnectNotice {
    fn connected(&self, ctx: &mut Ctx<'_>, actor: ObjId) {
        render_pending(ctx, actor);
        // E6a step 1 verify fold (R4 + R5): a page opened while no staffer was
        // connected reached nobody, and boot's own reconciliation records announced
        // nothing at all. Re-announcing the open backlog to the first staff arrival
        // hands both to a human; the server's fan-out dedupes on each record's step,
        // so nothing already read is said again. Same predicate the server's own
        // `connected_wizards` uses.
        if mush_core::perm::has_role(ctx.w, actor, mush_core::Role::Wizard) {
            crate::sweep::announce_open_escalations(ctx, crate::sweep::MAX_ANNOUNCE_BACKLOG);
        }
    }
}

/// The `Connected` hook, for the module's registration.
pub(crate) fn reconnect_hook() -> Arc<dyn EventHook> {
    Arc::new(ReconnectNotice)
}

/// Whether `actor` acts for `who` — the predicate that decides who may spend an
/// organization's money, see its escrow, and (DL-04) is told what its contracts
/// did: the object itself (or its MUSH controller, or staff) for an `Object`;
/// `economy.account.post` on the root compartment (or staff) for an
/// `Organization`; **nobody, staff included**, for a `System` principal.
#[must_use]
pub fn acts_for(w: &World, actor: ObjId, who: &PrincipalId) -> bool {
    (api::WorldAuth { w }).acts_for(&PrincipalId::Object(actor), who)
}

/// **Who is told what happened to `who`** (DL-04): the object itself for an
/// `Object`; for an `Organization`, every member and every grantee of the org who
/// holds `economy.account.post` on its root compartment **by grant** — staff are not
/// enumerated, because "acts for by fiat" is not "should be told about every org's
/// money" — in id order, deduplicated; nobody for a `System` principal.
///
/// **Bounded** at [`MAX_NOTICE_HOLDERS`] (E6a step 1 verify fold, D3): every holder
/// who is away costs a durable `econ:notice:{obj}` write inside the emitting post's
/// own batch, and one contract may name many organizations. Past the cap the list is
/// truncated in id order and the truncation is logged — a bound on how loud one event
/// can be, not on who may read the record.
#[must_use]
pub fn principal_holders(w: &World, who: &PrincipalId) -> Vec<ObjId> {
    match who {
        PrincipalId::Object(o) => vec![*o],
        PrincipalId::System(_) => Vec::new(),
        PrincipalId::Organization(org) => {
            let Some(store) = w.module_state::<mush_org::OrgStore>() else {
                return Vec::new();
            };
            let state = store.state();
            let Some(root) = state.org(*org).map(|o| o.root_scope) else {
                return Vec::new();
            };
            if !api::scope_coherent(w, Some(root), who) {
                return Vec::new();
            }
            let mut out: std::collections::BTreeSet<ObjId> = std::collections::BTreeSet::new();
            let candidates = state
                .members_in(*org)
                .map(|m| m.member)
                .chain(state.grants_in(*org).map(|g| g.grantee));
            for obj in candidates {
                if api::holds_capability(w, obj, "economy.account.post", root) {
                    out.insert(obj);
                }
            }
            if out.len() > MAX_NOTICE_HOLDERS {
                tracing::warn!(
                    org = ?who,
                    holders = out.len(),
                    cap = MAX_NOTICE_HOLDERS,
                    "economy: an organization's notice fan-out was truncated to its cap"
                );
            }
            out.into_iter().take(MAX_NOTICE_HOLDERS).collect()
        }
    }
}
