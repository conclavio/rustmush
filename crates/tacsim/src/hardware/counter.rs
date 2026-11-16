//! **`+core`: the fitting shop's counter** (doc 47 CO.6).
//!
//! The governed hardware has every act a core takes from a die to a hull (doc 17f M7; doc 17h
//! M8b), and before this unit nobody in play made the shop's half of them: the importer's stock
//! agent is a service character nobody logs into, and only the test rigs performed its acts.
//! The counter is that service. A player asks once, by a registered command; the counter's
//! tick then does the work one typed action at a time, as the shop's agent for the shop's acts
//! and as the player for the player's own, each checked in full by its provider:
//!
//! - **`+core/buy <I|II|III> [go]`** has a governed core made: the agent issues a certified die
//!   from the month's allotment and offers its integration; the player's acceptance pays the
//!   catalogue's price; the output is reserved; the shop's scheduled hours run; the player
//!   completes it (its title is born to them) and it is delivered into their hands. Class III
//!   needs the Registry's end-user registration (`+enduser`). Asked from anywhere.
//! - **`+core/fit <core>=<hull> [go]`** fits a governed core the player holds, fresh or stored
//!   (doc 47 CO.4), to a hull titled to them: the agent offers the refit; the player's
//!   acceptance pays the fitting fee; the fitting's hours run; the player's install puts the
//!   core in, and a core it displaces comes back to their hands. Asked at the shop, with the
//!   hull there: a ship docked where the shop stands, or a rig taken into it
//!   (`mush_economy::core_refit::at_shop`).
//! - **`+core`** reads the shop, the cores in the player's hands and what the counter is doing
//!   for them.
//!
//! **What the player consents to.** `go` is the consent to the price the preview named; the
//! counter submits no acceptance the player did not ask for, and asks nothing else of them.
//! **What it remembers.** One request a player, in RAM (the procurement tick's rule: the
//! durable records are the memory, RAM only paces). Each fire reads where the request stands
//! from the industry's acknowledged projection (`mush_industry::core_production::counter`) and
//! the world, and takes the next step. A restart forgets the request: what it had done stands
//! (an accepted order is the player's, paid, and its hours run on), and asking again picks up
//! from there. A step submitted and not seen to land is tried again after
//! [`STEP_PATIENCE`] fires, at most [`STEP_TRIES`] times, and then the counter tells the player
//! it could not, and stops.
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use mush_core::module::{ModError, Registrar, TickCadence};
use mush_core::{
    CmdError, CommitToken, Ctx, DefinitionRef, IdempotencyKeyText, ObjId, PerceivedRef, World,
};
use mush_economy::AccountId;
use mush_economy::core_integration::{CoreClass, EndUserStanding, end_user_standing};
use mush_industry::core_production::DieClass;
use mush_industry::core_production::counter::{self, CounterView};
use mush_inventory::{LotId, record::EndpointRef};
use mush_org::PrincipalId;
use serde_json::{Value, json};

use crate::launch::LaunchMode;
use crate::port::coop::{money_minor, say};
use crate::port_tick::{Fire, Step};

/// The verb.
pub const CORE_VERB: &str = "+core";
/// The tick's tag.
pub const CORE_TICK: &str = "tacsim.core_counter";
/// How often the counter works (Coarse).
pub const CORE_TICK_PERIOD: Duration = Duration::from_secs(2);
/// Fires a submitted step is given to be seen before it is tried again.
pub const STEP_PATIENCE: u8 = 3;
/// Tries of one step before the counter gives the request up.
pub const STEP_TRIES: u8 = 4;
/// Industry's integration offer, reserve and delivery actions (its own names are crate-private).
const INTEGRATION_OFFER: &str = "offer_core_integration";
const INTEGRATION_RESERVE: &str = "reserve_core_output";
const INTEGRATION_DELIVER: &str = "deliver_core_output";

/// `+core`'s help.
pub const CORE_HELP: &str = "+core  --  The fitting shop's counter: governed cores made, and fitted to your hulls.\n\
+core  reads where the shop stands, the cores in your hands and what the counter is doing for you.\n\
+core/buy <I|II|III>  previews a core of that class: its price and the shop's hours. Add go to order it.\n\
  The shop draws a certified die, you pay, the hours run, and the core is delivered into your hands,\n\
  titled to you. Class III needs your end-user registration (+enduser). Ask from anywhere.\n\
+core/fit <core>=<hull>  previews fitting a core you hold to a hull titled to you. Add go to fit it.\n\
  Ask at the shop with the hull there: a ship docked where the shop stands. The fee is paid at the\n\
  counter, the fitting's hours run, and the core goes in; a core it displaces comes back to you.\n\
  A Co-op charter is the Co-op's hull: +coop/buyout makes her yours first.";
/// The grammar.
pub const CORE_USAGE: &str =
    "Usage: +core  |  +core/buy <I|II|III> [go]  |  +core/fit <core>=<hull> [go]";
