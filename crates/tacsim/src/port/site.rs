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
//!   pools; since doc 20 cut SM.6 (D3) also **the sixteen block vaults**, one per block place,
//!   owned by [`SITES_PRINCIPAL`](crate::port::site::SITES_PRINCIPAL) with no scope and backed
//!   by `Container(block place)` — each block's deposits' endpoint. That is all it lays: **it
//!   submits nothing, credits nothing and computes no geology** (doc 18's refusal; lead ruling
//!   2);
//! - the **site clerk's opening** (doc 20 cut SM.6; §3.4, D-20-5): `site_step`, the port
//!   tick's `Arm::Site`, acting as the clerk through `Fire::as_principal`, one act a fire —
//!   industry's `DepositOpen` for each plan row (`crate::surface::opening`), then the worked
//!   share's `tacsim_site_work` (`SiteWorkProvider`) — and the Site readiness line that reads
//!   it ("opening, N of M", the pad closed until the last row lands). The clerk acts by its
//!   identity in the port map's `SiteRow`, with no grant (D13).

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
use crate::surface::opening::SiteOpening;

/// **The system principal that owns a site's vaults** (doc 20 §3.4; the SM.2 brief's decision
/// 13; cut SM.6, D13): the spoil vault and the sixteen block vaults — the deposits' endpoints.
/// A world object's owner is an `ObjId`, so the Things laid up here (the block places, the
/// clerk, the spoil) stay owned by the applying actor, as the stock clerk is.
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

/// **An org's name** in the pack: its `name` on the port entry that authors it, else its id.
#[must_use]
pub fn org_name(pack: &RulesPack, id: &CanonicalId) -> String {
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
}

/// **One block's tenure word** (doc 20 cut SM.4a; §10's `+tv` status line, D18): `commons`,
/// `open`, or `held by {org}` (the holder's name in `pack`, its id without one). A held block
/// the validator would refuse without a holder reads `open`, as [`tenure_parts`] counts it.
/// Claims are SM.6's and SM.10's: this is the site's authored word.
#[must_use]
pub fn block_tenure(pack: Option<&RulesPack>, site: &SiteDef, letter: char) -> String {
    let block = site.block(letter);
    match (block.tenure, block.holder) {
        (Tenure::Commons, _) => Tenure::Commons.word().to_owned(),
        (Tenure::Open, _) | (Tenure::Held, None) => Tenure::Open.word().to_owned(),
        (Tenure::Held, Some(holder)) => format!(
            "held by {}",
            pack.map_or_else(|| holder.to_string(), |pack| org_name(pack, holder))
        ),
    }
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
    let org_name = |id: &CanonicalId| org_name(pack, id);
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
        && !sites_vault_is_ours(w, vault, false)
    {
        return Err(format!(
            "{id}: the site's spoil vault #{vault} in the port map is gone or not owned by \
             {SITES_PRINCIPAL}; it is never re-created — repair the map by hand"
        ));
    }
    // Doc 20 cut SM.6 (D3): a recorded block vault gone, or not the sites' system
    // principal's, refuses in the spoil's sentence — never re-created.
    for (letter, vault) in &row.vaults {
        if !sites_vault_is_ours(w, *vault, false) {
            return Err(format!(
                "{id}: the site's block {letter} vault #{vault} in the port map is gone or not \
                 owned by {SITES_PRINCIPAL}; it is never re-created — repair the map by hand"
            ));
        }
    }
    Ok(())
}

/// The owner every site vault has (the spoil and the block vaults): `System(tacsim.sites)`.
fn sites_owner() -> Option<PrincipalId> {
    SystemPrincipalId::new(SITES_PRINCIPAL).map(PrincipalId::System)
}

