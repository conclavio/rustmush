//! Weapon arming, FIRE/CONTACTS/WEAPONSTATUS/STATUS/VIEW commands, and unit weapon status helpers.

use super::*;

/// **What a `FIRE` line asks for** (17b cut M1b.4), read by [`fire_form`] off
/// its first two words alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FireForm<'a> {
    /// `FIRE [<weapon>]`: the shot, every byte as it was before this cut.
    Shot,
    /// `FIRE <weapon|ALL> OFF`, and bare `FIRE OFF`: `None` clears every slot.
    Off {
        /// The weapon word as typed, or `None` for `ALL` and bare `OFF`.
        weapon: Option<&'a str>,
        /// The words after `OFF`, trimmed (refused unless empty).
        tail: &'a str,
    },
    /// `FIRE <weapon> WHEN <conditions>`.
    When {
        /// The weapon word as typed.
        weapon: &'a str,
        /// Everything after `WHEN`, trimmed.
        conditions: &'a str,
    },
}

/// One whitespace-delimited word off the front of `text`, and the rest with
/// its leading whitespace gone.
pub(super) fn first_word(text: &str) -> (&str, &str) {
    let text = text.trim_start();
    match text.find(char::is_whitespace) {
        Some(at) => (
            text.get(..at).unwrap_or_default(),
            text.get(at..).unwrap_or_default().trim_start(),
        ),
        None => (text, ""),
    }
}

/// **Which of `FIRE`'s three forms a line is** (17b cut M1b.4): the second
/// word, case-folded, `when` or `off` — or the whole line `off`. Every other
/// line — `FIRE`, `FIRE mg`, `FIRE all`, `FIRE when odds` — is
/// [`FireForm::Shot`] and goes to `choose_weapon` whole, exactly as before.
pub(super) fn fire_form(rest: &str) -> FireForm<'_> {
    let (first, after) = first_word(rest);
    let (second, tail) = first_word(after);
    if first.eq_ignore_ascii_case("off") && after.is_empty() {
        return FireForm::Off {
            weapon: None,
            tail: "",
        };
    }
    if first.is_empty() || second.is_empty() {
        return FireForm::Shot;
    }
    if second.eq_ignore_ascii_case("off") {
        let weapon = (!first.eq_ignore_ascii_case("all")).then_some(first);
        return FireForm::Off {
            weapon,
            tail: tail.trim(),
        };
    }
    if second.eq_ignore_ascii_case("when") {
        return FireForm::When {
            weapon: first,
            conditions: tail.trim(),
        };
    }
    FireForm::Shot
}

/// **`FIRE ALL OFF`'s reply** (17b cut M1b.4), and bare `FIRE OFF`'s: said
/// whether or not a slot stood.
pub const CLEARED_ALL: &str = "Cleared: your guns fire only when you say.";

/// **`FIRE <weapon|ALL> OFF`** (17b cut M1b.4; doc 17 §5's precedence: *a
/// cleared weapon fires only when you say*): one discrete
/// `IntentKind::Condition(Arming::Disarm { weapon })` on the fire channel.
///
/// The wreck first, as every order; the weapon through `choose_weapon` with
/// its own refusals; anything typed after `OFF` is the one outside refusal.
/// **The reply is unconditional and reads no mirror**: a reply computed off a
/// mirror up to one `Quiet` pulse old lied when an arming and an `OFF` fell
/// inside one pulse, and a disarm of nothing is a no-op in the arena — so
/// `Cleared: cannon fires only when you say.` / [`CLEARED_ALL`] whether or not
/// a slot stood. Power is not asked: clearing a slot is not a shot.
pub(super) fn fire_off(
    ctx: &mut Ctx<'_>,
    seat: &Seat,
    weapon: Option<&str>,
    tail: &str,
) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    require_alive(ctx.w, seat.driver)?;
    let (named, line) = match weapon {
        Some(word) => {
            let held = last_frame(ctx.w, seat.driver);
            let rows = carried(ctx.w, seat.unit, held.as_ref(), std::time::Instant::now());
            let row = choose_weapon(&rows, word)?;
            if !tail.is_empty() {
                return Err(fire_refused(&row.id, &CockpitError::NotThisCore));
            }
            let line = format!("Cleared: {} fires only when you say.", row.id);
            (Some(row.id), line)
        }
        None => {
            if !tail.is_empty() {
                return Err(CockpitError::Refused(order_refused(
                    IntentKind::fire("").as_str(),
                    None,
                    &CockpitError::NotThisCore.to_string(),
                )));
            }
            (None, CLEARED_ALL.to_owned())
        }
    };
    forward_intent(
        ctx,
        seat.driver,
        seat.unit,
        IntentKind::Condition(Arming::Disarm { weapon: named }),
    )?;
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    Ok(())
}

/// Route FIRE WHEN through the shared bounded native-rule parser. The held
/// frame supplies weapon names, classifications and the captured floor; fresh
/// authority, slot capacity and classification are rechecked by the arena.
/// A queued English reply names the rule but cannot assign its slot until landing.
pub(super) fn fire_when(
    ctx: &mut Ctx<'_>,
    seat: &Seat,
    word: &str,
    conditions: &str,
) -> Result<(), CockpitError> {
    station_rules(ctx, seat, &format!("FIRE {word} WHEN {conditions}"))
}

/// The most conditions one `FIRE … WHEN` rule joins with `AND` (doc 17 §5).
pub const MAX_ARMING_TERMS: usize = crate::arena::rules::MAX_RULE_TERMS;

/// The furthest a range term may name, metres.
pub const MAX_ARMING_RANGE_M: u32 = crate::arena::rules::MAX_RULE_RANGE_M;

/// Set one term of a rule, refusing a second value that differs from the
/// first (the same value twice is the same rule).
pub(super) fn arming_set<T: PartialEq>(slot: &mut Option<T>, value: T) -> Result<(), CockpitError> {
    match slot {
        Some(held) if *held != value => Err(CockpitError::NotThisCore),
        _ => {
            *slot = Some(value);
            Ok(())
        }
    }
}

/// The terms of one rule as typed, before they are compiled.
#[derive(Default)]
pub(super) struct ArmingTerms {
    /// `odds >= n`, whole per cent.
    pub(super) odds_pct: Option<u32>,
    /// `aspect = …`, as a mask.
    pub(super) aspects: Option<u8>,
    /// `rounds > n` / `rounds > n%` / `reserve`, in rounds.
    pub(super) keeps: Option<u32>,
    /// The nearest a `range >` or a `band` bounds it to, compiled: a `range >
    /// n` stores `n + 1` (17b cut M2.2, ruling (c)).
    pub(super) min_m: Option<u32>,
    /// The furthest a `range <` or a `band` bounds it to, compiled: a `range <
    /// n` stores `n - 1` (17b cut M2.2, ruling (c)).
    pub(super) max_m: Option<u32>,
    /// `range < n`'s `n` as typed (the reply's figure).
    pub(super) under_m: Option<u32>,
    /// `range > n`'s `n` as typed (the reply's figure).
    pub(super) past_m: Option<u32>,
    /// `band = …`'s metres, `(nearest, furthest)`.
    pub(super) band: Option<(u32, u32)>,
}

/// **What a `FIRE … WHEN` rule said, as typed** (17b cut M2.2, the lead's
/// ruling (c)): the figures [`arming_reply`] writes back, read off the
/// pilot's own words rather than off the compiled [`FireCondition`] — whose
/// range terms are strict (`range < 600` stores `max_range_m 599`), so a
/// reply read off it would print a figure the pilot never typed. Every M1b.4
/// reply is byte-identical through it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArmingEcho {
    /// The chance asked for, per mille: the odds term's, else the frame's
    /// floor (always `Some` from [`parse_arming`]).
    pub min_permille: Option<u16>,
    /// The `aspect = …` mask, when typed.
    pub aspects: Option<u8>,
    /// `range < n`'s `n`, metres.
    pub under_m: Option<u32>,
    /// `range > n`'s `n`, metres.
    pub past_m: Option<u32>,
    /// `band = …`'s `(nearest, furthest)` metres.
    pub band: Option<(u32, u32)>,
    /// The rounds a `rounds > …` or `reserve` term keeps.
    pub keeps: Option<u32>,
}

