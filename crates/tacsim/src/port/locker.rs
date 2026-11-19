//! **A player's locker at a venue** (doc 48 cut EX.1b; Q-48-49, doc 48b W-48-2c and G-6).
//!
//! An officer or a passenger without a hull had nowhere at a venue to take goods into. A locker is
//! that place: one economy vault per player and venue, opened on first use (`+locker/open`, where
//! the player stands in the venue). The venue's gate delivers into it when no hold of theirs is
//! docked there.
//!
//! - **The vault** is the player's, backed by a container: a locker object in LIMBO owned by
//!   the player, so the economy's built-in custody provider serves it (a backing no provider
//!   answers is never a live endpoint). A vault is a locker when a locker record names it; a
//!   locker is never a hull's hold, so the hull's checks never apply to it.
//! - **Capacity-bound.** The market's capacity plan of a locker vault is one compartment that
//!   takes any stowage, held to [`LOCKER_KG`] and [`LOCKER_L`] (provisional). Before EX.1b an
//!   off-hold vault had no plan, so it was free storage.
//! - **What it owes.** The charge is [`LOCKER_MINOR_PER_T_DAY`] an IC day per started tonne held
//!   (provisional, the berth's rate). **Doc 48 cut EX.4** collects it as a player-owed obligation
//!   (`crate::port::berth`): repriced to the tonnes held as they move, its unpaid days accruing as
//!   arrears, so a locker never blocks a trade over what it owes. The record's `accrued_to_ic` and
//!   `owed_minor` are EX.1b's display tally, kept on the wire and no longer moved.
//!
//! The record is `tacsim:locker:{player}:{venue}`, in the store's own block.

use mush_core::{CanonicalId, Ctx, ObjId, World};
use mush_economy::{CustodyRef, EconomyStore, Vault, VaultId, VaultStatus};
use mush_inventory::record::EndpointRef;
use mush_market::MarketId;
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::definitions::logistics::{MASS_KG, VOLUME_L};
use crate::port::market::{HOLD_PLAN_VERSION, HoldPlan, PlanCompartment};
use crate::store::TacsimStore;

/// The record's key prefix.
pub const LOCKER_PREFIX: &str = "tacsim:locker:";
/// The record's schema.
pub const LOCKER_SCHEMA: u16 = 1;
/// A locker's mass limit, kilograms (provisional).
pub const LOCKER_KG: u64 = 20_000;
/// A locker's volume limit, litres (provisional).
pub const LOCKER_L: u64 = 40_000;
/// The charge, minor units an IC day per started tonne (provisional: 1 cr, the berth's rate).
pub const LOCKER_MINOR_PER_T_DAY: i64 = 100;
/// IC seconds in an IC day.
const IC_DAY: i64 = 86_400;

/// The player's line when a locker opens (`{venue}`).
pub const LOCKER_OPENED: &str =
    "Your locker at {venue} is open. Goods you buy here with no hold docked go into it.";
/// The player's line when it was already open (`{venue}`).
pub const LOCKER_ALREADY: &str = "Your locker at {venue} is already open.";
/// The refusal when they stand in no venue.
pub const LOCKER_NO_VENUE: &str = "There is no market here to keep a locker at.";
/// The show line (`{venue}`, `{tonnes}`, `{rate}`, `{owed}`; doc 48 cut EX.4).
pub const LOCKER_SHOW: &str =
    "Your locker at {venue}: {tonnes} t held, charged {rate} an IC day; {owed} in arrears.";
/// The show line when they hold none here (`{venue}`).
pub const LOCKER_NONE: &str = "You have no locker at {venue}. +locker/open opens one.";

/// **One player's locker at one venue.**
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Locker {
    /// [`LOCKER_SCHEMA`].
    pub schema: u16,
    /// The player.
    pub player: ObjId,
    /// The venue's word.
    pub venue: SmolStr,
    /// The vault, `VaultId.0`.
    pub vault: u64,
    /// The locker object backing the vault.
    pub container: ObjId,
    /// The IC second it opened.
    pub opened_ic: i64,
    /// The IC second the charge is accrued to.
    pub accrued_to_ic: i64,
    /// What it owes and nothing has collected, minor units.
    pub owed_minor: i64,
}