/// Whether vault `id` exists, is owned by `System(tacsim.sites)` with no scope, and (when
/// `active` is asked) is Active — the spoil vault's and (doc 20 cut SM.6) every block vault's
/// one check.
fn sites_vault_is_ours(w: &World, id: u64, active: bool) -> bool {
    let Some(owner) = sites_owner() else {
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
    (live(w, thing) && sites_vault_is_ours(w, vault, true)).then_some(VaultId(vault))
}

/// **A block's vault, checked** (doc 20 cut SM.6, D3): the map's block place live and its
/// vault the sites' system principal's, no scope, Active; `None` otherwise.
#[must_use]
pub(crate) fn block_vault(w: &World, map: &PortMap, letter: char) -> Option<VaultId> {
    let row = map.site.as_ref()?;
    let key = letter.to_string();
    let place = row.blocks.get(&key).copied()?;
    let vault = row.vaults.get(&key).copied()?;
    (live(w, place) && sites_vault_is_ours(w, vault, true)).then_some(VaultId(vault))
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
        let owner = sites_owner()
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
    // Doc 20 cut SM.6 (§3.4; D3): one vault per block place — owned by `System(tacsim.sites)`
    // with no scope, backed by `Container(block place)`, Active — each block's deposits'
    // endpoint, one pool per ore good. Each recorded and the map written; an update lays
    // nothing recorded. Nothing is credited or submitted: the site clerk opens the deposits.
    for letter in BLOCK_LETTERS {
        let key = letter.to_string();
        if map
            .site
            .as_ref()
            .is_some_and(|row| row.vaults.contains_key(&key))
        {
            continue;
        }
        let place = map
            .site
            .as_ref()
            .and_then(|row| row.blocks.get(&key).copied())
            .ok_or_else(|| format!("{id}: block {letter}'s place is not recorded"))?;
        let owner = sites_owner()
            .ok_or_else(|| format!("{id}: {SITES_PRINCIPAL} is not a system principal id"))?;
        let vault = crate::port::apply::alloc_vault_id(ctx)
            .map_err(|e| format!("{id}: block {letter}'s vault: {e}"))?;
        mush_economy::create_vault(
            ctx,
            Vault {
                id: vault,
                owner,
                scope: None,
                name: SmolStr::new(format!("{} block {letter} ground", site.name)),
                backing: CustodyRef::Container(ObjId(place)),
                status: VaultStatus::Active,
                revision: 1,
            },
        );
        map.site
            .get_or_insert_with(SiteRow::default)
            .vaults
            .insert(key, vault.0);
        put_port_map(ctx, map);
    }
    Ok(())
}

/// **The readiness `Site` line** (the SM.2 brief's item 9; reported, never gated on the
/// desk): laid up with all sixteen block places, the clerk, (doc 20 cut SM.3b) the spoil —
/// its Thing live and its vault the sites' system principal's and Active — and (doc 20 cut
/// SM.6, D3) the sixteen block vaults ([`block_vault`]); else Missing with the count (a
/// recorded Thing or vault gone counts as missing). **Doc 20 cut SM.6 (D2):** laid up, the
/// line then reads the site's opening — Ready only when every deposit is open
/// (`SiteOpening::Open`), Missing while it is opening, warming or failed — except with the
/// geology layer dark (no key, a refused key, or no table for the site's body), when nothing
/// can open and the line is Ready ("no deposits while the geology layer is dark"), keeping
/// SM.1's "reported, never gated" and every unkeyed world's travel. `port_ready` reads it,
/// so a Missing line closes the pad (`SITE_CLOSED`): fail-closed, decision 6 — "the site
/// admits no TV before" its deposits exist (§3.4).
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
    let vaults = BLOCK_LETTERS
        .iter()
        .filter(|letter| block_vault(w, map, **letter).is_some())
        .count();
    let opening = match crate::surface::chart::site_context(w, entry) {
        Err(crate::surface::chart::SiteDark::Layer | crate::surface::chart::SiteDark::NoTable) => {
            OpeningWord::Dark
        }
        Err(_) => OpeningWord::Failed,
        Ok(_) if crate::surface::warm_failed(w, entry) => OpeningWord::Failed,
        Ok(_) => match crate::surface::opening::opening_line(w, entry) {
            SiteOpening::Open { total } => OpeningWord::Open(total),
            SiteOpening::Opening { done, total } => OpeningWord::Opening(done, total),
            SiteOpening::Warming => OpeningWord::Warming,
            SiteOpening::Failed => OpeningWord::Failed,
        },
    };
    site_line_text(
        entry,
        blocks,
        clerk,
        spoil_vault(w, map).is_some(),
        vaults,
        opening,
    )
}

/// What a laid-up site's line says of its opening (doc 20 cut SM.6, D2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpeningWord {
    /// The geology layer is dark: nothing can open; Ready.
    Dark,
    /// Every deposit open: Ready.
    Open(u16),
    /// `done` of `total`: Missing.
    Opening(u16, u16),
    /// The ground is still being mapped: Missing.
    Warming,
    /// The ground could not be mapped: Missing.
    Failed,
}

/// [`site_line`]'s words, pure (doc 20 cut SM.6's six rows, verbatim).
fn site_line_text(
    entry: &CanonicalId,
    blocks: usize,
    clerk: bool,
    spoil: bool,
    vaults: usize,
    opening: OpeningWord,
) -> (crate::port::LineState, String) {
    let count = crate::port::def::SITE_BLOCK_COUNT;
    if blocks == count && clerk && spoil && vaults == count {
        let laid = format!(
            "{entry} site: {count} block places, the site clerk, the spoil and {count} block \
             vaults laid up"
        );
        match opening {
            OpeningWord::Dark => (
                crate::port::LineState::Ready,
                format!("{laid}; no deposits while the geology layer is dark."),
            ),
            OpeningWord::Open(total) => (
                crate::port::LineState::Ready,
                format!("{laid}; {total} deposits open."),
            ),
            OpeningWord::Opening(done, total) => (
                crate::port::LineState::Missing,
                format!("{laid}; opening, {done} of {total}."),
            ),
            OpeningWord::Warming => (
                crate::port::LineState::Missing,
                format!("{laid}; opening: its ground is still being mapped."),
            ),
            OpeningWord::Failed => (
                crate::port::LineState::Missing,
                format!("{laid}; opening: its ground could not be mapped (see the log)."),
            ),
        }
    } else {
        let clerk = if clerk {
            "the site clerk"
        } else {
            "no site clerk"
        };
        let spoil = if spoil { "the spoil" } else { "no spoil" };
        (
            crate::port::LineState::Missing,
            format!(
                "{entry} site: {blocks} of {count} block places, {clerk}, {spoil} and {vaults} \
                 of {count} block vaults laid up."
            ),
        )
    }
}

