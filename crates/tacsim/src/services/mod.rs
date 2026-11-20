//! **Hired work between principals** (doc 48 cut EX.6; Q-48-11, -42, -46, -57): the haul and
//! the escort, the first service kinds that are not an older host's job moved onto the board.
//!
//! - [`haul`]: these goods, from here to there, by a deadline. The cargo travels as a held lot
//!   reserved to the job, and the fact is the delivery's inventory marker, named at posting.
//! - [`escort`]: stay with this hull through her passage or her field. The fact is written by
//!   the host that runs the journey.
//!
//! Each kind keeps one whole-document record a job in the store's own block (`tacsim:haul:` and
//! `tacsim:escort:`), loaded at boot by the store; an unreadable one is held until staff repair it.

pub mod escort;
pub mod haul;

use std::collections::BTreeMap;

use mush_core::{Ctx, World};

use crate::port::owned::Held;
use crate::store::TacsimStore;

/// **The services' records in RAM** (doc 48 cut EX.6): every haul and escort by its contract.
#[derive(Clone, Debug, Default)]
pub struct ServicesBook {
    /// The hauls, by contract.
    pub(crate) hauls: BTreeMap<u64, Held<haul::HaulJob>>,
    /// The escorts, by contract.
    pub(crate) escorts: BTreeMap<u64, Held<escort::EscortJob>>,
}

fn book(w: &World) -> Option<&ServicesBook> {
    w.module_state::<TacsimStore>().map(|store| &store.services)
}

fn decode<T: serde::de::DeserializeOwned>(
    key: &str,
    value: &serde_json::Value,
    check: impl FnOnce(&T) -> Result<(), String>,
) -> Held<T> {
    match serde_json::from_value::<T>(value.clone())
        .map_err(|error| error.to_string())
        .and_then(|record| check(&record).map(|()| record))
    {
        Ok(record) => Held::Present(Box::new(record)),
        Err(error) => {
            tracing::error!(
                %key, %error,
                "tacsim: a service record is unreadable; its job is held until staff repair the \
                 document"
            );
            Held::Unreadable
        }
    }
}

/// **Boot: one service document**: `true` when `key` is a haul's or an escort's (read, or held
/// unreadable), `false` for any other key. A record must name its own key.
pub(crate) fn load(store: &mut TacsimStore, key: &str, value: &serde_json::Value) -> bool {
    for (prefix, is_haul) in [(haul::HAUL_PREFIX, true), (escort::ESCORT_PREFIX, false)] {
        let Some(rest) = key.strip_prefix(prefix) else {
            continue;
        };
        let Ok(contract) = rest.parse::<u64>() else {
            tracing::warn!(%key, "tacsim: a service record has an unreadable key; skipped");
            return true;
        };
        if is_haul {
            let held = decode::<haul::HaulJob>(key, value, |job| {
                job.check()?;
                (job.contract == contract)
                    .then_some(())
                    .ok_or_else(|| "the record disagrees with its own key".to_owned())
            });
            store.services.hauls.insert(contract, held);
        } else {
            let held = decode::<escort::EscortJob>(key, value, |job| {
                job.check()?;
                (job.contract == contract)
                    .then_some(())
                    .ok_or_else(|| "the record disagrees with its own key".to_owned())
            });
            store.services.escorts.insert(contract, held);
        }
        return true;
    }
    false
}

/// The port arm's step (doc 48 cut EX.6): each haul's owed leg, and each escort's watch.
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    haul::tick(ctx);
    escort::tick(ctx);
}

/// Tell a character principal one line.
pub(crate) fn tell(ctx: &mut Ctx<'_>, who: &mush_org::PrincipalId, channel: &str, line: &str) {
    if let mush_org::PrincipalId::Object(object) = who {
        ctx.out.sys_to_actor(*object, channel, line.to_owned());
    }
}

/// A port word (its venue, its entry or its name) to its venue.
pub(crate) fn venue_word(w: &World, word: &str) -> Option<mush_market::MarketId> {
    let word = word.trim().to_ascii_lowercase();
    crate::port::market::ports(w)
        .into_iter()
        .find(|(entry, def, _)| {
            def.venue.as_str() == word
                || entry.as_str() == word
                || crate::port::coop::port_name(def).eq_ignore_ascii_case(&word)
        })
        .and_then(|(_, def, _)| mush_market::MarketId::new(def.venue.as_str()))
}

