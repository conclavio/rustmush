//! `@atlas/theme` (S2b, plan §3.3), replacing `mush-worldgen`'s `@theme`:
//!
//! ```text
//! @atlas/theme <place>=<field>:<value>   set one theme field on a place, or `world`
//! @atlas/theme/show [<place>]            the effective theme at a place, or here
//! @atlas/theme/exemplar <place>=<room>   copy a room's accepted desc in as an exemplar
//! ```
//!
//! A place is a region, locality, neighborhood or site by name or id, or `world`
//! (`#0`). The theme rides the place's own record (family `atlas.records`); the world
//! theme is its own document. Staff only, like the rest of `@atlas`.

use mush_core::output::display_name;
use mush_core::{CmdError, Ctx, ObjId, match_object};
use mush_proto::{OutputEvent, RichText};
use smol_str::SmolStr;

use crate::module::{AtlasState, NS, del_doc, is_staff, put_doc};
use crate::place::{PlaceRef, find_place};
use crate::record::{KIND_LOCALITY, KIND_NEIGHBORHOOD, KIND_REGION, KIND_SITE, record_key};
use crate::theme::{
    Theme, WORLD_THEME_KEY, WorldThemeDoc, render_theme_lines, resolve_place_theme, resolve_theme,
    theme_on, world_theme,
};

fn say(ctx: &mut Ctx<'_>, lines: Vec<String>) {
    let text = lines.join("\n");
    ctx.out
        .to_actor(ctx.actor, OutputEvent::Sys(RichText::tag(NS, text)));
}

fn place_arg(ctx: &Ctx<'_>, raw: &str) -> Result<PlaceRef, CmdError> {
    let st = ctx
        .state::<AtlasState>()
        .ok_or_else(|| CmdError::msg("The atlas is not composed on this server."))?;
    find_place(&st.records, raw).ok_or_else(|| {
        CmdError::msg(format!(
            "No place called {raw:?}. Name a region, locality, neighborhood or site, or `world`."
        ))
    })
}

/// Write `theme` onto `place`: the world document, or the place's own record with
/// its revision bumped.
fn store(ctx: &mut Ctx<'_>, place: &PlaceRef, theme: Theme) -> Result<(), CmdError> {
    theme.validate().map_err(CmdError::msg)?;
    let theme = if theme.is_empty() { None } else { Some(theme) };
    let Some(st) = ctx.state_mut::<AtlasState>() else {
        return Err(CmdError::msg("The atlas is not composed on this server."));
    };
    let (key, value): (SmolStr, Option<serde_json::Value>) = match place {
        PlaceRef::World => {
            st.world_theme.clone_from(&theme);
            (
                SmolStr::new_static(WORLD_THEME_KEY),
                theme.map(|t| serde_json::to_value(WorldThemeDoc { theme: t }).unwrap_or_default()),
            )
        }
        PlaceRef::Region(k) => {
            let r = st
                .records
                .regions
                .get_mut(k)
                .ok_or_else(|| CmdError::msg("That region is gone."))?;
            r.theme = theme;
            r.revision = r.revision.saturating_add(1);
            (
                record_key(KIND_REGION, k),
                Some(serde_json::to_value(&*r).unwrap_or_default()),
            )
        }
        PlaceRef::Locality(k) => {
            let r = st
                .records
                .localities
                .get_mut(k)
                .ok_or_else(|| CmdError::msg("That locality is gone."))?;
            r.theme = theme;
            r.revision = r.revision.saturating_add(1);
            (
                record_key(KIND_LOCALITY, k),
                Some(serde_json::to_value(&*r).unwrap_or_default()),
            )
        }
        PlaceRef::Neighborhood(k) => {
            let r = st
                .records
                .neighborhoods
                .get_mut(k)
                .ok_or_else(|| CmdError::msg("That neighborhood is gone."))?;
            r.theme = theme;
            r.revision = r.revision.saturating_add(1);
            (
                record_key(KIND_NEIGHBORHOOD, k),
                Some(serde_json::to_value(&*r).unwrap_or_default()),
            )
        }
        PlaceRef::Site(k) => {
            let r = st
                .records
                .sites
                .get_mut(k)
                .ok_or_else(|| CmdError::msg("That site is gone."))?;
            r.theme = theme;
            r.revision = r.revision.saturating_add(1);
            (
                record_key(KIND_SITE, k),
                Some(serde_json::to_value(&*r).unwrap_or_default()),
            )
        }
    };
    match value {
        Some(value) => put_doc(ctx, key, &value),
        None => del_doc(ctx, key),
    }
    Ok(())
}