// ---- doc 20 cut SM.6: the site clerk's opening (§3.4, D-20-5) ---------------------------------

/// The worked share's action (doc 20 cut SM.6, §3.4): the clerk's second receipted act.
pub const SITE_WORK_ACTION: &str = "tacsim_site_work";
/// Its outcome: fields `actor`, `deposit`, `receipt` — never a tonne.
pub const SITE_WORK_OUTCOME: &str = "tacsim_site_work_result";
/// A site open's idempotency-key prefix: `tacsim.site.open/{receipt}`.
pub const SITE_OPEN_KEY_PREFIX: &str = "tacsim.site.open/";
/// A worked transfer's idempotency-key prefix: `tacsim.site.work/{receipt}`.
pub const SITE_WORK_KEY_PREFIX: &str = "tacsim.site.work/";
/// Unlanded submissions of one row before the clerk pages (and keeps retrying: a row is
/// never skipped, D4 — the pad must not open without it). The stock arm's count.
pub const MAX_SITE_ATTEMPTS: u8 = 3;

/// **The lane site whose map records `clerk` as its site clerk**, latched (the stock clerk's
/// `clerk_port` shape): its map, its site block and its entry, cloned. The clerk's identity in
/// this row is its whole authority (D13).
pub(crate) fn clerk_site(w: &World, clerk: ObjId) -> Option<(PortMap, SiteDef)> {
    let store = w.module_state::<crate::store::TacsimStore>()?;
    let map = store.port_maps().find(|map| {
        map.pack.as_str() == crate::definitions::content::LANE_PACK
            && map
                .site
                .as_ref()
                .is_some_and(|row| row.clerk == Some(clerk.0))
    })?;
    if !crate::port::stock::entry_latched(w, &map.entry) {
        return None;
    }
    let (_, ports) = crate::port::readiness::report_ports(store, w);
    let site = ports
        .into_iter()
        .find(|(entry, _)| *entry == map.entry)
        .and_then(|(_, def)| def.site.clone())?;
    Some((map.clone(), site))
}

/// **Every latched lane site's live clerk** (the port tick's `Arm::Site` agents), entry order.
pub(crate) fn site_clerks(w: &World) -> Vec<ObjId> {
    let Some(store) = w.module_state::<crate::store::TacsimStore>() else {
        return Vec::new();
    };
    store
        .port_maps()
        .filter(|map| map.pack.as_str() == crate::definitions::content::LANE_PACK)
        .filter(|map| crate::port::stock::entry_latched(w, &map.entry))
        .filter_map(|map| map.site.as_ref().and_then(|row| row.clerk))
        .filter(|clerk| live(w, *clerk))
        .map(ObjId)
        .collect()
}

/// **An org's principal by its lane id** (lead ruling 7): the `OrgId` its port map row
/// records (`freeport_registry` → the Registry), from whichever lane port authors it.
#[must_use]
pub(crate) fn org_principal(w: &World, org: &CanonicalId) -> Option<PrincipalId> {
    w.module_state::<crate::store::TacsimStore>()?
        .port_maps()
        .filter(|map| map.pack.as_str() == crate::definitions::content::LANE_PACK)
        .find_map(|map| map.orgs.get(org))
        .map(|row| PrincipalId::Organization(mush_org::OrgId(row.org)))
}

/// **A site's plan, read without building anything** (lead ruling 1): the site context, the
/// **cached** ground (`site_ground_cached`, never a build), then `opening_plan`.
pub(crate) enum PlanRead {
    /// The layer is dark (no key, a refused key, no table for the body).
    Dark,
    /// The ground is cold: want it.
    Cold,
    /// The ground or the plan failed, with why.
    Failed(String),
    /// The plan.
    Plan(Vec<crate::surface::opening::OpeningRow>),
}

/// [`PlanRead`] for one site.
pub(crate) fn read_plan(w: &World, map: &PortMap, site: &SiteDef) -> PlanRead {
    let context = match crate::surface::chart::site_context(w, &map.entry) {
        Ok(context) => context,
        Err(crate::surface::chart::SiteDark::Layer | crate::surface::chart::SiteDark::NoTable) => {
            return PlanRead::Dark;
        }
        Err(dark) => return PlanRead::Failed(dark.reason()),
    };
    if crate::surface::warm_failed(w, &map.entry) {
        return PlanRead::Failed("its ground did not build this boot".to_owned());
    }
    let ground = match crate::surface::ground::site_ground_cached(w, &context.frame, &context.spec)
    {
        Ok(Some(ground)) => ground,
        Ok(None) => return PlanRead::Cold,
        // A capture that fails darkens the site for the boot (the once-a-boot set SM.4a's
        // `peek_site` uses since its round; the lead's hand fix on the judge's residual 7), so
        // the next fire reads `Failed` above without capturing again.
        Err(error) => {
            crate::surface::fail_site(w, &map.entry);
            return PlanRead::Failed(error);
        }
    };
    let Some(row) = map.site.as_ref() else {
        return PlanRead::Failed("no site row".to_owned());
    };
    match crate::surface::opening::opening_plan(&map.entry, site, row, &ground.geology) {
        Ok(plan) => PlanRead::Plan(plan),
        Err(error) => PlanRead::Failed(error),
    }
}