/// **Compile a `FIRE <weapon> WHEN …` rule** (17b cut M1b.4; D-UM-18's
/// slot-fitting subset — the rest of the rule language is the rule-language
/// cut's, after 17b cut M5.7's gate, the owner's OQ-4) into
/// the [`FireCondition`] the arena arms, off the weapon's row and the held
/// frame. Pure: it reads nothing else and forwards nothing.
///
/// `rest` is the text after `WHEN`, split on `AND` (case-folded) into at most
/// [`MAX_ARMING_TERMS`] conditions, no `OR`; each condition is one of the
/// eight terms, case-folded and whitespace-tolerant around `>=`, `<`, `>`,
/// `=` and `,`:
///
/// * `odds >= n`, or `odds <weapon> >= n` naming THIS weapon (by id,
///   initials or prefix, as `FIRE` resolves one), `n` a whole number 1 to 100
///   → `min_permille = 10 n`. **The floor**: `10 n` under the frame's
///   captured `odds_floor_permille` is [`CockpitError::Unlawful`]
///   (refuse-not-clamp: `The Compact floor is 10%; a condition may not ask for
///   less.`). There is no upper refusal: 100 is lawful and simply never holds
///   under the table's `p_max`;
/// * `aspect = a[,b[,c]]` with `front`, `side` (left and right) and `rear` →
///   the mask (a word twice is fine);
/// * `range < n [m]` → `max_range_m = n - 1`; `range > n [m]` →
///   `min_range_m = n + 1` (17b cut M2.2, the lead's ruling (c): STRICTLY under
///   and over, where M1b.4 stored `n` and the warrant's inclusive bounds let
///   a printed `600 m` fire under `range < 600`); `n` 1 to
///   [`MAX_ARMING_RANGE_M`] (`range < 1` stores 0), tested by the arena
///   against the lock's ROUNDED range — the figure `RANGE` prints. **A
///   declared effect**: a term holds one figure, so `range < 200 AND band =
///   close` (199 against the band's 200) is refused as a differing duplicate,
///   where M1b.4 read both as 200;
/// * `band = close|effective|far` → the metres [`band_bounds_m`] cuts for this
///   weapon's bands on this frame's sight: compiled to FIXED metres here, which
///   the reply prints (a weapon with no bands has no band, and is outside);
/// * `rounds > n` (0 to the weapon's full load) → `min_rounds_left = n`;
///   `rounds > n%` (0 to 100) → `rounds_max × n / 100`, rounded down;
/// * `reserve` → `min_rounds_left = 0` on a piloted hull (the cockpit reads no
///   card — 17b cut M5.4's `MIND` reads the seal, never a card's sliders): it
///   compiles to `rounds > 0`;
/// * `target = identified` → accepted, and adds nothing: every slot fires only
///   a lock its platform has classified;
/// * `lock` → the lock, the default and the only target word here.
///
/// Anything else — another word, `OR`, a fifth condition, a malformed or
/// missing number, an empty rule, a term repeated with another value — is the
/// ONE outside said-half [`CockpitError::NotThisCore`]. The grammar is read
/// whole before the floor is checked, so a rule both malformed and under the
/// floor is told it is malformed.
///
/// The compiled slot: `target: Lock`; `min_permille` the odds term's, else
/// **the frame's floor** (a rule with no odds term asks for the floor);
/// `aspects` [`ASPECTS_ANY`] unless named; `min_rounds_left` 0, `min_range_m`
/// 0 and `max_range_m` `u32::MAX` (unbound) unless named.
///
/// **17b cut M2.2**: with the slot, the [`ArmingEcho`] of the rule as typed —
/// what [`arming_reply`] writes back.
///
/// # Errors
///
/// [`CockpitError::NotThisCore`] or [`CockpitError::Unlawful`] — the
/// said-half; the caller adds `FIRE <weapon> refused.`.
pub fn parse_arming(
    weapon: &WeaponRow,
    rest: &str,
    frame: &Frame,
) -> Result<(FireCondition, ArmingEcho), CockpitError> {
    use crate::arena::conditions::{Comparison, Rule, RuleBand, RulePredicate};
    use crate::arena::rules::{RuleCommand, RuleContext, RuleParseError, RuleWeapon};

    let weapons: Vec<RuleWeapon> = frame_weapons(frame)
        .iter()
        .map(|row| RuleWeapon {
            id: row.id.clone(),
            rounds_max: u32::try_from(row.rounds_max.max(0)).unwrap_or(u32::MAX),
        })
        .collect();
    let context = RuleContext {
        weapons: &weapons,
        floor_permille: u16::try_from(frame.odds_floor_permille).unwrap_or(u16::MAX),
        contacts: &[],
    };
    let parsed =
        crate::arena::rules::parse_command(&format!("FIRE {} WHEN {rest}", weapon.id), &context)
            .map_err(|error| match error {
                RuleParseError::Unlawful { floor_pct } => CockpitError::Unlawful {
                    floor_pct: u32::from(floor_pct),
                },
                _ => CockpitError::NotThisCore,
            })?;
    let RuleCommand::Arm(Arming::Rules { mut rules, .. }) = parsed else {
        return Err(CockpitError::NotThisCore);
    };
    let Some(Rule::Fire {
        mut condition,
        predicates,
        ..
    }) = rules.pop()
    else {
        return Err(CockpitError::NotThisCore);
    };
    if condition.target != SlotTarget::Lock {
        return Err(CockpitError::NotThisCore);
    }
    // Compatibility projection for callers that still require FireCondition.
    // The station itself stages the typed rule. Only this old adapter freezes
    // a band at the held frame and compiles strict metres to inclusive bounds.
    let mut terms = ArmingTerms::default();
    for predicate in predicates {
        match predicate {
            RulePredicate::Odds {
                weapon: named,
                min_permille,
            } => {
                if named.as_ref().is_some_and(|named| *named != weapon.id) {
                    return Err(CockpitError::NotThisCore);
                }
                arming_set(&mut terms.odds_pct, u32::from(min_permille) / 10)?;
            }
            RulePredicate::Aspect { mask } => arming_set(&mut terms.aspects, mask)?,
            RulePredicate::Range {
                comparison: Comparison::Less,
                metres,
            } => {
                arming_set(&mut terms.under_m, metres)?;
                arming_set(&mut terms.max_m, metres.saturating_sub(1))?;
            }
            RulePredicate::Range {
                comparison: Comparison::Greater,
                metres,
            } => {
                arming_set(&mut terms.past_m, metres)?;
                arming_set(&mut terms.min_m, metres.saturating_add(1))?;
            }
            RulePredicate::Band { band } => {
                let bands = frame
                    .weapons
                    .iter()
                    .find(|row| row.id == weapon.id)
                    .map_or(&[][..], |row| row.bands_cm.as_slice());
                let Some(bounds) = band_bounds_m(frame.sight_cm, bands) else {
                    return Err(CockpitError::NotThisCore);
                };
                let (nearest, furthest) = bounds[match band {
                    RuleBand::Close => 0,
                    RuleBand::Effective => 1,
                    RuleBand::Far => 2,
                }];
                arming_set(&mut terms.band, (nearest, furthest))?;
                if nearest > 0 {
                    arming_set(&mut terms.min_m, nearest)?;
                }
                arming_set(&mut terms.max_m, furthest)?;
            }
            RulePredicate::Rounds {
                weapon: named,
                comparison: Comparison::Greater,
                amount,
                percent,
            } => {
                if named.as_ref().is_some_and(|named| *named != weapon.id) {
                    return Err(CockpitError::NotThisCore);
                }
                arming_set(
                    &mut terms.keeps,
                    if percent {
                        rounds_kept(weapon, amount)
                    } else {
                        amount
                    },
                )?;
            }
            RulePredicate::Target {
                state: crate::arena::conditions::TargetState::Identified,
            }
            | RulePredicate::Lock { present: true } => {}
            _ => return Err(CockpitError::NotThisCore),
        }
    }
    condition.min_permille = terms.odds_pct.map_or(context.floor_permille, |pct| {
        u16::try_from(pct * 10).unwrap_or(u16::MAX)
    });
    condition.aspects = terms.aspects.unwrap_or(ASPECTS_ANY);
    condition.min_rounds_left = terms.keeps.unwrap_or(0);
    condition.min_range_m = terms.min_m.unwrap_or(0);
    condition.max_range_m = terms.max_m.unwrap_or(u32::MAX);
    let echo = ArmingEcho {
        min_permille: Some(condition.min_permille),
        aspects: terms.aspects,
        under_m: terms.under_m,
        past_m: terms.past_m,
        band: terms.band,
        keeps: terms.keeps,
    };
    Ok((condition, echo))
}

/// `pct` per cent of the weapon's full load, rounded down.
pub(super) fn rounds_kept(weapon: &WeaponRow, pct: u32) -> u32 {
    let full = u64::try_from(weapon.rounds_max.max(0)).unwrap_or(0);
    u32::try_from(full.saturating_mul(u64::from(pct)) / 100).unwrap_or(u32::MAX)
}

/// **The side words a mask names** (17b cut M1b.3's rule, lifted by 17b cut
/// M1b.4 so the arming line and the arming's reply say one thing): `front`,
/// `side` (left or right), `rear`, in that order, each at most once.
pub(super) fn aspect_side_words(aspects: u8) -> Vec<&'static str> {
    let side = aspect_in(aspects, Aspect::Left) || aspect_in(aspects, Aspect::Right);
    [
        (aspect_in(aspects, Aspect::Front), "front"),
        (side, "side"),
        (aspect_in(aspects, Aspect::Rear), "rear"),
    ]
    .into_iter()
    .filter_map(|(on, word)| on.then_some(word))
    .collect()
}

