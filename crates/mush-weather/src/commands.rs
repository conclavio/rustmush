//! The staff `@weather` command family (design 04 §10.1). All handlers are
//! Wizard/Royalty-gated (`Role::Royalty`) via the command spec, so dispatch
//! enforces the gate before the handler runs.
//!
//! Surface:
//!
//! - `@weather <zone>` — show a zone's current conditions (+ panel keyframe).
//! - `@weather <zone>=<state>` — pin a state (pauses the machine).
//! - `@weather/release <zone>` — unpin, resume the machine.
//! - `@weather/climate <zone>=<name>` — assign a climate and opt the zone in
//!   (`<name>` = `none`/`off` opts the zone out).
//! - `@weather/list` — list every weather cell.
//!
//! `<zone>` is `here` (the actor's room's zone) or any object match — staff
//! normally pass the zone object's dbref.

use std::sync::Arc;

use mush_core::output::display_name;
use mush_core::{
    ArgShape, CmdError, CommandHandler, CommandSpec, Ctx, ModError, ObjId, PanelFeedHandle,
    Registrar, Role, match_object,
};
use smol_str::SmolStr;

use crate::cell::{WeatherCell, WeatherState, delete_cell, persist_cell};
use crate::climate::ClimateSet;
use crate::machine::{pick_transition_line, roll_temp};
use crate::{muffle, panel_payload, scan_zone, weather_event};

/// Register the `@weather` command.
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    climates: Arc<ClimateSet>,
    panel: PanelFeedHandle,
) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: SmolStr::new_static("@weather"),
            switches: vec![
                SmolStr::new_static("climate"),
                SmolStr::new_static("release"),
                SmolStr::new_static("list"),
            ],
            shape: ArgShape::EqPair,
            min_role: Role::Royalty,
            help: HELP.to_string(),
            ..CommandSpec::default()
        },
        Arc::new(WeatherCmd { climates, panel }),
    )?;
    Ok(())
}

const HELP: &str = "\
@weather <zone>                 -- show a zone's current conditions
@weather <zone>=<state>         -- pin a weather state (pauses the machine)
@weather/release <zone>         -- unpin and resume the machine
@weather/climate <zone>=<name>  -- assign a climate + opt the zone in
@weather/climate <zone>=none    -- opt the zone out of weather
@weather/list                   -- list every weather cell

