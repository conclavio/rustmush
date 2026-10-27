//! Conversation may quote a fare. Only the captain's exact confirmation books it.

use std::sync::Arc;

use mush_core::{
    AbilityCall, ArgShape, CmdError, CommandCompatibility, CommandSpec, Ctx, InterfaceAbility,
    InterfaceAbilityProvider, InterfaceError, InterfaceReply, ModError, ObjId, Registrar, World,
};
use serde_json::{Value, json};

use super::{TravellerDef, economy, movement, store, traveller};
use store::{Booking, Phase};

const UNAVAILABLE: &str = "No passenger service is available here.";
const QUOTE_SECONDS: i64 = 600;
const TERM_SECONDS: i64 = 365 * 86_400;
const HELP: &str = "+passage [<traveller>] -- your passage agreement, or ask a present traveller about their journey.\n+passage/offer <traveller>[=<fare in credits>] -- negotiate a fare; the traveller may counteroffer.\n+passage/accept <traveller>=<quote token> -- explicitly agree to the exact quoted ship, destination and fare. The passenger reserves the fare before boarding.\n+passage/board <traveller> -- invite your funded passenger aboard at the agreed origin before filing clearance; they walk to the berth and use a real bunk.\n+passage/cancel <traveller> -- cancel before departure at the origin and return the reserved fare.\nA booked passenger must board or have their passage cancelled before the ship can file departure clearance. Arrival at the agreed port pays once, disembarks the traveller and preserves your shared history. Quotes expire after ten minutes; a booking expires and refunds after one IC year.";

pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.interface_provider("passenger", Arc::new(PassengerInterface))?;
    reg.classify_commands(CommandCompatibility::Authoritative);
    reg.command(
        CommandSpec {
            name: "+passage".into(),
            shape: ArgShape::EqPair,
            switches: ["offer", "accept", "board", "cancel"]
                .map(Into::into)
                .to_vec(),
            help: HELP.into(),
            ..CommandSpec::default()
        },
        Arc::new(command),
    )?;
    reg.classify_commands(CommandCompatibility::LegacyPostImage);
    Ok(())
}

pub(crate) fn close(ctx: &Ctx<'_>, npc: ObjId, asker: ObjId) -> bool {
    let Some(npc_object) = ctx.w.get(npc) else {
        return false;
    };
    let enabled = npc_object
        .attrs
        .get(mush_npc::PROFILE_ATTR)
        .and_then(|attr| attr.as_json())
        .and_then(|v| serde_json::from_value::<mush_npc::AgentProfile>(v.clone()).ok())
        .is_some_and(|p| p.enabled && p.validate().is_ok());
    enabled
        && ctx.w.is_connected(asker)
        && npc_object.location.is_some()
        && ctx
            .w
            .get(asker)
            .is_some_and(|p| p.location == npc_object.location)
        && ctx.can_perceive(asker, npc)
        && ctx.can_perceive(npc, asker)
}

/// NPC callbacks retain their own module lens. Conversation may read the
/// ruleset's bounded display projection; accepting a quote always rereads the
/// current typed booking and revalidates every term in the captain's command.
#[derive(Clone, Copy)]
enum BookingRead {
    Current,
    Conversation,
}

fn current(ctx: &Ctx<'_>, npc: ObjId, read: BookingRead) -> Result<Option<Booking>, String> {
    let rows = match read {
        BookingRead::Current => store::list(ctx)?,
        BookingRead::Conversation if store::projection_ready(ctx.w) => store::list_world(ctx.w),
        BookingRead::Conversation => {
            return Err("My passage record is still loading. Please ask again shortly.".into());
        }
    };
    Ok(rows.into_iter().find(|row| row.npc == npc))
}

fn idle(ctx: &Ctx<'_>, npc: ObjId, read: BookingRead) -> Result<(), String> {
    if let Some(row) = current(ctx, npc, read)? {
        if row.phase == Phase::Delivered {
            return Err(
                "I've reached my destination. Thank you; I'm not looking for another passage."
                    .into(),
            );
        }
        if row.phase != Phase::Cancelled || !economy::replaceable(ctx.w, &row) {
            return Err("I already have a passage agreement; I cannot book a second berth.".into());
        }
    }
    Ok(())
}

