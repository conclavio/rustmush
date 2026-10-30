//! Squad and wingman `ORDER` / `RELEASE` parsing and execution.

use super::*;

// ---------------------------------------------------------------------------
// Commanding the machines (16b cut Y.1; doc 16 §3.7, §6; 16c N3, Q38)
// ---------------------------------------------------------------------------
//
// Doc 16 §6's deliberation had one issuer, the LLM seam, which the shipped
// server has off. `ORDER` is the second: a pilot tells their side's machines
// what to do with the same six objectives and the same `IntentKind::Order`
// the seam sends, so the arena cannot tell the two apart except by
// `Order::issuer` — the scope a target hint is written in. Who may order what is
// 16c Q38's default read as the cut Y plan's decision 2: the side's **lead**
// orders every squad of the side; **anyone else** orders the one unit of their
// pod pair when a machine flies it ([`Command`]). Every refusal is one line; the
// squad refusal is byte-identical for the other side's squad, a squad that does
// not exist and one the pilot may not order, and it is said **before** any
// contact is looked at, so a pilot learns nothing about the scope by naming a
// squad they cannot order.

/// **`ORDER`'s grammar**, said for anything [`parse_order`] cannot read (16b cut
/// Y.1; the two squad roles and the release since 16b cut Z.2). About 155
/// characters, inside the TUI notices ring's 200-character line budget.
pub const ORDER_USAGE: &str = "Usage: ORDER <squad|ALL> HOLD | ADVANCE <spot> | ENGAGE \
                               [<contact>] | WITHDRAW | SCREEN <bearing> | SUPPORT <callsign> \
                               [HOLD FIRE], or ORDER <squad|ALL> RELEASE";

/// **A `SUPPORT` callsign that names no tank of the pilot's own side** (16b cut
/// Z.2; 16c N6) — the other side's callsign, a provisional `X<n>`, a word that
/// is no callsign and a slot nobody flies, **byte for byte one sentence**, so a
/// pilot learns nothing about the other side's roster by naming it.
pub const NO_SUCH_FRIENDLY: &str = "No tank of your side flies that callsign.";

/// **A squad the pilot may not order** (16b cut Y.1) — the other side's, one that
/// does not exist, and the pilot's own side's beyond their authority, **byte for
/// byte one sentence**, so the count of the other side's squads is never learned.
pub const NO_SUCH_SQUAD: &str = "No such squad.";

/// **`ORDER ALL` from a pilot who commands nothing** (16b cut Y.1): a pilot who
/// does not lead the side and whose pod pair is a human, an empty pod, or a seat
/// with no slot at all.
pub const NO_SQUAD_TO_ORDER: &str = "You have no squad to order.";

/// **How long a pilot's order stands** (16b cut Y.1; the cut Y plan's decision 7),
/// milliseconds: the ten-minute ceiling, with no key of its own. A human order
/// stands ten minutes or until replaced; the arena's `order_expired` line is the
/// same as for a lance-leader's, and the world's lapse sentence (17b cut L.2) is
/// told to the pilot alone, in their own words: `Squad red-1: your hold order has
/// run out; back on their minds.` (`back on doctrine.` until 17b cut M5.6, the
/// M5.3 close's D9 ruling: one word for every mind, as the release says it).
pub const HUMAN_ORDER_TTL_MS: u32 = crate::deliberation::MAX_TTL_MS;

/// **The squad an `ORDER` names** (16b cut Y.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SquadWord {
    /// `ALL`: every squad the pilot may order.
    All,
    /// A rendered squad id, `red-1` or `blue-2`
    /// ([`crate::deliberation::SquadId::parse`]).
    Id(crate::deliberation::SquadId),
    /// A bare ordinal, `1`: the pilot's own side's squad of that number.
    Ordinal(u16),
}

/// **The objective an `ORDER` names**, in the pilot's words (16b cut Y.1) — the
/// spot and the contact still unresolved, because resolving them is the world's
/// business and comes after authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObjectiveWords {
    /// `HOLD`.
    Hold,
    /// `ADVANCE <spot>`: every word between `ADVANCE` and the `HOLD FIRE` tail,
    /// joined by one space, as typed (`The Ford`).
    Advance {
        /// The spot's name or letter, as typed.
        spot: String,
    },
    /// `ENGAGE [<contact>]`: at most one word, as typed.
    Engage {
        /// The contact's designator, as typed, or `None` for "what you find".
        contact: Option<String>,
    },
    /// `WITHDRAW`.
    Withdraw,
    /// `SCREEN <bearing>` (16b cut Z.2): the bearing to face, whole degrees
    /// clockwise from north, `0` to `359` — digits only, as `BEARING` prints
    /// one.
    Screen {
        /// The bearing, degrees.
        bearing: u32,
    },
    /// `SUPPORT <callsign>` (16b cut Z.2): exactly one word, as typed, resolved
    /// against the running match's callsigns after authority.
    Support {
        /// The charge's callsign, as typed.
        callsign: String,
    },
    /// `RELEASE` (16b cut Z.2; 16c Q44): hand the squad back to its hulls'
    /// own minds (17b cut M5.3's words; "back to doctrine" here until 17b cut
    /// M5.6). Nothing may follow it, `HOLD FIRE` included.
    Release,
}