/// One swapped submission's step: `Submitted`, or why not (the stock arm's).
fn swapped(result: Result<(), crate::port_tick::SwapRefused>) -> crate::port_tick::Step {
    use crate::port_tick::{Step, SwapRefused};
    match result {
        Ok(()) => Step::Submitted,
        Err(SwapRefused::AlreadyActed | SwapRefused::Owed | SwapRefused::Pending) => Step::Deferred,
        Err(SwapRefused::God | SwapRefused::Gone) => Step::Refused,
    }
}

/// Record a site's opening state.
fn record(w: &mut World, entry: &CanonicalId, state: SiteOpening) {
    if let Some(slot) = w.module_state_mut::<crate::surface::opening::SiteOpenings>() {
        slot.states.insert(entry.clone(), state);
    }
}

/// Page `key` once a boot.
fn page_once(w: &mut World, key: String) -> bool {
    w.module_state_mut::<crate::surface::opening::SiteOpenings>()
        .is_some_and(|slot| slot.paged.insert(key))
}

/// What one plan row needs next.
enum RowNeeds {
    /// The deposit is not open.
    Open,
    /// The deposit is open; its worked share is not moved.
    Work,
    /// The deposit was opened under another table: never worked silently (fail closed).
    Mismatch,
    /// Done.
    Done,
}