/// **The English `FIRE … WHEN` writes back** (17b cut M1b.4; doc 17 §5), with
/// **no slot number** — the number is the arena's to give:
/// `Arming {weapon}: fires your lock when {clauses}.`, the clauses joined
/// ` and ` in this order: `your chance to hit is {n}% or better` (always);
/// `it shows its {words}` (the mask's side words joined ` or `, only when not
/// every aspect); `it is under {b} m`, `it is past {a} m` or `it is between
/// {a} and {b} m` (only when bound); `{weapon} keeps {k} round(s)` (only above
/// nought). The gate's rule reads `Arming cannon: fires your lock when your
/// chance to hit is 60% or better and it shows its side or rear.`
///
/// **17b cut M2.2, the lead's ruling (c)**: read off the rule AS TYPED
/// ([`ArmingEcho`]) — `{a}` is the `range >` figure typed, else the band's
/// nearest above nought; `{b}` the `range <` figure typed, else the band's
/// furthest — so `range < 600` still reads `it is under 600 m` while the slot
/// stores 599 (and `WEAPONSTATUS`'s arming line, `arming_words`, shows the
/// stored `under 599m`, the last metre that fires). Every M1b.4 reply is
/// byte-identical. The chance clause is written whenever the echo carries
/// one, which [`parse_arming`]'s always does.
#[must_use]
pub fn arming_reply(weapon: &str, echo: &ArmingEcho) -> String {
    let mut clauses: Vec<String> = Vec::new();
    if let Some(permille) = echo.min_permille {
        clauses.push(format!(
            "your chance to hit is {}% or better",
            permille / 10
        ));
    }
    if let Some(aspects) = echo.aspects
        && aspects & ASPECTS_ANY != ASPECTS_ANY
    {
        let words = aspect_side_words(aspects);
        if !words.is_empty() {
            clauses.push(format!("it shows its {}", words.join(" or ")));
        }
    }
    let past = echo
        .past_m
        .or_else(|| echo.band.map(|(nearest, _)| nearest))
        .filter(|metres| *metres > 0);
    let under = echo
        .under_m
        .or_else(|| echo.band.map(|(_, furthest)| furthest))
        .filter(|metres| *metres < u32::MAX);
    match (past, under) {
        (Some(past), Some(under)) => {
            clauses.push(format!("it is between {past} and {under} m"));
        }
        (None, Some(under)) => clauses.push(format!("it is under {under} m")),
        (Some(past), None) => clauses.push(format!("it is past {past} m")),
        (None, None) => {}
    }
    if let Some(keeps) = echo.keeps.filter(|keeps| *keeps > 0) {
        clauses.push(format!("{weapon} keeps {keeps} round(s)"));
    }
    format!(
        "Arming {weapon}: fires your lock when {}.",
        clauses.join(" and ")
    )
}

/// `FIRE [<weapon>]` — shoot the lock (doc 16 §3.7).
///
/// **The pre-checks are doc 16 §3.7's refusal column, in its order**: the weapon has
/// to exist, then there has to be a lock, then the weapon has to be out of its cycle,
/// then it has to have a round, and *then* the tank has to be awake. Power last is
/// the doc's order and the useful one — a cold tank with no lock is told to get a
/// lock, which is the longer job.
///
/// Every one of them is a read of the **last frame**, so the pilot is told at once
/// instead of a pulse later; the arena refuses in the same order against its own
/// state ([`crate::arena::state::Arena::resolve_fire`]) when the mirror was stale, so
/// a frame one pulse behind changes which half answers and never what it says.
///
/// **16b cut AP.1** (16c `B-86`): the cycle is the last frame's countdown aged by
/// the wall clock since the frame was received ([`aged_weapons`]), unaged under a
/// staff tempo floor or on a frame built under one since lifted — so a `FIRE`
/// typed once the gun is truly ready is forwarded instead of refused locally, and
/// the arena takes it at its next wake. A remainder above nought is still refused,
/// in the same sentence. A `FIRE` forwarded inside the OS timer's overshoot of a
/// wake scheduled before the ready instant can still meet the arena's own refusal
/// a pulse later, which `help cockpit`'s "a pulse apart" paragraph already allows.
///
/// **17b cut M1b.4** (doc 17 §5, D-UM-18): the grammar grows two arms and no
/// verb — `FIRE <weapon> WHEN <condition> [AND <condition>]…` arms a slot
/// ([`fire_when`]) and `FIRE <weapon|ALL> OFF` clears one or every slot
/// ([`fire_off`]) — taken ONLY when [`fire_form`] reads `WHEN` or `OFF` as the
/// second word, or `OFF` alone, so every existing `FIRE` line goes on to the
/// pre-checks below byte for byte as it always did. A typed `FIRE` is never
/// gated by a slot (the owner's pin P1, 16c Q45).
///
/// **Doc 21 cut G.12a** (§7.3): three bare lines are read whole before any
/// other form — `FIRE HOLD` / `FIRE FREE` set the seat's master gate
/// ([`fire_gate`]) and `FIRE ALL` is one volley ([`fire_all`]). Bare means
/// nothing after the word, so `FIRE <weapon|ALL> HOLD WHEN <cond>` (a hold
/// rule) and `FIRE ALL OFF` keep their own arms below.
pub(super) fn fire(ctx: &mut Ctx<'_>, seat: &Seat, rest: &str) -> Result<(), CockpitError> {
    let bare = rest.trim();
    if bare.eq_ignore_ascii_case("hold") {
        return fire_gate(ctx, seat, false);
    }
    if bare.eq_ignore_ascii_case("free") {
        return fire_gate(ctx, seat, true);
    }
    if bare.eq_ignore_ascii_case("all") {
        return fire_all(ctx, seat);
    }
    let (_, tail) = first_word(rest);
    if first_word(tail).0.eq_ignore_ascii_case("hold") {
        return station_rules(ctx, seat, &format!("FIRE {rest}"));
    }
    match fire_form(rest) {
        FireForm::Shot => {}
        FireForm::Off { weapon, tail } => return fire_off(ctx, seat, weapon, tail),
        FireForm::When { weapon, conditions } => return fire_when(ctx, seat, weapon, conditions),
    }
    let actor = ctx.actor;
    // 16b cut AP.1: the one wall-clock read this verb ages the mirror by.
    let now = std::time::Instant::now();
    // 16b cut D.2 (`B-03`): a wreck fires nothing, and it is asked first — a
    // destroyed tank's weapons are gone, so every later check would answer about
    // the rack instead of about the tank.
    require_alive(ctx.w, seat.driver)?;
    let held = last_frame(ctx.w, seat.driver);
    let weapon = choose_weapon(&carried(ctx.w, seat.unit, held.as_ref(), now), rest)?;
    // **16b cut D.2** (`B-16`): every one of these is `FIRE <weapon> refused. …`,
    // which is the sentence the arena's own refusal says a pulse later. The
    // `NoSuchWeapon` above is the one that is not: `choose_weapon` failed to find
    // a weapon, so there is none to name — and (16b cut AE.3) neither is
    // `NoWeapon`, a bare `FIRE` on a platform that carries none.
    let refused = |said: &CockpitError| fire_refused(&weapon.id, said);
    if held
        .as_ref()
        .and_then(|held| frame_lock(&held.frame))
        .is_none()
    {
        return Err(refused(&CockpitError::NoLock));
    }
    if !weapon.ready {
        return Err(refused(&CockpitError::Cycling {
            secs: weapon.ready_in_ms.saturating_add(999) / 1_000,
        }));
    }
    if weapon.rounds <= 0 {
        return Err(refused(&CockpitError::Empty));
    }
    // **16b cut X.1** (doc 16 §9 item 1), in the arena's own order: the platform's
    // heat after the weapon's own two refusals and before the power. A platform
    // whose content declares no heat carries no `heat` on its frame and is never
    // refused here.
    //
    // **16b cut D.2** (`B-16`): and neither is a weapon that makes no heat, on
    // the platform's own row, which is the same clause `Arena::resolve_fire` and
    // `stock_mind::would_overheat` (the reflex's copy until 17b cut M4.6) apply
    // — pinned for the machine by the reflex's own tests until 17b cut M4.6c;
    // the mind's in `arena/stock_mind.rs`' d2 mirror off the view
    // (`d2_a_weapon_that_makes_no_heat_never_holds_the_mind_back`).
    if let Some(heat) = held.as_ref().and_then(|held| held.frame.heat)
        && heat.overheated()
        && weapon.heat_per_shot > 0
    {
        return Err(refused(&CockpitError::Overheated {
            heat: heat.heat,
            max: heat.max,
        }));
    }
    require_power(ctx.w, seat.driver).map_err(|said| refused(&said))?;
    forward_intent(
        ctx,
        seat.driver,
        seat.unit,
        IntentKind::Fire {
            weapon: weapon.id.clone(),
            slot: None,
        },
    )?;
    let line = format!("Firing {}.", weapon.id);
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    Ok(())
}

/// **`FIRE HOLD`'s reply** (doc 21 cut G.12a, §7.3), said whether or not the
/// gate was already held.
pub const WEAPONS_HOLD: &str = "Weapons hold: your rules stand but fire only when you say.";

/// **`FIRE FREE`'s reply** (doc 21 cut G.12a, §7.3), said whether or not the
/// gate was already free.
pub const WEAPONS_FREE: &str = "Weapons free.";

/// **The line `WEAPONSTATUS` and `MIND RULES` add while the gate is held**
/// (doc 21 cut G.12a): the rules are listed as before, and this says why none
/// of them fires. Nothing is added at weapons free, the default, so every
/// listing of a seat that never holds reads as it did.
pub const WEAPONS_HOLD_LINE: &str = "Weapons hold: no rule fires until FIRE FREE.";