impl ObjectiveWords {
    /// The objective's wire word — `crate::deliberation::Objective::as_str`'s,
    /// so the world's watch, the ledger and the sentence name it one way.
    /// `RELEASE` is no objective and answers the release intent's own word.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            ObjectiveWords::Hold => "hold",
            ObjectiveWords::Advance { .. } => "advance",
            ObjectiveWords::Engage { .. } => "engage",
            ObjectiveWords::Withdraw => "withdraw",
            ObjectiveWords::Screen { .. } => "screen",
            ObjectiveWords::Support { .. } => "support",
            ObjectiveWords::Release => "release",
        }
    }
}

/// **One `ORDER`, parsed** (16b cut Y.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderWords {
    /// Which squad.
    pub squad: SquadWord,
    /// What it is told.
    pub objective: ObjectiveWords,
    /// Whether the trailing `HOLD FIRE` was given.
    pub hold_fire: bool,
}

/// **Parse `ORDER`'s argument** (16b cut Y.1) — pure, so every line of the
/// grammar is pinned without a world:
///
/// ```text
/// ORDER <squad|ALL> HOLD                 [HOLD FIRE]
/// ORDER <squad|ALL> ADVANCE <spot>       [HOLD FIRE]
/// ORDER <squad|ALL> ENGAGE [<contact>]   [HOLD FIRE]
/// ORDER <squad|ALL> WITHDRAW             [HOLD FIRE]
/// ORDER <squad|ALL> SCREEN <bearing>     [HOLD FIRE]
/// ORDER <squad|ALL> SUPPORT <callsign>   [HOLD FIRE]
/// ORDER <squad|ALL> RELEASE
/// ```
///
/// Whitespace-split and case-folded: the keywords and the squad word are
/// compared lower-cased, and a spot, contact or callsign word is kept as typed
/// (all three are resolved case-insensitively later). The trailing `HOLD FIRE`
/// is the last two words **after** the objective word, so `ORDER red-1 HOLD
/// HOLD FIRE` holds and holds fire, and `ORDER red-1 HOLD FIRE` is usage rather
/// than a guess.
///
/// # Errors
///
/// [`CockpitError::Usage`] with [`ORDER_USAGE`] for an empty argument, a squad
/// word that is neither `ALL`, a squad id nor an ordinal, an unknown objective,
/// `ADVANCE` with no spot, `ENGAGE` with more than one word, `SCREEN` with no
/// word, a word that is not all digits or a bearing of 360 and above (16b cut
/// Z.2), `SUPPORT` with no word or two, `RELEASE` with anything after it
/// (`HOLD FIRE` included), and any other trailing word.
pub fn parse_order(rest: &str) -> Result<OrderWords, CockpitError> {
    let usage = || CockpitError::Usage(ORDER_USAGE.to_owned());
    let typed: Vec<&str> = rest.split_whitespace().collect();
    let folded: Vec<String> = typed.iter().map(|word| word.to_ascii_lowercase()).collect();
    let (Some(squad_word), Some(objective_word)) = (folded.first(), folded.get(1)) else {
        return Err(usage());
    };
    let squad = if squad_word == "all" {
        SquadWord::All
    } else if let Some(id) = crate::deliberation::SquadId::parse(squad_word) {
        SquadWord::Id(id)
    } else if let Ok(ordinal) = squad_word.parse::<u16>() {
        SquadWord::Ordinal(ordinal)
    } else {
        return Err(usage());
    };
    // The words after the objective, with the `HOLD FIRE` tail taken off.
    let mut args: &[&str] = typed.get(2..).unwrap_or(&[]);
    let mut hold_fire = false;
    if let [head @ .., hold, fire] = args
        && hold.eq_ignore_ascii_case("hold")
        && fire.eq_ignore_ascii_case("fire")
    {
        hold_fire = true;
        args = head;
    }
    let objective = match objective_word.as_str() {
        "hold" if args.is_empty() => ObjectiveWords::Hold,
        "withdraw" if args.is_empty() => ObjectiveWords::Withdraw,
        "advance" if !args.is_empty() => ObjectiveWords::Advance {
            spot: args.join(" "),
        },
        "engage" => match args {
            [] => ObjectiveWords::Engage { contact: None },
            [one] => ObjectiveWords::Engage {
                contact: Some((*one).to_owned()),
            },
            _ => return Err(usage()),
        },
        // **16b cut Z.2**: one word of digits, below a full turn.
        "screen" => match args {
            [one] if !one.is_empty() && one.bytes().all(|byte| byte.is_ascii_digit()) => {
                match one.parse::<u32>() {
                    Ok(bearing) if bearing < 360 => ObjectiveWords::Screen { bearing },
                    _ => return Err(usage()),
                }
            }
            _ => return Err(usage()),
        },
        "support" => match args {
            [one] => ObjectiveWords::Support {
                callsign: (*one).to_owned(),
            },
            _ => return Err(usage()),
        },
        // Nothing after it: a release holds no fire, because it is no order.
        "release" if args.is_empty() && !hold_fire => ObjectiveWords::Release,
        _ => return Err(usage()),
    };
    Ok(OrderWords {
        squad,
        objective,
        hold_fire,
    })
}

