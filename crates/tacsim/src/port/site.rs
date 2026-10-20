//! **A mining site's pure reads, its lay-up and its stores line** (doc 20 cut SM.2; §1
//! beats 2 and 3, §3.1, §3.3, §3.4; D-20-2, D-20-3, Q-20-7; the SM.2 brief's items 6, 8
//! and 10 and its lead overrides).
//!
//! A site is a vendor-less port entry with a `site` block ([`crate::port::SiteDef`]): a
//! pad in a 2,560 m surface region, sixteen blocks of tenure, and the ground under them,
//! which is its **body's** geology table (the lane's `BodyDef`) at the site's own
//! coordinates. This module holds what reads it:
//!
//! - [`geology_spec`](crate::port::site::geology_spec) and
//!   [`site_province`](crate::port::site::site_province) — the site on its body's pocket
//!   lattice (SM.1 decision 9; `div_euclid`, so a southern site snaps away from the
//!   equator, never toward it);
//! - [`survey_words`](crate::port::site::survey_words) and
//!   [`tenure_parts`](crate::port::site::tenure_parts) — `+chart`'s Registry survey (R0)
//!   and tenure lines; the survey needs the live key and an installed table, else it reads
//!   "unavailable" (decision 9), so nothing ore-related reaches a keyless world;
//! - [`landed_since`](crate::port::site::landed_since),
//!   [`landed_charge`](crate::port::site::landed_charge) and
//!   [`stores_line`](crate::port::site::stores_line) — **landed life support** (Q-20-7,
//!   lead ruling 5): nothing debits the locker while the hull sits on the pad (no tick
//!   touches the player's stores, D-18-4); the stay is charged **at lift**, inside the
//!   pilot's own departure burn, souls × whole IC days settled, capped so the locker still
//!   covers the leg home; every stores line prints the **effective** figure, the locker
//!   less the charge a lift now would make;
//! - `lay_up` — the `PortApplier`'s part (g): the sixteen block place Things and the site
//!   clerk, `DARK`, in the site's host room, recorded in the port map; since doc 20 cut
//!   SM.3b also **the spoil** — a `DARK` Thing backing a vault owned by
//!   [`SITES_PRINCIPAL`](crate::port::site::SITES_PRINCIPAL) with no scope (§4.3, decision
//!   6: the storefront's `Container` backing), into which `+tv/<n> dump` moves a hopper's
//!   pools; **it submits nothing** (doc 18's refusal; §3.4's site clerk is the pattern).
//!   SM.6 makes `SITES_PRINCIPAL` the block places' industry owner and grants the clerk.

use mush_core::{CanonicalId, Ctx, Flags, ObjId, ObjKind, World};
use mush_economy::{CustodyRef, EconomyStore, Vault, VaultId, VaultStatus};
use mush_org::{PrincipalId, SystemPrincipalId};
use smol_str::SmolStr;

use crate::pack::{LocationDef, PackEntry, RulesPack};
use crate::port::def::{BLOCK_LETTERS, SiteDef, Tenure};
use crate::port::map::{PortMap, SiteRow};
use crate::store::put_port_map;
use crate::surface::geology::{BodyGeology, OreFamily, PROVINCE_COLUMNS, PROVINCE_ROWS};
use crate::surface::ground::{SITE_POCKETS, SiteGeologySpec};

/// **The system principal SM.6 makes the owner** of a site's block places, deposits and
/// spoil (doc 20 §3.4; the SM.2 brief's decision 13). Declared here, used from SM.6: a
/// world object's owner is an `ObjId`, so the Things laid up here are owned by the
/// applying actor as the stock clerk is.
pub const SITES_PRINCIPAL: &str = "tacsim.sites";

/// **The system principal that owns every hopper vault** (doc 20 §4.2; cut SM.3a):
/// a terrain vehicle's hopper is a vault on its hopper store, owned by this principal
/// with no scope — never a pilot and never a hull's titled owner — so no player
/// passes `may_take` on a hopper, `live_endpoints` never offers one, and only the
/// server-authenticated acts (the drill's cycle, the unload) write it.
pub const VEHICLES_PRINCIPAL: &str = "tacsim.vehicles";

/// The stores line while the stay can still be charged in full (§1 beat 3): `{stores}`
/// the effective crew-days (what a lift now would leave), `{span}` the real time left.
pub const STORES_HERE: &str = "Stores {stores} crew-days: {span} here before short rations.";
/// The stores line once the leg home's reserve binds (Q-20-7): `{stores}`.
pub const STORES_SHORT_RATIONS: &str =
    "Stores {stores} crew-days: on short rations here; the stores for the leg home are kept back.";
