//! The command surface (plan §3.1):
//!
//! ```text
//! +atlas                  the neighborhood you are in: a map and who is where
//! +atlas <place>          a neighborhood by name
//! +atlas/region           the region: neighborhoods and corridors
//! @atlas/pack list                  every pack on the shelf and every latch
//! @atlas/pack show <id>             a pack's manifest, entries, brief and secrets
//! @atlas/pack plan <id>             the diff against the latch, and a fingerprint
//! @atlas/pack apply <fingerprint>   apply the plan you just printed
//! @atlas/pack retire <id> [reason]  retire a pack, refusing to orphan anything
//! ```
//!
//! `+atlas` is read-only. `@atlas` is staff (`SIM_ADMIN` or Royalty) and writes the
//! atlas's own records and the content latch through `mush-content`'s API — the
//! `legacy-post-image` lane, family `atlas.records`.

use std::sync::Arc;

use mush_content::{ContentState, Step, apply_pack, plan};
use mush_core::module::{ModError, Registrar};
use mush_core::{ArgShape, CanonicalId, CmdError, CommandSpec, Ctx, LIMBO};
use mush_proto::{OutputEvent, RichText};
use smol_str::SmolStr;

use crate::module::{AtlasState, NS, is_staff};
use crate::view;

/// The words `@atlas/pack` understands.
pub const PACK_VERBS: [&str; 5] = ["list", "show", "plan", "apply", "retire"];

pub(crate) fn register_views(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "+atlas".into(),
            shape: ArgShape::Raw,
            switches: vec![SmolStr::new_static("region")],
            help:
                "+atlas                 the neighborhood you are in, as a map (north up, five to\n\
                   \x20                      ten metres a character) and a list of its rooms and\n\
                   \x20                      who is in them\n\
                   +atlas <place>         a neighborhood by name\n\
                   +atlas/region          the region: every neighborhood's footprint, the\n\
                   \x20                      corridors between them and their checkpoints\n\
                   A room you may not enter is not on your map unless you are staff or standing\n\
                   in it. Limbo is on no map; the OOC Area <O> and the Staff lot <S> are."
                    .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_atlas),
    )
}

pub(crate) fn register_pack(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "@atlas".into(),
            shape: ArgShape::Raw,
            switches: vec![SmolStr::new_static("pack")],
            help: "@atlas/pack list                   every pack on the shelf, and every latch\n\
                   @atlas/pack show <id>              a pack's manifest, entries, brief and secrets\n\
                   @atlas/pack plan <id>              what applying it would create, update or\n\
                   \x20                                  retire, the validator's findings, and a\n\
                   \x20                                  fingerprint of that plan\n\
                   @atlas/pack apply <fingerprint>    apply the plan you just printed — a pack or\n\
                   \x20                                  a world that changed since gives a new\n\
                   \x20                                  fingerprint and refuses\n\
                   @atlas/pack retire <id> [reason]   retire a whole pack. Refused, naming the\n\
                   \x20                                  reference, while any room is occupied, holds\n\
                   \x20                                  something the pack did not put there, or is\n\
                   \x20                                  the destination of an exit from outside it.\n\
                   Staff only (SIM_ADMIN or Royalty). A pack is content: applying the same pack\n\
                   twice changes nothing, and a retired pack stays retired across restarts."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_atlas_pack),
    )
}

fn say(ctx: &mut Ctx<'_>, lines: Vec<String>) {
    let text = lines.join("\n");
    ctx.out
        .to_actor(ctx.actor, OutputEvent::Sys(RichText::tag(NS, text)));
}

// ---------------------------------------------------------------------------
// +atlas
// ---------------------------------------------------------------------------

