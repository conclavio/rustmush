//! The staff `@weather` command family (design 04 §10.1). All handlers are
//! Wizard/Royalty-gated (`Role::Royalty`) via the command spec, so dispatch
//! enforces the gate before the handler runs.
//!
//! Surface:
//!
//! - `@weather <place>` — show a place's current conditions (+ panel keyframe).
//! - `@weather <place>=<state>` — pin a state (pauses the machine).
//! - `@weather/release <place>` — unpin, resume the machine.
//! - `@weather/climate <place>=<name>` — assign a climate and opt the place in
//!   (`<name>` = `none`/`off` opts the place out).
//! - `@weather/list` — list every weather cell.
//!
//! `<place>` (S2b, doc 14 D-14-28) is a neighborhood or a region — by name, by pack
//! id, `here` (this room's neighborhood) or a room (its neighborhood).

use std::sync::Arc;

use mush_core::module::AsyncCompletion;
use mush_core::{
    ArgShape, CanonicalId, CmdError, CommandHandler, CommandSpec, Ctx, Deferred, Effect, ModError,
    ObjId, ObjKind, PanelFeedHandle, Powers, Registrar, Role, World, match_object, perm,
};
use smol_str::SmolStr;

use crate::cell::{WeatherCell, WeatherState, delete_cell, persist_cell};
use crate::climate::{ClimateDef, ClimateSet, WxStateDef};
use crate::flavor::{
    FlavorBucket, FlavorGenOutcome, FlavorGenSnapshot, FlavorKey, FlavorLine,
    MAX_FLAVOR_LINES_PER_POOL, MAX_FLAVOR_VARIANTS_PER_BUCKET,
};
use crate::machine::{pick_transition_line, roll_temp};
use crate::{
    NS, WeatherConfig, muffle, panel_payload, place_name, place_ref, scan_place, weather_event,
};

/// Register the `@weather` command and the flavor-generation async completion.
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    climates: Arc<ClimateSet>,
    panel: PanelFeedHandle,
    config: WeatherConfig,
) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: SmolStr::new_static("@weather"),
            switches: vec![
                SmolStr::new_static("climate"),
                SmolStr::new_static("release"),
                SmolStr::new_static("list"),
                SmolStr::new_static("generate"),
                SmolStr::new_static("lines"),
                SmolStr::new_static("approve"),
                SmolStr::new_static("reject"),
            ],
            shape: ArgShape::EqPair,
            min_role: Role::Royalty,
            help: HELP.to_string(),
            ..CommandSpec::default()
        },
        Arc::new(WeatherCmd {
            climates: Arc::clone(&climates),
            panel,
            config,
        }),
    )?;
    // The M10 flavor-generation re-entry: an off-loop `@weather/generate` call
    // returns here to store its PENDING lines and notify the staffer.
    reg.async_completion(Arc::new(WeatherFlavorCompletion { climates }))?;
    Ok(())
}

const HELP: &str = "\
@weather <place>                 -- show a place's current conditions
@weather <place>=<state>         -- pin a weather state (pauses the machine)
@weather/release <place>         -- unpin and resume the machine
@weather/climate <place>=<name>  -- assign a climate + opt the place in
@weather/climate <place>=none    -- opt the place out of weather
@weather/list                    -- list every weather cell

Flavor top-up (LLM; needs the BUILDER_LLM power):
@weather/generate [<climate>] [<state>]  -- generate PENDING flavor variants
@weather/lines [<climate>] [<state>]     -- list hand-written + generated lines
@weather/lines/approve <id>|all          -- approve pending line(s) into play
@weather/lines/reject <id>|all           -- reject (delete) pending line(s)

