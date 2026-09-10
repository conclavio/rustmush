//! The staff side of hydration (S3): `@atlas/cap <char>=<n>`, `@atlas/strike
//! <site>=<reason>`, `@atlas/standing [<char>]`, and `@atlas/propose`, which hands
//! a brief or a shelf pack to the planner the worldgen module registers.

use mush_core::{CmdError, Ctx, ObjId, match_object};
use mush_proto::{OutputEvent, RichText};

use crate::hydrate::{self, ProposeRequest, ProposeScope, ProposeSource};
use crate::module::{AtlasState, NS, is_staff};
use crate::place::{PlaceRef, find_place};

fn say(ctx: &mut Ctx<'_>, lines: Vec<String>) {
    let text = lines.join("\n");
    ctx.out
        .to_actor(ctx.actor, OutputEvent::Sys(RichText::tag(NS, text)));
}

fn state<'a>(ctx: &'a Ctx<'_>) -> Result<&'a AtlasState, CmdError> {
    ctx.state::<AtlasState>()
        .ok_or_else(|| CmdError::msg("The atlas is not composed on this server."))
}

/// A character by name, or a dbref.
fn character(ctx: &Ctx<'_>, raw: &str) -> Result<ObjId, CmdError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(CmdError::msg("Which character?"));
    }
    if let Some(who) = ctx.w.resolve_player(raw) {
        return Ok(who);
    }
    match_object(ctx.w, ctx.actor, raw)
        .ok()
        .filter(|o| {
            ctx.w
                .get(*o)
                .is_some_and(|o| o.kind == mush_core::ObjKind::Player)
        })
        .ok_or_else(|| CmdError::msg(format!("No character called {raw:?}.")))
}

/// `@atlas/cap <char>=<n>`.
pub(crate) fn cap_admin(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !is_staff(ctx.w, ctx.actor) {
        return Err(CmdError::msg("Permission denied."));
    }
    let raw = ctx.args.raw().trim().to_string();
    let (lhs, rhs) = raw
        .split_once('=')
        .ok_or_else(|| CmdError::msg("Usage: @atlas/cap <character>=<n>"))?;
    let who = character(ctx, lhs)?;
    let cap: u32 = rhs
        .trim()
        .parse()
        .map_err(|_| CmdError::msg("The cap is a whole number of outstanding requests."))?;
    if cap > 64 {
        return Err(CmdError::msg(
            "A cap of 64 is the most the atlas keeps track of.",
        ));
    }
    let rec = hydrate::set_cap(ctx, who, cap).map_err(CmdError::msg)?;
    let name = mush_core::output::display_name(ctx.w, who);
    say(
        ctx,
        vec![format!(
            "{name} may hold {cap} outstanding request{}; strikes cleared (revision {}).",
            if cap == 1 { "" } else { "s" },
            rec.revision
        )],
    );
    if ctx.w.get(who).is_some() && who != ctx.actor {
        ctx.out.info(
            who,
            format!(
                "Staff set your building cap to {cap} outstanding request{}.",
                if cap == 1 { "" } else { "s" }
            ),
        );
    }
    Ok(())
}

/// `@atlas/strike <site>=<reason>`.
pub(crate) fn strike_admin(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !is_staff(ctx.w, ctx.actor) {
        return Err(CmdError::msg("Permission denied."));
    }
    let raw = ctx.args.raw().trim().to_string();
    let (lhs, rhs) = raw
        .split_once('=')
        .ok_or_else(|| CmdError::msg("Usage: @atlas/strike <site>=<reason>"))?;
    let site = {
        let st = state(ctx)?;
        match find_place(&st.records, lhs.trim()) {
            Some(PlaceRef::Site(key)) => key,
            Some(_) => {
                return Err(CmdError::msg(
                    "A strike names a site (by name, or <pack>:<entry>).",
                ));
            }
            None => return Err(CmdError::msg(format!("No site called {:?}.", lhs.trim()))),
        }
    };
    let report = hydrate::strike(ctx, &site, rhs).map_err(CmdError::msg)?;
    let name = mush_core::output::display_name(ctx.w, report.character);
    say(
        ctx,
        vec![format!(
            "Struck: {} retired; {name} has {} strike{} and a cap of {}.",
            report.pack,
            report.strikes,
            if report.strikes == 1 { "" } else { "s" },
            report.cap
        )],
    );
    Ok(())
}