/// **`FIRE HOLD` / `FIRE FREE`** (doc 21 cut G.12a, §7.3; Q-21-9's default
/// free): the seat's master gate over its hull's `FIRE <weapon> WHEN` rules,
/// as one discrete `IntentKind::Condition(Arming::WeaponsFree { free })` on
/// the fire channel — `FIRE ALL OFF`'s delivery, so it lands at a pulse,
/// replays identically and is folded by the arena hash. Under hold every rule
/// stays armed and listed and none fires; hold rules, a typed `FIRE` or `FIRE
/// ALL` and a mind's own warranted shot are untouched (the arena's
/// `evaluate_conditions`). A `FIRE FREE` re-evaluates the slots as it lands,
/// so a rule whose conditions hold fires at that pulse.
///
/// The wreck first, as every order. **The reply is unconditional and reads no
/// mirror**, for [`fire_off`]'s reason: a repeat is a no-op in the arena and
/// says the same line. Power is not asked: a switch is not a shot.
pub(super) fn fire_gate(ctx: &mut Ctx<'_>, seat: &Seat, free: bool) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    require_alive(ctx.w, seat.driver)?;
    forward_intent(
        ctx,
        seat.driver,
        seat.unit,
        IntentKind::Condition(Arming::WeaponsFree { free }),
    )?;
    let line = if free { WEAPONS_FREE } else { WEAPONS_HOLD };
    ctx.out
        .sys_to_actor(actor, NOTICE_CATEGORY, line.to_owned());
    Ok(())
}

/// **The opener of a weapon's out-of-reach refusal** (doc 21 cut G.12a): the
/// lock is past the weapon's last band. No `FIRE <weapon>` says it — a typed
/// shot past the bands is the arena's to resolve — but a volley picks only
/// the guns that reach, so it has to say which do not.
pub const OUT_OF_REACH: &str = "Out of reach";

/// **The lock's range, metres, as `RANGE` and the frame's `lock_range_m`
/// print it** (16b cut D.4's step, `T-20`), or `None` with no lock. One
/// function, so the keyframe and [`fire_all`]'s out-of-reach line read one
/// figure.
pub(super) fn frame_lock_range_m(frame: &Frame) -> Option<u64> {
    frame_lock(frame).and_then(|contact| {
        frame
            .contacts
            .iter()
            .find(|held| held.id == contact)
            .map(|held| range_m_rounded(held.range_cm, lock_range_step_m(held.age_ms)))
    })
}

/// **Why `weapon` does not reach the lock** (doc 21 cut G.12a), as the
/// `said` half of `FIRE <weapon> refused. …`: the lock's printed range and
/// the weapon's last band edge, metres.
pub(super) fn out_of_reach(frame: &Frame, weapon: &str) -> String {
    let reach_m = frame
        .weapons
        .iter()
        .find(|carried| carried.id == weapon)
        .and_then(|carried| carried.bands_cm.last())
        .map(|cm| cm / 100);
    match (frame_lock_range_m(frame), reach_m) {
        (Some(range_m), Some(reach_m)) => {
            format!("{OUT_OF_REACH}: the lock is {range_m} m off and it reaches {reach_m} m.")
        }
        _ => format!("{OUT_OF_REACH} of the lock."),
    }
}

/// **Bare `FIRE ALL`: one volley** (doc 21 cut G.12a, §7.3; the `g` key).
///
/// Each weapon that is ready, has rounds and whose band reaches the lock
/// (its frame row carries a band: the arena's own reach test, against the
/// exact range) fires exactly as `FIRE <weapon>` does — one
/// `IntentKind::Fire` each, a seat's shot, never gated — in the roster's
/// order, one line a weapon: `Firing <weapon>.` for a shot, and the weapon's
/// own refusal (`FIRE <weapon> refused. …`) for one it skips — cycling,
/// empty, out of reach, or overheated where it makes heat — before going on.
///
/// Refused whole, in the existing words, when nothing would fire: a wreck
/// ([`require_alive`]); no weapon ([`CockpitError::NoWeapon`]); no lock
/// (`FIRE ALL refused. No lock: …`); no weapon in reach (the FIRST weapon's
/// out-of-reach line, the most specific thing to say); power not online
/// (`FIRE ALL refused. …`, the platform's refusal, asked once); otherwise
/// the first in-reach weapon's own refusal. Each forwarded shot counts on
/// the seat's order window as a typed `FIRE` does.
pub(super) fn fire_all(ctx: &mut Ctx<'_>, seat: &Seat) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    // 16b cut AP.1: the one wall-clock read this verb ages the mirror by.
    let now = std::time::Instant::now();
    require_alive(ctx.w, seat.driver)?;
    let held = last_frame(ctx.w, seat.driver);
    let rows = carried(ctx.w, seat.unit, held.as_ref(), now);
    if rows.is_empty() {
        return Err(CockpitError::NoWeapon);
    }
    let all = |said: &str| {
        CockpitError::Refused(order_refused(
            IntentKind::fire("").as_str(),
            Some("ALL"),
            said,
        ))
    };
    let Some(frame) = held
        .as_ref()
        .map(|held| &held.frame)
        .filter(|frame| frame_lock(frame).is_some())
    else {
        return Err(all(&CockpitError::NoLock.to_string()));
    };
    if let Some(first) = rows.first()
        && rows.iter().all(|row| row.range_band.is_none())
    {
        return Err(CockpitError::Refused(order_refused(
            IntentKind::fire(&first.id).as_str(),
            Some(&first.id),
            &out_of_reach(frame, &first.id),
        )));
    }
    require_power(ctx.w, seat.driver).map_err(|said| all(&said.to_string()))?;
    // Each row's skip, in `FIRE`'s own order: the weapon's two refusals, the
    // reach, then the platform's heat for a weapon that makes some.
    let overheated = frame.heat.filter(|heat| heat.overheated());
    let skips: Vec<Option<String>> = rows
        .iter()
        .map(|row| {
            let said = if !row.ready {
                CockpitError::Cycling {
                    secs: row.ready_in_ms.saturating_add(999) / 1_000,
                }
                .to_string()
            } else if row.rounds <= 0 {
                CockpitError::Empty.to_string()
            } else if row.range_band.is_none() {
                out_of_reach(frame, &row.id)
            } else if let Some(heat) = overheated
                && row.heat_per_shot > 0
            {
                CockpitError::Overheated {
                    heat: heat.heat,
                    max: heat.max,
                }
                .to_string()
            } else {
                return None;
            };
            Some(order_refused(
                IntentKind::fire(&row.id).as_str(),
                Some(&row.id),
                &said,
            ))
        })
        .collect();
    if skips.iter().all(Option::is_some) {
        let first = rows
            .iter()
            .zip(&skips)
            .find(|(row, _)| row.range_band.is_some())
            .or_else(|| rows.iter().zip(&skips).next())
            .and_then(|(_, skip)| skip.clone())
            .unwrap_or_default();
        return Err(CockpitError::Refused(first));
    }
    for (row, skip) in rows.iter().zip(skips) {
        let line = match skip {
            Some(line) => line,
            None => {
                forward_intent(
                    ctx,
                    seat.driver,
                    seat.unit,
                    IntentKind::Fire {
                        weapon: row.id.clone(),
                        slot: None,
                    },
                )?;
                format!("Firing {}.", row.id)
            }
        };
        ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    }
    Ok(())
}

/// What `CONTACTS` says of an empty scope.
pub(super) const NO_CONTACTS_LINE: &str = "No contacts.";

/// What `VIEW BRIEF` says in its contacts' place when the platform holds none
/// (16b cut AP.4, 16c `N20`): the brief's own sentence, never `CONTACTS`'
/// `No contacts.`, so a reader can tell a brief's empty slot from a
/// `CONTACTS` reply. A declared reader sentence (doc 16 section 11).
pub const NO_CONTACTS_IN_WINDOW: &str = "No contacts in the window.";

/// The fixed last line of every `VIEW BRIEF` (16b cut AP.4, 16c `N20`), after
/// the off-window line when there is one: where a brief ends, for a script.
/// A declared reader sentence (doc 16 section 11).
pub const BRIEF_END_LINE: &str = "End of brief.";

/// `CONTACTS` — what the platform holds (doc 16 §3.6 "What a pilot sees").
pub(super) fn contacts(ctx: &mut Ctx<'_>, seat: &Seat) -> Result<(), CockpitError> {
    let held = last_frame(ctx.w, seat.driver);
    // 16b cut B.4 (`T-60`): the ranges held against the mirror's memory, the
    // figures the panels were sent.
    let rows = held
        .as_ref()
        .map(|held| frame_contacts_held(&held.frame, &held.ranges))
        .unwrap_or_default();
    let mut lines: Vec<String> = Vec::new();
    if rows.is_empty() {
        lines.push(NO_CONTACTS_LINE.to_owned());
    }
    for row in &rows {
        lines.push(row.render());
    }
    say_and_frame(ctx, held.as_ref(), lines);
    Ok(())
}