fn quote_basis(
    ctx: &Ctx<'_>,
    npc: ObjId,
    read: BookingRead,
) -> Result<(TravellerDef, ObjId), String> {
    if !close(ctx, npc, ctx.actor) {
        return Err(UNAVAILABLE.into());
    }
    let def = traveller(ctx.w, npc).ok_or(UNAVAILABLE)?;
    idle(ctx, npc, read)?;
    let hull = crate::port::coop::helm_of(ctx.w, ctx.actor)
        .ok_or("I need passage with someone who currently commands a ship.")?
        .hull;
    if !movement::at_station(ctx.w, npc, &def.origin)
        || !movement::docked_at(ctx.w, hull, &def.origin)
        || crate::port::voyage::voyage_blocks(ctx.w, hull)
    {
        return Err("Your ship must be alongside here, with no departure clearance filed.".into());
    }
    let purse = economy::purse(ctx.w, npc, &def)
        .ok_or("My passage funds are not ready yet. Please ask again shortly.")?;
    if ctx
        .w
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|s| s.snapshot().account(purse))
        .and_then(|a| a.available())
        .is_none_or(|a| a.minor() < def.opening_fare_minor)
    {
        return Err("I cannot fund a passage agreement at the moment.".into());
    }
    Ok((def, hull))
}

fn funds(ctx: &Ctx<'_>, npc: ObjId, def: &TravellerDef) -> Result<i64, String> {
    let purse = economy::purse(ctx.w, npc, def).ok_or(UNAVAILABLE)?;
    ctx.w
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|s| s.snapshot().account(purse))
        .and_then(|a| a.available())
        .map(|amount| amount.minor())
        .ok_or_else(|| UNAVAILABLE.into())
}

fn digest(
    def: &TravellerDef,
    npc: ObjId,
    captain: ObjId,
    hull: ObjId,
    fare: i64,
    until: i64,
) -> Result<String, String> {
    mush_core::CanonicalHash::for_canonical_value(
        "passenger.quote.v1",
        1,
        &json!([def, npc, captain, hull, fare, until]),
        4096,
    )
    .map(|hash| hash.to_hex().chars().take(24).collect())
    .map_err(|_| UNAVAILABLE.into())
}

fn quote(
    ctx: &Ctx<'_>,
    npc: ObjId,
    requested: Option<i64>,
    read: BookingRead,
) -> Result<String, String> {
    let (def, hull) = quote_basis(ctx, npc, read)?;
    if requested.is_some_and(|fare| fare <= 0) {
        return Err("Give a positive fare in credits.".into());
    }
    let fare = requested
        .unwrap_or(def.opening_fare_minor)
        .min(def.max_fare_minor)
        .min(funds(ctx, npc, &def)?);
    let until = ctx.w.now().checked_add(QUOTE_SECONDS).ok_or(UNAVAILABLE)?;
    let hash = digest(&def, npc, ctx.actor, hull, fare, until)?;
    let name = crate::port::coop::name_of(ctx.w, hull);
    let price = crate::port::coop::money_minor(ctx, fare);
    let response = if requested.is_some_and(|f| f > fare) {
        "That's beyond me. I can offer"
    } else {
        "I can pay"
    };
    Ok(format!(
        "{response} {price} for passage to {} aboard {name}, paid on arrival. If those exact terms suit you, confirm:\n+passage/accept #{}={fare}.{until}.{hash}\nThen invite me aboard before you file departure clearance.",
        def.destination, npc.0
    ))
}