<place> is a neighborhood or a region on the atlas: by name, by pack id
(neighborhood.ooc_area, region.limbo), 'here' (this room's neighborhood) or a
room. A room's weather is its neighborhood's cell, else its region's; a room
off the map bears none. Rooms opt in with the OUTDOORS or SHELTERED flags;
INDOORS rooms stay silent.
Generated lines are PENDING until approved; only APPROVED lines are ever shown
to players. Transitions stay 100% deterministic; only the flavor text is LLM.
Royalty+ (the flavor top-up also needs the BUILDER_LLM power).";

struct WeatherCmd {
    climates: Arc<ClimateSet>,
    panel: PanelFeedHandle,
    config: WeatherConfig,
}

impl CommandHandler for WeatherCmd {
    fn call(&self, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
        // M10 flavor top-up switches, checked first (they share the verb).
        if ctx.switches.has("generate") {
            return self.generate(ctx);
        }
        if ctx.switches.has("lines") {
            if ctx.switches.has("approve") {
                return self.lines_review(ctx, true);
            }
            if ctx.switches.has("reject") {
                return self.lines_review(ctx, false);
            }
            return self.lines_list(ctx);
        }
        if ctx.switches.has("list") {
            return self.list(ctx);
        }
        let place_arg = ctx.args.lhs().trim().to_string();
        if ctx.switches.has("release") {
            return self.release(ctx, &place_arg);
        }
        if ctx.switches.has("climate") {
            let name = ctx.args.rhs().unwrap_or("").trim().to_string();
            return self.assign(ctx, &place_arg, &name);
        }
        match ctx.args.rhs().map(|s| s.trim().to_string()) {
            Some(state) => self.pin(ctx, &place_arg, &state),
            None => self.show(ctx, &place_arg),
        }
    }
}

impl WeatherCmd {
    /// Resolve a `<place>` argument to an atlas place id and its name: `here` or a
    /// room (its neighborhood), a pack id, or a neighborhood's or region's name.
    fn resolve_place(&self, ctx: &Ctx<'_>, arg: &str) -> Result<(CanonicalId, String), CmdError> {
        let arg = arg.trim();
        if arg.is_empty() {
            return Err(CmdError::msg(
                "Usage: @weather <place>[=<state>]  (see 'help @weather').",
            ));
        }
        let records = mush_atlas::place::records(ctx.w)
            .ok_or_else(|| CmdError::msg("The atlas is not composed on this server."))?;
        let room = if arg.eq_ignore_ascii_case("here") {
            Some(
                ctx.w
                    .get(ctx.actor)
                    .and_then(|o| o.location)
                    .ok_or_else(|| CmdError::msg("You are nowhere."))?,
            )
        } else {
            match_object(ctx.w, ctx.actor, arg)
                .ok()
                .filter(|o| ctx.w.get(*o).is_some_and(|o| o.kind == ObjKind::Room))
        };
        if let Some(room) = room {
            let nb = mush_atlas::place::neighborhood_of_room(records, room)
                .and_then(|k| records.neighborhoods.get(&k))
                .ok_or_else(|| {
                    CmdError::msg(
                        "That room is on no map; weather keys on a neighborhood or a region.",
                    )
                })?;
            return Ok((nb.key.pack.clone(), nb.name.clone()));
        }
        match mush_atlas::place::find_place(records, arg) {
            Some(mush_atlas::PlaceRef::Neighborhood(k)) => records
                .neighborhoods
                .get(&k)
                .map(|n| (n.key.pack.clone(), n.name.clone()))
                .ok_or_else(|| CmdError::msg("No such place.")),
            Some(mush_atlas::PlaceRef::Region(k)) => records
                .regions
                .get(&k)
                .map(|r| (r.key.pack.clone(), r.name.clone()))
                .ok_or_else(|| CmdError::msg("No such place.")),
            Some(_) => Err(CmdError::msg(
                "Weather keys on a neighborhood or a region, not a locality or a site.",
            )),
            None => Err(CmdError::msg(format!(
                "No neighborhood or region called {arg:?}. Name one, or a room in it."
            ))),
        }
    }

    /// `@weather <place>` — show current conditions and push a panel keyframe.
    fn show(&self, ctx: &mut Ctx<'_>, arg: &str) -> Result<(), CmdError> {
        let (place, name) = self.resolve_place(ctx, arg)?;
        let cell = ctx
            .w
            .module_state::<WeatherState>()
            .and_then(|s| s.cell(&place).cloned())
            .ok_or_else(|| {
                CmdError::msg("That place has no weather cell.  (See @weather/climate.)")
            })?;
        let pinned = if cell.pinned { "  [pinned]" } else { "" };
        ctx.out.info(
            ctx.actor,
            format!(
                "Weather for {name} ({place}): {}, {}\u{b0}C, climate {}{pinned}.",
                cell.state, cell.temp, cell.climate
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
            .keyframe(ctx, panel_payload(&name, &cell.state, cell.temp));
        Ok(())
    }

    /// `@weather <place>=<state>` — pin a state and broadcast the change.
    fn pin(&self, ctx: &mut Ctx<'_>, arg: &str, state: &str) -> Result<(), CmdError> {
        if state.is_empty() {
            return Err(CmdError::msg(
                "Pin to which state?  @weather <place>=<state>",
            ));
        }
        let (place, name) = self.resolve_place(ctx, arg)?;
        let cell = ctx
            .w
            .module_state::<WeatherState>()
            .and_then(|s| s.cell(&place).cloned())
            .ok_or_else(|| {
                CmdError::msg("Assign a climate first: @weather/climate <place>=<name>.")
            })?;
        let climate = self
            .climates
            .get(&cell.climate)
            .ok_or_else(|| CmdError::msg("That place's climate is not loaded."))?;
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
                .get_mut(&place)
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
            let extra = st
                .flavor
                .approved_texts(&cell.climate, &to, FlavorBucket::Transition);
            let line = pick_transition_line(climate, &from, &to, &extra, &mut st.rng);
            (from, temp, line, cell.clone())
        };

        ctx.persist(persist_cell(&updated));
        self.broadcast(ctx, &place, &line, &to, temp);
        ctx.out.info(
            ctx.actor,
            format!("Pinned {name} to {to} (was {from}).  /release to resume the machine."),
        );
        Ok(())
    }

    /// `@weather/release <place>` — unpin, resuming the machine.
    fn release(&self, ctx: &mut Ctx<'_>, arg: &str) -> Result<(), CmdError> {
        let (place, name) = self.resolve_place(ctx, arg)?;
        let updated = {
            let st = ctx
                .w
                .module_state_mut::<WeatherState>()
                .expect("weather state slot installed at registration");
            let cell = st
                .cells
                .get_mut(&place)
                .ok_or_else(|| CmdError::msg("That place has no weather cell."))?;
            if !cell.pinned {
                return Err(CmdError::msg("That place is not pinned."));
            }
            cell.pinned = false;
            cell.clone()
        };
        ctx.persist(persist_cell(&updated));
        ctx.out.info(
            ctx.actor,
            format!("Released {name}; the weather machine resumes."),
        );
        Ok(())
    }

    /// `@weather/climate <place>=<name>` — assign a climate (opt in), or
    /// `=none`/`=off` to opt out.
    fn assign(&self, ctx: &mut Ctx<'_>, arg: &str, name: &str) -> Result<(), CmdError> {
        let (place, place_name_) = self.resolve_place(ctx, arg)?;
        // Opt out.
        if name.is_empty() || name.eq_ignore_ascii_case("none") || name.eq_ignore_ascii_case("off")
        {
            let removed = ctx
                .w
                .module_state_mut::<WeatherState>()
                .is_some_and(|s| s.cells.remove(&place).is_some());
            if !removed {
                return Err(CmdError::msg("That place has no weather cell."));
            }
            ctx.persist(delete_cell(&place));
            ctx.out
                .info(ctx.actor, format!("{place_name_} opted out of weather."));
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
            let mut cell = WeatherCell::new(place.clone(), name, start.clone());
            cell.temp = temp;
            cell.since = now;
            let extra = st
                .flavor
                .approved_texts(name, &start, FlavorBucket::Transition);
            let line = pick_transition_line(climate, &start, &start, &extra, &mut st.rng);
            st.cells.insert(place.clone(), cell.clone());
            (temp, line, cell)
        };

        ctx.persist(persist_cell(&updated));
        self.broadcast(ctx, &place, &line, &start, temp);
        ctx.out.info(
            ctx.actor,
            format!("{place_name_} now uses climate {name}; weather is {start} ({temp}\u{b0}C)."),
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
            let name = place_name(ctx.w, &cell.place);
            let pinned = if cell.pinned { " [pinned]" } else { "" };
            ctx.out.info(
                ctx.actor,
                format!(
                    "  {name} ({}): {}, {}\u{b0}C, {}{pinned}",
                    cell.place, cell.state, cell.temp, cell.climate
                ),
            );
        }
        Ok(())
    }

    /// Broadcast a state line to a place's OUTDOORS/SHELTERED rooms (muffled
    /// for sheltered) and a matching panel delta.
    fn broadcast(
        &self,
        ctx: &mut Ctx<'_>,
        place: &CanonicalId,
        line: &str,
        icon: &SmolStr,
        temp: i16,
    ) {
        let scan = scan_place(ctx.w, place);
        for room in scan.rooms {
            let text = if room.sheltered {
                muffle(line)
            } else {
                line.to_string()
            };
            ctx.out.to_room(
                room.room,
                Vec::new(),
                weather_event(&scan.name, text, icon.clone()),
            );
            self.panel.delta_to(
                ctx,
                mush_core::Audience::Room {
                    room: room.room,
                    exclude: Vec::new(),
                    subject: None,
                },
                panel_payload(&scan.name, icon, temp),
            );
        }
    }

    // ------------------------------------------------------------------
    // M10 LLM flavor top-up
    // ------------------------------------------------------------------

    /// `@weather/generate [<climate>] [<state>]` — fire off-loop LLM flavor
    /// generation. The command spec already gates this on Royalty; it
    /// additionally requires the `BUILDER_LLM` power (the LLM-spend gate, which
    /// Wizards hold implicitly). Never blocks the loop: each `(climate, state)`
    /// target defers one [`Effect::ModuleAsync`] that re-enters via
    /// [`WeatherFlavorCompletion`] with PENDING lines. With `[llm].enabled =
    /// false` (or the kill switch) the off-loop call re-enters a "disabled"
    /// notice and stores nothing — everything else is unchanged.
    fn generate(&self, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
        if !perm::has_power(ctx.w, ctx.actor, Powers::BUILDER_LLM) {
            return Err(CmdError::msg(
                "You need the BUILDER_LLM power to spend weather LLM budget.",
            ));
        }
        let toks: Vec<String> = ctx
            .args
            .lhs()
            .split_whitespace()
            .map(str::to_string)
            .collect();
        let targets = self.resolve_targets(ctx, &toks)?;
        if targets.is_empty() {
            return Err(CmdError::msg(
                "Nothing to generate: no zone has a climate yet. Name one explicitly: \
                 @weather/generate <climate> [<state>].",
            ));
        }
        if self.config.flavor_variants == 0 {
            return Err(CmdError::msg(
                "Weather flavor generation is disabled because flavor_variants is zero.",
            ));
        }
        let variants = self
            .config
            .flavor_variants
            .min(MAX_FLAVOR_VARIANTS_PER_BUCKET);
        let mut fired = 0usize;
        let mut skipped_full = 0usize;
        for (climate_id, state) in &targets {
            let pools_full = ctx.w.module_state::<WeatherState>().is_some_and(|weather| {
                FlavorBucket::ALL.into_iter().all(|bucket| {
                    weather
                        .flavor
                        .pool(&FlavorKey::new(climate_id.clone(), state.clone(), bucket))
                        .len()
                        >= MAX_FLAVOR_LINES_PER_POOL
                })
            });
            if pools_full {
                skipped_full += 1;
                continue;
            }
            let Some(climate) = self.climates.get(climate_id) else {
                continue;
            };
            let snap = build_flavor_snapshot(ctx.w, climate, ctx.actor, state, variants);
            let payload = match serde_json::to_string(&snap) {
                Ok(p) => p,
                Err(e) => {
                    ctx.out.error(
                        ctx.actor,
                        format!("Could not prepare {climate_id}/{state}: {e}"),
                    );
                    continue;
                }
            };
            let correlation = ctx.w.next_correlation_id();
            ctx.out.defer(Deferred::Effect(Effect::ModuleAsync {
                module: SmolStr::new_static(NS),
                actor: ctx.actor,
                correlation,
                payload,
            }));
            fired += 1;
        }
        ctx.out.info(
            ctx.actor,
            format!(
                "Requested LLM flavor for {fired} (climate, state) bucket(s); PENDING lines \
                 arrive shortly (the world never blocks). {skipped_full} full bucket(s) were \
                 skipped. Review with @weather/lines."
            ),
        );
        Ok(())
    }

    /// Resolve the `[<climate>] [<state>]` args to concrete `(climate, state)`
    /// generation targets. No args ⇒ every in-use climate × all its states;
    /// one arg ⇒ that climate × all states; two ⇒ exactly that bucket.
    fn resolve_targets(
        &self,
        ctx: &Ctx<'_>,
        toks: &[String],
    ) -> Result<Vec<(SmolStr, SmolStr)>, CmdError> {
        match toks.len() {
            0 => {
                let mut climates: Vec<SmolStr> = ctx
                    .w
                    .module_state::<WeatherState>()
                    .map(|s| {
                        let mut v: Vec<SmolStr> =
                            s.cells.values().map(|c| c.climate.clone()).collect();
                        v.sort();
                        v.dedup();
                        v
                    })
                    .unwrap_or_default();
                climates.retain(|c| self.climates.contains(c));
                let mut out = Vec::new();
                for c in climates {
                    if let Some(def) = self.climates.get(&c) {
                        for st in &def.states {
                            out.push((c.clone(), st.id.clone()));
                        }
                    }
                }
                Ok(out)
            }
            1 => {
                let climate = self.require_climate(&toks[0])?;
                let def = self.climates.get(&climate).expect("climate checked");
                Ok(def
                    .states
                    .iter()
                    .map(|st| (climate.clone(), st.id.clone()))
                    .collect())
            }
            2 => {
                let climate = self.require_climate(&toks[0])?;
                let def = self.climates.get(&climate).expect("climate checked");
                let state = &toks[1];
                if def.state(state).is_none() {
                    let known: Vec<&str> = def.states.iter().map(|s| s.id.as_str()).collect();
                    return Err(CmdError::msg(format!(
                        "Unknown state {state:?} for climate {climate}. Known: {}.",
                        known.join(", ")
                    )));
                }
                Ok(vec![(climate, SmolStr::new(state))])
            }
            _ => Err(CmdError::msg(
                "Usage: @weather/generate [<climate>] [<state>].",
            )),
        }
    }

    /// Validate a climate name, returning its id or a helpful error.
    fn require_climate(&self, name: &str) -> Result<SmolStr, CmdError> {
        if self.climates.contains(name) {
            Ok(SmolStr::new(name))
        } else {
            Err(CmdError::msg(format!(
                "Unknown climate {name:?}. Known: {}.",
                self.climates
                    .ids()
                    .iter()
                    .map(SmolStr::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            )))
        }
    }

    /// `@weather/lines [<climate>] [<state>]` — list the flavor pool:
    /// hand-written lines marked `[builtin]` and generated lines marked
    /// `[PENDING #id]` / `[approved #id]`, so staff can review before approving.
    fn lines_list(&self, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
        let toks: Vec<String> = ctx
            .args
            .lhs()
            .split_whitespace()
            .map(str::to_string)
            .collect();
        let (climate_filter, state_filter) = match toks.len() {
            0 => (None, None),
            1 => (Some(self.require_climate(&toks[0])?), None),
            2 => (
                Some(self.require_climate(&toks[0])?),
                Some(SmolStr::new(&toks[1])),
            ),
            _ => {
                return Err(CmdError::msg(
                    "Usage: @weather/lines [<climate>] [<state>].",
                ));
            }
        };
        let climate_ids: Vec<SmolStr> = match &climate_filter {
            Some(c) => vec![c.clone()],
            None => self.climates.ids(),
        };
        let mut shown = false;
        for cid in &climate_ids {
            let Some(def) = self.climates.get(cid) else {
                continue;
            };
            let states: Vec<SmolStr> = match &state_filter {
                Some(s) => vec![s.clone()],
                None => def.states.iter().map(|s| s.id.clone()).collect(),
            };
            for state in &states {
                for bucket in FlavorBucket::ALL {
                    let builtin: Vec<String> = builtin_lines(def, state, bucket);
                    let key = FlavorKey::new(cid.clone(), state.clone(), bucket);
                    let generated: Vec<FlavorLine> = ctx
                        .w
                        .module_state::<WeatherState>()
                        .map(|s| s.flavor.pool(&key).to_vec())
                        .unwrap_or_default();
                    if builtin.is_empty() && generated.is_empty() {
                        continue;
                    }
                    shown = true;
                    ctx.out
                        .info(ctx.actor, format!("{cid} / {state} / {}:", bucket.as_str()));
                    for b in &builtin {
                        ctx.out.info(ctx.actor, format!("  [builtin]       {b}"));
                    }
                    for g in &generated {
                        let mark = if g.approved { "approved" } else { "PENDING " };
                        ctx.out
                            .info(ctx.actor, format!("  [{mark} #{:<4}] {}", g.id, g.text));
                    }
                }
            }
        }
        if !shown {
            ctx.out.info(ctx.actor, "No weather flavor lines to show.");
        }
        Ok(())
    }

    /// `@weather/lines/approve <id>|all` and `@weather/lines/reject <id>|all` —
    /// move a pending line into (or out of) the live selection pool. Approving
    /// is the ONLY way generated text ever reaches a player; rejecting deletes
    /// the line. Both persist the changed pool.
    fn lines_review(&self, ctx: &mut Ctx<'_>, approve: bool) -> Result<(), CmdError> {
        let arg = ctx.args.lhs().trim().to_string();
        if arg.is_empty() {
            return Err(CmdError::msg(if approve {
                "Usage: @weather/lines/approve <id>|all."
            } else {
                "Usage: @weather/lines/reject <id>|all."
            }));
        }
        let (changed, summary) = {
            let Some(st) = ctx.w.module_state_mut::<WeatherState>() else {
                return Err(CmdError::msg("Weather state unavailable."));
            };
            if arg.eq_ignore_ascii_case("all") {
                let keys = if approve {
                    st.flavor.approve_all_pending()
                } else {
                    st.flavor.reject_all_pending()
                };
                let n = keys.len();
                let summary = if approve {
                    format!("Approved every pending line ({n} pool(s) now in play).")
                } else {
                    format!("Rejected every pending line ({n} pool(s) changed).")
                };
                (keys, summary)
            } else {
                let Ok(id) = arg.parse::<u64>() else {
                    return Err(CmdError::msg(
                        "Give a numeric line id (see @weather/lines) or 'all'.",
                    ));
                };
                let text = st.flavor.find(id).map(|(_, l)| l.text.clone());
                let Some(text) = text else {
                    return Err(CmdError::msg(format!(
                        "No flavor line #{id} (see @weather/lines)."
                    )));
                };
                let key = if approve {
                    st.flavor.approve(id)
                } else {
                    st.flavor.reject(id)
                };
                match key {
                    Some(k) => {
                        let summary = if approve {
                            format!(
                                "Approved line #{id}: \"{text}\" -- now in the live pool for {}/{}.",
                                k.climate, k.state
                            )
                        } else {
                            format!("Rejected line #{id}: \"{text}\".")
                        };
                        (vec![k], summary)
                    }
                    None => {
                        // Only reachable on approve of an already-approved line.
                        return Err(CmdError::msg(format!("Line #{id} is already approved.")));
                    }
                }
            }
        };
        let ops: Vec<_> = {
            let st = ctx
                .w
                .module_state::<WeatherState>()
                .expect("weather state slot installed at registration");
            changed.iter().map(|k| st.flavor.persist_op(k)).collect()
        };
        for op in ops {
            ctx.persist(op);
        }
        ctx.out.info(ctx.actor, summary);
        Ok(())
    }
}

// ----------------------------------------------------------------------
// Flavor snapshot assembly + the async completion (re-entry)
// ----------------------------------------------------------------------

/// Snapshot the prompt context for one `(climate, state)` bucket: the climate's
/// style + the state's physical conditions, plus the theme of a place in that
/// climate (if any) for setting-consistency. Owned data only (treaty ruling 3).
fn build_flavor_snapshot(
    w: &World,
    climate: &ClimateDef,
    requester: ObjId,
    state: &SmolStr,
    variants: usize,
) -> FlavorGenSnapshot {
    let state_desc = climate
        .state(state)
        .map(state_desc_line)
        .unwrap_or_else(|| format!("state {state}"));
    let (theme_style, theme_vocab, theme_banned) = read_place_theme(w, &climate.id);
    FlavorGenSnapshot {
        requester,
        climate: climate.id.clone(),
        climate_name: climate.name.clone(),
        climate_style: climate.style.clone(),
        state: state.clone(),
        state_desc,
        theme_style,
        theme_vocab,
        theme_banned,
        variants,
    }
}

/// A one-line physical description of a weather state, grounding the flavor.
fn state_desc_line(def: &WxStateDef) -> String {
    format!(
        "visibility {}%, precipitation {}%, wind {}%, temperature {}..{}\u{b0}C",
        def.visibility, def.precip, def.wind, def.temp_band.0, def.temp_band.1
    )
}

/// The theme in effect at the first (lowest-id) place using `climate_id`, read
/// through the atlas (S2b): its style guide, vocabulary and banned words. Empty
/// when no place uses the climate or no theme resolves.
fn read_place_theme(w: &World, climate_id: &str) -> (String, Vec<String>, Vec<String>) {
    let Some(place) = w.module_state::<WeatherState>().and_then(|s| {
        s.cells
            .values()
            .filter(|c| c.climate == climate_id)
            .map(|c| c.place.clone())
            .min()
    }) else {
        return (String::new(), Vec::new(), Vec::new());
    };
    let Some(place) = place_ref(w, &place) else {
        return (String::new(), Vec::new(), Vec::new());
    };
    let theme = mush_atlas::theme::resolve_place_theme(w, &place);
    (theme.style_guide, theme.vocabulary, theme.banned)
}

/// The hand-written lines for `(state, bucket)` — the `[builtin]` half of a
/// `@weather/lines` listing (the generic destination/ambient pools; specific
/// `from->to` pairs are not attributed to a single bucket here).
fn builtin_lines(def: &ClimateDef, state: &str, bucket: FlavorBucket) -> Vec<String> {
    let pool = match bucket {
        FlavorBucket::Transition => def.lines.transition.get(state),
        FlavorBucket::Ambient => def.lines.ambient.get(state),
    };
    pool.map(|v| v.to_vec()).unwrap_or_default()
}

/// The re-entry for a finished `@weather/generate` (treaty ruling 3): store the
/// generated variants as PENDING and notify the requester. Nothing generated is
/// ever auto-shown — a staffer must `@weather/lines/approve` it first.
struct WeatherFlavorCompletion {
    climates: Arc<ClimateSet>,
}

impl AsyncCompletion for WeatherFlavorCompletion {
    fn on_complete(&self, ctx: &mut Ctx<'_>, correlation: u64, payload: &str) {
        let outcome: FlavorGenOutcome = match serde_json::from_str(payload) {
            Ok(o) => o,
            Err(e) => {
                tracing::error!(correlation, error = %e, "weather: bad flavor completion payload");
                return;
            }
        };
        match outcome {
            FlavorGenOutcome::Err {
                requester,
                climate,
                state,
                message,
            } => {
                if ctx.w.get(requester).is_some() {
                    ctx.out.error(
                        requester,
                        format!("Weather flavor for {climate}/{state}: {message}"),
                    );
                }
            }
            FlavorGenOutcome::Ok {
                requester,
                climate,
                state,
                transition,
                ambient,
                model,
            } => {
                // Re-validate against current data: the climate/state must still
                // ship, or the generated lines can never enter selection.
                let still_valid = self
                    .climates
                    .get(&climate)
                    .is_some_and(|c| c.state(&state).is_some());
                if !still_valid {
                    if ctx.w.get(requester).is_some() {
                        ctx.out.error(
                            requester,
                            format!(
                                "Discarding generated flavor: climate {climate}/{state} no longer exists."
                            ),
                        );
                    }
                    return;
                }
                let offered_t = transition.len();
                let offered_a = ambient.len();
                let (n_t, n_a, ops): (_, _, Vec<_>) = {
                    let Some(st) = ctx.state_mut::<WeatherState>() else {
                        return;
                    };
                    let mut ops = Vec::new();
                    let n_t = if transition.is_empty() {
                        0
                    } else {
                        let added = st.flavor.add_pending(
                            &climate,
                            &state,
                            FlavorBucket::Transition,
                            transition,
                        );
                        if added != 0 {
                            ops.push(st.flavor.persist_op(&FlavorKey::new(
                                climate.clone(),
                                state.clone(),
                                FlavorBucket::Transition,
                            )));
                        }
                        added
                    };
                    let n_a = if ambient.is_empty() {
                        0
                    } else {
                        let added =
                            st.flavor
                                .add_pending(&climate, &state, FlavorBucket::Ambient, ambient);
                        if added != 0 {
                            ops.push(st.flavor.persist_op(&FlavorKey::new(
                                climate.clone(),
                                state.clone(),
                                FlavorBucket::Ambient,
                            )));
                        }
                        added
                    };
                    (n_t, n_a, ops)
                };
                for op in ops {
                    ctx.persist(op);
                }
                if ctx.w.get(requester).is_some() {
                    ctx.out.info(
                        requester,
                        format!(
                            "Weather flavor for {climate}/{state}: {n_t} transition + {n_a} \
                             ambient PENDING line(s) added; {} returned line(s) were duplicate, \
                             oversized, or beyond the pool cap (model {model}). Review: \
                             @weather/lines {climate} {state}; approve: @weather/lines/approve <id>.",
                            offered_t + offered_a - n_t - n_a
                        ),
                    );
                }
            }
        }
    }
}