/// No Active launch, or no hardware composed.
pub const CORE_NOT_OPEN: &str = "There is no fitting shop on this world.";
/// The shop, for the status read: `{shop}`, `{place}`.
pub const CORE_SHOP: &str = "The {shop} stands at {place}.";
/// The shop stands nowhere a player can reach: `{shop}`.
pub const CORE_SHOP_NOWHERE: &str =
    "The {shop} takes orders by wire; it fits nothing until it stands at a berth.";
/// A core in hand, for the status read: `{core}`, `{class}`.
pub const CORE_IN_HAND: &str = "In your hands: {core}, a governed Class {class} core.";
/// Nothing in hand, nothing in progress.
pub const CORE_NOTHING: &str =
    "You hold no governed core and the counter is doing nothing for you.";
/// The counter's work in progress: `{what}`, `{stage}`.
pub const CORE_WORKING: &str = "The counter is {what}: {stage}.";
/// Already one request.
pub const CORE_BUSY: &str = "The counter is already working for you; +core reads where it stands.";
/// No credit account.
pub const CORE_NO_CREDIT: &str = "You hold no credit account to pay the shop from.";
/// Short of the price: `{price}`, `{available}`.
pub const CORE_FUNDS: &str = "That costs {price}; you have {available}.";
/// Class III without the registration.
pub const CORE_END_USER: &str =
    "A Class III core is built only for a registered end user; +enduser/apply registers you.";
/// The buy preview: `{class}`, `{price}`, `{hours}`.
pub const CORE_BUY_PREVIEW: &str =
    "A governed Class {class} core: {price}, made in {hours} IC hours from a      certified die.
It is delivered into your hands, titled to you. Add go to order it.";
/// The buy ordered: `{class}`, `{price}`.
pub const CORE_BUY_QUEUED: &str =
    "The counter takes your order for a Class {class} core, {price}. +core reads where it stands.";
/// The fit preview: `{core}`, `{hull}`, `{fee}`, `{hours}`.
pub const CORE_FIT_PREVIEW: &str =
    "Fitting {core} to the {hull}: {fee}, {hours} IC hours. Add go to fit it.";
/// The fit ordered: `{core}`, `{hull}`, `{fee}`.
pub const CORE_FIT_QUEUED: &str =
    "The counter takes the {hull} in hand to fit {core}, {fee}. +core reads where it stands.";
/// No such core in hand: `{what}`.
pub const CORE_NO_SUCH_CORE: &str = "You hold no governed core called {what}.";
/// No such hull: `{what}`.
pub const CORE_NO_SUCH_HULL: &str = "There is no hull called {what} here.";
/// Not titled to the actor: `{hull}`.
pub const CORE_NOT_YOURS: &str =
    "The {hull} is not titled to you; the shop fits a hull you hold clear title to.";
/// The actor not at the shop: `{shop}`.
pub const CORE_NOT_AT_SHOP: &str = "Fittings are signed at the {shop}.";
/// The hull is not one the shop fits: `{hull}`.
pub const CORE_UNFIT: &str =
    "The shop fits governed cores to Drays and prospector rigs; the {hull} is neither.";
/// The core was delivered: `{core}`, `{class}`.
pub const CORE_DELIVERED: &str =
    "The shop delivers {core} into your hands: a governed Class {class} core, titled to you.";
/// The core was fitted: `{core}`, `{hull}`.
pub const CORE_FITTED: &str = "The fitters have put {core} in the {hull}.";
/// The acceptance landed: `{what}`, `{hours}`.
pub const CORE_ACCEPTED: &str =
    "The counter has your payment; the shop works {hours} IC hours on {what}.";
/// The shipped hardware catalogue did not load, so the counter quotes nothing.
pub const CORE_NO_PRICE: &str = "The fitting shop has no price list.";
/// The importer's depot short of a die's chain goods (doc 48 unit PL.7): `{class}`, `{bill}`.
pub const CORE_NO_STOCK: &str = "The shop has no stock to cut a Class {class} die: a die takes {bill} from the importer's depot.";
/// The counter gave up: `{what}`.
pub const CORE_GAVE_UP: &str =
    "The counter could not go on {what}. What you paid stands on its order; staff have been told.";

/// What one request is.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Kind {
    /// A core made: the class, the die the counter drew, the offer it was offered under.
    Buy {
        class: DieClass,
        die: Option<LotId>,
        offer: Option<CommitToken>,
    },
    /// A core fitted: the core, its class and manufacture, the hull, the offer.
    Fit {
        core: ObjId,
        class: DieClass,
        manufacture: CommitToken,
        hull: ObjId,
        offer: Option<CommitToken>,
    },
}

/// The step a request last submitted, how many fires ago and how many times.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Tried {
    step: &'static str,
    fires: u8,
    tries: u8,
}

/// One player's request.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Request {
    kind: Kind,
    payer: AccountId,
    tried: Option<Tried>,
    /// The acceptance has been told.
    told_accepted: bool,
    /// The output's reservation was submitted. The projection does not carry it, so the counter
    /// remembers: it reserves an order it accepted itself, at once, before the shop's hours run.
    /// An order picked up after a restart is not reserved again (if it never was, its completion
    /// is refused and the counter gives the request up, telling the player).
    reserve_sent: bool,
    /// The counter submitted this request's acceptance itself.
    accepted_here: bool,
}