<zone> is 'here' (this room's zone) or an object match (usually a dbref).
Rooms opt in with the OUTDOORS or SHELTERED flags; INDOORS rooms stay silent.
Wizard/Royalty only.";

struct WeatherCmd {
    climates: Arc<ClimateSet>,
    panel: PanelFeedHandle,
}

impl CommandHandler for WeatherCmd {
    fn call(&self, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
        if ctx.switches.has("list") {
            return self.list(ctx);
        }
        let zone_arg = ctx.args.lhs().trim().to_string();
        if ctx.switches.has("release") {
            return self.release(ctx, &zone_arg);
        }
        if ctx.switches.has("climate") {
            let name = ctx.args.rhs().unwrap_or("").trim().to_string();
            return self.assign(ctx, &zone_arg, &name);
        }
        match ctx.args.rhs().map(|s| s.trim().to_string()) {
            Some(state) => self.pin(ctx, &zone_arg, &state),
            None => self.show(ctx, &zone_arg),
        }
    }
}

impl WeatherCmd {
    /// Resolve a `<zone>` argument to a zone object id.
    fn resolve_zone(&self, ctx: &Ctx<'_>, arg: &str) -> Result<ObjId, CmdError> {
        let arg = arg.trim();
        if arg.is_empty() {
            return Err(CmdError::msg(
                "Usage: @weather <zone>[=<state>]  (see 'help @weather').",
            ));
        }
        if arg.eq_ignore_ascii_case("here") {
            let room = ctx
                .w
                .get(ctx.actor)
                .and_then(|o| o.location)
                .ok_or_else(|| CmdError::msg("You are nowhere."))?;
            return ctx
                .w
                .get(room)
                .and_then(|o| o.zone)
                .ok_or_else(|| CmdError::msg("This room is not in a zone."));
        }
        Ok(match_object(ctx.w, ctx.actor, arg)?)
    }

    /// `@weather <zone>` — show current conditions and push a panel keyframe.
    fn show(&self, ctx: &mut Ctx<'_>, arg: &str) -> Result<(), CmdError> {
        let zone = self.resolve_zone(ctx, arg)?;
        let cell = ctx
            .w
            .module_state::<WeatherState>()
            .and_then(|s| s.cell(zone).cloned())
            .ok_or_else(|| {
                CmdError::msg("That zone has no weather cell.  (See @weather/climate.)")
            })?;
        let zone_name = display_name(ctx.w, zone);
        let pinned = if cell.pinned { "  [pinned]" } else { "" };
        ctx.out.info(
            ctx.actor,
            format!(
                "Weather for {zone_name} (#{}): {}, {}\u{b0}C, climate {}{pinned}.",
                zone.0, cell.state, cell.temp, cell.climate
            ),
        );
        if let Some(def) = self
            .climates
            .get(&cell.climate)
            .and_then(|c| c.state(&cell.state))
        {
            ctx.out.info(
                ctx.actor,
                format!(
                    "  visibility {}%, precip {}%, wind {}.",
                    def.visibility, def.precip, def.wind
                ),
            );
        }
        self.panel
            .keyframe(ctx, panel_payload(&zone_name, &cell.state, cell.temp));
        Ok(())
    }

    /// `@weather <zone>=<state>` — pin a state and broadcast the change.
    fn pin(&self, ctx: &mut Ctx<'_>, arg: &str, state: &str) -> Result<(), CmdError> {
        if state.is_empty() {
            return Err(CmdError::msg(
                "Pin to which state?  @weather <zone>=<state>",
            ));
        }
        let zone = self.resolve_zone(ctx, arg)?;
        let cell = ctx
            .w
            .module_state::<WeatherState>()
            .and_then(|s| s.cell(zone).cloned())
            .ok_or_else(|| {
                CmdError::msg("Assign a climate first: @weather/climate <zone>=<name>.")
            })?;
        let climate = self
            .climates
            .get(&cell.climate)
            .ok_or_else(|| CmdError::msg("That zone's climate is not loaded."))?;
        if climate.state(state).is_none() {
            let known: Vec<&str> = climate.states.iter().map(|s| s.id.as_str()).collect();
            return Err(CmdError::msg(format!(
                "Unknown state {state:?} for climate {}. Known: {}.",
                cell.climate,
                known.join(", ")
            )));
        }
        let season = ctx.season();
        let now = ctx.game_now_unix();
        let to = SmolStr::new(state);

        // Update the cell (roll a temperature for the new state) and pick the
        // broadcast line while the state slot is borrowed.
        let (from, temp, line, updated) = {
            let st = ctx
                .w
                .module_state_mut::<WeatherState>()
                .expect("weather state slot installed at registration");
            let cell = st
                .cells
                .get_mut(&zone)
                .expect("cell exists (checked above)");
            let from = cell.state.clone();
            let temp = climate
                .state(&to)
                .map(|def| roll_temp(def, season, &mut st.rng))
                .unwrap_or(cell.temp);
            cell.state = to.clone();
            cell.temp = temp;
            cell.pinned = true;
            cell.since = now;
            let line = pick_transition_line(climate, &from, &to, &mut st.rng);
            (from, temp, line, cell.clone())
        };

        ctx.persist(persist_cell(&updated));
        self.broadcast(ctx, zone, &line, &to, temp);
        ctx.out.info(
            ctx.actor,
            format!(
                "Pinned {} to {to} (was {from}).  /release to resume the machine.",
                display_name(ctx.w, zone)
            ),
        );
        Ok(())
    }

    /// `@weather/release <zone>` — unpin, resuming the machine.
    fn release(&self, ctx: &mut Ctx<'_>, arg: &str) -> Result<(), CmdError> {
        let zone = self.resolve_zone(ctx, arg)?;
        let updated = {
            let st = ctx
                .w
                .module_state_mut::<WeatherState>()
                .expect("weather state slot installed at registration");
            let cell = st
                .cells
                .get_mut(&zone)
                .ok_or_else(|| CmdError::msg("That zone has no weather cell."))?;
            if !cell.pinned {
                return Err(CmdError::msg("That zone is not pinned."));
            }
            cell.pinned = false;
            cell.clone()
        };
        ctx.persist(persist_cell(&updated));
        ctx.out.info(
            ctx.actor,
            format!(
                "Released {}; the weather machine resumes.",
                display_name(ctx.w, zone)
            ),
        );
        Ok(())
    }

    /// `@weather/climate <zone>=<name>` — assign a climate (opt in), or
    /// `=none`/`=off` to opt out.
    fn assign(&self, ctx: &mut Ctx<'_>, arg: &str, name: &str) -> Result<(), CmdError> {
        let zone = self.resolve_zone(ctx, arg)?;
        // Opt out.
        if name.is_empty() || name.eq_ignore_ascii_case("none") || name.eq_ignore_ascii_case("off")
        {
            let removed = ctx
                .w
                .module_state_mut::<WeatherState>()
                .is_some_and(|s| s.cells.remove(&zone).is_some());
            if !removed {
                return Err(CmdError::msg("That zone has no weather cell."));
            }
            ctx.persist(delete_cell(zone));
            ctx.out.info(
                ctx.actor,
                format!("{} opted out of weather.", display_name(ctx.w, zone)),
            );
            return Ok(());
        }
        if !self.climates.contains(name) {
            return Err(CmdError::msg(format!(
                "Unknown climate {name:?}. Known: {}.",
                self.climates
                    .ids()
                    .iter()
                    .map(SmolStr::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        let climate = self.climates.get(name).expect("climate exists (checked)");
        let start = climate.default_state();
        let season = ctx.season();
        let now = ctx.game_now_unix();

        let (temp, line, updated) = {
            let st = ctx
                .w
                .module_state_mut::<WeatherState>()
                .expect("weather state slot installed at registration");
            let temp = climate
                .state(&start)
                .map(|def| roll_temp(def, season, &mut st.rng))
                .unwrap_or(0);
            let mut cell = WeatherCell::new(zone, name, start.clone());
            cell.temp = temp;
            cell.since = now;
            let line = pick_transition_line(climate, &start, &start, &mut st.rng);
            st.cells.insert(zone, cell.clone());
            (temp, line, cell)
        };

        ctx.persist(persist_cell(&updated));
        self.broadcast(ctx, zone, &line, &start, temp);
        ctx.out.info(
            ctx.actor,
            format!(
                "{} now uses climate {name}; weather is {start} ({temp}\u{b0}C).",
                display_name(ctx.w, zone)
            ),
        );
        Ok(())
    }

    /// `@weather/list` — one line per cell.
    fn list(&self, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
        let cells: Vec<WeatherCell> = ctx
            .w
            .module_state::<WeatherState>()
            .map(|s| s.cells.values().cloned().collect())
            .unwrap_or_default();
        if cells.is_empty() {
            ctx.out.info(ctx.actor, "No weather cells.");
            return Ok(());
        }
        ctx.out
            .info(ctx.actor, format!("Weather cells ({}):", cells.len()));
        for cell in cells {
            let name = display_name(ctx.w, cell.zone);
            let pinned = if cell.pinned { " [pinned]" } else { "" };
            ctx.out.info(
                ctx.actor,
                format!(
                    "  {name} (#{}): {}, {}\u{b0}C, {}{pinned}",
                    cell.zone.0, cell.state, cell.temp, cell.climate
                ),
            );
        }
        Ok(())
    }

    /// Broadcast a state line to a zone's OUTDOORS/SHELTERED rooms (muffled
    /// for sheltered) and a matching panel delta.
    fn broadcast(&self, ctx: &mut Ctx<'_>, zone: ObjId, line: &str, icon: &SmolStr, temp: i16) {
        let scan = scan_zone(ctx.w, zone);
        for room in scan.rooms {
            let text = if room.sheltered {
                muffle(line)
            } else {
                line.to_string()
            };
            ctx.out.to_room(
                room.room,
                Vec::new(),
                weather_event(&scan.zone_name, text, icon.clone()),
            );
            self.panel.delta_to(
                ctx,
                mush_core::Audience::Room {
                    room: room.room,
                    exclude: Vec::new(),
                },
                panel_payload(&scan.zone_name, icon, temp),
            );
        }
    }
}