/// **A slot's pod pair** (16b cut Y.1; the cut Y plan's decision 2): pods are
/// claimed in pairs, `1↔2`, `3↔4`, `5↔6` (16b cut Z.3's six a side) — odd `n`
/// with `n + 1`, even `n` with `n − 1`.
/// Saturating at both ends: `0`, which no slot is, pairs with itself.
#[must_use]
pub const fn pair(slot: u8) -> u8 {
    if slot % 2 == 1 {
        slot.saturating_add(1)
    } else {
        slot.saturating_sub(1)
    }
}

/// **What one seat may order** (16b cut Y.1; 16c Q38 read as the cut Y plan's
/// decision 2), read once per `ORDER` in one borrow of the
/// [`crate::match_::MatchBook`].
///
/// **Why the wingman and not "the squad holding my pair".** A pilot's tank is in
/// no squad, and `crate::deliberation::squads_of` deals a side's *machines* in
/// unit-id order into squads of at most `[tacsim.arena].squad_size` (four; 16b
/// cut Z.1), so a side of up to four machines is always **one** squad —
/// so on a side of four machines or fewer "my pair's squad" would hand any pilot whose pair is a
/// machine every machine of the side, and let them silently undo the lead's
/// orders (latest-wins, no radio line). So a non-lead orders exactly one unit:
/// their pod pair's, addressed by the word of the squad that holds it (the
/// arena's own squad filter accepts it) and acknowledged by its callsign.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    /// The side's lead: every roster of the side, in ordinal order.
    Squads(Vec<crate::deliberation::SquadRoster>),
    /// Anyone else whose pod pair a machine flies: that one unit, the squad that
    /// holds it, and its callsign.
    Wingman {
        /// The squad holding the wingman.
        squad: crate::deliberation::SquadId,
        /// The wingman.
        unit: ObjId,
        /// Its callsign, as the arena resolved it.
        callsign: Designator,
    },
    /// Nothing: the pair is a human, empty, or the seat has no slot.
    None,
}

/// **The seat's command** (16b cut Y.1): the running match, the seat's own side
/// (for a bare ordinal) and the [`Command`].
///
/// The centre's slot for the driver gives the side, the slot index and the
/// account; the running record ([`ArenaRegistry::match_of_unit`] on the seat's
/// own tank) gives the squads and the callsigns. A lead
/// ([`crate::match_::lobby::Centre::leads_side`]) commands every roster of the
/// side; anyone else the unit whose callsign is `Slot { team, pair(index) }`
/// **and** that one roster holds — a unit in no roster is a human's tank or one
/// abandoned by `EXIT`, and is nobody's wingman.
pub(crate) fn command_of(
    w: &World,
    seat: &Seat,
) -> (Option<MatchId>, Option<crate::arena::state::PartyId>, Command) {
    let match_id = w
        .module_state::<ArenaRegistry>()
        .and_then(|registry| registry.match_of_unit(seat.unit));
    let Some(book) = w.module_state::<crate::match_::MatchBook>() else {
        return (match_id, None, Command::None);
    };
    let Some(slot) = book.centre().slot_of_pilot(seat.driver) else {
        return (match_id, None, Command::None);
    };
    let (team, index, account) = (slot.team, slot.index, slot.account);
    let leads = book.centre().leads_side(team, account);
    let Some(record) = match_id.and_then(|match_id| book.get(match_id)) else {
        return (match_id, Some(team), Command::None);
    };
    if leads {
        let mut rosters: Vec<crate::deliberation::SquadRoster> = record
            .squads
            .iter()
            .filter(|roster| roster.id.team == team)
            .cloned()
            .collect();
        rosters.sort_by_key(|roster| roster.id.index);
        return (match_id, Some(team), Command::Squads(rosters));
    }
    // Doc 32 cut CF.0: a callsign no longer spells the slot, so the paired
    // pod's tank is found by its slot and its callsign read off the record.
    let wanted = crate::match_::centre::tank_for_slot(w, team, pair(index));
    let wingman = wanted
        .and_then(|unit| record.callsigns.get_key_value(&unit))
        .and_then(|(unit, callsign)| {
            record
                .squads
                .iter()
                .find(|roster| roster.holds(*unit))
                .map(|roster| Command::Wingman {
                    squad: roster.id,
                    unit: *unit,
                    callsign: *callsign,
                })
        });
    (match_id, Some(team), wingman.unwrap_or(Command::None))
}