/// **The counter's RAM slot**: one request a player, and the key ordinal.
#[derive(Clone, Debug, Default)]
pub struct CounterState {
    requests: BTreeMap<ObjId, Request>,
    keys: (i64, u64),
}

impl CounterState {
    fn key(&mut self, at: i64, verb: &str, subject: &str) -> Option<IdempotencyKeyText> {
        if self.keys.0 != at {
            self.keys = (at, 0);
        }
        let ordinal = self.keys.1;
        self.keys.1 = ordinal.checked_add(1)?;
        IdempotencyKeyText::parse_canonical(format!("core.{verb}.{subject}.{at:x}.{ordinal:x}"))
            .ok()
    }
}

/// **Register the counter**: its RAM slot and its Coarse tick, unconditionally (a world with no
/// launch, or no hardware, visits nothing).
///
/// # Errors
///
/// Any registration the registrar refuses.
pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.state(CounterState::default());
    reg.tick(
        CORE_TICK,
        TickCadence::Coarse(CORE_TICK_PERIOD),
        Arc::new(counter_tick),
    )?;
    Ok(())
}

/// The shop the counter serves: the Active launch's hardware.
struct Shop {
    factory: ObjId,
    agent: ObjId,
}

/// **The launch's shop**: `Some` only where this crate's launch is composed and Active, with a
/// stock agent that is not GOD (a tick never acts as GOD).
fn shop(w: &World) -> Option<Shop> {
    if w.module_state_owner(std::any::TypeId::of::<
        crate::launch::registration::LaunchService,
    >())
    .map(|owner| owner.as_str())
        != Some(crate::NS)
    {
        return None;
    }
    let deployment = w
        .module_state::<crate::launch::registration::LaunchService>()?
        .0
        .deployment();
    if deployment.mode != LaunchMode::Active {
        return None;
    }
    let hardware = &deployment.hardware;
    let agent = hardware
        .agents
        .iter()
        .map(|a| a.actor)
        .find(|a| *a != mush_core::GOD)?;
    Some(Shop {
        factory: hardware.factory,
        agent,
    })
}

/// The name of `thing`, or its number.
fn name_of(w: &World, thing: ObjId) -> String {
    w.get(thing)
        .map_or_else(|| format!("#{}", thing.0), |o| o.name.clone())
}

/// The die class of an economy class.
const fn die_class(class: CoreClass) -> DieClass {
    match class {
        CoreClass::I => DieClass::I,
        CoreClass::II => DieClass::II,
        CoreClass::III => DieClass::III,
    }
}

/// The economy class of a die class.
const fn core_class(class: DieClass) -> CoreClass {
    match class {
        DieClass::I => CoreClass::I,
        DieClass::II => CoreClass::II,
        DieClass::III => CoreClass::III,
    }
}

/// The roman numeral of a class.
const fn roman(class: DieClass) -> &'static str {
    match class {
        DieClass::I => "I",
        DieClass::II => "II",
        DieClass::III => "III",
    }
}

/// The shop's hours of an integration and of a fitting, by class (the shipped catalogue's).
const fn hours(class: DieClass, fitting: bool) -> u32 {
    match (class, fitting) {
        (DieClass::I, false) => 24,
        (DieClass::II, false) => 48,
        (DieClass::III, false) => 96,
        (DieClass::I, true) => 1,
        (DieClass::II, true) => 2,
        (DieClass::III, true) => 4,
    }
}

/// The governed cores in `actor`'s hands titled to them: each with its class and manufacture.
fn cores_in_hand(w: &World, actor: ObjId) -> Vec<(ObjId, CoreClass, CommitToken)> {
    let Some(economy) = w.module_state::<mush_economy::EconomyStore>() else {
        return Vec::new();
    };
    let snapshot = economy.snapshot();
    w.get(actor)
        .map(|o| o.contents.clone())
        .unwrap_or_default()
        .into_iter()
        .filter(|thing| {
            snapshot
                .asset(*thing)
                .is_some_and(|t| t.title_owner == PrincipalId::Object(actor))
        })
        .filter_map(|thing| {
            let birth = economy.core_title_birth(thing).ok().flatten()?;
            Some((thing, birth.class, birth.acceptance))
        })
        .collect()
}

/// `text` names `thing`: `#<n>`, or its name in any case.
fn names(w: &World, thing: ObjId, text: &str) -> bool {
    let text = text.trim();
    text.strip_prefix('#')
        .and_then(|n| n.parse::<u32>().ok())
        .map_or_else(
            || {
                w.get(thing)
                    .is_some_and(|o| o.name.eq_ignore_ascii_case(text))
            },
            |n| n == thing.0,
        )
}

/// The actor's available credit and account, or the refusal.
fn wallet(ctx: &Ctx<'_>) -> Result<(AccountId, i64), String> {
    let account = crate::port::credit::own_account(ctx.w, ctx.actor)
        .ok_or_else(|| CORE_NO_CREDIT.to_owned())?;
    Ok((account.id, account.posted_balance.minor()))
}