/// `WEAPONSTATUS` — ammunition and readiness, per weapon (doc 16 §5).
///
/// **16b cut W.1 made the numbers live**: they come off the arena's own frame, which
/// is the single writer of the cycle and the rounds while a match runs, and carry the
/// band of the platform's lock where there is one.
///
/// **16b cut AP.1** (16c `B-86`): and each cycle is aged by the wall clock since the
/// frame was received ([`aged_weapons`]), so `cannon  cycling (1 s)` counts down to
/// `cannon  ready` between frames. The keyframe re-pushed beside the lines is the
/// frame's own, unaged.
pub(super) fn weapon_status(ctx: &mut Ctx<'_>, seat: &Seat) -> Result<(), CockpitError> {
    // 16b cut AP.1: read once, at the top of the verb.
    let now = std::time::Instant::now();
    let held = last_frame(ctx.w, seat.driver);
    let weapons = carried(ctx.w, seat.unit, held.as_ref(), now);
    let mut lines: Vec<String> = Vec::new();
    if weapons.is_empty() {
        lines.push("This unit carries no weapon.".to_owned());
    }
    if let Some(line) = held.as_ref().and_then(|held| lock_line(&held.frame)) {
        lines.push(line);
    }
    // Doc 21 cut G.12a: the master gate, said only while it holds.
    if held.as_ref().is_some_and(|held| !held.frame.weapons_free) {
        lines.push(WEAPONS_HOLD_LINE.to_owned());
    }
    for weapon in &weapons {
        lines.push(weapon.render(&weapons));
        // **17b cut M1b.4** (doc 17 §5, as built for the 120-column floor):
        // an armed weapon's slot on its own line, two spaces in, under its
        // row — `  S1: >=60% side/rear` — the line the TUI pane and the web
        // print under the same row ([`arming_words`]: three renderers, one
        // shape). The row itself is unchanged.
        if held.as_ref().is_none_or(|held| held.frame.rules.is_empty())
            && let Some(armed) = &weapon.armed
        {
            lines.push(format!("  {}", arming_words(armed)));
        }
    }
    if let Some(held) = &held {
        for rule in &held.frame.rules {
            lines.extend(rule_read_lines(rule, &held.frame));
        }
    }
    say_and_frame(ctx, held.as_ref(), lines);
    Ok(())
}

/// **The seat says it is a range** (16c `Q-PT1`, 2026-10-27): `true` while
/// the frame's match is [`crate::arena::state::MatchMode::Range`] — one
/// side's pilots against targets that neither move, sense nor shoot. Read off
/// [`Frame::mode`], which the keyframe already carries as `"mode"`: `STATUS`
/// opens with it ([`range_sentence`]), `WATCH`'s header marks it, and the
/// TUI's and the web's status pane and picture header print `RANGE` off the
/// same key — so a range is never read as a fight from the seat (the
/// 2026-10-27 dev2 playtest, doc 16c §4: four targets that classified as
/// blue contacts, powered up, took hits and died read as a side that never
/// shot back).
#[must_use]
pub fn on_range(frame: &Frame) -> bool {
    frame.mode == crate::arena::state::MatchMode::Range
}

/// **The sentence `STATUS` opens with on a range** (16c `Q-PT1`): ` On the
/// range: targets on the blue side.` for a red pilot — the targets stand on
/// the other side's pods, which is the lobby's invariant for the mode — and
/// the empty string on any other match. The pilot's own side and the match's
/// public mode: nothing the observer rule keeps.
#[must_use]
pub fn range_sentence(frame: &Frame) -> String {
    if on_range(frame) {
        format!(
            " On the range: targets on the {} side.",
            frame.party.ooc_other().as_str()
        )
    } else {
        String::new()
    }
}

/// `STATUS` — the status block of doc 16 §5, as the last frame carries it.
pub(super) fn status(ctx: &mut Ctx<'_>, seat: &Seat) -> Result<(), CockpitError> {
    let held = last_frame(ctx.w, seat.driver);
    let lines = match held.as_ref() {
        Some(held) => {
            let frame = &held.frame;
            let mut lines = vec![
                // **16b cut Q.1**, first: the name everybody who has told this
                // tank apart is calling it by. A pilot asked "who is RA?" over
                // the radio needs one place to look.
                // 16c `Q-PT1`: and that it is a range, when it is one.
                format!("Callsign {}.{}", frame.callsign, range_sentence(frame)),
                format!(
                    "{}{}: power {}, {} kph, heading {}°{}{}",
                    display_name(ctx.w, seat.unit),
                    // **16b cut D.3 (`T-60`)**: the chassis, when it is not the
                    // one the pod authors.
                    fitted_bracket(ctx.w, seat.unit, frame),
                    frame.power.as_str(),
                    cms_to_kph(frame.speed_cms),
                    frame.pose.heading_mdeg / 1_000,
                    if frame.held { ", HELD" } else { "" },
                    // 16b cut D.2 (`B-03`): beside `HELD`, and the one mark a
                    // pilot who has stopped being told anything needs.
                    if frame.wreck { ", DESTROYED" } else { "" }
                ),
                // **16b cut D.1**: the pilot's own alert state, off their own
                // scope, where the arena's one global tempo word used to be
                // ([`own_scope`]).
                format!(
                    "  at {} m east, {} m north; {}",
                    frame.pose.x_cm / 100,
                    frame.pose.y_cm / 100,
                    match own_scope(frame) {
                        "hostile" => "scope holds a hostile",
                        "unidentified" => "scope holds an unidentified contact",
                        _ => "scope clear",
                    }
                ),
            ];
            // **16b cut V.1**: the named ground nearest the pilot's own pose, on a
            // line of its own after the position and only when the ground names a
            // spot — so two pilots can say where they are in words. Surveyed
            // ground and the pilot's own position: nothing about any other unit.
            let (spots, cell_cm) = spots_of(ctx.w, Some(held.match_id));
            if let Some(line) = nearest_spot_line(&spots, &frame.pose, cell_cm) {
                lines.push(line);
            }
            // **16b cut X.1**: a line of its own, and only for a platform that
            // carries heat.
            if let Some(heat) = frame.heat {
                lines.push(format!(
                    "  heat {} of {}{}",
                    heat.heat,
                    heat.max,
                    if heat.overheated() {
                        ", OVERHEATED"
                    } else {
                        ""
                    }
                ));
            }
            // **16b cut D.4** (`B-22`): a line of its own, after the heat and only
            // for a platform whose own last pass ran under interference. The three
            // integers are the pilot's own instrument — the budget the pass really
            // used against what it has when nothing is jamming, and what is left
            // of the reach — and nothing here says whose the emitter is or where
            // it stands ([`crate::arena::perception::Receiver`]).
            if let Some(receiver) = frame.receiver {
                lines.push(receiver_line(receiver));
            }
            // **16b cut O.2** (16c N12): after the receiver and only for a
            // platform that carries a pod — the pilot's own switch.
            if let Some(on) = frame.ecm {
                lines.push(emcon_status_line(on).to_owned());
            }
            // **16b cut F.2**: doc 16 §5's *armor and internals per section as
            // current over max*, one line per section and wrapped to eighty
            // columns by construction — the section, its internals, then each
            // facet's armour. It is the recorded open point K7's whole surface,
            // and a pilot's own tank is outside the observer rule entirely.
            lines.extend(section_lines(&frame.sections));
            lines
        }
        None => vec![format!(
            "{}: power off. The panel has told you nothing yet.",
            display_name(ctx.w, seat.unit)
        )],
    };
    say_and_frame(ctx, held.as_ref(), lines);
    Ok(())
}

/// **The chassis bracket `STATUS`'s second line carries** (16b cut D.3, `T-60`),
/// or an empty string.
///
/// The smoke transcript's own complaint: a pilot who typed `SIM UNIT biped_mk1`
/// read `Red Tank 1: power online, …` all match, because the object's name is
/// pack content (`Red Tank 1` is the pod's tank whatever is fitted to it) and
/// nothing on the panel said otherwise. The name stays — renaming a persistent
/// object per match is a much worse answer — and the fitted type goes beside it
/// in the bracket the board already uses for exactly this, `[biped_mk1]`, spelled
/// by the board's own [`lobby::type_word`].
///
/// **Only when it differs from the authored type**, so a stock tank's line is
/// byte for byte the one cut D.1 wrote. The pod is read off the frame's own
/// callsign — the team and slot the lobby fielded this tank from — and then the
/// pod is asked for **its own tank by name**, because
/// [`crate::arena::state::Arena`]'s `resolve_callsigns` gives *every* unit a
/// `Designator::Slot`: the harness's three hundred and a staff `@arena/start`'s
/// two included. The designator alone therefore says nothing about where a tank
/// came from, and without the second question a staff-fielded biped would be
/// bracketed against the authored type of a centre pod it was never in. A unit
/// the centre does not field from that pod brackets nothing.
///
/// It is the pilot's own tank, so the observer rule is not in play: doc 16 §5
/// puts a pilot's own platform outside it entirely.
pub(super) fn fitted_bracket(w: &World, unit: ObjId, frame: &Frame) -> String {
    // Doc 32 cut CF.0: the callsign no longer spells the slot, so the pod is
    // the centre's own whose tank this is, on the frame's side, and no other.
    let team = frame.party;
    let Some(index) = centre::POD_ENTRIES
        .iter()
        .filter(|(side, _, _)| *side == team)
        .map(|(_, index, _)| *index)
        .find(|index| centre::tank_for_slot(w, team, *index) == Some(unit))
    else {
        return String::new();
    };
    let Some(authored) = lobby::authored_unit_type(w, team, index) else {
        return String::new();
    };
    // The registry's `DefinitionId` and the pack-local entry id are the same
    // string (see `crate::definitions`' module head), so the tactics document's
    // own schema is directly comparable with the arena pack's authored type
    // without re-resolving either through the catalog.
    let Some(fitted) = mush_tactics::module::unit_of(w, unit)
        .map(|held| held.schema().key().id.as_str().to_owned())
    else {
        return String::new();
    };
    if fitted == authored.as_str() {
        return String::new();
    }
    format!(" [{}]", lobby::type_word(w, &fitted))
}