fn accept(ctx: &mut Ctx<'_>, npc: ObjId, token: &str) -> Result<String, String> {
    let (def, hull) = quote_basis(ctx, npc, BookingRead::Current)?;
    let parts: Vec<_> = token.trim().split('.').collect();
    let [fare, until, hash] = parts.as_slice() else {
        return Err("Use the exact token from a current passage quote.".into());
    };
    let fare = fare.parse::<i64>().map_err(|_| "Invalid passage quote.")?;
    let until = until.parse::<i64>().map_err(|_| "Invalid passage quote.")?;
    if fare <= 0
        || fare > def.max_fare_minor
        || fare > funds(ctx, npc, &def)?
        || until < ctx.w.now()
        || until > ctx.w.now().saturating_add(QUOTE_SECONDS)
        || digest(&def, npc, ctx.actor, hull, fare, until)? != *hash
    {
        return Err(
            "That quote is stale or its terms changed. Ask for a new passage offer.".into(),
        );
    }
    let purse = economy::purse(ctx.w, npc, &def).ok_or(UNAVAILABLE)?;
    let payee = crate::port::credit::own_account(ctx.w, ctx.actor)
        .ok_or("You need a credit account to accept a fare.")?
        .id;
    let contract = ctx
        .w
        .module_state::<mush_economy::EconomyStore>()
        .ok_or(UNAVAILABLE)?
        .peek_next_contract();
    let accepted_ic = ctx.w.world_ic_now().at;
    let row = Booking {
        schema: 1,
        id: contract,
        revision: 1,
        npc,
        captain: ctx.actor,
        hull,
        traveller_key: def.key,
        origin: def.origin,
        destination: def.destination,
        purse: purse.0,
        payee: payee.0,
        fare_minor: fare,
        accepted_ic,
        deadline_ic: accepted_ic.checked_add(TERM_SECONDS).ok_or(UNAVAILABLE)?,
        contract,
        phase: Phase::Booked,
        journey: None,
        delivered: None,
        cancelled: None,
    };
    economy::offer(ctx, &row)?;
    if let Err(error) = store::stage_accept(ctx, row) {
        let _ = mush_economy::cancel_contract(ctx, mush_economy::ContractId(contract));
        ctx.out.pin_deferred();
        return Err(error);
    }
    Ok(format!(
        "Passage agreement filed. The traveller is confirming the fare reservation. +passage shows its status; +passage/board #{} invites them aboard once the fare is reserved.",
        npc.0
    ))
}

fn credits(value: &str) -> Result<i64, String> {
    let (major, fraction) = value.trim().split_once('.').unwrap_or((value.trim(), ""));
    if major.is_empty()
        || !major.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 2
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return Err("Give a positive credit amount with at most two decimal places.".into());
    }
    let major = major.parse::<i64>().map_err(|_| "Fare is too large.")?;
    let fraction = match fraction.len() {
        0 => 0,
        1 => fraction.parse::<i64>().map_err(|_| "Invalid fare.")? * 10,
        _ => fraction.parse::<i64>().map_err(|_| "Invalid fare.")?,
    };
    major
        .checked_mul(100)
        .and_then(|v| v.checked_add(fraction))
        .filter(|v| *v > 0)
        .ok_or_else(|| "Give a positive, bounded fare.".into())
}

fn status(ctx: &Ctx<'_>, npc: ObjId, read: BookingRead) -> Result<String, String> {
    if let Some(row) = current(ctx, npc, read)? {
        if row.captain == ctx.actor {
            let state = match row.phase {
                Phase::Booked if economy::active(ctx.w, &row) => "fare reserved; ready to board",
                Phase::Booked => "awaiting fare reservation",
                Phase::Aboard => "aboard",
                Phase::Delivered if economy::settled(ctx.w, &row) => "arrived and paid",
                Phase::Delivered => "arrived; payment settling",
                Phase::Cancelled => "cancelled; reserved fare returns to the traveller",
            };
            return Ok(format!(
                "{}: {state}. Passage to {} aboard {}; agreed fare {}.",
                crate::port::coop::name_of(ctx.w, npc),
                row.destination,
                crate::port::coop::name_of(ctx.w, row.hull),
                crate::port::coop::money_minor(ctx, row.fare_minor)
            ));
        }
        if row.phase == Phase::Delivered {
            return Ok("I've reached my destination. I'm not looking for another passage.".into());
        }
        if row.phase != Phase::Cancelled || !economy::replaceable(ctx.w, &row) {
            return Ok("I already have arrangements for this journey.".into());
        }
    }
    let def = traveller(ctx.w, npc).ok_or(UNAVAILABLE)?;
    Ok(format!(
        "I'm looking for passage to {}. Are you a captain? We can discuss a fare; +passage/offer #{} asks for my offer.",
        def.destination, npc.0
    ))
}