/// **The site clerk's step** (doc 20 cut SM.6; §3.4, D-20-5, D4): one act a fire, as the clerk
/// (`Fire::as_principal`), in plan order — industry's `DepositOpen` for a row whose deposit is
/// absent, else `tacsim_site_work` for a worked row whose cursor is absent — recording
/// "opening, N of M" until every row is done, then `Open`. A dark layer records nothing and
/// idles (the Site line reads it itself); a cold site is wanted and idles (the surface tick
/// warms one a fire; **nothing here builds a ground**); a failed ground or plan records
/// `Failed` and pages once. A row whose submission never lands pages after
/// [`MAX_SITE_ATTEMPTS`] and is **retried on every later fire, never skipped**. The step is
/// cheap once open: a site recorded `Open` this boot idles without reading the ground.
pub(crate) fn site_step(
    ctx: &mut Ctx<'_>,
    fire: &mut crate::port_tick::Fire,
    clerk: ObjId,
) -> crate::port_tick::Step {
    use crate::port_tick::Step;
    if crate::port_tick::gone(ctx.w, clerk) {
        return Step::Refused;
    }
    let Some((map, site)) = clerk_site(ctx.w, clerk) else {
        return Step::Idle;
    };
    let Some(industry) = ctx
        .w
        .module_state::<mush_industry::IndustryHandles>()
        .map(|handles| handles.0.clone())
    else {
        return Step::Refused;
    };
    let Some(pockets) = ctx
        .w
        .module_state::<crate::port::families::PortHandles>()
        .map(|handles| handles.families.pockets.clone())
    else {
        return Step::Refused;
    };
    let entry = map.entry.clone();
    if matches!(
        crate::surface::opening::opening_line(ctx.w, &entry),
        SiteOpening::Open { .. }
    ) {
        return Step::Idle;
    }
    let plan = match read_plan(ctx.w, &map, &site) {
        PlanRead::Dark => return Step::Idle,
        PlanRead::Cold => {
            crate::surface::want_site_ground(ctx.w, &entry);
            record(ctx.w, &entry, SiteOpening::Warming);
            return Step::Idle;
        }
        PlanRead::Failed(error) => {
            record(ctx.w, &entry, SiteOpening::Failed);
            if page_once(ctx.w, format!("failed/{entry}")) {
                tracing::error!(
                    site = %entry,
                    %error,
                    runbook = "doc 20 cut SM.6: the site cannot open (its pad stays closed); \
                               repair the site's ground or its port map, then reboot",
                    "tacsim: a site's opening cannot be planned"
                );
            }
            return Step::Idle;
        }
        PlanRead::Plan(plan) => plan,
    };
    let total = u16::try_from(plan.len()).unwrap_or(u16::MAX);
    let mut done = 0_u16;
    let mut next: Option<(usize, RowNeeds)> = None;
    for (i, row) in plan.iter().enumerate() {
        let deposit = match ctx.action_state(&industry.deposits, &row.deposit) {
            Ok(found) => found.cloned(),
            Err(_) => return Step::Refused,
        };
        let needs = match deposit {
            None => RowNeeds::Open,
            Some(_) if row.worked_t == 0 => RowNeeds::Done,
            Some(deposit) => match ctx.action_state(&pockets, &row.deposit) {
                Err(_) => return Step::Refused,
                Ok(Some(_)) => RowNeeds::Done,
                Ok(None) if deposit.initial_reserve() != row.gross_t => RowNeeds::Mismatch,
                Ok(None) => RowNeeds::Work,
            },
        };
        match needs {
            RowNeeds::Done => done = done.saturating_add(1),
            other => {
                if next.is_none() {
                    next = Some((i, other));
                }
            }
        }
    }
    let Some((i, needs)) = next else {
        record(ctx.w, &entry, SiteOpening::Open { total });
        if let Some(slot) = ctx
            .w
            .module_state_mut::<crate::surface::opening::SiteOpenings>()
        {
            let prefix = format!("{entry}/");
            slot.attempts.retain(|key, _| {
                !key.split_once('/')
                    .is_some_and(|(_, r)| r.starts_with(&prefix))
            });
        }
        return Step::Idle;
    };
    record(ctx.w, &entry, SiteOpening::Opening { done, total });
    let Some(row) = plan.get(i) else {
        return Step::Refused;
    };
    let (action, kind, prefix, parameters) = match needs {
        RowNeeds::Open => {
            // Lead ruling 7 (review F3): every deposit records the site's issuer as its
            // jurisdiction, and a deposit is never re-opened — so an issuer with no lane org
            // row opens nothing (the composed opener offers nothing either), the site reads
            // `Failed` and its pad stays closed until the map is repaired.
            if org_principal(ctx.w, &site.issuer).is_none() {
                record(ctx.w, &entry, SiteOpening::Failed);
                if page_once(ctx.w, format!("issuer/{entry}")) {
                    tracing::error!(
                        site = %entry,
                        issuer = %site.issuer,
                        runbook = "doc 20 cut SM.6: the site's issuer has no org row on any lane \
                                   port map, so no deposit can record its jurisdiction; the pad \
                                   stays closed — repair the port map, then reboot",
                        "tacsim: a site's issuer does not resolve"
                    );
                }
                return Step::Idle;
            }
            let Some(action) =
                mush_industry::action_ref(ctx.reg, mush_industry::IndustryVerb::DepositOpen)
            else {
                return Step::Refused;
            };
            (
                action,
                "open",
                SITE_OPEN_KEY_PREFIX,
                serde_json::json!({ "receipt": row.receipt }),
            )
        }
        RowNeeds::Work => {
            let Some(action) = ctx
                .w
                .module_state::<crate::port::families::PortHandles>()
                .and_then(|handles| handles.site_work.clone())
            else {
                return Step::Refused;
            };
            (
                action,
                "work",
                SITE_WORK_KEY_PREFIX,
                serde_json::json!({ "receipt": row.receipt }),
            )
        }
        RowNeeds::Mismatch => {
            if page_once(ctx.w, format!("mismatch/{}", row.receipt)) {
                tracing::error!(
                    site = %entry,
                    receipt = %row.receipt,
                    runbook = "doc 20 cut SM.6: a deposit was opened under another geology \
                               table; it is never worked silently and the pad stays closed — \
                               staff reconcile the deposit, then reboot",
                    "tacsim: a site deposit's reserve does not match its ground"
                );
            }
            return Step::Held;
        }
        RowNeeds::Done => return Step::Idle,
    };
    let attempt_key = format!("{kind}/{}", row.receipt);
    let attempts = ctx
        .w
        .module_state::<crate::surface::opening::SiteOpenings>()
        .and_then(|slot| slot.attempts.get(&attempt_key).copied())
        .unwrap_or(0);
    if attempts >= MAX_SITE_ATTEMPTS
        && !ctx.authoritative_pending(clerk)
        && page_once(ctx.w, attempt_key.clone())
    {
        tracing::error!(
            site = %entry,
            receipt = %row.receipt,
            act = kind,
            attempts,
            runbook = "doc 20 cut SM.6: a site row's act never lands; the clerk keeps retrying \
                       and the pad stays closed until it does",
            "tacsim: a site clerk's act never lands"
        );
    }
    let Ok(key) =
        mush_core::IdempotencyKeyText::parse_canonical(format!("{prefix}{}", row.receipt))
    else {
        return Step::Refused;
    };
    let step = swapped(fire.as_principal(ctx, clerk, clerk, |ctx| {
        ctx.submit_authoritative(
            action,
            mush_core::PerceivedRef::SelfActor,
            Vec::new(),
            parameters,
            key,
        );
    }));
    if step == Step::Submitted
        && let Some(slot) = ctx
            .w
            .module_state_mut::<crate::surface::opening::SiteOpenings>()
        && (slot.attempts.contains_key(&attempt_key)
            || slot.attempts.len()
                < crate::surface::opening::MAX_DEPOSITS_PER_SITE.saturating_mul(16))
    {
        slot.attempts
            .insert(attempt_key, attempts.saturating_add(1));
    }
    step
}