/// How wide a `STATUS` damage line may run before it is wrapped — inside the
/// eighty columns a line client is entitled to assume, with room for the
/// renderer's own margin.
pub(super) const DAMAGE_COLUMNS: usize = 76;

/// **The damage block `STATUS` prints** (16b cut F.2; doc 16 §5: *armor and
/// internals per section as current over max*).
///
/// Two lines per section — the section with its condition and its internals, then
/// its facets' armour, wrapped at `DAMAGE_COLUMNS` — and one for a section that
/// declares no facets. Empty for a platform the arena has told the world nothing
/// about, so a fixture with no structure prints the two lines it always did.
#[must_use]
pub fn section_lines(sections: &[crate::arena::step::FrameSection]) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for section in sections {
        let inside = match section.internals {
            Some((current, max)) => format!("  internals {current}/{max}"),
            None => String::new(),
        };
        lines.push(format!("  {}  {}{inside}", section.id, section.condition));
        let mut run = String::new();
        for facet in &section.facets {
            let cell = format!("{} {}/{}", facet.id, facet.armor, facet.armor_max);
            // Two spaces join a cell and a wrap pushes no comma, so `+ 2` is
            // the whole cost here (16b cut B.4 widened `scan_lines`' own test,
            // which does push one, and left this alone).
            if !run.is_empty() && run.len() + cell.len() + 2 > DAMAGE_COLUMNS {
                lines.push(run.clone());
                run.clear();
            }
            if run.is_empty() {
                run.push_str("    ");
            } else {
                run.push_str("  ");
            }
            run.push_str(&cell);
        }
        if !run.is_empty() {
            lines.push(run);
        }
    }
    lines
}

/// **What `VIEW` says to any second word but `BRIEF`** (16b cut AG.1): syntax
/// first, in the voice of `SPEED <kph>; a negative speed reverses.`, and the
/// same bytes for every word — a slot designator of a tank out there, a
/// provisional number, a word naming nothing, sixteen digits — so the refusal
/// is never an echo and never tells one word from another.
pub const VIEW_USAGE: &str = "VIEW [BRIEF]; BRIEF says the window in sentences.";

/// `VIEW` (either arm) with no frame held yet. A `const` since 16b cut AG.1,
/// beside its twin below, so both arms say the one string.
pub(super) const WINDOW_DARK_UNTOLD: &str =
    "The window is dark: the panel has told you nothing yet.";

/// `VIEW` (either arm) with a frame held but no grid to draw (16b cut B.4,
/// `B-23`). A `const` since 16b cut AG.1, whose brief arm also says it when the
/// ground has gone between the payload's read and the walk's.
pub(super) const WINDOW_DARK_NO_GROUND: &str =
    "The window is dark: there is no ground under the panel to draw.";

/// `VIEW` — the arena window, as the grid frame (doc 16 §3.6, §5).
///
/// **16b cut Q.3**: the panel goes first and the one line that may follow it
/// comes after, because the line is *about* the map the pilot has just been
/// shown ([`beyond_window_line`]). **16b cut V.1** added a second such line,
/// after the first: the named spots off the window ([`off_window_spots_line`]).
///
/// **16b cut AG.1** (16c N17) gave it one second word. `rest` empty is the map,
/// byte for byte as before; `BRIEF` (case-folded) is the same window said in
/// sentences ([`brief_lines`]) — built off the very payload the map would push,
/// and then NOT pushed: the brief goes through [`say_and_frame`] as `CONTACTS`
/// does, so the one grid a pilot is sent is still the one a bare `VIEW` sends
/// (16b cut Q.3's rule). Any other word is [`VIEW_USAGE`], the same bytes for
/// every word and never an echo of it, so `VIEW BA` says nothing `VIEW ZZ` does
/// not. The two dark-window lines are shared by both arms: the shape test runs
/// before they diverge.
pub(super) fn view(ctx: &mut Ctx<'_>, seat: &Seat, rest: &str) -> Result<(), CockpitError> {
    let brief = match rest.trim() {
        "" => false,
        word if word.eq_ignore_ascii_case("brief") => true,
        _ => return Err(CockpitError::Usage(VIEW_USAGE.to_owned())),
    };
    let actor = ctx.actor;
    // The map is the frame's; a line client's renderer draws it. Nothing is said
    // twice.
    let Some(held) = last_frame(ctx.w, seat.driver) else {
        ctx.out
            .sys_to_actor(actor, NOTICE_CATEGORY, WINDOW_DARK_UNTOLD);
        return Ok(());
    };
    let payload = view_frame_held(ctx, held.match_id, &held.frame, &held.ranges);
    // **16b cut B.4** (`B-23`): a frame is held but the window cannot be drawn —
    // the registry holds no arena for the match, or the pose is off the terrain
    // — so `view_frame` answered the bare status frame, which declares no shape
    // and a line client prints as nothing: `VIEW` used to answer with a blank.
    // The test is inline, on the key `view_window` writes; it is what
    // `mush_ansi::panel::is_grid` reads on the other side, and this crate takes
    // no dependency on a renderer crate to ask it.
    if payload
        .pointer("/schema/shape")
        .and_then(serde_json::Value::as_str)
        != Some("grid")
    {
        ctx.out
            .sys_to_actor(actor, NOTICE_CATEGORY, WINDOW_DARK_NO_GROUND);
        return Ok(());
    }
    if brief {
        // **16b cut AG.1**: the ground `view_frame_held` just read, read again
        // for the walk. A ground gone between the two reads is the dark window
        // above, never a panic.
        let lines = ctx
            .w
            .module_state::<ArenaRegistry>()
            .and_then(|registry| registry.arena_of(held.match_id))
            .map(|ground| {
                brief_lines(
                    &held.frame,
                    &ground.terrain,
                    &ground.key_spots,
                    &payload,
                    &held.ranges,
                )
            });
        let Some(lines) = lines else {
            ctx.out
                .sys_to_actor(actor, NOTICE_CATEGORY, WINDOW_DARK_NO_GROUND);
            return Ok(());
        };
        // The lines, then the status keyframe re-pushed — never the grid.
        say_and_frame(ctx, Some(&held), lines);
        return Ok(());
    }
    let beyond = beyond_window_line(&held.frame, &payload);
    // **16b cut V.1**: the named ground the window could not draw, after the
    // contacts' line — a spot is a place the pilot may name whether or not it
    // is in sight, because the ground is surveyed. Read against the drawing's
    // own spots, so a spot under a contact (drawn, then covered) is not listed.
    let (spots, cell_cm) = spots_of(ctx.w, Some(held.match_id));
    let off_window =
        off_window_spots_line(&spots, &drawn_spots(&payload), &held.frame.pose, cell_cm);
    ctx.out.push(
        mush_core::Audience::Actor(actor),
        OutputEvent::Panel {
            feed: SmolStr::new_static(FRAME_FEED),
            kind: mush_proto::PanelFrameKind::Keyframe,
            data: payload,
        },
    );
    if let Some(line) = beyond {
        ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    }
    if let Some(line) = off_window {
        ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    }
    Ok(())
}

/// Say what a read verb answers and push the frame it read (doc 16 §5: one feed,
/// and the line renderer prints the frame).
///
/// **16b cut Q.3** took the `with_view` flag off: `VIEW` is the one verb that
/// pushes the grid, and it now owns its own order — the panel first, then the
/// line it owes about what the panel could not fit.
pub(super) fn say_and_frame(ctx: &mut Ctx<'_>, held: Option<&CockpitFrame>, lines: Vec<String>) {
    let actor = ctx.actor;
    for line in lines {
        ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    }
    let Some(held) = held else {
        return;
    };
    // 16b cut B.4 (`T-60`): the frame with the ranges the pilot was told.
    let payload = crate::detachment::commands::enrich(
        ctx.w,
        actor,
        status_frame_held(held.match_id, &held.frame, &held.ranges),
    );
    ctx.out.push(
        mush_core::Audience::Actor(actor),
        OutputEvent::Panel {
            feed: SmolStr::new_static(FRAME_FEED),
            kind: mush_proto::PanelFrameKind::Keyframe,
            data: payload,
        },
    );
}

// ---------------------------------------------------------------------------
// The unit's own numbers
// ---------------------------------------------------------------------------

/// **The keyframe's `armed` object** (17b cut M1b.3; doc 17 §3, D-UM-4): the
/// slot, the target's KIND, the odds per mille and as the whole per cent,
/// the facet words in the mask, the rounds kept, and the two range bounds —
/// `null` for an unbound one. **Never a designator**: a
/// [`SlotTarget::Contact`] writes `"contact"` with no id (doc 17 §5, the
/// observer rule; it is unlawful to arm in 17b cut M1b).
#[must_use]
pub fn arming_json(armed: &FrameArming) -> serde_json::Value {
    serde_json::json!({
        "slot": armed.slot,
        "target": arming_target_word(armed.target),
        "min_permille": armed.min_permille,
        "percent": armed.min_permille / 10,
        "aspects": ASPECT_ORDER
            .iter()
            .filter(|aspect| aspect_in(armed.aspects, **aspect))
            .map(Aspect::as_str)
            .collect::<Vec<&str>>(),
        "keeps": armed.keeps,
        "min_range_m": (armed.min_range_m > 0).then_some(armed.min_range_m),
        "max_range_m": (armed.max_range_m < u32::MAX).then_some(armed.max_range_m),
    })
}

