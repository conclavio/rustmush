//! Private offers and journal reads; advancement requires the player's command.
use std::sync::Arc;

use mush_core::{
    AbilityCall, ArgShape, CmdError, CommandCompatibility, CommandSpec, Ctx, InterfaceAbility,
    InterfaceAbilityProvider, InterfaceError, InterfaceReply, ModError, ObjId, Registrar, World,
};
use serde_json::{Value, json};

use super::{
    available, close, definition, detail,
    model::{self, CheckEvidence, Objective, Prerequisite, QuestDef, QuestKey, QuestRecord, Stamp},
    paid, store,
};
use crate::port::passenger::store as passengers;

const UNAVAILABLE: &str = "No optional quest is available for you here right now.";
const OFFER_SECONDS: i64 = 600;
const HELP: &str = "+quest [<giver>] -- your private quest journal.\n+quest/offer <giver> -- ask a present acquaintance about a favor.\n+quest/accept <giver>=<token> -- explicitly accept the offered favor.\n+quest/check <giver> -- verify the next objective at its named location.\n+quest/finish <giver> -- return to the giver to compare notes.\nConversation can explain an offer; only these commands advance your journal. Offers expire after ten minutes.";

pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.interface_provider("quest", Arc::new(QuestInterface))?;
    reg.classify_commands(CommandCompatibility::Authoritative);
    reg.command(
        CommandSpec {
            name: "+quest".into(),
            shape: ArgShape::EqPair,
            switches: ["offer", "accept", "check", "finish"]
                .map(Into::into)
                .to_vec(),
            help: HELP.into(),
            ..CommandSpec::default()
        },
        Arc::new(command),
    )?;
    reg.classify_commands(CommandCompatibility::LegacyPostImage);
    Ok(())
}

#[derive(Clone, Copy)]
enum Read {
    Current,
    Conversation,
}

fn rows(ctx: &Ctx<'_>, read: Read) -> Result<Vec<QuestRecord>, String> {
    match read {
        Read::Current => store::list(ctx),
        Read::Conversation if store::projection_ready_for(ctx.w, ctx.actor) => {
            Ok(store::list_world(ctx.w))
        }
        Read::Conversation => Err("My journal is still loading. Please ask again shortly.".into()),
    }
}

fn active(ctx: &Ctx<'_>, npc: ObjId, read: Read) -> Result<Option<QuestRecord>, String> {
    let own: Vec<_> = rows(ctx, read)?
        .into_iter()
        .filter(|r| r.npc == npc && r.actor == ctx.actor)
        .collect();
    Ok(own
        .iter()
        .find(|r| r.completed.is_none())
        .or_else(|| own.last())
        .cloned())
}

fn basis(
    ctx: &Ctx<'_>,
    npc: ObjId,
    read: Read,
) -> Result<(QuestDef, Option<passengers::Booking>), String> {
    if !close(ctx, npc) {
        return Err(UNAVAILABLE.into());
    }
    let def = definition(ctx.w, npc).ok_or(UNAVAILABLE)?;
    if !crate::port::passenger::at_station(ctx.w, npc, &def.destination) {
        return Err(UNAVAILABLE.into());
    }
    let booking = if def.prerequisite == Prerequisite::PaidPassage {
        let bookings = match read {
            Read::Current => passengers::list(ctx)?,
            Read::Conversation if passengers::projection_ready(ctx.w) => {
                passengers::list_world(ctx.w)
            }
            Read::Conversation => {
                return Err("My passage record is still loading. Please ask again shortly.".into());
            }
        };
        Some(
            bookings
                .into_iter()
                .find(|b| paid(ctx.w, b, ctx.actor, npc, &def))
                .ok_or(UNAVAILABLE)?,
        )
    } else {
        None
    };
    if rows(ctx, read)?.iter().any(|r| {
        r.key.source == def.key
            && match &booking {
                Some(booking) => r.key.booking == Some(booking.id),
                None => r.key.actor == Some(ctx.actor),
            }
    }) {
        return Err("You already accepted this favor. Use +quest to read your journal.".into());
    }
    if !available(ctx.w, &def) {
        return Err(UNAVAILABLE.into());
    }
    Ok((def, booking))
}