/// `+chart`'s Registry survey under a site (R0): `{words}`, e.g. "metals fair, volatiles
/// rich, carbon feedstock lean, strategic elements barren".
pub const SURVEY_LINE: &str = "    Registry survey: {words}.";
/// The survey with the layer disabled or no table installed (decision 9).
pub const SURVEY_UNAVAILABLE: &str = "    Registry survey: unavailable.";
/// `+chart`'s tenure line under a site: `{parts}`, e.g. "16 commons", "16 open", "12 open;
/// 4 held by Occator Water & Propellant".
pub const TENURE_LINE: &str = "    Tenure: {parts}.";

/// Seconds in an IC day.
const DAY: i64 = 86_400;

/// **A body's display name** for a player sentence: the lane id with its first letter
/// capitalised (`luna` → `Luna`); the lane authors no body labels.
#[must_use]
pub fn body_name(body: &CanonicalId) -> String {
    let mut chars = body.as_str().chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// The centre pocket of a place on its body's lattice: `x = (lon × columns).div_euclid(
/// 360_000)` normalised into `0..columns`, `y = (lat × rows).div_euclid(180_000)`, in
/// checked `i128` — `div_euclid`, never `/`: five of the six sites are south of the
/// equator and `/` truncates toward zero.
fn centre_pocket(location: &LocationDef, body: &BodyGeology) -> Result<(i64, i64), String> {
    let (Some(lat), Some(lon)) = (location.lat_mdeg, location.lon_mdeg) else {
        return Err(format!("{} authors no coordinates", location.label));
    };
    let lattice = body.lattice();
    let off = || format!("{} is off {}'s lattice", location.label, body.body);
    let x = i128::from(lon)
        .checked_mul(i128::from(lattice.columns))
        .ok_or_else(off)?
        .div_euclid(360_000);
    let y = i128::from(lat)
        .checked_mul(i128::from(lattice.rows))
        .ok_or_else(off)?
        .div_euclid(180_000);
    let x = i64::try_from(x).map_err(|_| off())?;
    let y = i64::try_from(y).map_err(|_| off())?;
    let x = x.checked_rem_euclid(lattice.columns).ok_or_else(off)?;
    Ok((x, y))
}

/// **A site's geology spec on its body's lattice** (SM.1 decision 9; the SM.2 brief's
/// item 6): the centre pocket (`centre_pocket`), the origin `((x − 32).div_euclid(8) ×
/// 8, (y − 32).div_euclid(8) × 8)` — a field corner, so the 64-pocket square holds whole
/// fields — with x normalised into `0..columns`, and the shifts from
/// [`SiteDef::shifts`]. The body is the location's own (lead override 4: every caller
/// derives the spec from the site's lane entry, never by hand).
///
/// # Errors
///
/// A place without coordinates, or a figure that leaves its type.
pub fn geology_spec(
    site: &SiteDef,
    location: &LocationDef,
    body: &BodyGeology,
) -> Result<SiteGeologySpec, String> {
    if location.body.as_ref() != Some(&body.body) {
        return Err(format!(
            "{} does not stand on {}",
            location.label, body.body
        ));
    }
    let (x, y) = centre_pocket(location, body)?;
    let half = SITE_POCKETS / 2;
    let corner = |v: i64| -> Option<i64> {
        v.checked_sub(half)?
            .div_euclid(crate::surface::geology::POCKETS_PER_FIELD_SIDE)
            .checked_mul(crate::surface::geology::POCKETS_PER_FIELD_SIDE)
    };
    let off = || format!("{} is off {}'s lattice", location.label, body.body);
    let ox = corner(x)
        .and_then(|v| v.checked_rem_euclid(body.lattice().columns))
        .ok_or_else(off)?;
    let oy = corner(y).ok_or_else(off)?;
    Ok(SiteGeologySpec {
        body: body.body.clone(),
        origin: (ox, oy),
        shifts: site.shifts(),
    })
}

/// **The site's province on the 56 × 28 chart**: the province of its centre pocket.
///
/// # Errors
///
/// A place without coordinates, or a centre off the lattice.
pub fn site_province(location: &LocationDef, body: &BodyGeology) -> Result<(u32, u32), String> {
    let (x, y) = centre_pocket(location, body)?;
    let province = body
        .lattice()
        .province_of_pocket(x, y)
        .map_err(|error| error.to_string())?;
    if province.0 >= PROVINCE_COLUMNS || province.1 >= PROVINCE_ROWS {
        return Err(format!("{} is off {}'s chart", location.label, body.body));
    }
    Ok(province)
}

/// The location entry `place` names in `pack`.
fn location_of<'a>(pack: &'a RulesPack, place: &CanonicalId) -> Option<&'a LocationDef> {
    match pack.entry(place) {
        Some(PackEntry::Location(location)) => Some(location),
        _ => None,
    }
}