/// Refuse when `price` is more than `available`.
fn afford(ctx: &Ctx<'_>, price: i64, available: i64) -> Result<(), String> {
    if available < price {
        return Err(CORE_FUNDS
            .replace("{price}", &money_minor(ctx, price))
            .replace("{available}", &money_minor(ctx, available)));
    }
    Ok(())
}

/// **`+core`, `+core/buy`, `+core/fit`.**
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line said to the actor.
pub(crate) fn core_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !ctx
        .w
        .module_state::<crate::store::TacsimStore>()
        .is_some_and(crate::store::TacsimStore::enabled)
    {
        return Err(CmdError::msg(crate::commands::NOT_ENABLED));
    }
    // Doc 47 CO.13's sale shares the verb and answers for itself.
    if ctx.switches.has("sell") {
        return crate::hardware::sale::sale_cmd(ctx);
    }
    let Some(shop) = shop(ctx.w) else {
        return Err(CmdError::msg(CORE_NOT_OPEN));
    };
    let raw = ctx.args.raw().trim().to_owned();
    let (body, go) = match raw.rsplit_once(char::is_whitespace) {
        Some((body, word)) if word.eq_ignore_ascii_case("go") => (body.trim().to_owned(), true),
        _ if raw.eq_ignore_ascii_case("go") => (String::new(), true),
        _ => (raw.clone(), false),
    };
    if ctx.switches.has("buy") {
        return buy(ctx, &body, go).map_err(CmdError::msg);
    }
    if ctx.switches.has("fit") {
        return fit(ctx, &shop, &body, go).map_err(CmdError::msg);
    }
    if !raw.is_empty() {
        return Err(CmdError::msg(CORE_USAGE));
    }
    status(ctx, &shop);
    Ok(())
}

/// The status read.
fn status(ctx: &mut Ctx<'_>, shop: &Shop) {
    let shop_name = name_of(ctx.w, shop.factory);
    let line = match ctx.w.get(shop.factory).and_then(|o| o.location) {
        Some(place) => CORE_SHOP
            .replace("{shop}", &shop_name)
            .replace("{place}", &name_of(ctx.w, place)),
        None => CORE_SHOP_NOWHERE.replace("{shop}", &shop_name),
    };
    say(ctx, line);
    let held = cores_in_hand(ctx.w, ctx.actor);
    let working = ctx
        .w
        .module_state::<CounterState>()
        .and_then(|s| s.requests.get(&ctx.actor).cloned());
    if held.is_empty() && working.is_none() {
        say(ctx, CORE_NOTHING);
        return;
    }
    for (core, class, _) in held {
        let line = CORE_IN_HAND
            .replace("{core}", &name_of(ctx.w, core))
            .replace("{class}", roman(die_class(class)));
        say(ctx, line);
    }
    if let Some(request) = working {
        let view = counter::view(ctx.w, ctx.actor).unwrap_or_default();
        let (what, stage) = describe(ctx.w, &request, &view);
        say(
            ctx,
            CORE_WORKING
                .replace("{what}", &what)
                .replace("{stage}", stage),
        );
    }
}

/// **The importer's depot holds a die's chain goods** (doc 48 unit PL.7; Q-48-27, Q-48-48): a
/// die of `class` takes them from the depot when it is issued, so the counter refuses an order
/// it could not cut a die for. An integration of the class already offered to the actor needs no
/// new die (a restart's request picks it up), so it is not asked.
fn stocked(ctx: &Ctx<'_>, class: DieClass) -> Result<(), String> {
    let offered = counter::view(ctx.w, ctx.actor)
        .unwrap_or_default()
        .integrations
        .iter()
        .any(|line| {
            line.class == class
                && line
                    .order
                    .as_ref()
                    .is_none_or(|order| !order.cancelled && !order.delivered)
        });
    if offered {
        return Ok(());
    }
    let Some(depot) = ctx
        .w
        .module_state::<crate::launch::registration::LaunchService>()
        .map(|service| service.0.deployment().hardware.depot)
    else {
        return Err(CORE_NOT_OPEN.to_owned());
    };
    let chain = crate::hardware::PublishedHardware::shipped_once()
        .ok_or(CORE_NO_PRICE)?
        .catalogue()
        .chain()
        .map_err(|_| CORE_NO_PRICE.to_owned())?;
    let lines = &chain[class.index()];
    let short = lines.iter().any(|line| {
        crate::port_tick::read_pool(ctx, EndpointRef(depot), &line.commodity).unwrap_or(0)
            < line.units
    });
    if !short {
        return Ok(());
    }
    let bill = lines
        .iter()
        .map(|line| format!("{} {}", line.units, line.commodity.as_str()))
        .collect::<Vec<_>>()
        .join(" and ");
    Err(CORE_NO_STOCK
        .replace("{class}", roman(class))
        .replace("{bill}", &bill))
}

