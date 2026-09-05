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
}

#[derive(Deserialize)]
struct PendingNoticeWire {
    at: i64,
    text: SmolStr,
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
    let msg = clamp(msg.into());
    if ctx.w.is_connected(who) {
        ctx.out.sys_to_actor(who, "economy", msg);
        return;
    }
    queue_notice(ctx, who, msg);
}

/// Queue a notice without trying to deliver it.
pub fn queue_notice(ctx: &mut Ctx<'_>, who: ObjId, msg: impl Into<String>) {
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
        list.push(PendingNotice { at, text });
        list.clone()
    };
    api::push_op(
        ctx.out,
        ModuleWrite::JsonSet {
            key: notice_key(who),
            value: serde_json::to_value(&doc).expect("pending notices serialize"),
        },
    );
}

/// Deliver and clear one object's queued notices — the `Connected` hook's whole job.
///
/// Clearing is a **delete**, not an empty list: a player with nothing pending owns no
/// document at all.
pub(crate) fn render_pending(ctx: &mut Ctx<'_>, who: ObjId) {
    let list = {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else {
            return;
        };
        match store.notices.remove(&who) {
            Some(list) if !list.is_empty() => list,
            _ => return,
        }
    };
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

/// Renders whatever the actor missed while they were away.
#[derive(Debug)]
pub(crate) struct ReconnectNotice;

impl EventHook for ReconnectNotice {
    fn connected(&self, ctx: &mut Ctx<'_>, actor: ObjId) {
        render_pending(ctx, actor);
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
            out.into_iter().collect()
        }
    }
}
