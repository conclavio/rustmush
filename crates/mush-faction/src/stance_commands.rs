//! **The stance verbs** (doc 40 cut FR.1, Q-40-11): `+faction/stance <mine>=<other>/<stance>`
//! for an organization's declarers, `@faction/stance <from>=<to>/<stance>` for staff, and the
//! rows `+faction <name>` shows. The table itself is [`crate::stance`]'s.
//!
//! A side is named as a faction first, else as a mush-org organization; a hidden faction or a
//! hidden organization the viewer is not in is indistinguishable from a missing one, on both
//! the declaring and the shown side.

use mush_core::output::display_name;
use mush_core::{CmdError, Ctx, ObjId, World};

use crate::data::{FactionState, Stance, StanceRow, StanceSide, persist_del, persist_stance};

const USAGE_PLAYER: &str = "Usage: +faction/stance <yours>=<other>/<allied|neutral|hostile>";
const USAGE_STAFF: &str = "Usage: @faction/stance <from>=<to>/<allied|neutral|hostile>";
const NO_SUCH_SIDE: &str = "No faction or organization by that name.";

fn factions(w: &World) -> Option<&FactionState> {
    w.module_state::<FactionState>()
}

fn orgs(w: &World) -> Option<&mush_org::OrgState> {
    w.module_state::<mush_org::OrgStore>()
        .map(mush_org::OrgStore::state)
}

/// **The side `name` names, as `viewer` may know it**: a listed faction, else an organization
/// that is not hidden from them.
fn resolve_side(w: &World, viewer: ObjId, staff: bool, name: &str) -> Option<StanceSide> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    if let Some(st) = factions(w)
        && let Some(f) = st.faction_by_name(name)
    {
        return st
            .faction_listed(viewer, staff, f)
            .then_some(StanceSide::Faction(f.id));
    }
    let orgs = orgs(w)?;
    let by_id = name
        .strip_prefix('#')
        .and_then(|n| n.parse::<u64>().ok())
        .map(mush_org::OrgId);
    let org = orgs.orgs().find(|org| {
        Some(org.id) == by_id
            || org.name.eq_ignore_ascii_case(name)
            || org.aliases.iter().any(|a| a.eq_ignore_ascii_case(name))
    })?;
    side_listed(w, viewer, staff, StanceSide::Org(org.id)).then_some(StanceSide::Org(org.id))
}

/// Whether `viewer` may know `side` exists.
fn side_listed(w: &World, viewer: ObjId, staff: bool, side: StanceSide) -> bool {
    match side {
        StanceSide::Faction(id) => factions(w).is_some_and(|st| {
            st.factions
                .get(&id)
                .is_some_and(|f| st.faction_listed(viewer, staff, f))
        }),
        StanceSide::Org(id) => orgs(w).is_some_and(|orgs| {
            orgs.org(id)
                .is_some_and(|org| !org.hidden || staff || orgs.membership(id, viewer).is_some())
        }),
    }
}

/// A side's display name.
fn side_name(w: &World, side: StanceSide) -> String {
    match side {
        StanceSide::Faction(id) => factions(w)
            .and_then(|st| st.factions.get(&id))
            .map_or_else(|| format!("faction #{id}"), |f| f.name.to_string()),
        StanceSide::Org(id) => orgs(w)
            .and_then(|orgs| orgs.org(id))
            .map_or_else(|| format!("organization #{id}"), |org| org.name.to_string()),
    }
}

/// **Whether `who` declares for `side`** (Q-40-11): a faction's principal or a member at or above
/// its `officer_level`; an organization's member holding `org.stance.declare` on its root scope.
fn declares_for(w: &World, who: ObjId, side: StanceSide) -> bool {
    match side {
        StanceSide::Faction(id) => factions(w).is_some_and(|st| {
            st.is_officer(who, id)
                || st
                    .factions
                    .get(&id)
                    .is_some_and(|f| f.principals.contains(&who))
        }),
        StanceSide::Org(id) => {
            let Some(scope) = orgs(w).and_then(|orgs| orgs.org(id)).map(|o| o.root_scope) else {
                return false;
            };
            let Some(capability) = mush_org::CapabilityId::new(mush_org::STANCE_DECLARE_CAPABILITY)
            else {
                return false;
            };
            mush_org::authorize_in_world(
                w,
                &mush_org::AuthRequest {
                    actor: who,
                    capability,
                    scope,
                    now: w.now(),
                    amount: None,
                },
            )
            .is_allow()
        }
    }
}

/// **Who hears a declaration toward `side`**: its officers — a faction's members at or above
/// `officer_level`, an organization's active members who could declare for it.
fn officers_of(w: &World, side: StanceSide) -> Vec<ObjId> {
    match side {
        StanceSide::Faction(id) => factions(w).map_or_else(Vec::new, |st| {
            st.members_of(id)
                .map(|m| m.character)
                .filter(|who| st.is_officer(*who, id))
                .collect()
        }),
        StanceSide::Org(id) => orgs(w).map_or_else(Vec::new, |orgs| {
            orgs.members_in(id)
                .filter(|m| m.status == mush_org::MembershipStatus::Active)
                .map(|m| m.member)
                .filter(|who| declares_for(w, *who, side))
                .collect()
        }),
    }
}