/// **`ORDER <squad|ALL> <objective> [HOLD FIRE]`** — command the side's machines
/// (16b cut Y.1; doc 16 §3.7, §6; 16c N3, Q38).
///
/// In this order, and the order is the observer rule:
///
/// 1. [`parse_order`] — [`ORDER_USAGE`];
/// 2. [`require_alive`] — a wreck takes no order at all, as every ordering verb;
/// 3. **authority** ([`command_of`]) — a squad word outside the pilot's set is
///    [`NO_SUCH_SQUAD`], byte-identical whether it is the other side's, names
///    nothing, or is the own side's beyond the pilot's authority; `ALL` with an
///    empty set is [`NO_SQUAD_TO_ORDER`];
/// 4. **the objective** — an `ENGAGE` word resolved on the pilot's own held rows
///    ([`resolve_designator`]) and an `ADVANCE` word on the ground's named spots
///    ([`resolve_spot`]); a miss on either is the one byte-identical
///    [`NO_SUCH_CONTACT`], with or without a frame. After authority, so a pilot
///    with no right to order a squad learns nothing about contacts by naming one.
///    A row the pilot's panel classes as their own side drops the hint (the
///    machines never fire on their own side), so the order is a bare `ENGAGE`;
/// 5. **the order** — `issuer: Some(own tank)`, `by: the seat's character`
///    (17b cut L.1, D-UM-52: `CharacterRef::Character { id: seat.driver }`, the
///    author the bridge's admission checks on this same `command_of` rule),
///    [`HUMAN_ORDER_TTL_MS`] as both time to live and horizon, validated against
///    the window;
/// 6. **the span** (16b cut Y.2; doc 16 §2 W8) — the squads the pilot holds a
///    live order on ([`ArenaRegistry::live_orders_of`]) that this order does not
///    name, plus those it names, may not pass `[tacsim.command].max_span`
///    ([`span_refusal`]: [`CockpitError::SpanFull`] when the others alone fill
///    the span, [`CockpitError::SpanAll`] when only `ALL`'s several squads pass
///    it). A replacement of a squad's own live order never counts;
/// 7. **the rate** (16b cut Y.2) — one stamp per `ORDER`
///    ([`ArenaRegistry::note_order_stamp`]), refused past
///    `[tacsim.command].orders_per_minute` in sixty world seconds
///    ([`CockpitError::RateFull`]). Taken after the span and before the
///    fan-out, so an order no unit could be offered still spent it. Both are
///    reads and writes on the world's own RAM: a refusal reaches no pulse;
/// 8. **the fan-out** — [`crate::arena_bridge::forward_order`] per squad (the
///    seat authorised once, on the pilot's own tank), `Busy` when no unit could
///    be offered, and the world's watch on each squad reached
///    ([`ArenaRegistry::note_human_order`]);
/// 9. **the acknowledgement** ([`order_ack`]) to the issuer alone — nothing to
///    the room and nothing to the side: 16c Q38's *no radio line*.
///
/// It writes no document; the arena journals one `ordered` line per unit the
/// order lands on.
pub(super) fn order(ctx: &mut Ctx<'_>, seat: &Seat, rest: &str) -> Result<(), CockpitError> {
    if rest
        .split_whitespace()
        .next()
        .is_some_and(|word| word.starts_with("DT:"))
    {
        return crate::detachment::commands::order(ctx, seat, rest).map_err(CockpitError::Usage);
    }
    let actor = ctx.actor;
    let words = parse_order(rest)?;
    require_alive(ctx.w, seat.driver)?;

    // (3) Authority, before anything is resolved.
    let (match_id, own_team, command) = command_of(ctx.w, seat);
    let wanted = match words.squad {
        SquadWord::All => None,
        SquadWord::Id(id) => Some(id),
        SquadWord::Ordinal(index) => Some(crate::deliberation::SquadId {
            team: own_team.ok_or(CockpitError::NoSuchSquad)?,
            index,
        }),
    };
    // `(squad, units, who)` per squad the order goes to.
    let targets: Vec<(crate::deliberation::SquadId, Vec<ObjId>, String)> = match (&command, wanted)
    {
        (Command::Squads(rosters), None) => rosters
            .iter()
            .map(|roster| (roster.id, roster.units.clone(), roster.id.to_string()))
            .collect(),
        (Command::Squads(rosters), Some(id)) => rosters
            .iter()
            .find(|roster| roster.id == id)
            .map(|roster| vec![(roster.id, roster.units.clone(), roster.id.to_string())])
            .ok_or(CockpitError::NoSuchSquad)?,
        (
            Command::Wingman {
                squad,
                unit,
                callsign,
            },
            wanted,
        ) => {
            if wanted.is_some_and(|id| id != *squad) {
                return Err(CockpitError::NoSuchSquad);
            }
            vec![(*squad, vec![*unit], callsign.to_string())]
        }
        (Command::None, None) => return Err(CockpitError::NoSquadToOrder),
        (Command::None, Some(_)) => return Err(CockpitError::NoSuchSquad),
    };
    if targets.is_empty() {
        return Err(CockpitError::NoSquadToOrder);
    }
    let Some(match_id) = match_id else {
        return Err(CockpitError::NoSuchSquad);
    };

    // (4) The objective, on the issuer's own scope and the surveyed ground.
    let mut hint: Option<String> = None;
    let mut spot_named: Option<(SmolStr, char)> = None;
    let objective = match &words.objective {
        ObjectiveWords::Hold => crate::deliberation::Objective::Hold,
        ObjectiveWords::Withdraw => crate::deliberation::Objective::Withdraw,
        ObjectiveWords::Engage { contact: None } => {
            crate::deliberation::Objective::Engage { target_hint: None }
        }
        ObjectiveWords::Engage {
            contact: Some(word),
        } => {
            // `named_row`'s rule for a name with no frame: the one refusal.
            let held = last_frame(ctx.w, seat.driver).ok_or(CockpitError::NoSuchContact)?;
            let rows = frame_contacts_held(&held.frame, &held.ranges);
            let id = resolve_designator(&rows, word).ok_or(CockpitError::NoSuchContact)?;
            // **A row the pilot's own panel classes as their own side is no
            // target** (`best_hostile` skips a classified own-side row, so a
            // machine can never fire on it): the hint is dropped and the order
            // is a bare `ENGAGE`, acknowledged and journalled as one, so
            // neither the answer nor the debrief promises a shot that will not
            // come. Not a refusal: the pilot knows their own side, and the
            // squad was still told to engage.
            let row = rows.iter().find(|row| row.id == id);
            if row.is_some_and(|row| row.friendly) {
                crate::deliberation::Objective::Engage { target_hint: None }
            } else {
                hint = row.map(|row| row.designator.to_string());
                crate::deliberation::Objective::Engage {
                    target_hint: Some(id),
                }
            }
        }
        ObjectiveWords::Advance { spot } => {
            // V.1's rule: a word that is neither a contact nor a spot keeps the
            // one refusal.
            let (spots, _) = spots_of(ctx.w, Some(match_id));
            let (index, found) = resolve_spot(&spots, spot).ok_or(CockpitError::NoSuchContact)?;
            let letter = spot_letter(index).ok_or(CockpitError::NoSuchContact)?;
            spot_named = Some((found.name.clone(), letter));
            crate::deliberation::Objective::Advance {
                to: (found.x, found.y),
            }
        }
        // **16b cut Z.2 (16c N6)**: the bearing as typed, in millidegrees.
        ObjectiveWords::Screen { bearing } => crate::deliberation::Objective::Screen {
            facing_mdeg: bearing.saturating_mul(1_000),
        },
        // **16b cut Z.2 (16c N6)**: the charge, resolved against the running
        // match's own callsigns to a tank whose callsign is a slot of the
        // pilot's OWN side — the pilot's own tank included. The other side's
        // callsign, a provisional, a word that is no callsign and a slot
        // nobody flies are one byte-identical refusal, said after authority,
        // so the other side's roster is never learned by naming it. The
        // acknowledgement names the charge by its callsign as the match
        // prints it, not as typed.
        ObjectiveWords::Support { callsign } => {
            let own = own_team.ok_or(CockpitError::NoSuchFriendly)?;
            let (of, designator) = ctx
                .w
                .module_state::<crate::match_::MatchBook>()
                .and_then(|book| book.get(match_id))
                .and_then(|record| {
                    record
                        .callsigns
                        .iter()
                        .find(|(_, designator)| {
                            designator.party() == Some(own)
                                && designator.to_string().eq_ignore_ascii_case(callsign)
                        })
                        .map(|(unit, designator)| (*unit, *designator))
                })
                .ok_or(CockpitError::NoSuchFriendly)?;
            hint = Some(designator.to_string());
            crate::deliberation::Objective::Support { of }
        }
        // **16b cut Z.2 (16c Q44)**: a release carries no order at all; it
        // takes its own road from here (`release`).
        ObjectiveWords::Release => {
            return release(
                ctx,
                seat,
                match_id,
                targets,
                matches!(command, Command::Squads(_)),
            );
        }
    };

    // (5) The order.
    let now_ms = ctx.w.now().saturating_mul(1_000);
    let spot_word = spot_named.as_ref().map(|(name, _)| name.clone());
    let order_for = |squad: crate::deliberation::SquadId| crate::deliberation::Order {
        squad,
        objective,
        target: None,
        horizon_ms: HUMAN_ORDER_TTL_MS,
        constraints: crate::deliberation::Constraints {
            hold_fire: words.hold_fire,
            max_range_band: None,
        },
        ttl_ms: HUMAN_ORDER_TTL_MS,
        issued_ms: now_ms,
        issuer: Some(seat.unit),
        by: crate::deliberation::CharacterRef::Character { id: seat.driver },
        spot: spot_word.clone(),
    };
    // A spot is on the window by construction, so a refusal here is the bounds
    // check doing its job on something no pilot can type.
    let window = ctx
        .w
        .module_state::<ArenaRegistry>()
        .and_then(|registry| registry.arena_of(match_id))
        .map(|ground| (ground.terrain.width(), ground.terrain.height()));
    // **16b cut Z.2**: the leash in metres off this ground's own cell, for the
    // acknowledgement — 150 m on the shipped ground.
    let leash_m = ctx
        .w
        .module_state::<ArenaRegistry>()
        .and_then(|registry| registry.arena_of(match_id))
        .map_or(0, |ground| {
            u32::from(crate::arena::doctrine::SUPPORT_LEASH_CELLS)
                .saturating_mul(u32::from(ground.terrain.cell_cm()))
                / 100
        });
    for (squad, _, _) in &targets {
        order_for(*squad)
            .validate(window)
            .map_err(|why| CockpitError::Usage(why.to_string()))?;
    }

    // (6) The span (16b cut Y.2; doc 16 §2 W8), on the world's own watch. The
    // squads this order names are out of `others`, so re-ordering a squad the
    // pilot already holds never counts against them.
    let command_cfg = crate::match_::lobby::command_config(ctx.w).unwrap_or_default();
    let named: std::collections::BTreeSet<crate::deliberation::SquadId> =
        targets.iter().map(|(squad, _, _)| *squad).collect();
    let live = ctx
        .w
        .module_state::<ArenaRegistry>()
        .map(|registry| registry.live_orders_of(match_id, seat.unit, now_ms))
        .unwrap_or_default();
    let others = live.difference(&named).count();
    if let Some(refusal) = span_refusal(others, named.len(), command_cfg.max_span) {
        return Err(refusal);
    }

    // (7) The rate: one stamp per `ORDER`, spent before the fan-out.
    if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
        registry
            .note_order_stamp(seat.unit, now_ms, command_cfg.orders_per_minute)
            .map_err(|n| CockpitError::RateFull { n })?;
    }

    // (8) The fan-out: the seat authorised once, on the issuer's own tank.
    let mut reached: Vec<(crate::deliberation::SquadId, String)> = Vec::new();
    for (squad, units, who) in targets {
        let offered = crate::arena_bridge::forward_order(
            ctx,
            seat.driver,
            seat.unit,
            &units,
            order_for(squad),
        )?;
        if offered > 0 {
            reached.push((squad, who));
        }
    }
    if reached.is_empty() {
        return Err(CockpitError::Order(IntentRefusal::Busy));
    }
    let expires_ms = now_ms.saturating_add(i64::from(HUMAN_ORDER_TTL_MS));
    if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
        for (squad, _) in &reached {
            registry.note_human_order(
                match_id,
                *squad,
                SmolStr::new_static(words.objective.as_str()),
                seat.unit,
                crate::deliberation::CharacterRef::Character { id: seat.driver },
                expires_ms,
            );
        }
    }

    // (9) The acknowledgement, to the issuer alone.
    let who: Vec<String> = reached.into_iter().map(|(_, who)| who).collect();
    let line = order_ack(
        &who,
        &words.objective,
        hint.as_deref(),
        spot_named
            .as_ref()
            .map(|(name, letter)| (name.as_str(), *letter)),
        words.hold_fire,
        leash_m,
    );
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    Ok(())
}