/// **The Registry survey's words** (R0; §1 beat 2, `+chart`): `"{family} {word}"` in
/// [`OreFamily::ALL`] order, joined `", "` — the province of the site read through the
/// live key and the body's installed table. `None` when the layer is disabled, no table
/// is installed for the site's body, or the place reads off the lattice (decision 9: the
/// line then says "unavailable", nothing ore-related).
#[must_use]
pub fn survey_words(w: &World, pack: &RulesPack, site_location: &CanonicalId) -> Option<String> {
    let (mac, _) = crate::surface::live(w).ok()?;
    let location = location_of(pack, site_location)?;
    let body = crate::surface::body_geology(w, location.body.as_ref()?)?;
    let province = site_province(location, &body).ok()?;
    let ground = crate::surface::geology::province_at(&mac, &body, province).ok()?;
    let words = crate::surface::reading::read_r0(&ground);
    let parts: Vec<String> = OreFamily::ALL
        .into_iter()
        .zip(words)
        .map(|(family, word)| format!("{} {}", family.label(), word.name()))
        .collect();
    Some(parts.join(", "))
}

/// **The site's tenure in words** (`+chart`'s [`TENURE_LINE`]): the sixteen blocks
/// counted per tenure in the order commons, open, held — the held ones grouped by
/// holder, each named by its org's `name` in the pack (the id when no port authors it) —
/// joined `"; "`, zero groups left out.
#[must_use]
pub fn tenure_parts(pack: &RulesPack, site: &SiteDef) -> String {
    let mut commons = 0_usize;
    let mut open = 0_usize;
    let mut held: std::collections::BTreeMap<&CanonicalId, usize> =
        std::collections::BTreeMap::new();
    for letter in BLOCK_LETTERS {
        let block = site.block(letter);
        match (block.tenure, block.holder) {
            (Tenure::Commons, _) => commons = commons.saturating_add(1),
            (Tenure::Open, _) => open = open.saturating_add(1),
            (Tenure::Held, Some(holder)) => {
                let n = held.entry(holder).or_insert(0);
                *n = n.saturating_add(1);
            }
            // The validator pairs held with its holder; a held block without one reads
            // as open rather than naming nobody.
            (Tenure::Held, None) => open = open.saturating_add(1),
        }
    }
    let org_name = |id: &CanonicalId| -> String {
        pack.entries()
            .values()
            .find_map(|entry| match entry {
                PackEntry::Port(port) => port
                    .orgs
                    .iter()
                    .find(|org| &org.id == id)
                    .map(|org| org.name.clone()),
                _ => None,
            })
            .unwrap_or_else(|| id.to_string())
    };
    let mut parts: Vec<String> = Vec::new();
    if commons > 0 {
        parts.push(format!("{commons} commons"));
    }
    if open > 0 {
        parts.push(format!("{open} open"));
    }
    for (holder, n) in held {
        parts.push(format!("{n} held by {}", org_name(holder)));
    }
    parts.join("; ")
}

/// **The landing instant** (Q-20-7): the hull's Arrived journey whose destination is the
/// site's host room, its `arrival_at()` — the durable `cjourney:` record mush-sim keeps
/// until the next departure, so a restart reads the same instant. `None` otherwise (a
/// hull a staffer placed on a pad has no landed charge).
#[must_use]
pub fn landed_since(w: &World, hull: ObjId, map: &PortMap) -> Option<i64> {
    let journey = w.module_state::<mush_sim::JourneyBook>()?.get(hull)?;
    if !matches!(journey.state(), mush_sim::JourneyState::Arrived) {
        return None;
    }
    let host = map.host?;
    (journey.route().endpoints().1 == ObjId(host))
        .then(|| journey.arrival_at())
        .flatten()
}