/// **`+faction/stance` and `@faction/stance`**: declare `from`'s stance toward `to`. Staff name
/// any two sides; a player names one they declare for.
pub(crate) fn declare(ctx: &mut Ctx<'_>, staff_verb: bool) -> Result<(), CmdError> {
    let usage = if staff_verb {
        USAGE_STAFF
    } else {
        USAGE_PLAYER
    };
    let raw = ctx.args.raw().to_owned();
    let (lhs, rhs) = raw.split_once('=').ok_or_else(|| CmdError::msg(usage))?;
    let (other, word) = rhs.rsplit_once('/').ok_or_else(|| CmdError::msg(usage))?;
    let stance = Stance::parse(word).ok_or_else(|| CmdError::msg(usage))?;
    let actor = ctx.actor;
    let staff = staff_verb || crate::commands::is_admin(ctx.w, actor);
    let from = resolve_side(ctx.w, actor, staff, lhs).ok_or_else(|| CmdError::msg(NO_SUCH_SIDE))?;
    let to = resolve_side(ctx.w, actor, staff, other).ok_or_else(|| CmdError::msg(NO_SUCH_SIDE))?;
    let (from_name, to_name) = (side_name(ctx.w, from), side_name(ctx.w, to));
    if from == to {
        return Err(CmdError::msg(
            "An organization holds no stance toward itself.",
        ));
    }
    if !staff && !declares_for(ctx.w, actor, from) {
        return Err(CmdError::msg(format!(
            "You do not declare {from_name}'s stance: its principals and officers do, or a \
             member holding {} for an organization.",
            mush_org::STANCE_DECLARE_CAPABILITY
        )));
    }
    let now = ctx.w.now();
    let st = ctx
        .w
        .module_state_mut::<FactionState>()
        .ok_or_else(|| CmdError::msg("The faction registry is unavailable."))?;
    let was = st.stance(from, to);
    if was == stance {
        return Err(CmdError::msg(format!(
            "{from_name} already stands {} toward {to_name}.",
            stance.label()
        )));
    }
    let row = (stance != Stance::Neutral).then_some(StanceRow {
        from,
        to,
        stance,
        declared_by: actor,
        at: now,
    });
    match &row {
        Some(row) => {
            st.stances.insert((from, to), row.clone());
        }
        None => {
            st.stances.remove(&(from, to));
        }
    }
    let reciprocated = st.stance(to, from) == stance;
    match &row {
        Some(row) => persist_stance(ctx.out, row),
        None => persist_del(ctx.out, crate::data::stance_key(from, to)),
    }
    let mut line = format!(
        "{from_name} now stands {} toward {to_name}.",
        stance.label()
    );
    if stance == Stance::Allied && !reciprocated {
        line.push_str(&format!(
            " Unreciprocated: it counts in a field once {to_name} declares the same."
        ));
    }
    line.push_str(" A field already running is unchanged; the next one reads it.");
    ctx.out.sys_to_actor(actor, "faction", line);
    let notice = format!(
        "{from_name} has declared itself {} toward {to_name} ({}).",
        stance.label(),
        display_name(ctx.w, actor)
    );
    for officer in officers_of(ctx.w, to) {
        if officer != actor {
            ctx.out.sys_to_actor(officer, "faction", notice.clone());
        }
    }
    Ok(())
}

/// **The stance rows `viewer` may see for `side`** (Q-40-11): its own declarations, and the
/// others' toward it, each naming a side the viewer may know; a one-sided Allied reads
/// "unreciprocated".
pub(crate) fn stance_lines(w: &World, viewer: ObjId, staff: bool, side: StanceSide) -> Vec<String> {
    let Some(st) = factions(w) else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    for row in st.stances_naming(side) {
        let mine = row.from == side;
        let other = if mine { row.to } else { row.from };
        if !side_listed(w, viewer, staff, other) {
            continue;
        }
        let mutual = st.stance(row.to, row.from) == row.stance;
        if !mine && mutual {
            // Shown once, from this side's own row.
            continue;
        }
        let suffix = match (row.stance, mutual) {
            (Stance::Allied, false) => " (unreciprocated)",
            (_, true) => " (mutual)",
            _ => "",
        };
        let other = side_name(w, other);
        lines.push(if mine {
            format!("  Stance toward {other}: {}{suffix}.", row.stance.label())
        } else {
            format!("  {other} stands {} toward it{suffix}.", row.stance.label())
        });
    }
    lines
}

/// **`+faction <name>` for an organization** (doc 40 cut FR.1): its name and its stance rows,
/// when `name` is no faction but an organization the viewer may know.
pub(crate) fn org_info(ctx: &mut Ctx<'_>, name: &str) -> Option<()> {
    let actor = ctx.actor;
    let staff = crate::commands::is_admin(ctx.w, actor);
    let side = resolve_side(ctx.w, actor, staff, name)?;
    let StanceSide::Org(_) = side else {
        return None;
    };
    let mut lines = vec![format!("{} (an organization)", side_name(ctx.w, side))];
    lines.extend(stance_lines(ctx.w, actor, staff, side));
    for line in lines {
        ctx.out.sys_to_actor(actor, "faction", line);
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stance_word_parses_and_a_bad_one_does_not() {
        assert_eq!(Stance::parse("Hostile"), Some(Stance::Hostile));
        assert_eq!(Stance::parse(" allied "), Some(Stance::Allied));
        assert_eq!(Stance::parse("neutral"), Some(Stance::Neutral));
        assert_eq!(Stance::parse("rival"), None);
    }
}