/// `@atlas/standing [<char>]`: one character's standing, or every request.
pub(crate) fn standing_admin(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !is_staff(ctx.w, ctx.actor) {
        return Err(CmdError::msg("Permission denied."));
    }
    let raw = ctx.args.raw().trim().to_string();
    let lines = if raw.is_empty() {
        let st = state(ctx)?;
        let all = hydrate::requests(st);
        let mut lines = vec![format!(
            "{} request{} on the book:",
            all.len(),
            if all.len() == 1 { "" } else { "s" }
        )];
        for r in all.iter().take(40) {
            lines.push(format!(
                "  {} — {} for {} in {}: {}",
                r.id,
                r.kind,
                mush_core::output::display_name(ctx.w, r.character),
                st.records
                    .neighborhoods
                    .get(&r.neighborhood)
                    .map_or_else(|| r.neighborhood.to_string(), |n| n.name.clone()),
                r.status.label()
            ));
        }
        lines
    } else {
        let who = character(ctx, &raw)?;
        let st = state(ctx)?;
        hydrate::standing_lines(st, ctx.w, who)
    };
    say(ctx, lines);
    Ok(())
}

/// `@atlas/propose <prose>` · `@atlas/propose <place>=<prose>` · `@atlas/propose
/// --from <pack>`: hand a brief or a shelf pack to the planner. The scope is the
/// place named, else the neighborhood the staffer stands in, else its region, else
/// the world.
pub(crate) fn propose_admin(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !is_staff(ctx.w, ctx.actor) {
        return Err(CmdError::msg("Permission denied."));
    }
    let raw = ctx.args.raw().trim().to_string();
    if raw.is_empty() {
        return Err(CmdError::msg(
            "Usage: @atlas/propose <prose>  |  @atlas/propose <place>=<prose>  |  @atlas/propose --from <pack>",
        ));
    }
    let actor = ctx.actor;
    let (scope, source) = if let Some(pack) = raw.strip_prefix("--from") {
        let pack = pack.trim();
        let id = mush_core::CanonicalId::new(pack)
            .map_err(|e| CmdError::msg(format!("{pack:?} is not a pack id: {e}")))?;
        let st = state(ctx)?;
        if !st.shelf.contains_key(&id) {
            return Err(CmdError::msg(format!("{id} is not on the shelf.")));
        }
        let scope = st
            .shelf
            .get(&id)
            .map(|p| match p.manifest().kind() {
                mush_content::PackKind::Region => ProposeScope::World,
                mush_content::PackKind::Neighborhood => st
                    .records
                    .neighborhoods
                    .values()
                    .find(|n| n.key.pack == id)
                    .map_or(ProposeScope::World, |n| {
                        ProposeScope::Region(n.region.clone())
                    }),
                _ => st
                    .records
                    .neighborhoods
                    .values()
                    .find(|n| p.manifest().requires().iter().any(|r| r.id == n.key.pack))
                    .map_or(ProposeScope::World, |n| {
                        ProposeScope::Neighborhood(n.key.clone())
                    }),
            })
            .unwrap_or(ProposeScope::World);
        (scope, ProposeSource::FromPack(id))
    } else {
        let (place, prose) = match raw.split_once('=') {
            Some((place, prose)) if !place.trim().is_empty() => (Some(place.trim()), prose.trim()),
            Some((_, prose)) => (None, prose.trim()),
            None => (None, raw.as_str()),
        };
        if prose.is_empty() {
            return Err(CmdError::msg("Say what to build, in a sentence or a page."));
        }
        let st = state(ctx)?;
        let scope = match place {
            Some(place) => match find_place(&st.records, place) {
                Some(PlaceRef::Neighborhood(k)) => ProposeScope::Neighborhood(k),
                Some(PlaceRef::Region(k)) => ProposeScope::Region(k),
                Some(PlaceRef::World) => ProposeScope::World,
                Some(PlaceRef::Locality(k)) => st
                    .records
                    .localities
                    .get(&k)
                    .map_or(ProposeScope::World, |l| {
                        ProposeScope::Region(l.region.clone())
                    }),
                Some(PlaceRef::Site(k)) => st
                    .records
                    .sites
                    .get(&k)
                    .and_then(|s| s.neighborhoods.first().cloned())
                    .map_or(ProposeScope::World, ProposeScope::Neighborhood),
                None => return Err(CmdError::msg(format!("No place called {place:?}."))),
            },
            None => {
                let here = ctx.w.get(actor).and_then(|o| o.location);
                match here.and_then(|room| st.records.neighborhood_of_room(room)) {
                    Some(nb) => ProposeScope::Neighborhood(nb.key.clone()),
                    None => ProposeScope::World,
                }
            }
        };
        (scope, ProposeSource::Prose(prose.to_string()))
    };
    let line = hydrate::propose(
        ctx,
        ProposeRequest {
            author: actor,
            scope,
            source,
        },
    )
    .map_err(CmdError::msg)?;
    say(ctx, vec![line]);
    Ok(())
}