fn digest(
    def: &QuestDef,
    passage: &Option<passengers::Booking>,
    actor: ObjId,
    until: i64,
) -> Result<String, String> {
    mush_core::CanonicalHash::for_canonical_value(
        "tacsim.quest_offer.v1",
        1,
        &json!([def, passage, actor, until]),
        32_768,
    )
    .map(|hash| hash.to_hex())
    .map_err(|_| UNAVAILABLE.into())
}

fn offer(ctx: &Ctx<'_>, npc: ObjId, read: Read) -> Result<String, String> {
    let (def, passage) = basis(ctx, npc, read)?;
    let until = ctx.w.now().checked_add(OFFER_SECONDS).ok_or(UNAVAILABLE)?;
    let hash = digest(&def, &passage, ctx.actor, until)?;
    let steps = def
        .checkpoints
        .iter()
        .map(|c| format!("{}: {}", c.title, c.instructions))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(format!(
        "{}\n{}\n{}\nReturn to me afterward to compare notes. Completion bonus: +10 trust and +5 disposition with me, once per quest; no credit payment. All activities are available without accepting this optional quest.\nTo accept explicitly:\n+quest/accept #{}={until}.{hash}",
        def.title, def.introduction, steps, npc.0
    ))
}

fn journal(ctx: &Ctx<'_>, row: &QuestRecord) -> String {
    let giver = crate::port::coop::name_of(ctx.w, row.npc);
    let notes = row
        .checks
        .iter()
        .map(|c| {
            format!(
                "\nRecorded observation ({}): {}",
                c.checkpoint, c.detail_text
            )
        })
        .collect::<String>();
    if row.completed.is_some() {
        return format!(
            "{} — completed with {giver}.\n{giver}'s account: {}{notes}",
            row.definition.title, row.definition.completion
        );
    }
    let progress = match row.definition.checkpoints.get(row.checks.len()) {
        Some(c) => format!(
            "{} — {giver}\nNext: {}. {}\nAt that location: +quest/check #{}",
            row.definition.title, c.title, c.instructions, row.npc.0
        ),
        None => format!(
            "{} — observations recorded. Return to {giver} and use +quest/finish #{} to compare notes.",
            row.definition.title, row.npc.0
        ),
    };
    format!("{progress}{notes}")
}

fn status(ctx: &Ctx<'_>, npc: ObjId, read: Read) -> Result<String, String> {
    if let Some(row) = active(ctx, npc, read)? {
        return Ok(journal(ctx, &row));
    }
    basis(ctx, npc, read)?;
    Ok(format!(
        "I have a small favor to ask, if you're interested. Use +quest/offer #{} or ask me about it.",
        npc.0
    ))
}

fn accept(ctx: &mut Ctx<'_>, npc: ObjId, token: &str) -> Result<String, String> {
    let (def, passage) = basis(ctx, npc, Read::Current)?;
    let (until, hash) = token
        .split_once('.')
        .ok_or("Use the exact token from +quest/offer.")?;
    let until: i64 = until
        .parse()
        .map_err(|_| "Use the exact token from +quest/offer.")?;
    if until < ctx.w.now()
        || until > ctx.w.now().saturating_add(OFFER_SECONDS)
        || digest(&def, &passage, ctx.actor, until)? != hash
    {
        return Err(
            "This offer expired or its terms changed. Ask for a fresh +quest/offer.".into(),
        );
    }
    let row = QuestRecord {
        schema: 1,
        key: QuestKey {
            source: def.key.clone(),
            booking: passage.as_ref().map(|passage| passage.id),
            actor: if passage.is_none() {
                Some(ctx.actor)
            } else {
                None
            },
        },
        revision: 1,
        actor: ctx.actor,
        npc,
        passage,
        definition: def,
        accepted: Stamp::now(ctx.w),
        checks: Vec::new(),
        completed: None,
    };
    let title = row.definition.title.clone();
    store::stage_accept(ctx, row.clone())?;
    // Prepare capture before the next market event. This watch grants nothing;
    // both evidence and the typed accepted row must match before any progress.
    // The tick retries after interrupted acceptance or a cold owner service.
    let _ = super::evidence::watch(ctx, &row);
    Ok(format!(
        "Acceptance of {title} submitted. Use +quest to read the committed journal and next objective."
    ))
}