/// `+core/buy <class> [go]`.
fn buy(ctx: &mut Ctx<'_>, body: &str, go: bool) -> Result<(), String> {
    let class = match body.to_ascii_uppercase().as_str() {
        "I" | "1" => DieClass::I,
        "II" | "2" => DieClass::II,
        "III" | "3" => DieClass::III,
        _ => return Err(CORE_USAGE.to_owned()),
    };
    if class == DieClass::III
        && !matches!(
            end_user_standing(ctx.w, ctx.actor),
            EndUserStanding::Registered { .. }
        )
    {
        return Err(CORE_END_USER.to_owned());
    }
    let (payer, available) = wallet(ctx)?;
    // Doc 48 unit PL.7: the catalogue's price, which the launch's payment policy charges.
    let price = crate::hardware::PublishedHardware::shipped_once()
        .ok_or(CORE_NO_PRICE)?
        .catalogue()
        .finished_minor(core_class(class));
    afford(ctx, price, available)?;
    stocked(ctx, class)?;
    let price_text = money_minor(ctx, price);
    if !go {
        say(
            ctx,
            CORE_BUY_PREVIEW
                .replace("{class}", roman(class))
                .replace("{price}", &price_text)
                .replace("{hours}", &hours(class, false).to_string()),
        );
        return Ok(());
    }
    queue(
        ctx,
        Request {
            kind: Kind::Buy {
                class,
                die: None,
                offer: None,
            },
            payer,
            tried: None,
            told_accepted: false,
            reserve_sent: false,
            accepted_here: false,
        },
    )?;
    say(
        ctx,
        CORE_BUY_QUEUED
            .replace("{class}", roman(class))
            .replace("{price}", &price_text),
    );
    Ok(())
}

/// Whether the shop fits `hull`'s unit: a Dray or a prospector rig, cored or not.
fn fits_chassis(w: &World, hull: ObjId) -> bool {
    mush_tactics::module::unit_of(w, hull).is_some_and(|unit| {
        let id = unit.schema().key().id.as_str();
        let chassis = crate::definitions::core_replacement::surface_chassis(id);
        chassis == crate::definitions::content::DRAY
            || chassis == crate::definitions::content::TV_PROSPECTOR
    })
}

/// `+core/fit <core>=<hull> [go]`.
fn fit(ctx: &mut Ctx<'_>, shop: &Shop, body: &str, go: bool) -> Result<(), String> {
    let (core_text, hull_text) = body
        .split_once('=')
        .map(|(c, h)| (c.trim(), h.trim()))
        .filter(|(c, h)| !c.is_empty() && !h.is_empty())
        .ok_or_else(|| CORE_USAGE.to_owned())?;
    let shop_name = name_of(ctx.w, shop.factory);
    if !mush_economy::core_refit::at_shop(ctx.w, ctx.actor, shop.factory) {
        return Err(CORE_NOT_AT_SHOP.replace("{shop}", &shop_name));
    }
    let (core, class, manufacture) = cores_in_hand(ctx.w, ctx.actor)
        .into_iter()
        .find(|(core, ..)| names(ctx.w, *core, core_text))
        .ok_or_else(|| CORE_NO_SUCH_CORE.replace("{what}", core_text))?;
    // The hull is looked for beside the requester and inside the shop, both of them at the
    // shop once the requester is, so a hull found is at the shop.
    let here = ctx.w.get(ctx.actor).and_then(|o| o.location);
    let candidates: Vec<ObjId> = [here, Some(shop.factory)]
        .into_iter()
        .flatten()
        .flat_map(|place| {
            ctx.w
                .get(place)
                .map(|o| o.contents.clone())
                .unwrap_or_default()
        })
        .filter(|thing| mush_tactics::module::unit_of(ctx.w, *thing).is_some())
        .collect();
    let hull = candidates
        .into_iter()
        .find(|thing| names(ctx.w, *thing, hull_text))
        .ok_or_else(|| CORE_NO_SUCH_HULL.replace("{what}", hull_text))?;
    let hull_name = name_of(ctx.w, hull);
    if crate::port::ship_mind::holds(ctx, hull) {
        return Err(crate::port::ship_mind::MIND_HOLDS_HER.replace("{name}", &hull_name));
    }
    let clear = ctx
        .w
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| e.snapshot().asset(hull).cloned())
        .is_some_and(|t| {
            t.title_owner == PrincipalId::Object(ctx.actor)
                && t.scope.is_none()
                && t.encumbrances.is_empty()
        });
    if !clear {
        return Err(CORE_NOT_YOURS.replace("{hull}", &hull_name));
    }
    if !fits_chassis(ctx.w, hull) {
        return Err(CORE_UNFIT.replace("{hull}", &hull_name));
    }
    let (payer, available) = wallet(ctx)?;
    let fee = mush_economy::core_refit::FITTING_MINOR;
    afford(ctx, fee, available)?;
    let class = die_class(class);
    let (core_name, fee_text) = (name_of(ctx.w, core), money_minor(ctx, fee));
    if !go {
        say(
            ctx,
            CORE_FIT_PREVIEW
                .replace("{core}", &core_name)
                .replace("{hull}", &hull_name)
                .replace("{fee}", &fee_text)
                .replace("{hours}", &hours(class, true).to_string()),
        );
        return Ok(());
    }
    queue(
        ctx,
        Request {
            kind: Kind::Fit {
                core,
                class,
                manufacture,
                hull,
                offer: None,
            },
            payer,
            tried: None,
            told_accepted: false,
            reserve_sent: false,
            accepted_here: false,
        },
    )?;
    say(
        ctx,
        CORE_FIT_QUEUED
            .replace("{core}", &core_name)
            .replace("{hull}", &hull_name)
            .replace("{fee}", &fee_text),
    );
    Ok(())
}