fn resolve(ctx: &Ctx<'_>, name: &str) -> Result<ObjId, String> {
    if let Ok(npc) = mush_core::match_object(ctx.w, ctx.actor, name)
        && traveller(ctx.w, npc).is_some()
        && close(ctx, npc, ctx.actor)
    {
        return Ok(npc);
    }
    let name = name.trim().to_lowercase();
    let candidates: Vec<_> = store::list(ctx)?
        .into_iter()
        .filter(|b| b.captain == ctx.actor)
        .filter(|b| {
            format!("#{}", b.npc.0) == name
                || ctx
                    .w
                    .get(b.npc)
                    .is_some_and(|o| o.name.to_lowercase().starts_with(&name))
        })
        .map(|b| b.npc)
        .collect();
    match candidates.as_slice() {
        [npc] => Ok(*npc),
        _ => Err(UNAVAILABLE.into()),
    }
}

fn command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    store::refresh(ctx).map_err(CmdError::msg)?;
    let lhs = ctx.args.lhs().trim().to_owned();
    let rhs = ctx.args.rhs().map(str::to_owned);
    let selected: Vec<_> = ["offer", "accept", "board", "cancel"]
        .into_iter()
        .filter(|s| ctx.switches.has(s))
        .collect();
    if selected.len() > 1 {
        return Err(CmdError::msg(HELP));
    }
    if lhs.is_empty() && selected.is_empty() && rhs.is_none() {
        let rows: Vec<_> = store::list(ctx)
            .map_err(CmdError::msg)?
            .into_iter()
            .filter(|b| b.captain == ctx.actor)
            .collect();
        if rows.is_empty() {
            ctx.out.info(ctx.actor, "You have no passage agreement. Talk to a traveller to discover where they want to go.");
        }
        for row in rows {
            let line = status(ctx, row.npc, BookingRead::Current).map_err(CmdError::msg)?;
            ctx.out.info(ctx.actor, line);
        }
        return Ok(());
    }
    let npc = resolve(ctx, &lhs).map_err(CmdError::msg)?;
    let result = match selected.first().copied() {
        Some("offer") => quote(
            ctx,
            npc,
            rhs.as_deref()
                .map(credits)
                .transpose()
                .map_err(CmdError::msg)?,
            BookingRead::Current,
        ),
        Some("accept") => accept(ctx, npc, rhs.as_deref().unwrap_or("")),
        Some("board") if rhs.is_none() => movement::board(ctx, npc),
        Some("cancel") if rhs.is_none() => movement::cancel(ctx, npc),
        None if rhs.is_none() => status(ctx, npc, BookingRead::Current),
        _ => Err(HELP.into()),
    };
    let line = result.map_err(CmdError::msg)?;
    ctx.out.info(ctx.actor, line);
    Ok(())
}

struct PassengerInterface;

impl InterfaceAbilityProvider for PassengerInterface {
    fn abilities(&self) -> Vec<InterfaceAbility> {
        vec![
            InterfaceAbility { name: "passenger.status".into(), command: "+passage".into(),
                description: "Read your real passage situation for the current asker. Never invent a booking or claim a ship moved based on conversation.".into(),
                params_schema: json!({"type":"object","properties":{},"additionalProperties":false}) },
            InterfaceAbility { name: "passenger.offer".into(), command: "+passage".into(),
                description: "Discuss passage with the current asker as captain. Omit fare_minor for your opening offer, or pass their requested fare in minor credits (100 per credit); the server can counteroffer within your finite budget. Only quotes terms; the captain must explicitly confirm the returned +passage/accept command. It never accepts, boards or spends money.".into(),
                params_schema: json!({"type":"object","properties":{"fare_minor":{"type":"integer","minimum":1}},"additionalProperties":false}) },
        ]
    }