impl Locker {
    /// The record's own rules.
    ///
    /// # Errors
    ///
    /// The rule broken, in words.
    pub fn check(&self) -> Result<(), String> {
        if self.schema != LOCKER_SCHEMA || self.venue.is_empty() || self.player.0 == 0 {
            return Err("a locker names its schema, its player and its venue".to_owned());
        }
        if self.accrued_to_ic < self.opened_ic || self.owed_minor < 0 {
            return Err(
                "a locker accrues forward from its opening and owes nothing negative".to_owned(),
            );
        }
        Ok(())
    }

    /// **Accrue the charge to `now_ic`** in whole IC days, at `held_kg` (every started tonne
    /// counts). Returns the days added; a clock that moved back adds nothing.
    pub fn accrue(&mut self, held_kg: u64, now_ic: i64) -> i64 {
        let days = now_ic.saturating_sub(self.accrued_to_ic).max(0) / IC_DAY;
        if days == 0 {
            return 0;
        }
        let tonnes = i64::try_from(held_kg.div_ceil(1_000)).unwrap_or(i64::MAX);
        let charge = tonnes
            .saturating_mul(days)
            .saturating_mul(LOCKER_MINOR_PER_T_DAY);
        self.owed_minor = self.owed_minor.saturating_add(charge);
        self.accrued_to_ic = self
            .accrued_to_ic
            .saturating_add(days.saturating_mul(IC_DAY));
        days
    }
}

/// The record's key.
#[must_use]
pub fn locker_key(player: ObjId, venue: &str) -> String {
    format!("{LOCKER_PREFIX}{}:{venue}", player.0)
}

/// The `(player, venue)` a key names.
#[must_use]
pub fn locker_key_owner(key: &str) -> Option<(ObjId, SmolStr)> {
    let rest = key.strip_prefix(LOCKER_PREFIX)?;
    let (player, venue) = rest.split_once(':')?;
    let player = player.parse::<u32>().ok().filter(|id| *id != 0)?;
    (!venue.is_empty()).then(|| (ObjId(player), SmolStr::new(venue)))
}

/// Whether `endpoint` is a locker's vault: a locker record names it.
#[must_use]
pub fn is_locker(w: &World, endpoint: EndpointRef) -> bool {
    w.module_state::<TacsimStore>().is_some_and(|store| {
        store
            .lockers
            .values()
            .any(|locker| locker.vault == endpoint.vault().0)
    })
}

/// **A locker's capacity plan**: one compartment taking every stowage the goods table names,
/// held to [`LOCKER_KG`] and [`LOCKER_L`]. `None` when the goods table does not load.
#[must_use]
pub fn locker_plan() -> Option<HoldPlan> {
    let goods = crate::definitions::goods::goods_table().ok()?;
    let digest = crate::definitions::goods::goods_digest(crate::definitions::goods::GOODS_JSON)
        .ok()?
        .to_hex();
    let accepts: std::collections::BTreeSet<CanonicalId> =
        goods.goods.iter().map(|g| g.stowage.clone()).collect();
    let mass = CanonicalId::new(MASS_KG).ok()?;
    let volume = CanonicalId::new(VOLUME_L).ok()?;
    Some(HoldPlan {
        v: HOLD_PLAN_VERSION,
        goods: digest,
        compartments: vec![PlanCompartment {
            id: CanonicalId::new("locker").ok()?,
            accepts,
            limits: [(mass, LOCKER_KG), (volume, LOCKER_L)]
                .into_iter()
                .collect(),
        }],
    })
}

/// The player's locker at `venue`, when it is open.
#[must_use]
pub fn locker_of<'a>(w: &'a World, player: ObjId, venue: &MarketId) -> Option<&'a Locker> {
    w.module_state::<TacsimStore>()?
        .lockers
        .get(&(player, SmolStr::new(venue.as_str())))
}