fn cmd_atlas(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let staff = is_staff(ctx.w, actor);
    let query = ctx.args.raw().trim().to_string();
    let region_view = ctx.switches.has("region");
    let here = ctx.w.get(actor).and_then(|o| o.location);
    let Some(state) = ctx.state::<AtlasState>() else {
        return Err(CmdError::msg("The atlas is not composed on this server."));
    };
    let lines = if region_view {
        let region = if query.is_empty() {
            here.and_then(|room| state.records.neighborhood_of_room(room))
                .and_then(|nb| state.records.region_of(nb))
        } else {
            state
                .records
                .regions
                .values()
                .find(|r| r.name.eq_ignore_ascii_case(&query) || r.key.pack.as_str() == query)
        };
        match region {
            Some(region) => view::region(state, ctx.w, actor, region, staff),
            None if query.is_empty() => return Err(CmdError::msg(nowhere(here))),
            None => return Err(CmdError::msg(format!("No region called {query:?}."))),
        }
    } else {
        let nb = if query.is_empty() {
            here.and_then(|room| state.records.neighborhood_of_room(room))
        } else {
            let needle = query.to_ascii_lowercase();
            let mine = here
                .and_then(|room| state.records.neighborhood_of_room(room))
                .map(|nb| nb.region.clone());
            state
                .records
                .neighborhoods
                .values()
                .filter(|nb| staff || mine.as_ref() == Some(&nb.region))
                .find(|nb| nb.name.to_ascii_lowercase() == needle || nb.key.pack.as_str() == needle)
                .or_else(|| {
                    state
                        .records
                        .neighborhoods
                        .values()
                        .filter(|nb| staff || mine.as_ref() == Some(&nb.region))
                        .find(|nb| nb.name.to_ascii_lowercase().contains(&needle))
                })
        };
        match nb {
            Some(nb) => view::neighborhood(state, ctx.w, actor, nb, staff),
            None if query.is_empty() => return Err(CmdError::msg(nowhere(here))),
            None => {
                return Err(CmdError::msg(format!(
                    "No neighborhood called {query:?} that you know."
                )));
            }
        }
    };
    say(ctx, lines);
    Ok(())
}

fn nowhere(here: Option<mush_core::ObjId>) -> String {
    if here == Some(LIMBO) {
        "Limbo is on no map. The OOC Area <O> and the Staff lot <S> are.".to_string()
    } else {
        "You are nowhere the atlas knows.".to_string()
    }
}

// ---------------------------------------------------------------------------
// @atlas/pack
// ---------------------------------------------------------------------------

fn cmd_atlas_pack(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !is_staff(ctx.w, ctx.actor) {
        return Err(CmdError::msg("Permission denied."));
    }
    if !ctx.switches.has("pack") {
        return Err(CmdError::msg(
            "Usage: @atlas/pack list|show|plan|apply|retire — see help @atlas.",
        ));
    }
    let raw = ctx.args.raw().trim().to_string();
    let mut words = raw
        .splitn(3, char::is_whitespace)
        .map(str::trim)
        .filter(|w| !w.is_empty());
    let verb = words.next().unwrap_or("").to_ascii_lowercase();
    let first = words.next().map(str::to_string);
    let rest = words.next().map(str::to_string);
    match verb.as_str() {
        "list" => pack_list(ctx),
        "show" => pack_show(ctx, first.as_deref()),
        "plan" => pack_plan(ctx, first.as_deref()),
        "apply" => pack_apply(ctx, first.as_deref(), rest.as_deref()),
        "retire" => pack_retire(ctx, first.as_deref(), rest.as_deref()),
        _ => Err(CmdError::msg(format!(
            "Usage: @atlas/pack {} — see help @atlas.",
            PACK_VERBS.join("|")
        ))),
    }
}

fn pack_id(word: Option<&str>) -> Result<CanonicalId, CmdError> {
    let word = word.ok_or_else(|| CmdError::msg("Which pack? Give its id."))?;
    CanonicalId::new(word).map_err(|e| CmdError::msg(format!("{word:?} is not a pack id: {e}")))
}

fn latch_word(content: &ContentState, id: &CanonicalId) -> String {
    match content.latches.applied(id) {
        None if content.latches.is_held(id) => "latch held undecodable".to_string(),
        None => "not applied".to_string(),
        Some(a) if a.is_retired() => format!("retired (was v{})", a.version()),
        Some(a) => {
            let mut s = format!(
                "applied v{} ({} entries, {} objects)",
                a.version(),
                a.entries().len(),
                a.placed().len()
            );
            if let Some(anomaly) = a.anomaly() {
                s.push_str(&format!("; anomaly: {anomaly}"));
            }
            s
        }
    }
}