fn check(ctx: &mut Ctx<'_>, npc: ObjId) -> Result<String, String> {
    let row = active(ctx, npc, Read::Current)?
        .ok_or("Accept this favor before recording an observation.")?;
    if row.completed.is_some() {
        return Err("This favor is already complete.".into());
    }
    let checkpoint = row.definition.checkpoints.get(row.checks.len()).ok_or(
        "Your observations are already recorded. Return to the giver and use +quest/finish.",
    )?;
    if ctx.w.get(ctx.actor).and_then(|p| p.location) != Some(checkpoint.room)
        || !ctx.can_perceive(ctx.actor, checkpoint.room)
    {
        return Err(format!(
            "Go to {} first. {}",
            checkpoint.title, checkpoint.instructions
        ));
    }
    if !matches!(checkpoint.objective, Objective::Observation) {
        let evidence = super::evidence::capture(ctx, &row, checkpoint)?;
        let text = evidence.detail_text.clone();
        store::stage_check(ctx, &row, evidence)?;
        return Ok(format!(
            "Verified: {text}\nObjective submitted; use +quest for the next step."
        ));
    }
    let text = detail(ctx.w, checkpoint)
        .ok_or("The reference detail is unavailable; no observation was recorded.")?;
    let evidence = CheckEvidence {
        checkpoint: checkpoint.id.clone(),
        room: checkpoint.room,
        detail_hash: model::detail_hash(&text)?,
        detail_text: text.clone(),
        at: Stamp::now(ctx.w),
        proof: None,
    };
    store::stage_check(ctx, &row, evidence)?;
    Ok(format!(
        "You check the {}: {text}\nObservation submitted; use +quest for the next step.",
        checkpoint.detail
    ))
}

fn finish(ctx: &mut Ctx<'_>, npc: ObjId) -> Result<String, String> {
    if !close(ctx, npc) {
        return Err("Return to the giver to compare notes in person.".into());
    }
    let row = active(ctx, npc, Read::Current)?.ok_or(UNAVAILABLE)?;
    if row.completed.is_some() {
        return Ok("You already completed this favor together.".into());
    }
    if row.checks.len() != row.definition.checkpoints.len() {
        return Err(
            "There are still observations to make. Use +quest for the next objective.".into(),
        );
    }
    if !super::evidence::completion_ready(ctx.w, &row) {
        return Err("The original trade payments are unavailable or were reversed. Ask again after the receipts finish loading.".into());
    }
    store::stage_finish(ctx, &row)?;
    Ok(format!(
        "You compare notes. Completion of {} submitted; your shared experience will be recorded once.",
        row.definition.title
    ))
}

fn resolve(ctx: &Ctx<'_>, name: &str) -> Result<ObjId, String> {
    if name.is_empty() {
        return Err(HELP.into());
    }
    if let Ok(npc) = mush_core::match_object(ctx.w, ctx.actor, name)
        && definition(ctx.w, npc).is_some()
        && close(ctx, npc)
    {
        return Ok(npc);
    }
    let name = name.to_lowercase();
    let mut candidates: Vec<_> = store::list(ctx)?
        .into_iter()
        .filter(|r| r.actor == ctx.actor)
        .filter(|r| {
            name == format!("#{}", r.npc.0)
                || ctx
                    .w
                    .get(r.npc)
                    .is_some_and(|o| o.name.to_lowercase().starts_with(&name))
        })
        .map(|r| r.npc)
        .collect();
    candidates.sort();
    candidates.dedup();
    match candidates.as_slice() {
        [npc] => Ok(*npc),
        _ => Err(UNAVAILABLE.into()),
    }
}

