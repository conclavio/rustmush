//! Private notices, and the bounded queue for the ones nobody was there to read
//! (E5b, 08d §4's "notices for offline players").
//!
//! Owner answer 8 is the constraint the whole file exists for: **a home must not
//! disappear because a player was offline**, and the corollary is that the notices
//! that say so must still be there when they come back. `mush_mail::deliver` is
//! crate-private, so housing keeps its own list — the `mush-mail` *pattern*, not its
//! code: queue while away, render once on `Connected`.
//!
//! Three bounds, all of them the difference between a notice and a nuisance:
//!
//! - **at most [`crate::MAX_PENDING_NOTICES`] queued per object**, oldest dropped
//!   (the FIFO every bounded tail in the E-series uses). A tenancy in trouble for a
//!   year must not greet its player with four hundred lines.
//! - **at most [`crate::MAX_NOTICE_BYTES`] per line**, because every stored string in
//!   this family is bounded.
//! - **no duplicates**: the ladder already sends one notice per transition and the
//!   window's decay already governs the rest, so an identical line still queued is a
//!   retry, not news.

use mush_core::{Ctx, ModuleWrite, ObjId};
use smol_str::SmolStr;

use crate::module::{
    HousingStore, MAX_NOTICE_BYTES, MAX_PENDING_NOTICES, PendingNotice, notice_key, push_op,
};

/// Tell one person something about their home.
///
/// Connected → straight to them. Not connected → queued, durably, for
/// [`render_pending`] to deliver at their next login. There is deliberately no third
/// case: a notice that is neither shown nor kept is a notice that did not happen, and
/// the whole point of the story window is that the tenant was told.
pub(crate) fn notify(ctx: &mut Ctx<'_>, who: ObjId, msg: impl Into<String>) {
    let msg = clamp(msg.into());
    if ctx.w.is_connected(who) {
        ctx.out.sys_to_actor(who, "housing", msg);
        return;
    }
    queue(ctx, who, msg);
}

/// Queue a notice without trying to deliver it (the boot and reconciliation paths,
/// where there is nobody attached to anything yet).
pub(crate) fn queue(ctx: &mut Ctx<'_>, who: ObjId, msg: impl Into<String>) {
    let text = SmolStr::new(clamp(msg.into()));
    let at = ctx.w.now();
    let doc = {
        let Some(store) = ctx.w.module_state_mut::<HousingStore>() else {
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
    push_op(
        ctx.out,
        ModuleWrite::JsonSet {
            key: notice_key(who),
            value: serde_json::to_value(&doc).expect("pending notices serialize"),
        },
    );
}

/// Deliver and clear one object's queued notices — the `Connected` hook's whole job.
///
/// Clearing is a **delete**, not an empty list: a player with nothing pending should
/// own no document at all, so the store does not accumulate one husk per character
/// who was ever behind on rent.
pub(crate) fn render_pending(ctx: &mut Ctx<'_>, who: ObjId) {
    let list = {
        let Some(store) = ctx.w.module_state_mut::<HousingStore>() else {
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
        "housing",
        format!(
            "HOME: {n} notice{} while you were away.",
            if n == 1 { "" } else { "s" }
        ),
    );
    for note in &list {
        ctx.out.sys_to_actor(who, "housing", note.text.to_string());
    }
    push_op(
        ctx.out,
        ModuleWrite::Del {
            key: notice_key(who),
        },
    );
}

/// What `+home` prints as "the pending notice" — the most recent queued line, if the
/// tenant has one that has not been rendered yet.
#[must_use]
pub(crate) fn latest_pending(store: &HousingStore, who: ObjId) -> Option<&PendingNotice> {
    store.notices.get(&who).and_then(|list| list.last())
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
