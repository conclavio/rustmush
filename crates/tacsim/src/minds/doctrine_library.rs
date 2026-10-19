//! Public teaching pages over the actual stock preset constructors. This module
//! has no World, actor, library, source-store or commercial authority input.

use mush_core::CmdError;

use crate::arena::card::{PRESETS, preset};
use crate::config::ReflexConfig;

const MAX_SELECTOR_BYTES: usize = 32;
const MAX_PAGE_BYTES: usize = 4_096;
const SELECT: &str = "Use @mind/doctrine [Line|Sentry|Sniper|Skirmisher|Hunter|Rearguard].";

/// Values come from the active server's doctrine configuration. The page is an
/// armed example, not a reading of any actor's selected or sealed hull.
pub(super) fn page(selector: &str, cfg: &ReflexConfig) -> Result<String, CmdError> {
    if selector.len() > MAX_SELECTOR_BYTES
        || selector
            .bytes()
            .any(|byte| !byte.is_ascii_alphabetic() && byte != b' ')
    {
        return Err(CmdError::msg(SELECT));
    }
    let selector = selector.trim();
    let page = if selector.is_empty() {
        let mut lines = vec![
            "Stickney teaching library: Standard Doctrine".to_owned(),
            "Six free public pages. Read one with @mind/doctrine <preset>.".to_owned(),
        ];
        for name in PRESETS {
            lines.push(format!(
                "  {name}: {}",
                lesson(name).ok_or_else(|| CmdError::msg(SELECT))?
            ));
        }
        lines.push("Each page reads this server's configured card. Read help doctrine for selection and inert fields.".to_owned());
        lines.join("\n")
    } else {
        let name = PRESETS
            .into_iter()
            .find(|name| name.eq_ignore_ascii_case(selector))
            .ok_or_else(|| CmdError::msg(SELECT))?;
        let card = preset(name, cfg).ok_or_else(|| CmdError::msg(SELECT))?;
        let mut lines = vec![
            format!("Stickney teaching library: Standard Doctrine / {name}"),
            lesson(name)
                .ok_or_else(|| CmdError::msg(SELECT))?
                .to_owned(),
            "Current configured card, armed example:".to_owned(),
        ];
        lines.extend(
            card.read_back(cfg, true)
                .into_iter()
                .map(|line| line.replace("[M2]", "[inert]")),
        );
        lines.extend([
            "[inert] fields are stored but Standard Doctrine does not fly them. Fire terms are also inert on an unarmed hull.".to_owned(),
            format!("To select in the lobby: SIM MIND Standard PROFILE {name}. MIND CARD reads your actual seal in the cockpit."),
            "This page only reads teaching content; it does not change your choice, draft or running sortie.".to_owned(),
        ]);
        lines.join("\n")
    };
    if page.len() > MAX_PAGE_BYTES {
        return Err(CmdError::msg(
            "Doctrine teaching page exceeds its output bound.",
        ));
    }
    Ok(page)
}

fn lesson(name: &str) -> Option<&'static str> {
    Some(match name {
        "Line" => {
            "The baseline: compare range, firing, retreat and search terms before changing a card."
        }
        "Sentry" => {
            "Raises the firing threshold, disables hunting and stays home after retreat. Its screen stance holds no arc."
        }
        "Sniper" => {
            "Uses the edge of weapon reach, a higher firing threshold and an ammunition reserve."
        }
        "Skirmisher" => {
            "Uses a closer range band and fires at flank faces. Its orbit label does not make it orbit."
        }
        "Hunter" => "Pairs its range band with a short hunting delay and permissive firing odds.",
        "Rearguard" => {
            "Sets retreat health and firing-odds thresholds, and stays home after retreat."
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