/// **The stay's charge at a lift at `now_ic`** (Q-20-7; lead ruling 5): `souls ×
/// ⌈(now − since) / 86 400⌉` (0 when `now <= since`), capped at `locker −
/// home_crew_days` (saturating) — the one-leg-home floor, so a lift always keeps the
/// stores for the leg home. `locker` is the raw pool, which does not fall while the
/// hull is settled (decision 4).
#[must_use]
pub fn landed_charge(souls: u32, since: i64, now_ic: i64, locker: u64, home_crew_days: u32) -> u32 {
    let elapsed = now_ic.saturating_sub(since);
    if elapsed <= 0 {
        return 0;
    }
    let days = u64::try_from(elapsed.saturating_add(DAY - 1) / DAY).unwrap_or(u64::MAX);
    let charge = u64::from(souls).saturating_mul(days);
    let cap = locker.saturating_sub(u64::from(home_crew_days));
    u32::try_from(charge.min(cap)).unwrap_or(u32::MAX)
}

/// **Real seconds as a stay's span**: `{d} d {h} h`, `{h} h` under a day, `under an hour`
/// under an hour.
#[must_use]
pub fn pad_span(real_secs: i64) -> String {
    let secs = real_secs.max(0);
    let (days, hours) = (secs / DAY, (secs % DAY) / 3_600);
    if days > 0 {
        format!("{days} d {hours} h")
    } else if hours > 0 {
        format!("{hours} h")
    } else {
        "under an hour".to_owned()
    }
}

/// **The stores line** (§1 beat 3, Q-20-7). `None` when `souls` is 0. Prints the
/// **effective** stores `E = locker − landed_charge(souls, since, now, locker, home)` —
/// what a lift now would leave — so the figure falls one crew-day a soul each IC day and
/// stops at the leg home. With `B = ⌊(locker − home) / souls⌋` whole IC days: while
/// `since + B × 86 400 > now`, [`STORES_HERE`] with the real time left to that instant at
/// the world's rate ([`pad_span`]), or `{n} IC days` while the clock is stopped; else
/// [`STORES_SHORT_RATIONS`].
#[must_use]
pub fn stores_line(
    w: &World,
    locker: u64,
    souls: u32,
    home_crew_days: u32,
    since: i64,
    now_ic: i64,
) -> Option<String> {
    if souls == 0 {
        return None;
    }
    let charge = landed_charge(souls, since, now_ic, locker, home_crew_days);
    let effective = locker.saturating_sub(u64::from(charge));
    let spare = locker.saturating_sub(u64::from(home_crew_days));
    let days = i64::try_from(spare / u64::from(souls)).unwrap_or(i64::MAX);
    let short_at = since.saturating_add(days.saturating_mul(DAY));
    if short_at > now_ic {
        let span = match crate::port::voyage::real_until(w, short_at) {
            Some(real) => pad_span(real),
            None => {
                let left = short_at.saturating_sub(now_ic);
                format!("{} IC days", left.saturating_add(DAY - 1) / DAY)
            }
        };
        Some(
            STORES_HERE
                .replace("{stores}", &effective.to_string())
                .replace("{span}", &span),
        )
    } else {
        Some(STORES_SHORT_RATIONS.replace("{stores}", &effective.to_string()))
    }
}

/// Whether `obj` is live: present and not `GOING`.
fn live(w: &World, obj: u32) -> bool {
    w.get(ObjId(obj))
        .is_some_and(|o| !o.flags.contains(Flags::GOING))
}

/// **The preflight's site refusals** (the SM.2 brief's item 8): every recorded block
/// place and the clerk is live — a recorded Thing gone is never re-created, the stock
/// clerk's sentence (`crate::port::vendor`'s preflight).
///
/// # Errors
///
/// The first recorded Thing that is gone.
pub(crate) fn preflight(w: &World, id: &CanonicalId, map: Option<&PortMap>) -> Result<(), String> {
    let Some(row) = map.and_then(|m| m.site.as_ref()) else {
        return Ok(());
    };
    for (letter, thing) in &row.blocks {
        if !live(w, *thing) {
            return Err(format!(
                "{id}: the site's block place {letter} #{thing} in the port map is gone; it is \
                 never re-created — repair the map by hand"
            ));
        }
    }
    if let Some(clerk) = row.clerk
        && !live(w, clerk)
    {
        return Err(format!(
            "{id}: the site clerk #{clerk} in the port map is gone; it is never re-created — \
             repair the map by hand"
        ));
    }
    // Doc 20 cut SM.3b (decision 6): a recorded spoil Thing gone, or a recorded spoil vault
    // gone or not the sites' system principal's, refuses — never re-created.
    if let Some(spoil) = row.spoil
        && !live(w, spoil)
    {
        return Err(format!(
            "{id}: the site's spoil #{spoil} in the port map is gone; it is never re-created — \
             repair the map by hand"
        ));
    }
    if let Some(vault) = row.spoil_vault
        && !spoil_vault_is_ours(w, vault, false)
    {
        return Err(format!(
            "{id}: the site's spoil vault #{vault} in the port map is gone or not owned by \
             {SITES_PRINCIPAL}; it is never re-created — repair the map by hand"
        ));
    }
    Ok(())
}