/// **Open the player's locker at `venue`** (first use): the vault, then the record. `Ok(false)`
/// when it was open already.
///
/// # Errors
///
/// The sentence for a world with no economy, or a backing that will not form.
pub fn open(ctx: &mut Ctx<'_>, player: ObjId, venue: &MarketId) -> Result<bool, String> {
    if locker_of(ctx.w, player, venue).is_some() {
        return Ok(false);
    }
    let vault = ctx
        .w
        .module_state_mut::<EconomyStore>()
        .ok_or("The economy is not running.")?
        .alloc_vault()
        .ok_or("No vault can be opened right now.")?;
    let name = format!("locker at {}", venue.as_str());
    let container = ctx
        .w
        .create(
            mush_core::ObjKind::Thing,
            &name,
            player,
            Some(mush_core::LIMBO),
        )
        .map_err(|_| "The locker cannot be made right now.".to_owned())?;
    mush_economy::create_vault(
        ctx,
        Vault {
            id: vault,
            owner: PrincipalId::Object(player),
            scope: None,
            name: SmolStr::new(name),
            backing: CustodyRef::Container(container),
            status: VaultStatus::Active,
            revision: 1,
        },
    );
    let now = ctx.w.game_now_unix();
    let record = Locker {
        schema: LOCKER_SCHEMA,
        player,
        venue: SmolStr::new(venue.as_str()),
        vault: vault.0,
        container,
        opened_ic: now,
        accrued_to_ic: now,
        owed_minor: 0,
    };
    crate::store::put_locker(ctx, &record);
    Ok(true)
}

/// The kilograms a locker's vault holds, over every good of the goods table.
#[must_use]
pub fn held_kg(ctx: &Ctx<'_>, vault: VaultId) -> u64 {
    let Some(goods) = ctx
        .w
        .module_state::<TacsimStore>()
        .and_then(TacsimStore::goods)
    else {
        return 0;
    };
    goods
        .goods
        .iter()
        .map(|good| {
            // Doc 48e (48d C-03): a divisible good's lots in the locker pay rent too.
            let units = crate::port_tick::read_held(ctx, EndpointRef(vault), &good.id).unwrap_or(0);
            good.mass_kg_per_unit().unwrap_or(0).saturating_mul(units)
        })
        .fold(0u64, u64::saturating_add)
}

/// **`+locker`'s spec** (doc 48 cut EX.1b): the verb, its one switch and its help.
#[must_use]
pub(crate) fn locker_spec() -> mush_core::CommandSpec {
    mush_core::CommandSpec {
        name: "+locker".into(),
        shape: mush_core::ArgShape::Raw,
        switches: ["open"].iter().copied().map(SmolStr::new).collect(),
        help: "+locker                          your locker at the market you stand in: what it \
               holds and what it owes\n\
               +locker/open                     open one here. A locker holds 20 t and 40 m3. \
               Goods you buy with no hold of yours docked here go into it. It costs 1 cr an IC \
               day a started tonne, from your own account; a day you cannot pay is owed as \
               arrears, paid when your account is funded."
            .into(),
        ..mush_core::CommandSpec::default()
    }
}