/// The four facets in mask order — `front`, `left`, `right`, `rear`
/// ([`crate::arena::perception::Aspect::code`]).
pub(super) const ASPECT_ORDER: [Aspect; 4] =
    [Aspect::Front, Aspect::Left, Aspect::Right, Aspect::Rear];

/// **A slot's target, as a word** (17b cut M1b.3): `lock`, `best` or
/// `contact` — the KIND, never which contact.
#[must_use]
pub const fn arming_target_word(target: SlotTarget) -> &'static str {
    match target {
        SlotTarget::Lock => "lock",
        SlotTarget::BestHostile => "best",
        SlotTarget::Contact { .. } => "contact",
    }
}

/// **The arming line** (17b cut M1b.3; doc 17 §5's `WEAPONSTATUS` sentence as
/// built: the arming on its own line under the weapon's row, for the
/// 120-column floor) — ONE formatter for the three renderers: line mode's
/// `WEAPONSTATUS` prints it indented two spaces under the weapon's row (17b
/// cut M1b.4), and the TUI weapons pane and the web's `weaponLines` print the
/// same words (their copies of this rule, pinned equal by the fixture scene).
///
/// `S{n}:` then the terms, one space apart, compact for the TUI's 46-column
/// floor: `best` when the target is the best hostile (`lock` is the default
/// and prints nothing; a `contact` target prints `contact` and no id); then
/// `>={percent}%` always; then the facets as de-duplicated side words —
/// `front`, `side` (left or right), `rear`, in that order, joined `/` — only
/// when the mask is not all four (an empty mask prints none); then the range
/// with the unit attached, `{a}-{b}m`, `under {b}m` or `past {a}m`, only when
/// bound; then `keeps {k}` only above nought. `S1: >=60% side/rear keeps 8`;
/// `S1: >=60% side/rear 200-600m keeps 8`; T1's `S1: best >=10% under 420m`.
#[must_use]
pub fn arming_words(armed: &FrameArming) -> String {
    let mut terms: Vec<String> = vec![format!("S{}:", armed.slot)];
    match armed.target {
        SlotTarget::Lock => {}
        SlotTarget::BestHostile => terms.push("best".to_owned()),
        SlotTarget::Contact { .. } => terms.push("contact".to_owned()),
    }
    terms.push(format!(">={}%", armed.min_permille / 10));
    if armed.aspects & ASPECTS_ANY != ASPECTS_ANY {
        let words = aspect_side_words(armed.aspects);
        if !words.is_empty() {
            terms.push(words.join("/"));
        }
    }
    match (armed.min_range_m > 0, armed.max_range_m < u32::MAX) {
        (true, true) => terms.push(format!("{}-{}m", armed.min_range_m, armed.max_range_m)),
        (false, true) => terms.push(format!("under {}m", armed.max_range_m)),
        (true, false) => terms.push(format!("past {}m", armed.min_range_m)),
        (false, false) => {}
    }
    if armed.keeps > 0 {
        terms.push(format!("keeps {}", armed.keeps));
    }
    terms.join(" ")
}

/// One weapon, as [`frame_weapons`] and [`weapons_of`] read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeaponRow {
    /// The component id (`cannon`, `machine_gun`).
    pub id: SmolStr,
    /// Whether its cycle has finished.
    pub ready: bool,
    /// **How long until it has**, milliseconds; `0` when it is ready (16b cut W.1).
    /// Zero from [`weapons_of`], whose world-side cells a running match does not
    /// move.
    pub ready_in_ms: i64,
    /// Rounds left.
    pub rounds: i64,
    /// Rounds a full load carries.
    pub rounds_max: i64,
    /// **The band the platform's lock falls in for this weapon** (16b cut W.1), or
    /// `None` with no lock, with the lock out of the weapon's reach, or from
    /// [`weapons_of`], which has no lock to read.
    pub range_band: Option<crate::arena::weapons::FrameBand>,
    /// **This weapon's own chance against the lock**, per mille (16b cut F.2; doc
    /// 16 §3.5 step 5 and §5's *per weapon*), or `None` with no lock and from
    /// [`weapons_of`], which has none to resolve against.
    pub p_permille: Option<u32>,
    /// **What one shot of it adds to the platform's heat** (16b cut D.2, `B-16`),
    /// and `0` for a weapon that makes none — which is every weapon of a stock
    /// tank, and also what [`weapons_of`] answers, because a unit no arena holds
    /// has no heat state to read.
    ///
    /// The `FIRE` pre-check skips the heat refusal for a row of nought, which is
    /// the clause [`crate::arena::state::Arena::resolve_fire`] applies a pulse
    /// later.
    pub heat_per_shot: u32,
    /// **The slot the weapon is armed in** (17b cut M1b.3; D-UM-4), off the
    /// frame's own row, or `None` — and always `None` from [`weapons_of`],
    /// which reads no arena. 17b cut M1b.4's `WEAPONSTATUS` prints it through
    /// [`arming_words`] on its own line under the row; nothing prints it here.
    pub armed: Option<FrameArming>,
}

impl WeaponRow {
    /// The line `WEAPONSTATUS` prints for this row (doc 16 §5's status block).
    ///
    /// `cannon  ready  40 of 40 round(s)  band 1 of 2  46% to hit` while there is
    /// something to shoot at, and `cannon  cycling (4 s)  39 of 40 round(s)` while
    /// the breech is still busy. The band and the percentage are only ever printed
    /// when the platform has a lock, because that is the only time either means
    /// anything — and the percentage is **this weapon's**, which is the whole of
    /// 16b cut F.2's item 4.
    ///
    /// **16b cut D.2** (`B-17`) opens the short name the pilot may type —
    /// `machine_gun (mg)` — where this row has one that no other row shares.
    /// That is why the whole list is the argument: a short name is a fact about
    /// the *platform's* weapons and not about one of them, and a row that
    /// advertised an ambiguous `m` would be advertising a refusal.
    #[must_use]
    pub fn render(&self, rows: &[WeaponRow]) -> String {
        let named = match weapon_short_of(rows, &self.id) {
            Some(short) => format!("{} ({short})", self.id),
            None => self.id.to_string(),
        };
        let ready = if self.ready {
            "ready".to_owned()
        } else {
            format!(
                "cycling ({} s)",
                self.ready_in_ms.saturating_add(999) / 1_000
            )
        };
        let band = match self.range_band {
            Some(band) => format!("  band {} of {}", band.index, band.of),
            None => String::new(),
        };
        let chance = match self.p_permille {
            Some(permille) => format!("  {}% to hit", permille / 10),
            None => String::new(),
        };
        format!(
            "{named}  {ready}  {} of {} round(s){band}{chance}",
            self.rounds, self.rounds_max
        )
    }
}

/// The cell a weapon component's ammunition lives in (doc 16 §3.3).
pub(super) const AMMO_CELL: &str = "ammo";

/// The cell its cycle lives in.
pub(super) const READY_AT_CELL: &str = "ready_at_ms";

/// **Every weapon a unit carries**, in component order (doc 16 §3.3's
/// components).
///
/// A weapon is a component with both an `ammo` and a `ready_at_ms` cell, which
/// is exactly what the unit decoder gives one (`definitions/unit.rs`). Read
/// structurally rather than by looking the weapon definition up, because the
/// question here is what *this* tank is carrying now.
#[must_use]
pub fn weapons_of(w: &World, unit: ObjId) -> Vec<WeaponRow> {
    let Some(cells) = mush_tactics::module::unit_of(w, unit) else {
        return Vec::new();
    };
    let (Ok(ammo), Ok(ready_at)) = (
        mush_tactics::CellId::new(AMMO_CELL),
        mush_tactics::CellId::new(READY_AT_CELL),
    ) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for (component, _) in cells.components() {
        let Some(rounds) = cells.cell(&mush_tactics::CellAddr::Component {
            component: component.clone(),
            cell: ammo.clone(),
        }) else {
            continue;
        };
        let Some(cycle) = cells.cell(&mush_tactics::CellAddr::Component {
            component: component.clone(),
            cell: ready_at.clone(),
        }) else {
            continue;
        };
        rows.push(WeaponRow {
            id: SmolStr::new(component.as_str()),
            ready: cycle.current == 0,
            ready_in_ms: 0,
            rounds: rounds.current,
            rounds_max: rounds.max,
            range_band: None,
            p_permille: None,
            // 16b cut D.2: a unit no arena holds has no heat state to read, and
            // no frame either — so nothing is ever refused for heat off this row.
            heat_per_shot: 0,
            // 17b cut M1b.3: a unit no arena holds holds no slot.
            armed: None,
        });
    }
    rows
}

