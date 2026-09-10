//! `@atlas/window` (plan §3.5): staff open, list and close windows by hand —
//! over the rooms they name (their own by default) or over an extent in metres.

use mush_core::{CmdError, Ctx, ObjId};
use mush_sim::{DomainId, WindowOpener};

use crate::geom::{PointCm, RectCm, cm};
use crate::module::{AtlasState, is_staff};
use crate::window::{OpenRequest, STREET_CELL_CM, open, summary};

const USAGE: &str = "Usage: @atlas/window  |  @atlas/window open [<room>[,<room>…] | <x0>,<y0>,<x1>,<y1>][=<cell cm>]  |  @atlas/window close <n>";

/// `@atlas/window …`
///
/// # Errors
///
/// A refusal to print.
pub fn window_admin(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    if !is_staff(ctx.w, actor) {
        return Err(CmdError::msg("Windows are staff's to open by hand."));
    }
    let raw = ctx.args.raw().trim().to_string();
    let (verb, rest) = match raw.split_once(char::is_whitespace) {
        Some((v, r)) => (v.trim().to_ascii_lowercase(), r.trim().to_string()),
        None => (raw.to_ascii_lowercase(), String::new()),
    };
    match verb.as_str() {
        "" | "list" => list(ctx),
        "open" => open_cmd(ctx, actor, &rest),
        "close" => close_cmd(ctx, &rest),
        _ => Err(CmdError::msg(USAGE)),
    }
}

fn list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lines: Vec<String> = {
        let st = ctx
            .state::<AtlasState>()
            .ok_or_else(|| CmdError::msg("The atlas module is not composed on this server."))?;
        st.windows
            .open
            .values()
            .map(|e| summary(ctx.w, e))
            .collect()
    };
    let actor = ctx.actor;
    if lines.is_empty() {
        ctx.out.info(actor, "No windows are open.");
    } else {
        ctx.out
            .info(actor, format!("Windows open ({}):", lines.len()));
        for line in lines {
            ctx.out.info(actor, format!("  {line}"));
        }
    }
    Ok(())
}

fn parse_extent(text: &str) -> Option<RectCm> {
    let nums: Option<Vec<f64>> = text
        .split(',')
        .map(|n| n.trim().parse::<f64>().ok())
        .collect();
    let nums = nums?;
    let [x0, y0, x1, y1] = nums.as_slice() else {
        return None;
    };
    let min = PointCm {
        x: cm(x0.min(*x1)).ok()?,
        y: cm(y0.min(*y1)).ok()?,
    };
    let max = PointCm {
        x: cm(x0.max(*x1)).ok()?,
        y: cm(y0.max(*y1)).ok()?,
    };
    (min.x < max.x && min.y < max.y).then_some(RectCm { min, max })
}