/// A port word (its venue, its entry or its name) to its entry: a field's site.
pub(crate) fn entry_word(w: &World, word: &str) -> Option<String> {
    let word = word.trim().to_ascii_lowercase();
    crate::port::market::ports(w)
        .into_iter()
        .find(|(entry, def, _)| {
            def.venue.as_str() == word
                || entry.as_str() == word
                || crate::port::coop::port_name(def).eq_ignore_ascii_case(&word)
        })
        .map(|(entry, _, _)| entry.to_string())
}

/// The name of the port whose venue is `venue`, else the venue's word.
pub(crate) fn venue_name(w: &World, venue: &str) -> String {
    mush_market::MarketId::new(venue)
        .and_then(|id| crate::port::market::port_of_venue(w, &id))
        .map_or_else(
            || venue.to_owned(),
            |(def, _)| crate::port::coop::port_name(def),
        )
}

/// The venue `hull` is docked at, if any.
pub(crate) fn docked_venue(w: &World, hull: mush_core::ObjId) -> Option<mush_market::MarketId> {
    match crate::port::ship::berth_of(w, hull) {
        crate::port::ship::Berth::Docked { def, .. } => {
            mush_market::MarketId::new(def.venue.as_str())
        }
        crate::port::ship::Berth::UnderWay | crate::port::ship::Berth::Elsewhere => None,
    }
}

/// The venue the actor stands in, if any.
pub(crate) fn standing_venue(w: &World, actor: mush_core::ObjId) -> Option<mush_market::MarketId> {
    w.get(actor)
        .and_then(|o| o.location)
        .and_then(|room| mush_market::venue_gate(w)?.venue_of_room(w, room))
}

/// Who takes a job: the actor, or an org the actor acts for (`as=<org>`, Q-48-58), and the
/// account it is paid into and escrows from.
pub(crate) fn taker(
    ctx: &mut Ctx<'_>,
    as_org: Option<&str>,
) -> Result<(mush_org::PrincipalId, mush_economy::AccountId), String> {
    match as_org {
        None => crate::port::credit::own_account(ctx.w, ctx.actor)
            .map(|a| (mush_org::PrincipalId::Object(ctx.actor), a.id))
            .ok_or_else(|| "You have no account to be paid into.".to_owned()),
        Some(name) => {
            let name = name.strip_prefix("org:").unwrap_or(name);
            let org = mush_economy::resolve_org_visible(ctx, name)
                .map(mush_org::PrincipalId::Organization)
                .filter(|org| mush_economy::acts_for(ctx.w, ctx.actor, org))
                .ok_or_else(|| crate::surface::retrieval::NOT_FOR_ORG.to_owned())?;
            let account = ctx
                .w
                .module_state::<mush_economy::EconomyStore>()
                .and_then(|store| {
                    store
                        .snapshot()
                        .accounts_of(&org)
                        .find(|a| {
                            a.instrument.as_str() == crate::economy::CREDIT
                                && a.status == mush_economy::AccountStatus::Active
                        })
                        .map(|a| a.id)
                })
                .ok_or_else(|| crate::surface::retrieval::ORG_NO_ACCOUNT.to_owned())?;
            Ok((org, account))
        }
    }
}

/// **The capability to post jobs for an organization** (doc 50 cut OP.3, Q-50-23): a member who
/// holds it on the organization's root compartment may post a haul or an escort `as=<org>`, the
/// fee paid from the organization's account.
pub const JOBS_POST_CAPABILITY: &str = "tacsim.jobs.post";

/// The line a poster who may not post for that organization reads.
pub const NOT_POSTER: &str = "You may not post jobs for that organization.";

