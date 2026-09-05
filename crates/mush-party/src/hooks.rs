//! The one world-event observer this crate registers (E6a).
//!
//! A destroyed object cannot be on a roster: core's destroy dispersal honours no hook
//! of ours, so a party holding a dead member would keep answering `+party` with a
//! name nothing resolves and would count that member against
//! [`MAX_PARTY_SIZE`](crate::MAX_PARTY_SIZE) forever. `ObjDestroyed` therefore drops
//! them from every live party — roster, invitations and role label alike — through
//! the record's own [`Party::remove_member`], so the last member leaving still
//! disbands `Empty` and a leaving leader still hands the lead on.
//!
//! **A `Locked` party is not touched.** Its roster is frozen against a contract this
//! crate cannot read (D-E6-5), and silently editing it would be the one thing the
//! lock exists to prevent; the ruleset finds out when it settles, which is where the
//! knowledge to act on it lives. The party is left exactly as it is.
//!
//! Panic-free, exhaustive, and bounded by the live projection, like the tick.

use mush_core::{Ctx, WorldEvent};

use crate::module::{PartyStore, put_party};
use crate::record::Party;

/// Drop a destroyed object from every live, unlocked party.
pub(crate) fn on_obj_destroyed(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::ObjDestroyed { obj } = ev else {
        return;
    };
    let gone = *obj;
    let touched: Vec<Party> = match ctx.w.module_state::<PartyStore>() {
        Some(store) => store
            .live_parties()
            .filter(|p| p.is_forming() && (p.is_member(gone) || p.is_invited(gone)))
            .cloned()
            .collect(),
        None => return,
    };
    for mut p in touched {
        p.invited.retain(|i| i.who != gone);
        let _: bool = p.remove_member(gone);
        p.bump();
        put_party(ctx, p);
    }
}