/// **`ORDER <squad|ALL> RELEASE`** (16b cut Z.2; 16c Q44) — hand the squads the
/// pilot may order back to their own minds ("to doctrine" here until 17b cut
/// M5.7, as the acknowledgement's words moved at 17b cut M5.3), from
/// [`order`]'s step 4, after the parse,
/// [`require_alive`] and authority (the same set, and the same
/// [`NO_SUCH_SQUAD`] and [`NO_SQUAD_TO_ORDER`], as every other arm).
///
/// Then, in this order:
///
/// 1. **the rate** — one stamp spent ([`ArenaRegistry::note_order_stamp`]): a
///    release is an `ORDER`, and a pilot who could release freely could spin
///    orders and releases past `[tacsim.command].orders_per_minute`;
/// 2. **no span check**: a release takes no squad, it gives one back;
/// 3. **the fan-out** — [`crate::arena_bridge::forward_release`] per squad (the
///    seat authorised once, on the pilot's own tank), `Busy` when no unit could
///    be offered;
/// 4. **the watch** — after every target was offered the release, remove this
///    pilot's watch. A side lead releasing a whole squad also removes its
///    commander watch; a wingman's release retains that squad-wide watch;
/// 5. **the acknowledgement** ([`order_ack`] with [`ObjectiveWords::Release`]),
///    to the issuer alone.
///
/// A release to a squad under no order is acknowledged, and the arena applies
/// and journals nothing: the machines are on their own minds already.
pub(super) fn release(
    ctx: &mut Ctx<'_>,
    seat: &Seat,
    match_id: MatchId,
    targets: Vec<(crate::deliberation::SquadId, Vec<ObjId>, String)>,
    whole_squads: bool,
) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let now_ms = ctx.w.now().saturating_mul(1_000);
    let command_cfg = crate::match_::lobby::command_config(ctx.w).unwrap_or_default();
    if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
        registry
            .note_order_stamp(seat.unit, now_ms, command_cfg.orders_per_minute)
            .map_err(|n| CockpitError::RateFull { n })?;
    }
    let mut reached: Vec<(crate::deliberation::SquadId, String, bool)> = Vec::new();
    for (squad, units, who) in targets {
        let offered =
            crate::arena_bridge::forward_release(ctx, seat.driver, seat.unit, &units, squad)?;
        if offered > 0 {
            reached.push((squad, who, offered == units.len()));
        }
    }
    if reached.is_empty() {
        return Err(CockpitError::Order(IntentRefusal::Busy));
    }
    if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
        for (squad, _, complete) in &reached {
            if *complete {
                registry.note_order_released(match_id, *squad, seat.unit);
                if whole_squads {
                    registry.note_commander_order_released(match_id, *squad);
                }
            }
        }
    }
    let who: Vec<String> = reached.into_iter().map(|(_, who, _)| who).collect();
    let line = order_ack(&who, &ObjectiveWords::Release, None, None, false, 0);
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    Ok(())
}