/// **Every weapon the pilot can name**, live where a frame says so (16b cut W.1).
///
/// The frame's rows while the arena has spoken, and [`weapons_of`]'s structural read
/// of the M32 components **only where there is no frame at all** — the same
/// fail-closed rule [`power_of`] follows, and the reason a pilot who has just
/// boarded can still be told the cannon exists.
///
/// **16b cut D.1 narrowed the fallback to that one case.** It used to fall back
/// whenever the frame's list was *empty*, so a pilot whose turret had been shot
/// off read `cannon ready 40 of 40 round(s)` off the pre-match world record,
/// typed `FIRE`, was told `Firing cannon.` and learned the truth a pulse later
/// from the arena's own refusal. An empty list on a live frame is a **fact**: the
/// arena is the single writer while a match runs (D-16-1), and what it says the
/// platform carries is what the platform carries. `WEAPONSTATUS` then prints
/// `This unit carries no weapon.` and a bare `FIRE` refuses in the cockpit with
/// [`CockpitError::NoWeapon`] (16b cut AE.3), which is the same sentence, one
/// pulse earlier.
///
/// **16b cut AP.1** (16c `B-86`): the frame's rows are read through
/// [`aged_weapons`] at `now`, so each countdown is the frame's own aged by the wall
/// time since the world received it — unaged under a staff tempo floor, or on a
/// frame built under one since lifted (`floored`).
pub(super) fn carried(
    w: &World,
    unit: ObjId,
    held: Option<&CockpitFrame>,
    now: std::time::Instant,
) -> Vec<WeaponRow> {
    match held {
        Some(held) => aged_weapons(held, now, floored(w, held)),
        None => weapons_of(w, unit),
    }
}

/// **The short name of a weapon id** (16b cut D.2, `B-17`): the initials of an
/// id of two or more `_`-separated words, and `None` for a one-word id.
///
/// `machine_gun` gives `mg`; `cannon` and `laser` give nothing, because a
/// one-letter `c` is not a name, it is a keystroke that would collide with the
/// first prefix anybody added.
///
/// **It is derived and not content-declared**, and that is a deliberate
/// departure from §5 Q14's default. Declaring short names means a new version of
/// all six unit packs, a re-blessed digest for each, and a story for every roster
/// tank's `DefinitionRef` on the live box — all for one two-letter word that the
/// id already spells. The derivation has the property the declaration was wanted
/// for (the pilot types two letters) and costs no content at all.
///
/// **It is not [`crate::match_::lobby`]'s private `short_name`, and the two are
/// unrelated.** That one answers the LAST SEGMENT of a canonical *unit* id —
/// `tacsim.unit.line_tank` gives `line_tank` — which is the opposite rule about a
/// different kind of id. This one is initials, over a component id.
#[must_use]
pub fn weapon_initials(id: &str) -> Option<String> {
    let words: Vec<&str> = id.split('_').filter(|word| !word.is_empty()).collect();
    if words.len() < 2 {
        return None;
    }
    let mut short = String::with_capacity(words.len());
    for word in words {
        for letter in word.chars().take(1) {
            short.push(letter.to_ascii_lowercase());
        }
    }
    Some(short)
}

/// **The short name `id` may be typed as on this platform** (16b cut D.2,
/// `B-17`), or `None` when it has none or when another row shares it.
///
/// A shared short name is not a name: two rows answering to `mg` means `FIRE mg`
/// is [`CockpitError::AmbiguousWeapon`], and a `WEAPONSTATUS` row that advertised
/// it would be advertising a refusal.
#[must_use]
pub fn weapon_short_of(rows: &[WeaponRow], id: &str) -> Option<String> {
    let short = weapon_initials(id)?;
    let shared = rows.iter().any(|row| {
        !row.id.eq_ignore_ascii_case(id)
            && weapon_initials(&row.id).is_some_and(|other| other == short)
    });
    (!shared).then_some(short)
}

/// The weapon `FIRE` means: the one the pilot named, else [`DEFAULT_WEAPON`],
/// else the first the unit carries — answering the whole row, so the caller's
/// pre-checks read the same numbers the choice was made from.
///
/// **16b cut D.2** (`B-17`) gave the named case three steps, case-folded and
/// trimmed, each tried in full before the next:
///
/// 1. the **exact id** — `FIRE machine_gun`, which is what every existing script
///    and every help line types and which therefore can never be ambiguous;
/// 2. the **short name** ([`weapon_initials`]) — `FIRE mg`;
/// 3. the **prefix** — `FIRE can`, `FIRE mach`.
///
/// Two or more candidates at a step is [`CockpitError::AmbiguousWeapon`] naming
/// them, and no candidate at any step is the existing
/// [`CockpitError::NoSuchWeapon`]. A bare `FIRE` on a platform with no rows at
/// all is [`CockpitError::NoWeapon`] (16b cut AE.3): the pilot named nothing, so
/// the refusal names nothing. A step is not fallen through on ambiguity:
/// `m` matching two short names must not quietly become a prefix search that
/// matches the same two.
///
/// `B-17`: `help cockpit` told a pilot to type `FIRE mg` for four cuts and no
/// such weapon existed, because the ids are component ids.
pub(super) fn choose_weapon(rows: &[WeaponRow], rest: &str) -> Result<WeaponRow, CockpitError> {
    let named = rest.trim();
    if !named.is_empty() {
        let folded = named.to_ascii_lowercase();
        let steps: [fn(&WeaponRow, &str) -> bool; 3] = [
            |row, word| row.id.eq_ignore_ascii_case(word),
            |row, word| weapon_initials(&row.id).is_some_and(|short| short == word),
            |row, word| row.id.to_ascii_lowercase().starts_with(word),
        ];
        for matches in steps {
            let found: Vec<&WeaponRow> = rows.iter().filter(|row| matches(row, &folded)).collect();
            match found.as_slice() {
                [] => {}
                [one] => return Ok((*one).clone()),
                many => {
                    return Err(CockpitError::AmbiguousWeapon {
                        word: named.to_owned(),
                        ids: many
                            .iter()
                            .map(|row| row.id.as_str())
                            .collect::<Vec<&str>>()
                            .join(" or "),
                    });
                }
            }
        }
        return Err(CockpitError::NoSuchWeapon(named.to_owned()));
    }
    rows.iter()
        .find(|weapon| weapon.id.as_str() == DEFAULT_WEAPON)
        .or_else(|| rows.first())
        .cloned()
        .ok_or(CockpitError::NoWeapon)
}

/// **Both of a unit's speed ceilings in cm/s** (16b cut D.3, `T-60`): forward,
/// and the reverse magnitude.
///
/// One read of the same `tracked` rules the arena's own apply reads
/// ([`crate::arena::state::TrackedRules::reverse_ceiling_cms`]), so `SPEED`'s
/// refusal and the arena's clamp cannot disagree about where the floor is. A unit
/// with no tactical record at all makes nothing in either direction.
pub(super) fn speed_ceilings(w: &World, unit: ObjId) -> (u32, u32) {
    match mush_tactics::module::unit_of(w, unit) {
        Some(cells) => {
            let rules = crate::match_::tracked_rules(cells);
            (rules.max_speed_cms, rules.reverse_ceiling_cms())
        }
        None => (0, 0),
    }
}

/// Kilometres an hour to centimetres a second, and back. Integer both ways (doc
/// 16 §3.2: the wire is whole centimetres), so a pilot's round number is a round
/// number of cm/s and the readback is the same round number.
pub(super) fn kph_to_cms(kph: i32) -> i32 {
    kph.saturating_mul(100_000) / 3_600
}

/// See [`kph_to_cms`].
pub(super) fn cms_to_kph(cms: i32) -> i32 {
    cms.saturating_mul(36) / 1_000
}

/// **The contact a pilot's word names** (16b cut Q.1), or `None`.
///
/// It replaces cut K.2's `parse_contact`, and it **replaces** it: the sixteen
/// base-16 digits stop being typable altogether (16b §0.2 — no second lane). The
/// owner's first hand-played match is the reason: `lock 29b81a…` is not a thing a
/// person types while being shot at.
///
/// Two spellings resolve, both case-folded and trimmed:
///
/// * the row's **current** designator — `BA` while that contact is classified
///   now, `X2` while it is not;
/// * the row's **alias** rendered `X<n>`, which is the number the platform gave
///   the target before it was told apart, so a lock taken as `X2` is still
///   `LOCK X2` afterwards.
///
/// Everything else is `None`, and every `None` is the one byte-identical
/// [`NO_SUCH_CONTACT`]. There is deliberately no second wording for *well formed
/// but not held*: because a row carries the slot designator only while its
/// contact is classified **now**, `LOCK BA` against an unheld, a still-unknown or
/// a re-acquired-unknown `BA` has to be indistinguishable from `LOCK ZZ`, or the
/// refusal itself would say which tanks are out there.
///
/// The arena still only ever receives `IntentKind::Lock { contact: ContactId }`,
/// so a lock taken under one spelling survives the rename by construction.
#[must_use]
pub fn resolve_designator(rows: &[ContactRow], word: &str) -> Option<ContactId> {
    let named = word.trim();
    if named.is_empty() {
        return None;
    }
    rows.iter()
        .find(|row| {
            row.designator.to_string().eq_ignore_ascii_case(named)
                || row.alias.is_some_and(|n| {
                    Designator::Provisional(n)
                        .to_string()
                        .eq_ignore_ascii_case(named)
                })
        })
        .map(|row| row.id)
}