fn pack_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (Some(atlas), Some(content)) = (ctx.state::<AtlasState>(), ctx.state::<ContentState>())
    else {
        return Err(CmdError::msg("The atlas is not composed on this server."));
    };
    let mut lines: Vec<String> = vec!["Packs on the shelf:".to_string()];
    for pack in atlas.shelf.values() {
        let m = pack.manifest();
        lines.push(format!(
            " {}@{} ({}, {} entries, digest {}…) — {}",
            m.id(),
            m.version(),
            m.kind().label(),
            pack.entries().len(),
            m.digest().to_hex().chars().take(12).collect::<String>(),
            latch_word(content, m.id())
        ));
    }
    let off_shelf: Vec<String> = content
        .latches
        .all()
        .filter(|a| !atlas.shelf.contains_key(a.pack()))
        .map(|a| {
            format!(
                " {} (not on the shelf) — {}",
                a.pack(),
                latch_word(content, a.pack())
            )
        })
        .collect();
    if !off_shelf.is_empty() {
        lines.push("Latched, not on the shelf:".to_string());
        lines.extend(off_shelf);
    }
    if lines.len() == 1 {
        lines.push(" (none)".to_string());
    }
    say(ctx, lines);
    Ok(())
}

fn pack_show(ctx: &mut Ctx<'_>, id: Option<&str>) -> Result<(), CmdError> {
    let id = pack_id(id)?;
    let (Some(atlas), Some(content)) = (ctx.state::<AtlasState>(), ctx.state::<ContentState>())
    else {
        return Err(CmdError::msg("The atlas is not composed on this server."));
    };
    let Some(pack) = atlas.shelf.get(&id) else {
        return Err(CmdError::msg(format!("{id} is not on the shelf.")));
    };
    let m = pack.manifest();
    let mut lines = vec![
        format!(
            "{}@{} — {} pack, digest {}",
            m.id(),
            m.version(),
            m.kind().label(),
            m.digest().to_hex()
        ),
        format!(" latch: {}", latch_word(content, m.id())),
    ];
    if m.requires().is_empty() {
        lines.push(" requires: nothing".to_string());
    } else {
        for req in m.requires() {
            lines.push(format!(
                " requires: {}@{}{}",
                req.id,
                req.version,
                req.digest.map_or(String::new(), |d| format!(
                    " (digest {}…)",
                    d.to_hex().chars().take(12).collect::<String>()
                ))
            ));
        }
    }
    let mut by_kind: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for entry in pack.entries().values() {
        *by_kind.entry(entry.kind().to_string()).or_default() += 1;
    }
    lines.push(format!(
        " entries: {}",
        by_kind
            .iter()
            .map(|(k, n)| format!("{n} {k}"))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    if let Some(brief) = pack.brief() {
        if let Some(prose) = brief.get("prose").and_then(|p| p.as_str()) {
            let short: String = prose.chars().take(300).collect();
            lines.push(format!(
                " brief: {short}{}",
                if prose.chars().count() > 300 {
                    "…"
                } else {
                    ""
                }
            ));
        }
        if let Some(origin) = brief.get("origin").and_then(|p| p.as_str()) {
            lines.push(format!(" origin: {origin}"));
        }
    }
    let secrets: Vec<String> = atlas
        .records
        .secrets
        .values()
        .filter(|s| s.key.pack == id)
        .map(|s| format!(" secret {}: {}", s.key.entry, s.truth))
        .collect();
    if !secrets.is_empty() {
        lines.push(" secrets (staff eyes only; never in any prompt):".to_string());
        lines.extend(secrets);
    }
    say(ctx, lines);
    Ok(())
}

fn pack_plan(ctx: &mut Ctx<'_>, id: Option<&str>) -> Result<(), CmdError> {
    let id = pack_id(id)?;
    let actor = ctx.actor;
    let (plan, problems) = {
        let (Some(atlas), Some(content)) = (ctx.state::<AtlasState>(), ctx.state::<ContentState>())
        else {
            return Err(CmdError::msg("The atlas is not composed on this server."));
        };
        let Some(pack) = atlas.shelf.get(&id) else {
            return Err(CmdError::msg(format!("{id} is not on the shelf.")));
        };
        let previous = content.latches.applied(&id);
        let plan = plan(pack, previous).map_err(|e| CmdError::msg(e.to_string()))?;
        let mut problems = crate::validate::check_pack(pack, &content.appliers);
        if let Err(e) = mush_content::check_requires(pack, &content.latches.loaded()) {
            problems.push(e.to_string());
        }
        (plan, problems)
    };
    let fingerprint = plan.fingerprint();
    let mut lines = vec![format!(
        "Plan for {}@{}: {} create, {} update, {} retire, {} unchanged",
        plan.pack,
        plan.version,
        plan.count(Step::Create),
        plan.count(Step::Update),
        plan.count(Step::Retire),
        plan.count(Step::Unchanged)
    )];
    for entry in &plan.entries {
        if entry.step == Step::Unchanged {
            continue;
        }
        lines.push(format!(
            " {:<8} {} ({}){}",
            entry.step.label(),
            entry.id,
            entry.kind,
            entry.existing.map_or(String::new(), |o| format!(" {o}"))
        ));
    }
    if problems.is_empty() {
        lines.push(" validator: no findings".to_string());
    } else {
        lines.push(format!(
            " validator: {} finding(s) — apply will refuse or stop at the first:",
            problems.len()
        ));
        for p in problems.iter().take(20) {
            lines.push(format!("  - {p}"));
        }
    }
    if plan.is_noop() {
        lines.push(" Nothing to do.".to_string());
    } else {
        lines.push(format!(
            " fingerprint {fingerprint} — @atlas/pack apply {fingerprint}"
        ));
    }
    if let Some(state) = ctx.state_mut::<AtlasState>() {
        state.last_plan.insert(actor, (id, fingerprint));
    }
    say(ctx, lines);
    Ok(())
}

fn pack_apply(
    ctx: &mut Ctx<'_>,
    first: Option<&str>,
    second: Option<&str>,
) -> Result<(), CmdError> {
    let actor = ctx.actor;
    // `apply <fingerprint>` uses the last plan; `apply <id> <fingerprint>` is explicit.
    let (id, given) = match (first, second) {
        (Some(id), Some(fp)) => (pack_id(Some(id))?, fp.trim().to_ascii_lowercase()),
        (Some(fp), None) => {
            let Some(state) = ctx.state::<AtlasState>() else {
                return Err(CmdError::msg("The atlas is not composed on this server."));
            };
            let Some((id, _)) = state.last_plan.get(&actor) else {
                return Err(CmdError::msg("Plan a pack first: @atlas/pack plan <id>."));
            };
            (id.clone(), fp.trim().to_ascii_lowercase())
        }
        (None, _) => {
            return Err(CmdError::msg(
                "Usage: @atlas/pack apply <fingerprint> — from @atlas/pack plan.",
            ));
        }
    };
    let pack = {
        let (Some(atlas), Some(content)) = (ctx.state::<AtlasState>(), ctx.state::<ContentState>())
        else {
            return Err(CmdError::msg("The atlas is not composed on this server."));
        };
        let Some(pack) = atlas.shelf.get(&id) else {
            return Err(CmdError::msg(format!("{id} is not on the shelf.")));
        };
        let current =
            plan(pack, content.latches.applied(&id)).map_err(|e| CmdError::msg(e.to_string()))?;
        if current.fingerprint() != given {
            return Err(CmdError::msg(format!(
                "That is not the plan for {id} as it stands now (fingerprint {}). Plan it again and read it.",
                current.fingerprint()
            )));
        }
        if current.is_noop() {
            return Err(CmdError::msg(format!(
                "{id} is applied at this version; nothing to do."
            )));
        }
        pack.clone()
    };
    let (applied, report) = apply_pack(ctx, &pack).map_err(|e| CmdError::msg(e.to_string()))?;
    if let Some(state) = ctx.state_mut::<AtlasState>() {
        state.last_plan.remove(&actor);
    }
    let mut lines = vec![format!(
        "Applied {}@{}: {} created, {} updated, {} retired, {} unchanged.",
        applied.pack(),
        applied.version(),
        report.created,
        report.updated,
        report.retired,
        report.unchanged
    )];
    if let Some(anomaly) = &report.anomaly {
        lines.push(format!(" STOPPED PART WAY: {anomaly}"));
        lines.push(
            " What was made is latched; fix the pack and plan again to continue.".to_string(),
        );
    }
    say(ctx, lines);
    Ok(())
}

fn pack_retire(ctx: &mut Ctx<'_>, id: Option<&str>, reason: Option<&str>) -> Result<(), CmdError> {
    let id = pack_id(id)?;
    let reason = reason
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .unwrap_or("retired by staff")
        .to_string();
    let (applied, report) = mush_content::module::retire_pack(ctx, &id, &reason)
        .map_err(|e| CmdError::msg(e.to_string()))?;
    let mut lines = vec![format!(
        "Retired {}: {} entries removed{}.",
        applied.pack(),
        report.retired,
        if applied.is_retired() {
            "; the latch is kept, marked retired, so a restart does not bring it back"
        } else {
            ""
        }
    )];
    if let Some(anomaly) = &report.anomaly
        && !applied.is_retired()
    {
        lines.push(format!(" STOPPED PART WAY: {anomaly}"));
    }
    say(ctx, lines);
    Ok(())
}