/// **The one work row a clerk's offer carries** (reviewer F5): the first plan row, in plan
/// order, with a worked share, whose deposit exists and whose cursor is absent — the row
/// [`site_step`] submits, since it walks the same plan in the same order and acts on the first
/// undone row. `Err(receipt)` when that row's deposit was opened under another table (its
/// reserve is not the plan's gross): the offer is then withheld, never skipping the row.
fn next_work_row(
    engine: &mush_core::Engine,
    pockets: &mush_core::StateFamilyHandle<
        mush_industry::OreDepositId,
        crate::surface::cursor::PocketCursor,
    >,
    clerk: ObjId,
) -> Option<Result<(PortMap, crate::surface::opening::OpeningRow), String>> {
    let w = &engine.world;
    let (map, site) = clerk_site(w, clerk)?;
    let industry = w.module_state::<mush_industry::IndustryHandles>()?;
    let PlanRead::Plan(plan) = read_plan(w, &map, &site) else {
        return None;
    };
    let state = engine.action_state();
    for row in plan {
        if row.worked_t == 0 {
            continue;
        }
        let Ok(deposit) = state.get(&industry.0.deposits, &row.deposit) else {
            return None;
        };
        let Some(deposit) = deposit else {
            continue;
        };
        match state.get(pockets, &row.deposit) {
            Ok(Some(_)) => continue,
            Ok(None) => {}
            Err(_) => return None,
        }
        if deposit.initial_reserve() != row.gross_t {
            return Some(Err(row.receipt));
        }
        return Some(Ok((map, row)));
    }
    None
}

/// **`tacsim_site_work`** (doc 20 cut SM.6; §3.4, D6, D15): the worked share of one (block,
/// family) moved from the deposit's pool into the site's spoil, and the deposit's pocket cursor
/// inserted, **in one bundle**. Offered to a site clerk only, carrying the next pending row
/// only (`next_work_row`); the sidecar's existence is the replay guard. No player is ever
/// offered it, and its outcome names no tonne.
pub(crate) struct SiteWorkProvider {
    /// The action.
    pub action: mush_core::DefinitionRef,
    /// The outcome.
    pub outcome: mush_core::DefinitionRef,
    /// `tacsim.pockets`.
    pub pockets: mush_core::StateFamilyHandle<
        mush_industry::OreDepositId,
        crate::surface::cursor::PocketCursor,
    >,
    /// The delegated inventory families.
    pub inventory: mush_inventory::InventoryFamilies,
    /// Receipts whose mismatch the offer has logged this boot.
    pub warned: std::sync::Mutex<std::collections::BTreeSet<String>>,
}

fn work_invalid() -> mush_core::ActionError {
    crate::port_tick::port_error(
        mush_core::ActionErrorCode::InvalidRequest,
        "the site work is self-scoped, takes no targets and names one receipt",
    )
}

/// One offered work row as the offer's fixed `rows` value.
fn work_row_value(
    map: &PortMap,
    w: &World,
    row: &crate::surface::opening::OpeningRow,
    clerk: ObjId,
) -> Option<serde_json::Value> {
    let spoil = spoil_vault(w, map)?;
    Some(serde_json::json!({
        "deposit": crate::surface::cursor::cursor_key_text(&row.deposit),
        "commodity": row.good.as_str(),
        "from": row.vault.0,
        "to": spoil.0,
        "worked_t": row.worked_t,
        "cursor": row.cursor,
        "clerk": clerk.0,
    }))
}

impl mush_core::ActionProvider for SiteWorkProvider {
    fn validate_invocation(
        &self,
        invocation: &mush_core::ActionInvocation,
    ) -> Result<(), mush_core::ActionError> {
        if invocation.subject() != &mush_core::PerceivedRef::SelfActor
            || !invocation.targets().is_empty()
            || invocation
                .parameters()
                .get("receipt")
                .and_then(serde_json::Value::as_str)
                .is_none()
        {
            return Err(work_invalid());
        }
        Ok(())
    }