/// The owner every spoil vault has: `System(tacsim.sites)`.
fn spoil_owner() -> Option<PrincipalId> {
    SystemPrincipalId::new(SITES_PRINCIPAL).map(PrincipalId::System)
}

/// Whether vault `id` exists, is owned by `System(tacsim.sites)` with no scope, and (when
/// `active` is asked) is Active.
fn spoil_vault_is_ours(w: &World, id: u64, active: bool) -> bool {
    let Some(owner) = spoil_owner() else {
        return false;
    };
    w.module_state::<EconomyStore>()
        .and_then(|economy| economy.snapshot().vault(VaultId(id)).cloned())
        .is_some_and(|vault| {
            vault.owner == owner
                && vault.scope.is_none()
                && (!active || vault.status == VaultStatus::Active)
        })
}

/// **A site's spoil vault, checked** (doc 20 cut SM.3b; the read `+tv/<n> dump` plans
/// with): the map's spoil Thing live and its vault the sites' system principal's, no scope,
/// Active; `None` otherwise (the dump then refuses `DUMP_NO_SPOIL`).
#[must_use]
pub(crate) fn spoil_vault(w: &World, map: &PortMap) -> Option<VaultId> {
    let row = map.site.as_ref()?;
    let thing = row.spoil?;
    let vault = row.spoil_vault?;
    (live(w, thing) && spoil_vault_is_ours(w, vault, true)).then_some(VaultId(vault))
}

/// One `DARK` Thing named `name` in `host`, owned by the applying actor.
fn dark_thing(ctx: &mut Ctx<'_>, id: &CanonicalId, name: &str, host: ObjId) -> Result<u32, String> {
    let thing = ctx
        .w
        .create(ObjKind::Thing, name, ctx.actor, Some(host))
        .map_err(|e| format!("{id}: {name}: {e}"))?;
    ctx.w
        .set_flag(thing, Flags::DARK, true)
        .map_err(|e| format!("{id}: {name}: {e}"))?;
    Ok(thing.0)
}

/// **The `PortApplier`'s part (g)** (doc 20 §3.1, §3.4; the SM.2 brief's item 8): for
/// each letter `A`..=`P` not yet in the map's site row, one `DARK` Thing `"{site} block
/// {L}"` in the site's **host room** (the lane place room, never walkable, never in a
/// neighborhood), recorded and the map written; then the clerk, `"{site} site clerk"`,
/// the same way. An update re-lays nothing recorded; nothing is submitted.
///
/// # Errors
///
/// A Thing the world would not make, named.
pub(crate) fn lay_up(
    ctx: &mut Ctx<'_>,
    id: &CanonicalId,
    site: &SiteDef,
    map: &mut PortMap,
    host: ObjId,
) -> Result<(), String> {
    for letter in BLOCK_LETTERS {
        let key = letter.to_string();
        if map
            .site
            .as_ref()
            .is_some_and(|row| row.blocks.contains_key(&key))
        {
            continue;
        }
        let thing = dark_thing(ctx, id, &format!("{} block {letter}", site.name), host)?;
        map.site
            .get_or_insert_with(SiteRow::default)
            .blocks
            .insert(key, thing);
        put_port_map(ctx, map);
    }
    if map.site.as_ref().is_none_or(|row| row.clerk.is_none()) {
        let thing = dark_thing(ctx, id, &format!("{} site clerk", site.name), host)?;
        map.site.get_or_insert_with(SiteRow::default).clerk = Some(thing);
        put_port_map(ctx, map);
    }
    // Doc 20 cut SM.3b (§4.3; decision 6): the spoil — a DARK Thing, then the vault it
    // backs, owned by `System(tacsim.sites)` with no scope (so `may_take` refuses every
    // player and `live_endpoints` never offers it), each recorded and the map written.
    if map.site.as_ref().is_none_or(|row| row.spoil.is_none()) {
        let thing = dark_thing(ctx, id, &format!("{} spoil", site.name), host)?;
        map.site.get_or_insert_with(SiteRow::default).spoil = Some(thing);
        put_port_map(ctx, map);
    }
    if map
        .site
        .as_ref()
        .is_none_or(|row| row.spoil_vault.is_none())
    {
        let thing = map
            .site
            .as_ref()
            .and_then(|row| row.spoil)
            .ok_or_else(|| format!("{id}: the spoil Thing is not recorded"))?;
        let owner = spoil_owner()
            .ok_or_else(|| format!("{id}: {SITES_PRINCIPAL} is not a system principal id"))?;
        let vault = crate::port::apply::alloc_vault_id(ctx)
            .map_err(|e| format!("{id}: the spoil vault: {e}"))?;
        mush_economy::create_vault(
            ctx,
            Vault {
                id: vault,
                owner,
                scope: None,
                name: SmolStr::new(format!("{} spoil", site.name)),
                backing: CustodyRef::Container(ObjId(thing)),
                status: VaultStatus::Active,
                revision: 1,
            },
        );
        map.site.get_or_insert_with(SiteRow::default).spoil_vault = Some(vault.0);
        put_port_map(ctx, map);
    }
    Ok(())
}