fn open_cmd(ctx: &mut Ctx<'_>, actor: ObjId, rest: &str) -> Result<(), CmdError> {
    let (what, cell) = match rest.split_once('=') {
        Some((w, c)) => (
            w.trim(),
            Some(c.trim().parse::<u16>().map_err(|_| {
                CmdError::msg("The cell size is centimetres: 100, 200, 500 or 1000.")
            })?),
        ),
        None => (rest.trim(), None),
    };
    let here = ctx
        .w
        .get(actor)
        .and_then(|o| o.location)
        .ok_or_else(|| CmdError::msg("You are nowhere."))?;
    let (anchors, bbox): (Vec<ObjId>, Option<RectCm>) = if what.is_empty() {
        (vec![here], None)
    } else if let Some(extent) = parse_extent(what) {
        // The rooms whose footprints touch the extent anchor it; the staffer's
        // own room first when it is one of them.
        let rooms: Vec<ObjId> = {
            let st = ctx
                .state::<AtlasState>()
                .ok_or_else(|| CmdError::msg("The atlas module is not composed on this server."))?;
            let mut rooms: Vec<ObjId> = st
                .records
                .roomgeo
                .values()
                .filter(|g| {
                    g.footprint.shape().is_ok_and(|s| {
                        let b = s.bbox();
                        b.min.x < extent.max.x
                            && extent.min.x < b.max.x
                            && b.min.y < extent.max.y
                            && extent.min.y < b.max.y
                    })
                })
                .map(|g| g.room)
                .collect();
            rooms.sort();
            if let Some(i) = rooms.iter().position(|r| *r == here) {
                rooms.swap(0, i);
            }
            rooms
        };
        if rooms.is_empty() {
            return Err(CmdError::msg("No room's footprint touches that extent."));
        }
        (rooms, Some(extent))
    } else {
        let mut rooms = Vec::new();
        for name in what.split(',') {
            let room = mush_core::matchit::match_object(ctx.w, actor, name.trim())
                .map_err(|_| CmdError::msg(format!("No such room: {}.", name.trim())))?;
            if !rooms.contains(&room) {
                rooms.push(room);
            }
        }
        (rooms, None)
    };
    let domain = open(
        ctx,
        OpenRequest {
            anchors,
            bbox,
            opened_by: WindowOpener::Staff { by: actor },
            cell_cm: cell.or(Some(STREET_CELL_CM)),
            participants: Vec::new(),
        },
    )
    .map_err(CmdError::msg)?;
    ctx.out.info(
        actor,
        format!("Window #{domain} opens; its map is being drawn. @atlas/window lists it."),
    );
    Ok(())
}

fn close_cmd(ctx: &mut Ctx<'_>, rest: &str) -> Result<(), CmdError> {
    let n: u64 = rest
        .trim()
        .trim_start_matches('#')
        .parse()
        .map_err(|_| CmdError::msg("Which window? Its number, from @atlas/window."))?;
    let released =
        crate::window::close(ctx, DomainId(n), "staff closed it").map_err(CmdError::msg)?;
    let actor = ctx.actor;
    ctx.out.info(
        actor,
        format!("Window #{n} closed; {} released.", released.len()),
    );
    Ok(())
}

/// `@atlas/feature <feature>=<state>`: the wreckage by hand (S5's fights write
/// it themselves). A feature is named `<pack>:<entry>`, or by its entry alone
/// when that is unambiguous.
///
/// # Errors
///
/// A refusal to print.
pub fn feature_admin(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    if !is_staff(ctx.w, actor) {
        return Err(CmdError::msg("Features are staff's to set by hand."));
    }
    let raw = ctx.args.raw().trim().to_string();
    let (what, state_text) = raw.split_once('=').ok_or_else(|| {
        CmdError::msg("Usage: @atlas/feature <feature>=<intact|damaged[:n]|destroyed>")
    })?;
    let new_state = crate::window::parse_feature_state(state_text)
        .ok_or_else(|| CmdError::msg("A state is intact, damaged[:n] or destroyed."))?;
    let key = {
        let st = ctx
            .state::<AtlasState>()
            .ok_or_else(|| CmdError::msg("The atlas module is not composed on this server."))?;
        let what = what.trim();
        let mut matches: Vec<&crate::record::RecordKey> = st
            .records
            .features
            .keys()
            .filter(|k| k.suffix() == what || k.entry.as_str() == what)
            .collect();
        matches.sort();
        match matches.as_slice() {
            [one] => (*one).clone(),
            [] => return Err(CmdError::msg(format!("No feature {what}."))),
            many => {
                return Err(CmdError::msg(format!(
                    "{} features match {what}: name one as <pack>:<entry>.",
                    many.len()
                )));
            }
        }
    };
    let windows = crate::window::set_feature_state(ctx, &key, new_state).map_err(CmdError::msg)?;
    ctx.out.info(
        actor,
        format!(
            "{key} is now {new_state:?}; {} window(s) redraw.",
            windows.len()
        ),
    );
    Ok(())
}