/// **`+locker`** (doc 48 cut EX.1b): your locker at the venue you stand in, what it holds and
/// what it owes; `+locker/open` opens it. Showing accrues what it owes to today.
///
/// # Errors
///
/// The layer off, or no venue here.
pub(crate) fn locker_cmd(ctx: &mut Ctx<'_>) -> Result<(), mush_core::CmdError> {
    crate::commands::require_enabled(ctx)?;
    let actor = ctx.actor;
    let venue = ctx
        .w
        .get(actor)
        .and_then(|o| o.location)
        .and_then(|room| mush_market::venue_gate(ctx.w)?.venue_of_room(ctx.w, room))
        .ok_or_else(|| mush_core::CmdError::msg(LOCKER_NO_VENUE))?;
    let line = if ctx.switches.has("open") {
        let opened = open(ctx, actor, &venue).map_err(mush_core::CmdError::msg)?;
        let line = if opened {
            LOCKER_OPENED
        } else {
            LOCKER_ALREADY
        };
        line.replace("{venue}", venue.as_str())
    } else if let Some(record) = locker_of(ctx.w, actor, &venue).cloned() {
        let held = held_kg(ctx, VaultId(record.vault));
        let tonnes = i64::try_from(held.div_ceil(1_000)).unwrap_or(i64::MAX);
        let ns = crate::port::berth::locker_namespace(venue.as_str(), actor);
        let owed = crate::port::berth::arrears_of(ctx.w, &PrincipalId::Object(actor), &ns);
        let money =
            |minor: i64| crate::port::board::money(ctx.w, mush_economy::Amount::from_minor(minor));
        LOCKER_SHOW
            .replace("{venue}", venue.as_str())
            .replace("{tonnes}", &crate::port::gauges::part_tonnes(held))
            .replace(
                "{rate}",
                &money(tonnes.saturating_mul(LOCKER_MINOR_PER_T_DAY)),
            )
            .replace("{owed}", &money(owed))
    } else {
        LOCKER_NONE.replace("{venue}", venue.as_str())
    };
    ctx.out.sys_to_actor(actor, "tacsim", line);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locker() -> Locker {
        Locker {
            schema: LOCKER_SCHEMA,
            player: ObjId(7),
            venue: SmolStr::new("ceres"),
            vault: 40,
            container: ObjId(41),
            opened_ic: 1_000,
            accrued_to_ic: 1_000,
            owed_minor: 0,
        }
    }

    #[test]
    fn ex1b_a_locker_accrues_whole_days_per_started_tonne() {
        let mut l = locker();
        // Under a day: nothing.
        assert_eq!(l.accrue(2_500, 1_000 + IC_DAY - 1), 0);
        assert_eq!(l.owed_minor, 0);
        // Two days at 2.5 t: three started tonnes, 1 cr each a day.
        assert_eq!(l.accrue(2_500, 1_000 + 2 * IC_DAY + 5), 2);
        assert_eq!(l.owed_minor, 2 * 3 * LOCKER_MINOR_PER_T_DAY);
        assert_eq!(l.accrued_to_ic, 1_000 + 2 * IC_DAY);
        // An empty locker owes nothing more; a clock that moved back adds nothing.
        assert_eq!(l.accrue(0, 1_000 + 3 * IC_DAY), 1);
        assert_eq!(l.owed_minor, 6 * LOCKER_MINOR_PER_T_DAY);
        assert_eq!(l.accrue(9_000, 10), 0);
        assert!(l.check().is_ok());
    }

    #[test]
    fn ex1b_a_locker_key_names_its_owner_and_venue() {
        let key = locker_key(ObjId(42), "stickney");
        assert_eq!(key, "tacsim:locker:42:stickney");
        assert_eq!(
            locker_key_owner(&key),
            Some((ObjId(42), SmolStr::new("stickney")))
        );
        assert_eq!(locker_key_owner("tacsim:locker:0:ceres"), None);
        assert_eq!(locker_key_owner("tacsim:locker:9:"), None);
    }

    #[test]
    fn ex1b_a_locker_is_capacity_bound() {
        let plan = locker_plan().expect("the plan forms");
        let compartment = plan.compartments.first().expect("one compartment");
        assert_eq!(
            compartment
                .limits
                .get(&CanonicalId::new(MASS_KG).expect("id")),
            Some(&LOCKER_KG)
        );
        // 21 t of metals overfill it; 20 t fit.
        let metals = CanonicalId::new("metals").expect("id");
        let mut empty = |_: &CanonicalId| Ok(0);
        assert!(crate::port::market::plan_check(&plan, &metals, 20, &mut empty).is_ok());
        assert!(crate::port::market::plan_check(&plan, &metals, 21, &mut empty).is_err());
    }
}