    fn invoke(
        &self,
        ctx: &mut Ctx<'_>,
        call: AbilityCall,
        name: &str,
        input: &Value,
    ) -> Result<InterfaceReply, InterfaceError> {
        if !matches!(name, "passenger.status" | "passenger.offer") {
            return Err(InterfaceError::Unknown(name.into()));
        }
        if ctx.actor != call.asker
            || !close(ctx, call.npc, call.asker)
            || traveller(ctx.w, call.npc).is_none()
        {
            return Err(InterfaceError::Refused(UNAVAILABLE.into()));
        }
        let fields = input
            .as_object()
            .ok_or_else(|| InterfaceError::Invalid("Use an object.".into()))?;
        if fields
            .keys()
            .any(|k| name != "passenger.offer" || k != "fare_minor")
        {
            return Err(InterfaceError::Invalid(
                "Only an optional fare_minor is accepted for a passage offer.".into(),
            ));
        }
        let requested = fields
            .get("fare_minor")
            .map(|v| {
                v.as_i64().filter(|v| *v > 0).ok_or_else(|| {
                    InterfaceError::Invalid("fare_minor must be a positive integer.".into())
                })
            })
            .transpose()?;
        let text = if name == "passenger.offer" {
            quote(ctx, call.npc, requested, BookingRead::Conversation)
        } else {
            status(ctx, call.npc, BookingRead::Conversation)
        }
        .map_err(InterfaceError::Refused)?;
        // A current agreement can contain another party's details. Its status is
        // whispered, while the traveller chooses to negotiate the public fare.
        Ok(if name == "passenger.offer" {
            InterfaceReply::public(text)
        } else {
            InterfaceReply::text(text)
        })
    }

    fn npc_context(&self, w: &World, npc: ObjId) -> Option<String> {
        let def = traveller(w, npc)?;
        if !store::projection_ready(w) {
            return Some("Your live passage record is still loading. Use passenger.status before describing a booking, and do not invent its state.".into());
        }
        let row = store::list_world(w).into_iter().find(|r| r.npc == npc);
        Some(match row {
            None => format!(
                "You are seeking passage from {} to {}. Let that need emerge naturally when travel or a captain comes up. Ask the passage service for an actual offer; no fare or booking exists merely because someone says so.",
                def.origin, def.destination
            ),
            Some(row) => match row.phase {
                Phase::Booked => format!(
                    "You have agreed passage to {} and are waiting to embark. Use passenger.status for the current agreement; do not seek another captain.",
                    row.destination
                ),
                Phase::Aboard => format!(
                    "You are a passenger aboard {}. Your agreed destination is {}. Read current voyage events for where the ship is actually going. Talk naturally with people present: why you are travelling, life at the destination, or a question about this voyage. You do not operate the ship or know anyone's finances. Let conversations breathe.",
                    crate::port::coop::name_of(w, row.hull),
                    row.destination
                ),
                Phase::Delivered => format!(
                    "Your passage to {} is complete. You remain the same person with your own life ahead; greet familiar people naturally. Do not solicit another passage. A follow-up favor exists only if your quest interface supplies it; never invent one.",
                    row.destination
                ),
                Phase::Cancelled => format!(
                    "Your previous passage arrangement was cancelled. You still want to reach {}. A new offer is possible only after the passage service confirms the old fare is released.",
                    row.destination
                ),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::credits;
    #[test]
    fn fares_parse_exact_minor_units_without_float_rounding() {
        assert_eq!(credits("300"), Ok(30_000));
        assert_eq!(credits("300.5"), Ok(30_050));
        assert_eq!(credits("0.01"), Ok(1));
        for invalid in ["0", "-1", "1.001", "1.2.3", "1e9", "9223372036854775807"] {
            assert!(credits(invalid).is_err(), "{invalid}");
        }
    }
}