fn command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let _ = passengers::refresh(ctx); // Paid prerequisites perform their own fallible read.
    store::refresh(ctx).map_err(CmdError::msg)?;
    let lhs = ctx.args.lhs().trim().to_owned();
    let rhs = ctx.args.rhs().map(str::to_owned);
    let switches: Vec<_> = ["offer", "accept", "check", "finish"]
        .into_iter()
        .filter(|s| ctx.switches.has(s))
        .collect();
    if switches.len() > 1 {
        return Err(CmdError::msg(HELP));
    }
    if lhs.is_empty() && switches.is_empty() && rhs.is_none() {
        let own: Vec<_> = store::list(ctx)
            .map_err(CmdError::msg)?
            .into_iter()
            .filter(|r| r.actor == ctx.actor)
            .collect();
        if own.is_empty() {
            ctx.out.info(
                ctx.actor,
                "Your quest journal is empty. Talk to acquaintances to discover favors.",
            );
        }
        for row in own {
            ctx.out.info(ctx.actor, journal(ctx, &row));
        }
        return Ok(());
    }
    let npc = resolve(ctx, &lhs).map_err(CmdError::msg)?;
    let text = match (switches.first().copied(), rhs.as_deref()) {
        (None, None) => status(ctx, npc, Read::Current),
        (Some("offer"), None) => offer(ctx, npc, Read::Current),
        (Some("accept"), Some(token)) => accept(ctx, npc, token),
        (Some("check"), None) => check(ctx, npc),
        (Some("finish"), None) => finish(ctx, npc),
        _ => Err(HELP.into()),
    }
    .map_err(CmdError::msg)?;
    ctx.out.info(ctx.actor, text);
    Ok(())
}

struct QuestInterface;
impl InterfaceAbilityProvider for QuestInterface {
    fn abilities(&self) -> Vec<InterfaceAbility> {
        [ ("quest.status", "Read only the current asker's verified quest situation. Do not reveal another person's journal or invent progress."),
          ("quest.offer", "Offer the authored favor to an eligible current asker. Returns explicit acceptance instructions. Never accepts, checks an objective, finishes or grants a reward.") ]
            .into_iter().map(|(name, description)| InterfaceAbility { name: name.into(), command: "+quest".into(), description: description.into(),
                params_schema: json!({"type":"object","properties":{},"additionalProperties":false}) }).collect()
    }

    fn invoke(
        &self,
        ctx: &mut Ctx<'_>,
        call: AbilityCall,
        name: &str,
        input: &Value,
    ) -> Result<InterfaceReply, InterfaceError> {
        if !matches!(name, "quest.status" | "quest.offer") {
            return Err(InterfaceError::Unknown(name.into()));
        }
        if ctx.actor != call.asker || !close(ctx, call.npc) || definition(ctx.w, call.npc).is_none()
        {
            return Err(InterfaceError::Refused(UNAVAILABLE.into()));
        }
        if !input.as_object().is_some_and(|o| o.is_empty()) {
            return Err(InterfaceError::Invalid(
                "Use an empty object; the service supplies the current asker.".into(),
            ));
        }
        let text = if name == "quest.offer" {
            offer(ctx, call.npc, Read::Conversation)
        } else {
            status(ctx, call.npc, Read::Conversation)
        }
        .map_err(InterfaceError::Refused)?;
        Ok(InterfaceReply::text(text))
    }

    fn npc_context(&self, w: &World, npc: ObjId) -> Option<String> {
        definition(w, npc)?;
        Some("You may offer an optional authored quest. Use quest.status or quest.offer for the current asker's verified availability, instructions and bonus. Let conversation lead into it and respect refusal. The ordinary services work without any quest or prescribed order; explain those commands when useful. Replies are private. Never infer acceptance, work, payment or completion from dialogue. Only returned +quest commands advance the journal; no other quest or reward is promised.".into())
    }
}