/// Keep `request` for the actor, refusing a second.
fn queue(ctx: &mut Ctx<'_>, request: Request) -> Result<(), String> {
    let actor = ctx.actor;
    let state = ctx
        .state_mut::<CounterState>()
        .ok_or_else(|| CORE_NOT_OPEN.to_owned())?;
    if state.requests.contains_key(&actor) {
        return Err(CORE_BUSY.to_owned());
    }
    state.requests.insert(actor, request);
    Ok(())
}

/// **What the counter is doing for `who`**, as `+core` says it (`"<what>: <stage>"`); `None`
/// when it is doing nothing for them.
#[must_use]
pub fn working(w: &World, who: ObjId) -> Option<String> {
    let request = w
        .module_state::<CounterState>()?
        .requests
        .get(&who)?
        .clone();
    let view = counter::view(w, who).unwrap_or_default();
    let (what, stage) = describe(w, &request, &view);
    Some(format!("{what}: {stage}"))
}

/// What a request is, and the stage it stands at, for the status read.
fn describe(w: &World, request: &Request, view: &CounterView) -> (String, &'static str) {
    match &request.kind {
        Kind::Buy { class, offer, .. } => {
            let what = format!("making you a Class {} core", roman(*class));
            let line = offer.and_then(|o| view.integrations.iter().find(|l| l.offer == o));
            let stage = match line.and_then(|l| l.order) {
                None if line.is_some() => "the shop has offered it; your payment is going in",
                None => "the shop is drawing a die",
                Some(_) if request.accepted_here && !request.reserve_sent => {
                    "paid; the shop is reserving its output"
                }
                Some(order) if !order.worked => "paid; the shop is at work",
                Some(order) if order.completed.is_none() => {
                    "the work is done; it is being finished"
                }
                Some(_) => "finished; it is being delivered",
            };
            (what, stage)
        }
        Kind::Fit {
            core, hull, offer, ..
        } => {
            let what = format!("fitting {} to the {}", name_of(w, *core), name_of(w, *hull));
            let line = offer.and_then(|o| view.refits.iter().find(|l| l.offer == o));
            let stage = match line.and_then(|l| l.order) {
                None if line.is_some() => "the shop has offered it; your fee is going in",
                None => "the shop is drawing up the fitting",
                Some((_, false)) => "paid; the fitters are at work",
                Some((_, true)) => "the work is done; the core is going in",
            };
            (what, stage)
        }
    }
}

