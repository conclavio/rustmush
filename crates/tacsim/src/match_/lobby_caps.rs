//! Class admission over the complete prospective lobby, without publication.
//! Selection is volatile with the rest of the lobby. A running centre's selected
//! caps ride its existing durable live record and are restored with that record.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Hull {
    pub team: Team,
    pub index: u8,
    pub class: CoreClass,
}

pub(super) fn fill_cap(w: &World, centre: &Centre, team: Team) -> CoreClass {
    minds_config(w).unwrap_or_default().fill_cap(
        centre
            .fill_classes
            .iter()
            .find(|row| row.team == team)
            .map(|row| row.class),
    )
}

/// The pilot/default distinction is ownership of the claimed slot, not whether
/// that pilot happens to be seated at this moment.
pub(super) fn slot_choice(w: &World, centre: &Centre, slot: &Slot) -> Result<SealChoice, SimError> {
    let Some(choice) = &slot.mind else {
        return Ok(SealChoice::unchosen());
    };
    let held = choice.resolve(w, slot.pilot).ok_or(SimError::MindNotHeld {
        team: slot.team.as_str(),
        index: Some(slot.index),
    })?;
    let doctrine_only = centre.doctrine_only
        || (centre.state == MatchState::Selection
            && Team::ALL.iter().all(|team| centre.side_len(*team) > 0));
    Ok(if doctrine_only && !held.is_doctrine() {
        SealChoice {
            held: HeldMind::doctrine(),
            profiles: Vec::new(),
            chose: true,
            substituted: true,
        }
    } else {
        SealChoice {
            held,
            profiles: choice.profiles.clone(),
            chose: true,
            substituted: false,
        }
    })
}

pub(super) fn preview(w: &World, centre: &Centre) -> Result<Vec<Hull>, SimError> {
    preview_visible(w, centre, None, None)
}

