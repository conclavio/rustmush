//! Thin sortie adapters: inspect terms, then submit their exact acceptance.
use super::{CrewDuty, CrewHandles, HireTerms, NpcMissionScope, offers, runtime::check_contract};
use mush_core::{CanonicalHash, CmdError, Ctx, IdempotencyKeyText, ObjId, PerceivedRef};
use serde_json::json;

pub(crate) const SWITCHES: &[&str] = &["crew", "hire", "contract", "board", "unseat"];
pub(crate) fn handles(ctx: &Ctx<'_>) -> bool {
    SWITCHES.iter().any(|s| ctx.switches.has(s))
}
fn unavailable() -> CmdError {
    CmdError::msg("Crew or contract unavailable. Use +sortie/crew for current hiring terms.")
}
fn npc(raw: &str) -> Result<ObjId, CmdError> {
    let raw = raw.trim().strip_prefix('#').unwrap_or(raw.trim());
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(unavailable());
    }
    raw.parse::<u32>()
        .ok()
        .filter(|id| *id > 0)
        .map(ObjId)
        .ok_or_else(unavailable)
}
fn line(ctx: &mut Ctx<'_>, text: impl Into<String>) {
    ctx.out
        .sys_to_actor(ctx.actor, "tacsim.detachment", text.into());
}
pub(crate) fn describe(terms: &HireTerms) -> String {
    let scope = match terms.mission_scope {
        NpcMissionScope::SubordinateOperator => "unit operator",
        NpcMissionScope::DetachmentCommander => "detachment commander",
    };
    let duties = terms
        .duties
        .iter()
        .map(|duty| match duty {
            CrewDuty::Mining => "mining",
            CrewDuty::Combat => "combat",
            CrewDuty::Recovery => "recovery",
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("{scope}; duties: {duties}; hiring port: {}", terms.port)
}
pub(crate) fn command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.iter().count() != 1 || ctx.args.raw().len() > 256 {
        return Err(unavailable());
    }
    let handles = ctx
        .w
        .module_state::<CrewHandles>()
        .cloned()
        .ok_or_else(unavailable)?;
    let raw = ctx.args.raw().trim().to_owned();
    if ctx.switches.has("board") || ctx.switches.has("unseat") {
        let npc = npc(&raw).or_else(|_| {
            mush_core::match_object(ctx.w, ctx.actor, &raw)
                .map_err(|_| CmdError::msg("Crew or contract unavailable."))
        })?;
        let message = if ctx.switches.has("unseat") {
            crate::port::boarding::unseat_npc(ctx, npc, &handles)
        } else {
            super::boarding::board(ctx, npc, &handles)
        }
        .map_err(CmdError::msg)?;
        line(ctx, message);
        return Ok(());
    }
    if ctx.switches.has("crew") {
        if !raw.is_empty() {
            return Err(unavailable());
        }
        let rows = offers(ctx.w, ctx.actor, |npc| {
            check_contract(
                npc,
                ctx.action_state(&handles.contracts, &npc)
                    .map_err(|e| e.to_string())?,
            )
        })
        .map_err(|_| unavailable())?;
        if rows.is_empty() {
            line(ctx, "No NPC crew available for hire in this port room.");
        }
        for offer in rows {
            let name = ctx
                .w
                .get(offer.npc)
                .map(|o| o.name.as_str())
                .ok_or_else(unavailable)?
                .to_owned();
            line(
                ctx,
                format!("{name} (#{}): {}", offer.npc.0, describe(&offer.terms)),
            );
            line(
                ctx,
                format!(
                    "Accept: +sortie/hire #{}={}",
                    offer.npc.0,
                    offer.terms_hash.to_hex()
                ),
            );
        }
        line(
            ctx,
            "These offers have no wage charge. Hiring does not move the character aboard.",
        );
        return Ok(());
    }
    if ctx.switches.has("contract") {
        let npc = npc(&raw)?;
        let contract = ctx
            .action_state(&handles.contracts, &npc)
            .map_err(|_| unavailable())?
            .filter(|c| c.employer == ctx.actor || c.npc == ctx.actor)
            .cloned()
            .ok_or_else(unavailable)?;
        check_contract(npc, Some(&contract)).map_err(|_| unavailable())?;
        line(
            ctx,
            format!(
                "Crew contract — NPC #{}; employer #{}; {}.",
                contract.npc.0,
                contract.employer.0,
                describe(&contract.terms)
            ),
        );
        line(
            ctx,
            "Contract recorded. Physical presence is checked separately at preparation and launch.",
        );
        if contract.employer == ctx.actor {
            line(ctx, format!("Board: +sortie/board #{}", npc.0));
            line(
                ctx,
                format!("Leave a returned unit: +sortie/unseat #{}", npc.0),
            );
        }
        return Ok(());
    }
    let (object, terms) = raw.split_once('=').ok_or_else(unavailable)?;
    let npc = npc(object)?;
    let terms = terms.trim();
    if terms.len() != 64
        || !terms
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(unavailable());
    }
    if ctx.authoritative_pending(ctx.actor)
        || ctx.authoritative_commit_pending()
        || ctx.authoritative_queue_full()
    {
        return Err(CmdError::msg(
            "An authoritative request is already pending.",
        ));
    }
    let digest = CanonicalHash::for_canonical_value(
        "tacsim.crew_hire_input.v1",
        1,
        &json!({"actor":ctx.actor,"npc":npc,"terms":terms,"at":ctx.w.now()}),
        1024,
    )
    .map_err(|_| unavailable())?;
    let idempotency = IdempotencyKeyText::parse_canonical(format!("crew-hire/{}", digest.to_hex()))
        .map_err(|_| unavailable())?;
    ctx.submit_authoritative(
        handles.action,
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({"npc":npc.0.to_string(),"terms":terms}),
        idempotency,
    );
    Ok(())
}