    fn project_offer(
        &self,
        action: &mush_core::DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, mush_core::ActionError> {
        let mush_core::SubjectKey::Object { id: subject } = query.truth_subject() else {
            return Ok(None);
        };
        if action != &self.action
            || query.actor() != *subject
            || query.projected_subject() != &mush_core::PerceivedRef::SelfActor
        {
            return Ok(None);
        }
        let clerk = query.actor();
        let (map, row) = match next_work_row(engine, &self.pockets, clerk) {
            None => return Ok(None),
            Some(Err(receipt)) => {
                if let Ok(mut warned) = self.warned.lock()
                    && warned.insert(receipt.clone())
                {
                    tracing::error!(
                        %receipt,
                        "tacsim: a site deposit's reserve is not its plan's gross; its worked \
                         share is never moved silently"
                    );
                }
                return Ok(None);
            }
            Some(Ok(found)) => found,
        };
        let Some(value) = work_row_value(&map, &engine.world, &row, clerk) else {
            return Ok(None);
        };
        let integrity = |message: &'static str| {
            crate::port_tick::port_error(mush_core::ActionErrorCode::Integrity, message)
        };
        let definitions = engine
            .registry
            .definitions()
            .ok_or_else(|| integrity("the site work's definition registry is unavailable"))?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| integrity("the site work's guard registry is unavailable"))?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| integrity("the site work's definition guard is unavailable"))?;
        let receipt = mush_core::ProjectedParameter {
            id: CanonicalId::new("receipt")
                .map_err(|_| integrity("the site work's parameter id is invalid"))?,
            required: true,
            kind: mush_core::ProjectedParameterKind::Choice {
                choices: vec![row.receipt.clone()],
            },
        };
        let mut rows = serde_json::Map::new();
        rows.insert(row.receipt.clone(), value);
        Ok(Some(mush_core::ProviderOfferDraft {
            fixed_parameters: serde_json::json!({ "rows": serde_json::Value::Object(rows) }),
            parameters: mush_core::ProjectedParameterSchema::new(vec![receipt])
                .map_err(|_| integrity("the site work's parameter schema is invalid"))?,
            targets: mush_core::ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "tacsim-site-work",
                "Work a site block",
                "A site clerk moves a block's worked share into the spoil.",
            )
            .map_err(|_| integrity("the site work's presentation is invalid"))?,
            guards: guards
                .finish()
                .map_err(|_| integrity("the site work's offer guards are invalid"))?,
        }))
    }

    fn prepare_authoritative(
        &self,
        invocation: &mush_core::ActionInvocation,
        txn: &mut mush_core::ActionTxn<'_>,
    ) -> Result<mush_core::AuthoritativeActionOutput, mush_core::ActionError> {
        use mush_core::ActionErrorCode;
        use serde_json::Value;
        let error = crate::port_tick::port_error;
        self.validate_invocation(invocation)?;
        let parameters = invocation.parameters();
        let receipt = parameters
            .get("receipt")
            .and_then(Value::as_str)
            .ok_or_else(work_invalid)?;
        let entry = parameters
            .get("rows")
            .and_then(Value::as_object)
            .and_then(|rows| rows.get(receipt))
            .and_then(Value::as_object)
            .ok_or_else(work_invalid)?;
        let number = |key: &str| {
            entry
                .get(key)
                .and_then(Value::as_u64)
                .ok_or_else(work_invalid)
        };
        let deposit = entry
            .get("deposit")
            .and_then(Value::as_str)
            .and_then(crate::surface::cursor::cursor_key_parse)
            .ok_or_else(work_invalid)?;
        let commodity = entry
            .get("commodity")
            .and_then(Value::as_str)
            .and_then(|good| CanonicalId::new(good).ok())
            .ok_or_else(work_invalid)?;
        let from = number("from")?;
        let to = number("to")?;
        let worked_t = number("worked_t")?;
        let clerk = u32::try_from(number("clerk")?).map_err(|_| work_invalid())?;
        let cursor: Vec<crate::surface::cursor::CursorRow> = entry
            .get("cursor")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .ok_or_else(work_invalid)?;
        // 1. The row is the acting clerk's (D13: the clerk's identity is its authority).
        let actor = invocation.actor();
        if clerk != actor.0 {
            return Err(error(
                ActionErrorCode::Unauthorized,
                "that site row is another clerk's",
            ));
        }
        // The deposit is the receipt's own (the receipt is the id, D5).
        if mush_industry::derive_system_deposit_id(receipt).ok() != Some(deposit) {
            return Err(error(
                ActionErrorCode::Integrity,
                "the site row does not name its own deposit",
            ));
        }
        // 2. The sidecar is the replay guard.
        let prior = txn.get(&self.pockets, &deposit).map_err(|_| {
            error(
                ActionErrorCode::Integrity,
                "the pocket cursors are unavailable",
            )
        })?;
        if prior.is_some() {
            return Err(error(
                ActionErrorCode::Conflict,
                "that block is already worked",
            ));
        }
        // 4 (checked first, so nothing is staged for a bad row): the cursor accounts for the
        // whole transfer.
        let record = crate::surface::cursor::PocketCursor {
            schema: crate::surface::cursor::POCKET_CURSOR_SCHEMA,
            deposit,
            rows: cursor,
        };
        if record.check().is_err() || record.drawn_total() != Some(worked_t) || worked_t == 0 {
            return Err(error(
                ActionErrorCode::Integrity,
                "the site row's cursor does not account for its share",
            ));
        }
        // 3. The share moves, deposit pool to spoil, in this bundle.
        self.inventory
            .transfer(
                txn,
                mush_inventory::EndpointRef(VaultId(from)),
                mush_inventory::EndpointRef(VaultId(to)),
                &commodity,
                worked_t,
            )
            .map_err(|_| {
                error(
                    ActionErrorCode::ProviderRejected,
                    "the block's worked share could not be moved",
                )
            })?;
        txn.insert(&self.pockets, &deposit, record).map_err(|_| {
            error(
                ActionErrorCode::Integrity,
                "the pocket cursor did not stage",
            )
        })?;
        let outcome = mush_core::RulesValue::new(
            self.outcome.clone(),
            serde_json::json!({
                "actor": actor.0,
                "deposit": crate::surface::cursor::cursor_key_text(&deposit),
                "receipt": receipt,
            }),
        )
        .map_err(|_| {
            error(
                ActionErrorCode::BoundsExceeded,
                "the site work's outcome exceeds canonical bounds",
            )
        })?;
        Ok(mush_core::AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
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

    /// Doc 20 cut SM.3b, re-blessed by SM.6 (D2, D3): the `Site` line's texts — the spoil and
    /// the block vaults counted, and a laid-up site's opening read — the six rows, verbatim.
    #[test]
    fn sm6_the_site_line_reads_the_vaults_and_the_opening() {
        let entry = cid("kerwan_port");
        let laid = "kerwan_port site: 16 block places, the site clerk, the spoil and 16 block \
                    vaults laid up";
        assert_eq!(
            site_line_text(&entry, 16, true, true, 16, OpeningWord::Dark),
            (
                crate::port::LineState::Ready,
                format!("{laid}; no deposits while the geology layer is dark.")
            )
        );
        assert_eq!(
            site_line_text(&entry, 16, true, true, 16, OpeningWord::Open(41)),
            (
                crate::port::LineState::Ready,
                format!("{laid}; 41 deposits open.")
            )
        );
        assert_eq!(
            site_line_text(&entry, 16, true, true, 16, OpeningWord::Opening(7, 41)),
            (
                crate::port::LineState::Missing,
                format!("{laid}; opening, 7 of 41.")
            )
        );
        assert_eq!(
            site_line_text(&entry, 16, true, true, 16, OpeningWord::Warming),
            (
                crate::port::LineState::Missing,
                format!("{laid}; opening: its ground is still being mapped.")
            )
        );
        assert_eq!(
            site_line_text(&entry, 16, true, true, 16, OpeningWord::Failed),
            (
                crate::port::LineState::Missing,
                format!("{laid}; opening: its ground could not be mapped (see the log).")
            )
        );
        assert_eq!(
            site_line_text(&entry, 16, true, false, 16, OpeningWord::Dark),
            (
                crate::port::LineState::Missing,
                "kerwan_port site: 16 of 16 block places, the site clerk, no spoil and 16 of 16 \
                 block vaults laid up."
                    .to_owned()
            )
        );
        assert_eq!(
            site_line_text(&entry, 15, false, true, 3, OpeningWord::Open(4)),
            (
                crate::port::LineState::Missing,
                "kerwan_port site: 15 of 16 block places, no site clerk, the spoil and 3 of 16 \
                 block vaults laid up."
                    .to_owned()
            )
        );
    }

    /// Doc 20 cut SM.6 (reviewer F5): the largest work row — a 64-row cursor of 24 t pockets
    /// in the 200s and the longest lane receipt — fits the offer's fixed parameters with room to
    /// spare (measured ≈ 2.6 KiB against `MAX_ACTION_PARAMETER_BYTES`).
    #[test]
    fn sm6_the_largest_work_row_fits_the_offer() {
        let cursor: Vec<crate::surface::cursor::CursorRow> = (193..=256)
            .map(|pocket| crate::surface::cursor::CursorRow {
                pocket,
                drawn_t: 24,
                lock: None,
            })
            .collect();
        let receipt = "rheasilvia_port/p/strategic_elements";
        let mut rows = serde_json::Map::new();
        rows.insert(
            receipt.to_owned(),
            serde_json::json!({
                "deposit": "ffffffffffffffff",
                "commodity": "strategic_ore",
                "from": u64::MAX,
                "to": u64::MAX,
                "worked_t": 64 * 24,
                "cursor": cursor,
                "clerk": u32::MAX,
            }),
        );
        let bytes = serde_json::to_vec(&serde_json::json!({ "rows": rows }))
            .unwrap_or_default()
            .len();
        assert!(
            bytes > 1_500 && bytes < 4_096,
            "the largest work row is {bytes} bytes"
        );
        assert!(bytes < mush_core::MAX_ACTION_PARAMETER_BYTES / 16);
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

    /// **One block's tenure word** (doc 20 cut SM.4a; the `+tv` status's ground line, D18; the
    /// fixer round's finding T5), over the shipped lane's own sites: Occator Rim's F, G, J and K
    /// read `held by Occator Water & Propellant` (the holder's name, never its id) and its other
    /// blocks `open`; Freeport Commons's blocks `commons`; Ahuna Flank's `open`. Without the
    /// pack the holder's id stands; a held block without a holder reads `open`.
    #[test]
    fn sm4a_block_tenure_names_commons_open_and_the_holder() {
        let Ok(pack) = crate::pack::load_pack(crate::definitions::content::LANE_JSON) else {
            panic!("the shipped lane loads");
        };
        let site_of = |entry: &str| match pack.entry(&cid(entry)) {
            Some(PackEntry::Port(port)) => port
                .site
                .clone()
                .unwrap_or_else(|| panic!("{entry} is a site")),
            _ => panic!("{entry} is a port entry"),
        };
        let rim = site_of("occator_port");
        for letter in ['F', 'G', 'J', 'K'] {
            assert_eq!(
                block_tenure(Some(&pack), &rim, letter),
                "held by Occator Water & Propellant",
                "{letter}"
            );
        }
        assert_eq!(block_tenure(Some(&pack), &rim, 'A'), "open");
        assert_eq!(block_tenure(None, &rim, 'F'), "held by occator");
        let commons = site_of("commons_port");
        for letter in BLOCK_LETTERS {
            assert_eq!(block_tenure(Some(&pack), &commons, letter), "commons");
        }
        assert_eq!(
            block_tenure(Some(&pack), &site_of("ahuna_port"), 'K'),
            "open"
        );
        let headless = site(&[(
            "F",
            BlockDef {
                tenure: Some(Tenure::Held),
                ..BlockDef::default()
            },
        )]);
        assert_eq!(block_tenure(Some(&pack), &headless, 'F'), "open");
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
