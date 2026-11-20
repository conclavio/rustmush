//! **The law asked for a venue and a good** (doc 50 unit LW.1; Q-50-32, Q-50-33, Q-50-34,
//! Q-50-65 as ruled, doc 50a A-3, B-4, C-1).
//!
//! A venue's rule for a good is the law of its region's de facto holder: the `restricted` list of
//! that holder's terms row (KC.2's schedule, [`crate::polity::terms_at`]). It is asked at the
//! moment of a public trade, through the market's proceeds policy
//! (`mush_market::ProceedsPolicy::may_trade`), of either side: a seller into a public bid, a buyer
//! at a public listing, the poster of a public ask or bid. A private offer between two parties is
//! never asked (Q-50-34): it goes through, and is an offense only if it comes to light.
//!
//! - **The papers** ([`papers_clear`], Q-50-65): one test, the picket's. A restricted good whose
//!   paper is [`PICKET_PAPERS`] is lifted by a Freeport Registry prospecting license **and** a
//!   live registered claim on a strategic rock in an orbital cluster, both. A paper this crate
//!   does not know lifts nothing.
//! - **The claims mirror** ([`PapersMirror`]): the claims are an action-state family that only
//!   this module's own lens reads, and the market asks its policy with the world alone. The port
//!   tick therefore copies every live claim on a strategic rock into the store (`refresh`),
//!   and the board reads the copy through the same predicate the picket's live read uses
//!   ([`crate::space::mining::claim_covers`]). The copy is at most one port fire old.

use mush_core::{CanonicalId, Ctx, ObjId, World};
use mush_market::{MarketId, ProceedsRefusal};
use mush_org::PrincipalId;

use crate::surface::claims::BlockClaim;

/// **The picket's papers** (Q-50-65): the paper id a terms row names for "a Freeport prospecting
/// license and a registered claim on a strategic rock".
pub const PICKET_PAPERS: &str = "freeport_prospecting";

/// The board's refusal: `{label}`, `{venue}`.
pub const RESTRICTED_REFUSED: &str = "The {venue} board trades {label} only with a holder of a \
     Freeport Registry prospecting license and a registered claim on a strategic rock; nothing was \
     traded.";

/// The most claims the mirror keeps (a lane's strategic rocks are a few dozen).
pub const MAX_MIRRORED_CLAIMS: usize = 256;

/// **The live claims on strategic rocks**, copied from the claims family on the port tick.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PapersMirror {
    /// The claims, as last read.
    pub claims: Vec<BlockClaim>,
    /// The IC second of the last read; `None` before the first.
    pub read_ic: Option<i64>,
}

/// **Copy the strategic rocks' claims into the store** (the port tick, with this module's own
/// lens): one point read a strategic rock of every orbital cluster.
pub(crate) fn refresh(ctx: &mut Ctx<'_>) {
    let Some(handles) = ctx
        .w
        .module_state::<crate::port::families::PortHandles>()
        .cloned()
    else {
        return;
    };
    let mut claims = Vec::new();
    for cluster in crate::space::mining::all_orbital_clusters(ctx.w) {
        for rock in &cluster.rocks {
            if !rock
                .spectral
                .families()
                .contains(&crate::surface::OreFamily::StrategicElements)
            {
                continue;
            }
            let key = crate::surface::claims::BlockKey {
                site: cluster.id.clone(),
                letter: rock.letter,
            };
            if let Ok(Some(record)) = ctx.action_state(&handles.families.claims, &key)
                && let Some(claim) = record.claim.as_ref()
                && claims.len() < MAX_MIRRORED_CLAIMS
            {
                claims.push(claim.clone());
            }
        }
    }
    let now = ctx.w.world_ic_now().at;
    if let Some(store) = ctx.w.module_state_mut::<crate::store::TacsimStore>() {
        store.papers = PapersMirror {
            claims,
            read_ic: Some(now),
        };
    }
}

/// **Whether `actor` holds the picket's papers** (Q-50-65), read from the mirror: a prospecting
/// license in force (or in its grace) and a live claim on a strategic rock covering them.
#[must_use]
pub fn papers_clear(w: &World, actor: ObjId, now_ic: i64) -> bool {
    if crate::port::licence::licence_gate(w, actor, now_ic).is_err() {
        return false;
    }
    w.module_state::<crate::store::TacsimStore>()
        .is_some_and(|store| {
            store
                .papers
                .claims
                .iter()
                .any(|claim| crate::space::mining::claim_covers(w, claim, actor, now_ic))
        })
}

/// **The lane port entry of `venue`**.
#[must_use]
pub fn port_entry_of(w: &World, venue: &MarketId) -> Option<CanonicalId> {
    crate::port::market::ports(w)
        .into_iter()
        .find(|(_, def, _)| def.venue.as_str() == venue.as_str())
        .map(|(entry, _, _)| entry)
}

/// **The paper that lifts `commodity`'s restriction at `venue`**, when the venue's de facto
/// holder restricts it: the restricted row of its terms (KC.2). `None` where it is free.
#[must_use]
pub fn restriction_at(w: &World, venue: &MarketId, commodity: &CanonicalId) -> Option<CanonicalId> {
    let entry = port_entry_of(w, venue)?;
    crate::polity::terms_at(w, &entry)?
        .row?
        .restricted
        .into_iter()
        .find(|r| &r.good == commodity)
        .map(|r| r.paper)
}

/// **The law of a public trade** (`ProceedsPolicy::may_trade`): free unless the venue restricts
/// the good, and then only for a character holding its paper. An organization holds no
/// prospecting license, so it is refused a restricted good on the public board.
///
/// # Errors
///
/// [`RESTRICTED_REFUSED`], filled.
pub fn may_trade(
    w: &World,
    trader: &PrincipalId,
    venue: &MarketId,
    commodity: &CanonicalId,
) -> Result<(), ProceedsRefusal> {
    let Some(paper) = restriction_at(w, venue, commodity) else {
        return Ok(());
    };
    // Doc 50 unit LW.4 (Q-50-39): an actor under the venue holder's restriction is refused a
    // restricted good, papers or none.
    if let PrincipalId::Object(actor) = trader
        && let Some(polity) =
            port_entry_of(w, venue).and_then(|entry| crate::offense::jurisdiction_at(w, &entry))
        && crate::port::record::restricted(w, *actor, polity)
    {
        return Err(ProceedsRefusal {
            line: crate::port::record::board_refusal(w, polity),
        });
    }
    let now = w.world_ic_now().at;
    let clear = match trader {
        PrincipalId::Object(actor) => {
            paper.as_str() == PICKET_PAPERS && papers_clear(w, *actor, now)
        }
        PrincipalId::Organization(_) | PrincipalId::System(_) => false,
    };
    if clear {
        return Ok(());
    }
    let label = w
        .module_state::<crate::store::TacsimStore>()
        .and_then(crate::store::TacsimStore::goods)
        .and_then(|g| g.good(commodity).map(|d| d.label.clone()))
        .unwrap_or_else(|| commodity.to_string());
    let venue_name = crate::port::market::ports(w)
        .into_iter()
        .find(|(_, def, _)| def.venue.as_str() == venue.as_str())
        .map_or_else(
            || venue.to_string(),
            |(_, def, _)| crate::port::coop::port_name(def),
        );
    Err(ProceedsRefusal {
        line: RESTRICTED_REFUSED
            .replace("{venue}", &venue_name)
            .replace("{label}", &label),
    })
}