/// **The readiness `Site` line** (the SM.2 brief's item 9; reported, never gated on the
/// desk): Ready with all sixteen block places, the clerk and (doc 20 cut SM.3b) the spoil
/// live — its Thing live and its vault the sites' system principal's and Active — else
/// Missing with the count (a recorded Thing gone counts as missing). `port_ready` reads it,
/// so a Missing line closes the pad (`SITE_CLOSED`): fail-closed, decision 6.
#[must_use]
pub(crate) fn site_line(
    w: &World,
    entry: &CanonicalId,
    map: &PortMap,
) -> (crate::port::LineState, String) {
    let (blocks, clerk) = map.site.as_ref().map_or((0, false), |row| {
        let blocks = BLOCK_LETTERS
            .iter()
            .filter(|letter| {
                row.blocks
                    .get(letter.to_string().as_str())
                    .is_some_and(|thing| live(w, *thing))
            })
            .count();
        (blocks, row.clerk.is_some_and(|clerk| live(w, clerk)))
    });
    site_line_text(entry, blocks, clerk, spoil_vault(w, map).is_some())
}

/// [`site_line`]'s words, pure: Ready with every block place, the clerk and the spoil,
/// else Missing with what is there.
fn site_line_text(
    entry: &CanonicalId,
    blocks: usize,
    clerk: bool,
    spoil: bool,
) -> (crate::port::LineState, String) {
    let count = crate::port::def::SITE_BLOCK_COUNT;
    if blocks == count && clerk && spoil {
        (
            crate::port::LineState::Ready,
            format!("{entry} site: {count} block places, the site clerk and the spoil laid up."),
        )
    } else {
        let clerk = if clerk {
            "the site clerk"
        } else {
            "no site clerk"
        };
        let spoil = if spoil { "the spoil" } else { "no spoil" };
        (
            crate::port::LineState::Missing,
            format!("{entry} site: {blocks} of {count} block places, {clerk} and {spoil} laid up."),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::port::def::{BlockDef, SiteTenureDef, Zone};
    use crate::surface::geology::{GeologyTable, GeologyTableWire};

    fn cid(s: &str) -> CanonicalId {
        CanonicalId::new(s).unwrap_or_else(|_| CanonicalId::new("x").expect("x"))
    }

    fn table() -> GeologyTable {
        let wire: GeologyTableWire = serde_json::from_str(
            r#"{
              "province": {
                "metals": { "lean": 1000 }, "volatiles": { "lean": 1000 },
                "carbon_feedstock": { "lean": 1000 }, "strategic_elements": { "barren": 1000 }
              },
              "field": {
                "barren": { "barren": 1000 }, "lean": { "lean": 1000 },
                "fair": { "fair": 1000 }, "rich": { "rich": 1000 }
              },
              "pocket": {
                "lean": { "lean": 1000 }, "fair": { "fair": 1000 }, "rich": { "rich": 1000 }
              }
            }"#,
        )
        .unwrap_or_else(|_| panic!("the test table parses"));
        GeologyTable::from_wire(&wire).unwrap_or_else(|_| panic!("the test table checks"))
    }

    fn ceres() -> BodyGeology {
        BodyGeology::new(cid("ceres"), 470, table()).unwrap_or_else(|_| panic!("ceres"))
    }

    fn place(lat: i32, lon: i32) -> LocationDef {
        LocationDef {
            label: "Here".to_owned(),
            role: cid(crate::pack::SITE_ROLE),
            body: Some(cid("ceres")),
            lat_mdeg: Some(lat),
            lon_mdeg: Some(lon),
        }
    }

    fn site(blocks: &[(&str, BlockDef)]) -> SiteDef {
        SiteDef {
            name: "Here".to_owned(),
            issuer: cid("freeport_registry"),
            zone: Zone::Core,
            tenure: SiteTenureDef {
                default: Tenure::Open,
            },
            blocks: blocks
                .iter()
                .map(|(l, b)| ((*l).to_owned(), b.clone()))
                .collect(),
        }
    }

    /// Doc 20 cut SM.3b: the `Site` line's three texts — Ready with the spoil, Missing for
    /// a missing spoil, Missing for a missing clerk and blocks.
    #[test]
    fn sm3b_the_site_line_names_the_spoil() {
        let entry = cid("kerwan_port");
        assert_eq!(
            site_line_text(&entry, 16, true, true),
            (
                crate::port::LineState::Ready,
                "kerwan_port site: 16 block places, the site clerk and the spoil laid up."
                    .to_owned()
            )
        );
        assert_eq!(
            site_line_text(&entry, 16, true, false),
            (
                crate::port::LineState::Missing,
                "kerwan_port site: 16 of 16 block places, the site clerk and no spoil laid up."
                    .to_owned()
            )
        );
        assert_eq!(
            site_line_text(&entry, 15, false, true),
            (
                crate::port::LineState::Missing,
                "kerwan_port site: 15 of 16 block places, no site clerk and the spoil laid up."
                    .to_owned()
            )
        );
    }

    /// `pad_span`'s three shapes.
    #[test]
    fn sm2_pad_span_reads_days_hours_or_under_an_hour() {
        assert_eq!(pad_span(0), "under an hour");
        assert_eq!(pad_span(3_599), "under an hour");
        assert_eq!(pad_span(3_600), "1 h");
        assert_eq!(pad_span(86_399), "23 h");
        assert_eq!(pad_span(86_400), "1 d 0 h");
        assert_eq!(pad_span(3 * 86_400 + 5 * 3_600 + 59), "3 d 5 h");
        assert_eq!(pad_span(-5), "under an hour");
    }

    /// `body_name` capitalises the first letter of a lane id.
    #[test]
    fn sm2_body_name_capitalises_the_id() {
        assert_eq!(body_name(&cid("luna")), "Luna");
        assert_eq!(body_name(&cid("ceres")), "Ceres");
    }

    /// The spec snaps the centre's 64-pocket square to field corners **away** from the
    /// equator in the south (div_euclid), normalises x into the lattice, and carries the
    /// letters' shifts.
    #[test]
    fn sm2_geology_spec_snaps_with_div_euclid_and_wraps_x() {
        let body = ceres();
        let lattice = body.lattice();
        // Juling Shadow: 35.9 S. The centre row is the floor of a negative product.
        let south = place(-35_900, 168_600);
        let spec = geology_spec(&site(&[]), &south, &body).unwrap_or_else(|e| panic!("{e}"));
        let y = (i128::from(-35_900_i32) * i128::from(lattice.rows)).div_euclid(180_000);
        let y = i64::try_from(y).unwrap_or_default();
        assert_eq!(spec.origin.1, (y - 32).div_euclid(8) * 8);
        assert!(spec.origin.1 <= y - 32, "snapped below, never toward zero");
        assert!(spec.origin.1 > y - 32 - 8);
        assert_eq!(spec.origin.0.rem_euclid(8), 0);
        assert_eq!(spec.origin.1.rem_euclid(8), 0);
        // A centre at longitude 0 wraps its west half to the far side.
        let seam = place(0, 0);
        let spec = geology_spec(&site(&[]), &seam, &body).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(spec.origin.0, lattice.columns - 32);
        assert!((0..lattice.columns).contains(&spec.origin.0));
        // The shifts are the letters'.
        let shifted = site(&[(
            "A",
            BlockDef {
                prospectivity: 1,
                ..BlockDef::default()
            },
        )]);
        let spec = geology_spec(&shifted, &seam, &body).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(spec.shifts[12], 1);
        // Another body's place is refused; a place without coordinates too.
        let mut elsewhere = place(0, 0);
        elsewhere.body = Some(cid("vesta"));
        assert!(geology_spec(&site(&[]), &elsewhere, &body).is_err());
        let mut bare = place(0, 0);
        bare.lat_mdeg = None;
        assert!(geology_spec(&site(&[]), &bare, &body).is_err());
        // The province is the centre's.
        let province = site_province(&south, &body).unwrap_or_else(|e| panic!("{e}"));
        assert!(province.0 < 56 && province.1 < 28);
        assert!(
            province.1 < 14,
            "a southern site's province is in the southern rows"
        );
    }

    /// The tenure words: commons, open, held by holder name, zero groups left out.
    #[test]
    fn sm2_tenure_parts_count_and_name_the_holders() {
        let Ok(pack) = crate::pack::load_pack(crate::definitions::content::LANE_JSON) else {
            panic!("the shipped lane loads");
        };
        let held = BlockDef {
            tenure: Some(Tenure::Held),
            holder: Some(cid("occator")),
            ..BlockDef::default()
        };
        let rim = site(&[
            ("F", held.clone()),
            ("G", held.clone()),
            ("J", held.clone()),
            ("K", held),
        ]);
        assert_eq!(
            tenure_parts(&pack, &rim),
            "12 open; 4 held by Occator Water & Propellant"
        );
        assert_eq!(tenure_parts(&pack, &site(&[])), "16 open");
        let mut commons = site(&[]);
        commons.tenure.default = Tenure::Commons;
        assert_eq!(tenure_parts(&pack, &commons), "16 commons");
        let mixed = site(&[(
            "A",
            BlockDef {
                tenure: Some(Tenure::Commons),
                ..BlockDef::default()
            },
        )]);
        assert_eq!(tenure_parts(&pack, &mixed), "1 commons; 15 open");
    }

    /// The charge: whole IC days a soul, capped at the leg home; nothing before arrival.
    #[test]
    fn sm2_landed_charge_is_whole_days_capped_at_the_leg_home() {
        let since = 1_000_000;
        assert_eq!(landed_charge(1, since, since, 5, 1), 0);
        assert_eq!(landed_charge(1, since, since - 10, 5, 1), 0);
        assert_eq!(landed_charge(1, since, since + 1, 5, 1), 1);
        assert_eq!(landed_charge(1, since, since + 86_400, 5, 1), 1);
        assert_eq!(landed_charge(1, since, since + 86_401, 5, 1), 2);
        assert_eq!(landed_charge(2, since, since + 86_400, 50, 2), 2);
        // Ten days, one soul, a locker of 5 with 1 kept for home: capped at 4.
        assert_eq!(landed_charge(1, since, since + 10 * 86_400, 5, 1), 4);
        // A locker already at (or under) the leg home charges nothing.
        assert_eq!(landed_charge(1, since, since + 10 * 86_400, 1, 1), 0);
        assert_eq!(landed_charge(1, since, since + 10 * 86_400, 0, 1), 0);
        assert_eq!(landed_charge(0, since, since + 10 * 86_400, 5, 1), 0);
    }

    /// The stores line's effective figure falls one crew-day a soul a day and stops at the
    /// leg home, then reads short rations.
    #[test]
    fn sm2_stores_line_falls_a_day_at_a_time_and_stops_at_the_leg_home() {
        let (mut world, _registry) = mush_testkit::boot_world();
        let _ = &mut world;
        let since = world.world_ic_now().at;
        let read = |now: i64| stores_line(&world, 5, 1, 1, since, now).unwrap_or_default();
        let figure = |line: &str| -> u64 {
            line.trim_start_matches("Stores ")
                .split(' ')
                .next()
                .and_then(|n| n.parse().ok())
                .unwrap_or(u64::MAX)
        };
        let figures: Vec<u64> = (0..=6).map(|d| figure(&read(since + d * 86_400))).collect();
        assert_eq!(figures, vec![5, 4, 3, 2, 1, 1, 1]);
        assert!(read(since + 86_400).ends_with("here before short rations."));
        assert_eq!(
            read(since + 4 * 86_400),
            "Stores 1 crew-days: on short rations here; the stores for the leg home are kept back."
        );
        assert_eq!(stores_line(&world, 5, 0, 1, since, since), None);
    }
}