/// **The span's answer** (16b cut Y.2; doc 16 §2 W8): `others` squads the
/// issuer holds a live order on that this order does not name, `named` squads
/// it names, `max_span` the configured span. `None` when `others + named` fits.
///
/// The sentence is chosen by what would pass, never by the squad word, so each
/// says only what is true:
///
/// - `others` alone fill the span — no squad at all can be ordered until one of
///   them runs out, whether the pilot named one or `ALL` (a wingman's `ALL`
///   names one squad, and so may an ex-lead's): [`CockpitError::SpanFull`];
/// - `others` leave room but `ALL`'s several squads pass it — ordering them one
///   at a time does pass: [`CockpitError::SpanAll`]. One squad named can never
///   land here (`others + 1 > max_span` is `others >= max_span`).
#[must_use]
pub(super) fn span_refusal(others: usize, named: usize, max_span: u32) -> Option<CockpitError> {
    let span = usize::try_from(max_span).unwrap_or(usize::MAX);
    let would = others.saturating_add(named);
    if would <= span {
        None
    } else if others >= span {
        Some(CockpitError::SpanFull { held: others })
    } else {
        Some(CockpitError::SpanAll {
            would,
            span: max_span,
        })
    }
}

/// **`ORDER`'s acknowledgement** (16b cut Y.1) — what the order means, in the
/// doctrine's own words (doc 16 §11 E.1/E.2: an ordered `Withdraw` stands and
/// fights inside its bands unless it holds fire; an `Advance` or a `Hold`
/// suppresses the hostile edges; `HOLD FIRE` stops the trigger and nothing else).
///
/// `who` is the squad ids for a lead (`red-1`, `red-1 and red-2`, `red-1, red-2
/// and red-3`) and the wingman's callsign (`RB`) for anyone else; `hint` is the
/// designator the issuer's own row prints (`BA`); `spot` is `(name, letter)`.
/// Verbatim, with `red-1`:
///
/// ```text
/// Ordered red-1 to hold: it stops where it stands and fires nothing.
/// Ordered red-1 to advance on The Ford (A): it drives there and fights nothing on the way.
/// Ordered red-1 to engage BA: it closes and fires on BA where it can see it, else on what it finds.
/// Ordered red-1 to engage: it closes and fires on what it finds.
/// Ordered red-1 to engage BA, holding fire: it closes and locks but fires nothing.
/// Ordered red-1 to engage, holding fire: it closes and locks on what it finds but fires nothing.
/// Ordered red-1 to withdraw: it breaks for its entry zone and still fights inside its bands.
/// Ordered red-1 to withdraw, holding fire: it breaks for its entry zone and fires nothing.
/// ```
///
/// `HOLD FIRE` on a hold or an advance adds `, holding fire` before the colon and
/// changes nothing else (those arms fire nothing already; the constraint is still
/// set). Every acknowledgement ends ` It stands for 10 minutes or until your next
/// order to it.`
///
/// **16b cut Z.2** (16c N6, Q44): the two squad roles and the release. For a
/// `SUPPORT`, `hint` is the charge's callsign as the match prints it and
/// `leash_m` the leash in metres off the ground's cell (`SUPPORT_LEASH_CELLS ×
/// cell_cm / 100`, 150 on the shipped ground); the bearing prints as `BEARING`
/// prints one. Verbatim, with `red-1`:
///
/// ```text
/// Ordered red-1 to screen 090°: it holds where it stands, faces 090° and fires on what comes inside its arc without giving chase.
/// Ordered red-1 to screen 090°, holding fire: it holds where it stands, faces 090° and locks what comes inside its arc but fires nothing.
/// Ordered red-1 to support RA: it keeps within 150 m of RA and fights what it finds there.
/// Ordered red-1 to support RA, holding fire: it keeps within 150 m of RA and locks what it finds but fires nothing.
/// Released red-1 to its mind: it fights on its own until it is ordered again.
/// Released red-1 and red-2 to their minds: they fight on their own until they are ordered again.
/// ```
///
/// The four order lines take the ten-minute tail; the release takes none, and
/// its sentence is true on a server whose machines' own commander may order the
/// squad again at its next signal — which dropping the watch allows on purpose.
/// The plural is the release's alone: the ordered lines keep Y.1's singular.
///
/// **17b cut M5.3**: the release said `to doctrine: it fights on its own
/// reflexes …` until that cut. A released squad flies the mind sealed for
/// each of its hulls, which a lead's `SIM AI MIND` may make another than
/// Standard Doctrine (a fixture's mind before 17b cut M6), so the sentence
/// names none — one word for every mind.
///
/// **17b cut M5.6** (the owner's OQ-6: the text made true in M5, the behaviour
/// in the residues cut after it; STOP D): the two `SUPPORT` arms said the
/// squad `fires first on what RA is fighting` / `locks what RA is fighting`
/// until that cut. A supporter keeps station on its charge and fights what it
/// finds there — nothing on its own frame says what the charge is fighting
/// (D-UM-15 dropped that read), so the sentence no longer claims it.
#[must_use]
pub fn order_ack(
    who: &[String],
    objective: &ObjectiveWords,
    hint: Option<&str>,
    spot: Option<(&str, char)>,
    hold_fire: bool,
    leash_m: u32,
) -> String {
    if matches!(objective, ObjectiveWords::Release) {
        let many = who.len() > 1;
        let who = join_names(who);
        return if many {
            format!(
                "Released {who} to their minds: they fight on their own until they are ordered \
                 again."
            )
        } else {
            format!("Released {who} to its mind: it fights on its own until it is ordered again.")
        };
    }
    let who = join_names(who);
    let holding = if hold_fire { ", holding fire" } else { "" };
    let said = match (objective, hold_fire) {
        (ObjectiveWords::Hold, _) => {
            format!("to hold{holding}: it stops where it stands and fires nothing.")
        }
        (ObjectiveWords::Advance { .. }, _) => {
            let on = spot.map_or_else(String::new, |(name, letter)| {
                format!(" on {name} ({letter})")
            });
            format!("to advance{on}{holding}: it drives there and fights nothing on the way.")
        }
        (ObjectiveWords::Engage { .. }, false) => match hint {
            Some(hint) => format!(
                "to engage {hint}: it closes and fires on {hint} where it can see it, else on \
                 what it finds."
            ),
            None => "to engage: it closes and fires on what it finds.".to_owned(),
        },
        (ObjectiveWords::Engage { .. }, true) => match hint {
            Some(hint) => {
                format!("to engage {hint}, holding fire: it closes and locks but fires nothing.")
            }
            None => "to engage, holding fire: it closes and locks on what it finds but fires \
                     nothing."
                .to_owned(),
        },
        (ObjectiveWords::Withdraw, false) => {
            "to withdraw: it breaks for its entry zone and still fights inside its bands."
                .to_owned()
        }
        (ObjectiveWords::Withdraw, true) => {
            "to withdraw, holding fire: it breaks for its entry zone and fires nothing.".to_owned()
        }
        (ObjectiveWords::Screen { bearing }, false) => format!(
            "to screen {bearing:03}°: it holds where it stands, faces {bearing:03}° and fires on \
             what comes inside its arc without giving chase."
        ),
        (ObjectiveWords::Screen { bearing }, true) => format!(
            "to screen {bearing:03}°, holding fire: it holds where it stands, faces {bearing:03}° \
             and locks what comes inside its arc but fires nothing."
        ),
        (ObjectiveWords::Support { callsign }, false) => {
            let charge = hint.unwrap_or(callsign);
            format!(
                "to support {charge}: it keeps within {leash_m} m of {charge} and fights what it \
                 finds there."
            )
        }
        (ObjectiveWords::Support { callsign }, true) => {
            let charge = hint.unwrap_or(callsign);
            format!(
                "to support {charge}, holding fire: it keeps within {leash_m} m of {charge} and \
                 locks what it finds but fires nothing."
            )
        }
        // Answered above: a release is no order and takes no tail.
        (ObjectiveWords::Release, _) => String::new(),
    };
    let minutes = HUMAN_ORDER_TTL_MS / 60_000;
    format!("Ordered {who} {said} It stands for {minutes} minutes or until your next order to it.")
}

/// `a`, `a and b`, `a, b and c` — the names an acknowledgement lists.
pub(super) fn join_names(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [head @ .., last] => format!("{} and {last}", head.join(", ")),
    }
}