/// What the next step of a request is.
#[derive(Clone, Debug, PartialEq)]
enum Next {
    /// Submit `action` as `who` with `parameters`; `step` names it.
    Submit {
        step: &'static str,
        who: ObjId,
        action: &'static str,
        parameters: Value,
    },
    /// Nothing to do this fire (the shop's hours run).
    Wait,
    /// Done: the line to tell the player.
    Done(String),
    /// Given up: why, for staff.
    GiveUp(&'static str),
}

/// **The next step of `requester`'s `request`**, reading where it stands. It may note what it
/// learned (the die, the offer) on the request.
#[allow(clippy::too_many_lines)] // one ordered list of steps a kind
fn next(w: &World, shop: &Shop, requester: ObjId, request: &mut Request) -> Next {
    let Some(view) = counter::view(w, requester) else {
        return Next::Wait;
    };
    let now = w.now();
    let payer = request.payer.0.to_string();
    let (reserve_sent, accepted_here) = (request.reserve_sent, request.accepted_here);
    match &mut request.kind {
        Kind::Buy { class, die, offer } => {
            if offer.is_none()
                && let Some(lot) = *die
                && let Some(line) = view.integrations.iter().find(|l| l.lot == Some(lot))
            {
                *offer = Some(line.offer);
            }
            // A request asked again (after a restart, which forgets the counter's RAM) picks up
            // the requester's own order of the class still under way, or an offer still open.
            if offer.is_none() && die.is_none() {
                *offer = view
                    .integrations
                    .iter()
                    .rev()
                    .find(|l| {
                        l.class == *class
                            && match l.order {
                                Some(o) => !o.cancelled && !o.delivered,
                                None => l.expires_utc > now,
                            }
                    })
                    .map(|l| l.offer);
            }
            let line = offer.and_then(|o| view.integrations.iter().find(|l| l.offer == o));
            let Some(line) = line else {
                if die.is_none() {
                    *die = counter::unoffered_dies(w, shop.agent)
                        .into_iter()
                        .find(|(_, c, _)| c == class)
                        .map(|(_, _, lot)| lot);
                }
                return match *die {
                    Some(lot) => Next::Submit {
                        step: "offer",
                        who: shop.agent,
                        action: INTEGRATION_OFFER,
                        parameters: json!({"requester": requester.0, "class": *class,
                            "supplier_lot": lot.to_string()}),
                    },
                    None => Next::Submit {
                        step: "die",
                        who: shop.agent,
                        action: mush_industry::core_production::ISSUE_ACTION,
                        parameters: json!({ "class": *class }),
                    },
                };
            };
            let Some(order) = line.order else {
                if line.expires_utc <= now {
                    return Next::GiveUp("the integration's offer lapsed unaccepted");
                }
                return Next::Submit {
                    step: "accept",
                    who: requester,
                    action: mush_economy::core_integration::ACCEPT_ACTION,
                    parameters: json!({"offer": line.offer.to_hex(), "payer": payer}),
                };
            };
            let id = json!({ "order": order.id.to_string() });
            if order.cancelled {
                return Next::GiveUp("the integration was cancelled");
            }
            if accepted_here && !reserve_sent && !order.worked && order.completed.is_none() {
                return Next::Submit {
                    step: "reserve",
                    who: requester,
                    action: INTEGRATION_RESERVE,
                    parameters: id,
                };
            }
            if !order.worked {
                return Next::Wait;
            }
            let Some(core) = order.completed else {
                return Next::Submit {
                    step: "complete",
                    who: requester,
                    action: mush_economy::core_integration::COMPLETE_ACTION,
                    parameters: id,
                };
            };
            if !order.delivered || w.get(core).and_then(|o| o.location) != Some(requester) {
                if order.delivered {
                    return Next::GiveUp("the delivered core is not in its requester's hands");
                }
                return Next::Submit {
                    step: "deliver",
                    who: requester,
                    action: INTEGRATION_DELIVER,
                    parameters: id,
                };
            }
            Next::Done(
                CORE_DELIVERED
                    .replace("{core}", &name_of(w, core))
                    .replace("{class}", roman(*class)),
            )
        }
        Kind::Fit {
            core,
            class,
            manufacture,
            hull,
            offer,
        } => {
            let installed = w.get(*core).and_then(|o| o.location) == Some(*hull);
            if installed {
                return Next::Done(
                    CORE_FITTED
                        .replace("{core}", &name_of(w, *core))
                        .replace("{hull}", &name_of(w, *hull)),
                );
            }
            if offer.is_none() {
                *offer = view
                    .refits
                    .iter()
                    .rev()
                    .find(|l| {
                        l.hull == *hull
                            && l.incoming == *core
                            && l.order.is_none()
                            && l.expires_utc > now
                    })
                    .map(|l| l.offer);
            }
            let line = offer.and_then(|o| view.refits.iter().find(|l| l.offer == o));
            let Some(line) = line else {
                return Next::Submit {
                    step: "offer",
                    who: shop.agent,
                    action: mush_economy::core_refit::OFFER_ACTION,
                    parameters: json!({"requester": requester.0, "hull": hull.0,
                        "incoming": core.0, "manufacture": manufacture.to_hex(), "class": *class}),
                };
            };
            match line.order {
                None if line.expires_utc <= now => {
                    Next::GiveUp("the refit's offer lapsed unaccepted")
                }
                None => Next::Submit {
                    step: "accept",
                    who: requester,
                    action: mush_economy::core_refit::ACCEPT_ACTION,
                    parameters: json!({"offer": line.offer.to_hex(), "payer": payer}),
                },
                Some((_, false)) => Next::Wait,
                Some((order, true)) => Next::Submit {
                    step: "install",
                    who: requester,
                    action: mush_economy::core_refit::INSTALL_ACTION,
                    parameters: json!({ "order": order.to_string() }),
                },
            }
        }
    }
}

/// Whether the request's acceptance has landed, and the hours and words to tell it with.
fn accepted(w: &World, requester: ObjId, request: &Request) -> Option<(u32, String)> {
    let view = counter::view(w, requester)?;
    match &request.kind {
        Kind::Buy { class, offer, .. } => {
            let o = (*offer)?;
            view.integrations
                .iter()
                .find(|l| l.offer == o && l.order.is_some())
                .map(|_| {
                    (
                        hours(*class, false),
                        format!("your Class {} core", roman(*class)),
                    )
                })
        }
        Kind::Fit {
            core,
            class,
            hull,
            offer,
            ..
        } => {
            let o = (*offer)?;
            view.refits
                .iter()
                .find(|l| l.offer == o && l.order.is_some())
                .map(|_| {
                    (
                        hours(*class, true),
                        format!("fitting {} to the {}", name_of(w, *core), name_of(w, *hull)),
                    )
                })
        }
    }
}

/// The registered action of industry's named `id`.
fn action(ctx: &Ctx<'_>, id: &str) -> Option<DefinitionRef> {
    let mut found = ctx
        .reg
        .definitions()?
        .iter()
        .map(|d| d.reference())
        .filter(|r| {
            r.key().owner.as_str() == mush_industry::NS
                && r.key().kind.as_str() == "action"
                && r.key().id.as_str() == id
        });
    let one = found.next()?;
    found.next().is_none().then(|| one.clone())
}

/// **The counter's tick**: for each request, one step. Infallible: a refusal pages (a log line),
/// and a request whose step will not land is given up and its player told.
fn counter_tick(ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<mush_core::SceneId>) {
    let Some(mut state) = ctx.w.module_state::<CounterState>().cloned() else {
        return;
    };
    if state.requests.is_empty() {
        return;
    }
    let Some(shop) = shop(ctx.w) else {
        return;
    };
    let mut fire = Fire::new(BTreeMap::new());
    let requesters: Vec<ObjId> = state.requests.keys().copied().collect();
    for requester in requesters {
        let Some(mut request) = state.requests.get(&requester).cloned() else {
            continue;
        };
        let outcome = next(ctx.w, &shop, requester, &mut request);
        if !request.told_accepted
            && let Some((hours, what)) = accepted(ctx.w, requester, &request)
        {
            request.told_accepted = true;
            ctx.out.info(
                requester,
                CORE_ACCEPTED
                    .replace("{what}", &what)
                    .replace("{hours}", &hours.to_string()),
            );
        }
        match outcome {
            Next::Wait => {
                request.tried = None;
                state.requests.insert(requester, request);
            }
            Next::Done(line) => {
                ctx.out.info(requester, line);
                state.requests.remove(&requester);
            }
            Next::GiveUp(why) => {
                give_up(ctx, &mut state, requester, &request, why);
            }
            Next::Submit {
                step,
                who,
                action: name,
                parameters,
            } => {
                let tried = match request.tried {
                    Some(t) if t.step == step && t.fires < STEP_PATIENCE => {
                        request.tried = Some(Tried {
                            fires: t.fires + 1,
                            ..t
                        });
                        state.requests.insert(requester, request);
                        continue;
                    }
                    Some(t) if t.step == step => t.tries,
                    _ => 0,
                };
                if tried >= STEP_TRIES {
                    give_up(ctx, &mut state, requester, &request, step);
                    continue;
                }
                let Some(definition) = action(ctx, name) else {
                    give_up(
                        ctx,
                        &mut state,
                        requester,
                        &request,
                        "the action is not registered",
                    );
                    continue;
                };
                let at = ctx.w.world_ic_now().at;
                let Some(key) = state.key(at, step, &requester.0.to_string()) else {
                    continue;
                };
                let submitted =
                    crate::port::stock::swapped(fire.as_principal(ctx, who, who, |ctx| {
                        ctx.submit_authoritative(
                            definition,
                            PerceivedRef::SelfActor,
                            Vec::new(),
                            parameters,
                            key,
                        );
                    }));
                fire.settle(who, submitted);
                match submitted {
                    Step::Submitted => {
                        request.reserve_sent |= step == "reserve";
                        request.accepted_here |= step == "accept";
                        request.tried = Some(Tried {
                            step,
                            fires: 0,
                            tries: tried + 1,
                        });
                    }
                    Step::Refused => {
                        give_up(ctx, &mut state, requester, &request, "its actor is gone");
                        continue;
                    }
                    _ => {}
                }
                state.requests.insert(requester, request);
            }
        }
    }
    if let Some(slot) = ctx.state_mut::<CounterState>() {
        *slot = state;
    }
}

/// Give `requester`'s request up, telling them and staff's log.
fn give_up(
    ctx: &mut Ctx<'_>,
    state: &mut CounterState,
    requester: ObjId,
    request: &Request,
    why: &str,
) {
    tracing::warn!(
        requester = requester.0,
        ?request,
        why,
        "tacsim: the fitting shop's counter gave a request up"
    );
    let view = counter::view(ctx.w, requester).unwrap_or_default();
    let (what, _) = describe(ctx.w, request, &view);
    ctx.out
        .info(requester, CORE_GAVE_UP.replace("{what}", &what));
    state.requests.remove(&requester);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every line the counter says holds under the 120-column floor.
    #[test]
    fn co6_the_counter_lines_hold_the_floor() {
        for line in CORE_HELP.lines() {
            assert!(
                line.chars().count() <= 120,
                "{} columns: {line}",
                line.chars().count()
            );
        }
        for text in [
            CORE_USAGE,
            CORE_BUY_PREVIEW,
            CORE_BUY_QUEUED,
            CORE_FIT_PREVIEW,
            CORE_FIT_QUEUED,
            CORE_GAVE_UP,
            CORE_DELIVERED,
            CORE_FITTED,
            CORE_ACCEPTED,
            CORE_UNFIT,
            CORE_NOT_YOURS,
            CORE_END_USER,
        ] {
            for line in text.lines() {
                assert!(
                    line.chars().count() <= 96,
                    "{} columns: {line}",
                    line.chars().count()
                );
            }
        }
    }

    /// The hours the counter tells are the shipped catalogue's: the integration's and the
    /// fitting's work units, an IC hour each.
    #[test]
    fn co6_the_hours_are_the_catalogue_s() {
        let shipped = crate::hardware::PublishedHardware::shipped().expect("the shipped hardware");
        for class in [DieClass::I, DieClass::II, DieClass::III] {
            assert_eq!(
                u64::from(hours(class, false)),
                shipped.work_units(crate::hardware::HardwareProcess::Integrate, class)
            );
            assert_eq!(
                u64::from(hours(class, true)),
                shipped.work_units(crate::hardware::HardwareProcess::Refit, class)
            );
        }
    }
}