/// **Who posts a job** (doc 50 cut OP.3, Q-50-23): the actor, from their own account; or, with
/// `as=<org>`, the organization, when the actor holds [`JOBS_POST_CAPABILITY`] on its root
/// compartment, from the organization's credit account.
pub(crate) fn poster(
    ctx: &mut Ctx<'_>,
    as_org: Option<&str>,
) -> Result<(mush_org::PrincipalId, mush_economy::AccountId), String> {
    let Some(name) = as_org else {
        return crate::port::credit::own_account(ctx.w, ctx.actor)
            .map(|a| (mush_org::PrincipalId::Object(ctx.actor), a.id))
            .ok_or_else(|| "You have no account to pay the fee from.".to_owned());
    };
    let name = name.strip_prefix("org:").unwrap_or(name);
    let org = mush_economy::resolve_org_visible(ctx, name).ok_or_else(|| NOT_POSTER.to_owned())?;
    let root = ctx
        .w
        .module_state::<mush_org::OrgStore>()
        .and_then(|store| store.state().org(org).map(|o| o.root_scope))
        .ok_or_else(|| NOT_POSTER.to_owned())?;
    let capability =
        mush_org::CapabilityId::new(JOBS_POST_CAPABILITY).ok_or_else(|| NOT_POSTER.to_owned())?;
    let request = mush_org::AuthRequest {
        actor: ctx.actor,
        capability,
        scope: root,
        now: ctx.w.now(),
        amount: None,
    };
    if !mush_org::authorize_in_world(ctx.w, &request).is_allow() {
        return Err(NOT_POSTER.to_owned());
    }
    let principal = mush_org::PrincipalId::Organization(org);
    let account = ctx
        .w
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|store| {
            store
                .snapshot()
                .accounts_of(&principal)
                .find(|a| {
                    a.instrument.as_str() == crate::economy::CREDIT
                        && a.status == mush_economy::AccountStatus::Active
                })
                .map(|a| a.id)
        })
        .ok_or_else(|| crate::surface::retrieval::ORG_NO_ACCOUNT.to_owned())?;
    Ok((principal, account))
}

/// **An organization's vault at `venue`** (doc 50 cut OP.3): its storefront there, from the lane's
/// port map, as `VaultId.0`.
#[must_use]
pub fn org_vault(w: &World, org: mush_org::OrgId, venue: &str) -> Option<u64> {
    crate::port::market::ports(w)
        .into_iter()
        .filter(|(_, def, _)| def.venue.as_str() == venue)
        .find_map(|(_, _, map)| {
            let map = map?;
            let (id, _) = map.orgs.iter().find(|(_, row)| row.org == org.0)?;
            map.vendors.get(id).map(|row| row.vault)
        })
}

/// The state of contract `id`, if the economy has it.
pub(crate) fn contract_state(w: &World, id: u64) -> Option<mush_economy::ContractState> {
    w.module_state::<mush_economy::EconomyStore>()?
        .snapshot()
        .contract(mush_economy::ContractId(id))
        .map(|c| c.state)
}

/// Split `key=value` options from the words of `raw`.
pub(crate) fn options(raw: &str) -> (Vec<String>, BTreeMap<String, String>) {
    let mut words = Vec::new();
    let mut opts = BTreeMap::new();
    for part in raw.split_whitespace() {
        match part.split_once('=') {
            Some((k, v)) if !k.is_empty() && !k.starts_with('#') => {
                opts.insert(k.to_ascii_lowercase(), v.to_owned());
            }
            _ => words.push(part.to_owned()),
        }
    }
    (words, opts)
}

/// A whole-credit figure (`1200`, `1200cr`) in minor units.
pub(crate) fn credits(word: &str) -> Option<i64> {
    let word = word.trim().trim_end_matches("cr");
    word.parse::<i64>()
        .ok()
        .filter(|n| *n > 0)
        .and_then(|n| n.checked_mul(100))
}

#[cfg(test)]
mod tests {
    #[test]
    fn ex6_options_split_words_from_terms() {
        let (words, opts) = super::options("12 metals=ceres fee=300 value=900 go");
        assert_eq!(words, vec!["12", "go"]);
        assert_eq!(opts.get("metals").map(String::as_str), Some("ceres"));
        assert_eq!(opts.get("fee").map(String::as_str), Some("300"));
        assert_eq!(super::credits("300cr"), Some(30_000));
        assert_eq!(super::credits("0"), None);
    }
}