fn current(ctx: &Ctx<'_>, place: &PlaceRef) -> Theme {
    let world = world_theme(ctx.w);
    ctx.state::<AtlasState>()
        .and_then(|st| theme_on(&st.records, world.as_ref(), place))
        .unwrap_or_default()
}

fn place_name(ctx: &Ctx<'_>, place: &PlaceRef) -> String {
    ctx.state::<AtlasState>()
        .map(|st| place.name(&st.records))
        .unwrap_or_else(|| "the world".to_string())
}

/// The `@atlas/theme` family; `raw` is everything after the switches.
pub(crate) fn theme_admin(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    if !is_staff(ctx.w, actor) {
        return Err(CmdError::msg("Permission denied."));
    }
    let raw = ctx.args.raw().trim().to_string();
    if ctx.switches.has("show") {
        return theme_show(ctx, &raw);
    }
    if ctx.switches.has("exemplar") {
        return theme_exemplar(ctx, &raw);
    }
    theme_set(ctx, &raw)
}

fn theme_set(ctx: &mut Ctx<'_>, raw: &str) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @atlas/theme <place>=<field>:<value>  (place: a region, \
                         locality, neighborhood or site, or `world`).";
    let (lhs, rhs) = raw.split_once('=').ok_or_else(|| CmdError::msg(USAGE))?;
    let (field, value) = rhs.split_once(':').ok_or_else(|| CmdError::msg(USAGE))?;
    let place = place_arg(ctx, lhs)?;
    let mut theme = current(ctx, &place);
    let canonical = theme.set_field(field, value).map_err(CmdError::msg)?;
    store(ctx, &place, theme)?;
    let name = place_name(ctx, &place);
    say(ctx, vec![format!("Theme {canonical} set on {name}.")]);
    Ok(())
}

fn theme_show(ctx: &mut Ctx<'_>, raw: &str) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let (title, theme) = if raw.is_empty() {
        let room = ctx
            .w
            .get(actor)
            .and_then(|o| o.location)
            .ok_or_else(|| CmdError::msg("You are nowhere."))?;
        (
            format!("Effective theme for {}:", display_name(ctx.w, room)),
            resolve_theme(ctx.w, room),
        )
    } else if let Ok(place) = place_arg(ctx, raw) {
        (
            format!("Effective theme at {}:", place_name(ctx, &place)),
            resolve_place_theme(ctx.w, &place),
        )
    } else {
        let obj = match_object(ctx.w, actor, raw).map_err(|e| CmdError::msg(e.to_string()))?;
        (
            format!("Effective theme for {}:", display_name(ctx.w, obj)),
            resolve_theme(ctx.w, obj),
        )
    };
    let mut lines = vec![title];
    lines.extend(render_theme_lines(&theme));
    say(ctx, lines);
    Ok(())
}

fn theme_exemplar(ctx: &mut Ctx<'_>, raw: &str) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @atlas/theme/exemplar <place>=<room>.";
    let (lhs, rhs) = raw.split_once('=').ok_or_else(|| CmdError::msg(USAGE))?;
    let place = place_arg(ctx, lhs)?;
    let room: ObjId =
        match_object(ctx.w, ctx.actor, rhs.trim()).map_err(|e| CmdError::msg(e.to_string()))?;
    let desc = ctx
        .w
        .attr(room, "desc")
        .and_then(|a| a.as_text())
        .map(str::to_string)
        .filter(|d| !d.trim().is_empty())
        .ok_or_else(|| CmdError::msg("That room has no accepted description to copy."))?;
    let mut theme = current(ctx, &place);
    theme.exemplars.push(desc);
    let count = theme.exemplars.len();
    store(ctx, &place, theme)?;
    let name = place_name(ctx, &place);
    say(
        ctx,
        vec![format!(
            "Exemplar copied from {} into {name}'s theme ({count} total).",
            display_name(ctx.w, room)
        )],
    );
    Ok(())
}
