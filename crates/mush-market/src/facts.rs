//! Doc 18 cut IC7 — the `market.*` fact provider (L10).
//!
//! D-E7-30 gives each crate exactly one namespace and `EconomyModule::with_facts`
//! **panics** on a duplicate; the market had none until a Draw sale needed one, so this
//! is the sixth, `market`, with one kind, [`FACT_LOT_DRAWN`]. `mush_sim`'s
//! `sim.station.served` is the precedent: an applied marker written in the
//! authoritative bundle, a RAM mirror, and a provider that answers from the mirror.
//!
//! **Answered from the mirror of a durable receipt, never from a status.** The Draw
//! writes its `market_lot_draws` receipt inside its own bundle, so a Draw that did not
//! commit leaves none; the market sweep's mirror arm (`boot::reconcile_draws`) is the
//! only writer of `Quote::drawn_at`, and this answers only from that. The fact is
//! therefore **behind** the receipt by at most the mirror's rotation and never ahead —
//! the inventory mirror's rule (`mush-inventory/src/facts.rs`) — because
//! `FactProvider::resolve` receives `&World` and typed state lives on the engine.
//!
//! **Who can see the id, truthfully (F8).** The id is `"{quote}/{accepted_at}"`, where
//! `accepted_at` is the real `World::now()` second the deal came into force. A purchase
//! contract is `Parties`-visible and `@contract/show` renders a fact condition as
//! `the fact market/market.lot.drawn/<id>`, so the contract's own parties read it; a
//! third party cannot derive it without the accept instant. Naming the fact in a
//! contract of one's own gains nothing either: it answers only from the receipt mirror,
//! i.e. only once the market itself has seen those goods move.

use mush_core::World;
use mush_economy::{FactProvider, FactState};
use mush_org::PrincipalId;

use crate::ids::QuoteId;
use crate::record::{FACT_LOT_DRAWN, PaidKind};
use crate::store::{MarketStore, NS};

/// The one `market.*` provider this crate registers (doc 18 cut IC7). The game's root
/// wires it into `EconomyModule::with_facts` in cut IC8; a composition with Draw listings
/// and without it refuses every Draw sale before money moves.
#[derive(Clone, Copy, Debug, Default)]
pub struct MarketFactProvider;

impl FactProvider for MarketFactProvider {
    fn provider(&self) -> &str {
        NS
    }

    fn resolve(&self, w: &World, kind: &str, id: &str, revision: Option<u64>) -> Option<FactState> {
        // A receipt is written once and never amended, so any named revision other
        // than 1 is a reference to a world this crate never had.
        if kind != FACT_LOT_DRAWN || revision.is_some_and(|revision| revision != 1) {
            return None;
        }
        let mut parts = id.split('/');
        let (Some(quote), Some(accepted_at), None) = (parts.next(), parts.next(), parts.next())
        else {
            return None;
        };
        let quote = QuoteId(quote.parse::<u64>().ok()?);
        let accepted_at = accepted_at.parse::<i64>().ok()?;
        let q = w.module_state::<MarketStore>()?.quote(quote)?;
        let lot = q.lot.as_ref()?;
        if lot.kind != PaidKind::Draw || lot.accepted_at != accepted_at {
            return None;
        }
        let at = q.drawn_at?;
        let actor = match q.buyer {
            PrincipalId::Object(buyer) => Some(buyer),
            PrincipalId::Organization(_) | PrincipalId::System(_) => None,
        };
        Some(FactState {
            satisfied: true,
            actor,
            // Real seconds: when the market's mirror learned of the move, behind the
            // Draw itself by at most the mirror lag.
            at,
        })
    }
}