pub(super) fn preview_visible(
    w: &World,
    centre: &Centre,
    visible: Option<Team>,
    viewer: Option<ObjId>,
) -> Result<Vec<Hull>, SimError> {
    let cfg = minds_config(w).unwrap_or_default();
    let (_, match_cfg, _) = configs(w).ok_or(SimError::Unavailable)?;
    let mut rows = Vec::new();
    let range = range_sides(centre);
    let doctrine_only = centre.doctrine_only
        || (centre.state == MatchState::Selection
            && Team::ALL.iter().all(|team| centre.side_len(*team) > 0));
    for team in Team::ALL {
        let privileged = viewer.is_none_or(|viewer| is_staff(w, viewer));
        let mut disclose = privileged;
        let result = (|| -> Result<(), SimError> {
            for slot in centre.slots.iter().filter(|slot| slot.team == team) {
                disclose = privileged || viewer == Some(slot.pilot);
                let choice = slot_choice(w, centre, slot)?;
                let chassis = effective_type(w, slot).ok_or(SimError::NoUnit {
                    team: team.as_str(),
                    index: slot.index,
                })?;
                if let Some(requested) = &slot.mind {
                    let held = requested
                        .resolve(w, slot.pilot)
                        .ok_or(SimError::MindNotHeld {
                            team: team.as_str(),
                            index: Some(slot.index),
                        })?;
                    check_rating(w, &chassis, &held)?;
                }
                rows.push(Hull {
                    team,
                    index: slot.index,
                    class: preview_class(w, &chassis, &choice, cfg.max_class)?,
                });
            }
            disclose = privileged || centre.fill_minds.iter().find(|fill| fill.team == team).is_none_or(|fill| {
                matches!(fill.by, FillSetter::Lead(account) if viewer.and_then(|viewer| account_of(w, viewer).ok()) == Some(account))
            });
            let pods = if range.is_some_and(|(_, targets)| targets == team) {
                (1..=centre.range_targets.unwrap_or(0))
                    .map(|index| {
                        centre::tank_for_slot(w, team, index)
                            .map(|tank| (index, tank))
                            .ok_or(SimError::NoUnit {
                                team: team.as_str(),
                                index,
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?
            } else if match_cfg.ai_fill && range.is_none() {
                let fielded = centre
                    .slots
                    .iter()
                    .filter(|slot| slot.team == team)
                    .filter_map(|slot| centre::tank_for_slot(w, team, slot.index))
                    .collect::<Vec<_>>();
                fill_pods(w, centre, &match_cfg, team, &fielded)
            } else {
                Vec::new()
            };
            let choice = if !pods.is_empty() && range.is_none() {
                centre
                    .fill_minds
                    .iter()
                    .find(|fill| fill.team == team)
                    .map(|fill| {
                        fill_held(w, centre, fill)
                            .map(|held| fill_seal_choice(&held, fill, doctrine_only))
                            .ok_or(SimError::MindNotHeld {
                                team: team.as_str(),
                                index: None,
                            })
                    })
                    .transpose()?
                    .unwrap_or_else(SealChoice::unchosen)
            } else {
                SealChoice::unchosen()
            };
            let cap = fill_cap(w, centre, team);
            for (index, _) in pods {
                let target = range.is_some_and(|(_, targets)| targets == team);
                let chassis = if target {
                    SmolStr::new_static(crate::definitions::content::TARGET_MK1)
                } else {
                    authored_unit_type(w, team, index).ok_or(SimError::NoUnit {
                        team: team.as_str(),
                        index,
                    })?
                };
                if !target
                    && let Some(fill) = centre.fill_minds.iter().find(|fill| fill.team == team)
                    && let Some(held) = fill_held(w, centre, fill)
                {
                    check_rating(w, &chassis, &held)?;
                }
                rows.push(Hull {
                    team,
                    index,
                    class: preview_class(w, &chassis, &choice, cap)?,
                });
            }
            Ok(())
        })();
        result.map_err(|error| {
            if !disclose {
                SimError::OtherHullCannotSeal
            } else if visible.is_some_and(|own| own != team) {
                SimError::OtherSideCannotSeal
            } else {
                error
            }
        })?;
    }
    rows.sort_by_key(|row| (row.team, row.index));
    Ok(rows)
}

fn check_rating(w: &World, chassis: &str, held: &HeldMind) -> Result<(), SimError> {
    if let Some((core, true)) = core_of_type(w, chassis)
        && held.rated > core
        && core > CoreClass::Zero
    {
        return Err(SimError::OverClass {
            mind: held.name().to_owned(),
            rated: held.rated,
            chassis: type_word(w, chassis),
            core,
        });
    }
    Ok(())
}

fn preview_class(
    w: &World,
    chassis: &str,
    choice: &SealChoice,
    cap: CoreClass,
) -> Result<CoreClass, SimError> {
    // An unchosen default can be corrected after claiming a slot on a server
    // whose floor is above Line. Its class is exact; checkout still validates
    // the final default card if the player never chooses a lawful profile.
    if choice.chose {
        fit_on_chassis(w, chassis, &choice.held, &choice.profiles, cap)
    } else {
        Ok(sealed_class(w, &choice.held, Some(chassis), cap))
    }
}

pub(super) fn admit(w: &World, centre: &Centre, actor: ObjId) -> Result<(), SimError> {
    let visible = centre.slot_of_pilot(actor).map(|slot| slot.team);
    let rows = preview_visible(w, centre, visible, Some(actor))?;
    admit_rows(w, &rows, visible)
}

fn admit_rows(w: &World, rows: &[Hull], visible: Option<Team>) -> Result<(), SimError> {
    let cfg = minds_config(w).unwrap_or_default();
    let pool = crate::match_::pool::pool_for(&cfg, rows.len());
    for team in Team::ALL {
        crate::match_::pool::admit_side(
            pool,
            rows.iter()
                .filter(|row| row.team == team)
                .map(|row| cfg.classes.ceiling_per_s(row.class)),
        )
        .map_err(|(_, refusal)| {
            if visible.is_some_and(|own| own != team) {
                SimError::OtherSideCannotSeal
            } else {
                SimError::Pool(refusal)
            }
        })?;
    }
    Ok(())
}

/// Preserve the existing recovery rule: a strictly lower load can repair an
/// already oversized side, provided neither the other side nor hull count grows.
pub(super) fn admit_change(
    w: &World,
    before: &Centre,
    after: &Centre,
    actor: ObjId,
) -> Result<(), SimError> {
    let visible = after
        .slot_of_pilot(actor)
        .or_else(|| before.slot_of_pilot(actor))
        .map(|slot| slot.team);
    let next = preview_visible(w, after, visible, Some(actor))?;
    if let Ok(previous) = preview(w, before) {
        let cfg = minds_config(w).unwrap_or_default();
        let load = |rows: &[Hull], team| {
            rows.iter()
                .filter(|row| row.team == team)
                .map(|row| cfg.classes.ceiling_per_s(row.class))
                .fold(0_u64, u64::saturating_add)
        };
        if next.len() <= previous.len()
            && Team::ALL
                .iter()
                .all(|team| load(&next, *team) <= load(&previous, *team))
            && Team::ALL
                .iter()
                .any(|team| load(&next, *team) < load(&previous, *team))
        {
            return Ok(());
        }
    }
    admit_rows(w, &next, visible)
}
