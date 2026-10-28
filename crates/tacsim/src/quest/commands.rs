//! Private offers and journal reads; advancement requires the player's command.
use std::sync::Arc;

use mush_core::{
    AbilityCall, ArgShape, CmdError, CommandCompatibility, CommandSpec, Ctx, InterfaceAbility,
    InterfaceAbilityProvider, InterfaceError, InterfaceReply, ModError, ObjId, Registrar, World,
};
use serde_json::{Value, json};

use super::{
    available, close, definitions, detail,
    model::{
        self, CheckEvidence, ClosureKind, Objective, Prerequisite, QuestDef, QuestKey, QuestRecord,
        Stamp,
    },
    paid, store,
};
use crate::port::passenger::store as passengers;

const UNAVAILABLE: &str = "No optional quest is available for you here right now.";
const OFFER_SECONDS: i64 = 600;
const HELP: &str = "+quest [<giver>[=<quest-key>]] -- your private quest journal.\n+quest/offer <giver>[=<quest-key>] -- ask a present acquaintance about available favors.\n+quest/accept <giver>=<token> -- explicitly accept one offered favor/path.\n+quest/decline <giver>=<token> -- permanently decline that offer, without a penalty or reward.\n+quest/check <giver>[=<quest-key>] -- verify the next objective at its named location.\n+quest/finish <giver>[=<quest-key>] -- return to the giver to compare notes.\n+quest/abandon <giver>[=<quest-key>] -- close an accepted quest from anywhere, without a penalty or reward.\nUse the quest-key shown in your journal when a giver has several active favors. Declined and abandoned quests cannot be restarted; accepting a path keeps that choice even if abandoned. Conversation can explain an offer; only these commands advance your journal. Offers expire after ten minutes.";

pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.interface_provider("quest", Arc::new(QuestInterface))?;
    reg.classify_commands(CommandCompatibility::Authoritative);
    reg.command(
        CommandSpec {
            name: "+quest".into(),
            shape: ArgShape::EqPair,
            switches: ["offer", "accept", "check", "finish", "decline", "abandon"]
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

fn own_rows(ctx: &Ctx<'_>, npc: ObjId, read: Read) -> Result<Vec<QuestRecord>, String> {
    Ok(rows(ctx, read)?
        .into_iter()
        .filter(|r| r.npc == npc && r.actor == ctx.actor)
        .collect())
}

fn selected(
    ctx: &Ctx<'_>,
    npc: ObjId,
    selector: Option<&str>,
    switch: &str,
) -> Result<QuestRecord, String> {
    let own = own_rows(ctx, npc, Read::Current)?;
    let candidates: Vec<_> = if let Some(key) = selector {
        own.into_iter()
            .filter(|r| r.definition.key == key)
            .collect()
    } else {
        let active: Vec<_> = own.iter().filter(|r| r.is_active()).cloned().collect();
        if active.is_empty() { own } else { active }
    };
    match candidates.as_slice() {
        [row] => Ok(row.clone()),
        [] => Err(
            "Accept this favor before recording progress. Use +quest to read your journal.".into(),
        ),
        _ => Err(format!(
            "This giver has several quests in your journal. Choose one explicitly:\n{}",
            candidates
                .iter()
                .map(|r| format!(
                    "{}: +quest/{switch} #{}={}",
                    r.definition.title, npc.0, r.definition.key
                ))
                .collect::<Vec<_>>()
                .join("\n")
        )),
    }
}

struct Basis {
    definition: QuestDef,
    passage: Option<passengers::Booking>,
    prerequisite_receipt: Option<QuestKey>,
}

fn basis_for(
    ctx: &Ctx<'_>,
    npc: ObjId,
    def: QuestDef,
    own: &[QuestRecord],
    read: Read,
) -> Result<Basis, String> {
    if !crate::port::passenger::at_station(ctx.w, npc, &def.destination) || !available(ctx.w, &def)
    {
        return Err(UNAVAILABLE.into());
    }
    let passage = if def.prerequisite == Prerequisite::PaidPassage {
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
    if own.iter().any(|r| {
        r.key.source == def.key
            && match &passage {
                Some(booking) => r.key.booking == Some(booking.id),
                None => r.key.actor == Some(ctx.actor),
            }
    }) {
        return Err("This favor is already in your journal. Use +quest to read it.".into());
    }
    if let Some(choice) = &def.choice
        && own.iter().any(|r| {
            !r.closed
                .as_ref()
                .is_some_and(|closed| closed.kind == ClosureKind::Declined)
                && r.definition
                    .choice
                    .as_ref()
                    .is_some_and(|taken| taken.group == choice.group && taken.path != choice.path)
        })
    {
        return Err(
            "You already chose another path for this follow-up. That choice remains recorded."
                .into(),
        );
    }
    let prerequisite_receipt = match &def.prerequisite {
        Prerequisite::Completed { quest } => Some(
            own.iter()
                .find(|row| {
                    row.key.source == *quest && row.completed.is_some() && row.closed.is_none()
                })
                .ok_or(UNAVAILABLE)?
                .key
                .clone(),
        ),
        _ => None,
    };
    Ok(Basis {
        definition: def,
        passage,
        prerequisite_receipt,
    })
}

fn bases(
    ctx: &Ctx<'_>,
    npc: ObjId,
    selector: Option<&str>,
    read: Read,
) -> Result<Vec<Basis>, String> {
    if !close(ctx, npc) {
        return Err(UNAVAILABLE.into());
    }
    let own: Vec<_> = rows(ctx, read)?
        .into_iter()
        .filter(|row| row.actor == ctx.actor)
        .collect();
    let defs = definitions(ctx.w, npc);
    if let Some(key) = selector {
        let def = defs
            .into_iter()
            .find(|def| def.key == key)
            .ok_or(UNAVAILABLE)?;
        return basis_for(ctx, npc, def, &own, read).map(|basis| vec![basis]);
    }
    let eligible: Vec<_> = defs
        .into_iter()
        .filter_map(|def| basis_for(ctx, npc, def, &own, read).ok())
        .collect();
    if eligible.is_empty() {
        Err(UNAVAILABLE.into())
    } else {
        Ok(eligible)
    }
}

fn digest(basis: &Basis, actor: ObjId, until: i64) -> Result<String, String> {
    mush_core::CanonicalHash::for_canonical_value(
        "tacsim.quest_offer.v1",
        1,
        &json!([
            basis.definition,
            basis.passage,
            basis.prerequisite_receipt,
            actor,
            until
        ]),
        32_768,
    )
    .map(|hash| hash.to_hex())
    .map_err(|_| UNAVAILABLE.into())
}

fn offer_one(ctx: &Ctx<'_>, npc: ObjId, basis: &Basis) -> Result<String, String> {
    let def = &basis.definition;
    let until = ctx.w.now().checked_add(OFFER_SECONDS).ok_or(UNAVAILABLE)?;
    let hash = digest(basis, ctx.actor, until)?;
    let steps = def
        .checkpoints
        .iter()
        .map(|c| format!("{}: {}", c.title, c.instructions))
        .collect::<Vec<_>>()
        .join("\n");
    let choice = def.choice.as_ref().map(|choice| format!("\nPath: {}. Accepting records this choice and rules out the alternative path, even if you later abandon it. Declining this offer leaves the alternative available.", choice.path)).unwrap_or_default();
    Ok(format!(
        "{}\n{}\n{}{}\nReturn to me afterward to compare notes. Completion bonus: +10 trust and +5 disposition with me, once per quest; no credit payment. All activities are available without accepting this optional quest.\nTo accept explicitly:\n+quest/accept #{}={until}.{hash}\nTo decline this quest permanently, without a penalty or reward:\n+quest/decline #{}={until}.{hash}",
        def.title, def.introduction, steps, choice, npc.0, npc.0
    ))
}

fn offer(ctx: &Ctx<'_>, npc: ObjId, selector: Option<&str>, read: Read) -> Result<String, String> {
    bases(ctx, npc, selector, read)?
        .iter()
        .map(|basis| offer_one(ctx, npc, basis))
        .collect::<Result<Vec<_>, _>>()
        .map(|offers| offers.join("\n\n"))
}

fn journal(ctx: &Ctx<'_>, row: &QuestRecord) -> String {
    let giver = crate::port::coop::name_of(ctx.w, row.npc);
    let path = row
        .definition
        .choice
        .as_ref()
        .map(|choice| format!("\nChosen path: {}.", choice.path))
        .unwrap_or_default();
    if let Some(closed) = &row.closed {
        let state = match closed.kind {
            ClosureKind::Declined => "declined",
            ClosureKind::Abandoned => "abandoned",
        };
        let path = if closed.kind == ClosureKind::Abandoned {
            path
        } else {
            String::new()
        };
        return format!(
            "{} — {state}; closed without a quest reward or penalty. This quest cannot be restarted.{path}\nQuest key: {}",
            row.definition.title, row.definition.key
        );
    }
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
            "{} — completed with {giver}.\n{giver}'s account: {}{path}{notes}\nQuest key: {}",
            row.definition.title, row.definition.completion, row.definition.key
        );
    }
    let progress = match row.definition.checkpoints.get(row.checks.len()) {
        Some(c) => format!(
            "{} — {giver}\nNext: {}. {}\nAt that location: +quest/check #{}={}",
            row.definition.title, c.title, c.instructions, row.npc.0, row.definition.key
        ),
        None => format!(
            "{} — observations recorded. Return to {giver} and use +quest/finish #{}={} to compare notes.",
            row.definition.title, row.npc.0, row.definition.key
        ),
    };
    format!(
        "{progress}{path}{notes}\nTo close this quest permanently without a reward or penalty: +quest/abandon #{}={}\nQuest key: {}",
        row.npc.0, row.definition.key, row.definition.key
    )
}

fn status(ctx: &Ctx<'_>, npc: ObjId, selector: Option<&str>, read: Read) -> Result<String, String> {
    let mut parts: Vec<_> = own_rows(ctx, npc, read)?
        .iter()
        .filter(|row| selector.is_none_or(|key| row.definition.key == key))
        .map(|row| journal(ctx, row))
        .collect();
    if let Ok(offers) = bases(ctx, npc, selector, read) {
        for basis in offers {
            parts.push(format!(
                "Optional favor available: {}. Use +quest/offer #{}={} or ask me about it.",
                basis.definition.title, npc.0, basis.definition.key
            ));
        }
    }
    if parts.is_empty() {
        Err(UNAVAILABLE.into())
    } else {
        Ok(parts.join("\n\n"))
    }
}

fn accept(ctx: &mut Ctx<'_>, npc: ObjId, token: &str, decline: bool) -> Result<String, String> {
    let (until, hash) = token
        .split_once('.')
        .ok_or("Use the exact token from +quest/offer.")?;
    let until: i64 = until
        .parse()
        .map_err(|_| "Use the exact token from +quest/offer.")?;
    let stale = "This offer expired or its terms changed. Ask for a fresh +quest/offer.";
    if until < ctx.w.now() || until > ctx.w.now().saturating_add(OFFER_SECONDS) {
        return Err(stale.into());
    }
    let basis = bases(ctx, npc, None, Read::Current)?
        .into_iter()
        .find(|basis| digest(basis, ctx.actor, until).is_ok_and(|candidate| candidate == hash));
    let Some(Basis {
        definition: def,
        passage,
        prerequisite_receipt,
    }) = basis
    else {
        return Err(stale.into());
    };
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
        prerequisite_receipt,
        definition: def,
        accepted: Stamp::now(ctx.w),
        checks: Vec::new(),
        completed: None,
        closed: None,
    };
    let title = row.definition.title.clone();
    if decline {
        store::stage_decline(ctx, row)?;
        return Ok(format!(
            "Decline of {title} submitted. This quest will be closed without a reward or penalty. Use +quest to read the committed journal."
        ));
    }
    store::stage_accept(ctx, row.clone())?;
    // Prepare capture before the next market event. This watch grants nothing;
    // both evidence and the typed accepted row must match before any progress.
    // The tick retries after interrupted acceptance or a cold owner service.
    let _ = super::evidence::watch(ctx, &row);
    Ok(format!(
        "Acceptance of {title} submitted. Use +quest to read the committed journal and next objective."
    ))
}

fn check(ctx: &mut Ctx<'_>, npc: ObjId, selector: Option<&str>) -> Result<String, String> {
    let row = selected(ctx, npc, selector, "check")?;
    if !row.is_active() {
        return Err(
            "This favor is already complete or closed. Use +quest to read your journal.".into(),
        );
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

fn finish(ctx: &mut Ctx<'_>, npc: ObjId, selector: Option<&str>) -> Result<String, String> {
    if !close(ctx, npc) {
        return Err("Return to the giver to compare notes in person.".into());
    }
    let row = selected(ctx, npc, selector, "finish")?;
    if row.completed.is_some() {
        return Ok("You already completed this favor together.".into());
    }
    if row.closed.is_some() {
        return Err("This favor is closed and cannot be finished or restarted.".into());
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

fn abandon(ctx: &mut Ctx<'_>, npc: ObjId, selector: Option<&str>) -> Result<String, String> {
    let row = selected(ctx, npc, selector, "abandon")?;
    if !row.is_active() {
        return Err("This favor is already complete or closed.".into());
    }
    store::stage_abandon(ctx, &row)?;
    Ok(format!(
        "Abandonment of {} submitted. This quest cannot be restarted and no quest reward or penalty will be given. Any chosen path remains recorded. Use +quest to read the committed journal.",
        row.definition.title
    ))
}

fn resolve(ctx: &Ctx<'_>, name: &str) -> Result<ObjId, String> {
    if name.is_empty() {
        return Err(HELP.into());
    }
    if let Ok(npc) = mush_core::match_object(ctx.w, ctx.actor, name)
        && !definitions(ctx.w, npc).is_empty()
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
    let switches: Vec<_> = ["offer", "accept", "check", "finish", "decline", "abandon"]
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
    let text = match (switches.first().copied(), rhs.as_deref().map(str::trim)) {
        (None, selector) => status(ctx, npc, selector, Read::Current),
        (Some("offer"), selector) => offer(ctx, npc, selector, Read::Current),
        (Some("accept"), Some(token)) => accept(ctx, npc, token, false),
        (Some("decline"), Some(token)) => accept(ctx, npc, token, true),
        (Some("check"), selector) => check(ctx, npc, selector),
        (Some("finish"), selector) => finish(ctx, npc, selector),
        (Some("abandon"), selector) => abandon(ctx, npc, selector),
        _ => Err(HELP.into()),
    }
    .map_err(CmdError::msg)?;
    ctx.out.info(ctx.actor, text);
    Ok(())
}

struct QuestInterface;

/// InterfaceReply truncates oversized strings. Keep complete commands and
/// consent terms instead: large journals/catalogs link to the full command
/// view, and the model can request one exact quest for a smaller response.
fn conversation_reply(
    ctx: &Ctx<'_>,
    npc: ObjId,
    selector: Option<&str>,
    offering: bool,
    full: String,
) -> Result<String, String> {
    if full.len() <= mush_core::MAX_INTERFACE_REPLY_BYTES {
        return Ok(full);
    }
    let mut parts = Vec::new();
    if !offering {
        for row in own_rows(ctx, npc, Read::Conversation)?
            .into_iter()
            .filter(|row| selector.is_none_or(|key| row.definition.key == key))
        {
            let state = match &row.closed {
                Some(closed) if closed.kind == ClosureKind::Declined => "declined",
                Some(_) => "abandoned",
                None if row.completed.is_some() => "completed",
                None => "active",
            };
            let outcome = if row.completed.is_some() {
                format!(" Giver's account: {}", row.definition.completion)
            } else {
                String::new()
            };
            let path = row
                .definition
                .choice
                .as_ref()
                .filter(|_| {
                    !row.closed
                        .as_ref()
                        .is_some_and(|closed| closed.kind == ClosureKind::Declined)
                })
                .map(|choice| format!(" Chosen path: {}.", choice.path))
                .unwrap_or_default();
            parts.push(format!(
                "{}: {state}.{path}{outcome}\nRead: +quest #{}={}",
                row.definition.title, npc.0, row.definition.key
            ));
        }
    }
    if let Ok(offers) = bases(ctx, npc, selector, Read::Conversation) {
        for basis in offers {
            let def = basis.definition;
            let path = def
                .choice
                .as_ref()
                .map(|choice| format!(" (path: {})", choice.path))
                .unwrap_or_default();
            parts.push(format!("Available: {}{path}\nRead the full offer: +quest/offer #{}={}\nOr call quest.offer with quest=\"{}\".", def.title, npc.0, def.key, def.key));
        }
    }
    Ok(compact_reply(npc, parts))
}

fn compact_reply(npc: ObjId, parts: Vec<String>) -> String {
    let mut text = "Private quest summary. Accepting a path fixes that choice even if abandoned. Declining leaves alternatives available. Declined or abandoned quests cannot restart and give no quest reward or penalty. Ordinary services remain independent.\n".to_owned();
    let footer = format!(
        "\nFull details: +quest #{}. Read a full offer before accepting or declining it; use its exact token. For one quest, call quest.status or quest.offer with its exact quest key.",
        npc.0
    );
    for part in parts {
        if text.len() + part.len() + 1 + footer.len() > mush_core::MAX_INTERFACE_REPLY_BYTES {
            break;
        }
        text.push_str(&part);
        text.push('\n');
    }
    text.push_str(&footer);
    text
}

impl InterfaceAbilityProvider for QuestInterface {
    fn abilities(&self) -> Vec<InterfaceAbility> {
        [ ("quest.status", "Read only the current asker's verified quest situation. Do not reveal another person's journal or invent progress."),
          ("quest.offer", "Read eligible authored favors for the current asker. Supply an exact quest key to select one. Returns explicit acceptance instructions or a command to read full terms when the catalog is large. Never chooses, accepts, declines, abandons, checks, finishes or grants rewards.") ]
            .into_iter().map(|(name, description)| InterfaceAbility { name: name.into(), command: "+quest".into(), description: description.into(),
                params_schema: json!({"type":"object","properties":{"quest":{"type":"string","description":"Optional exact quest key returned by this service."}},"additionalProperties":false}) }).collect()
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
        if ctx.actor != call.asker
            || !close(ctx, call.npc)
            || definitions(ctx.w, call.npc).is_empty()
        {
            return Err(InterfaceError::Refused(UNAVAILABLE.into()));
        }
        if !input.as_object().is_some_and(|o| {
            o.iter().all(|(key, value)| {
                key == "quest"
                    && value
                        .as_str()
                        .is_some_and(|key| !key.is_empty() && key.len() <= 160)
            })
        }) {
            return Err(InterfaceError::Invalid(
                "Use an empty object or an exact quest string; the service supplies the current asker.".into(),
            ));
        }
        let selector = input.get("quest").and_then(Value::as_str);
        let text = if name == "quest.offer" {
            offer(ctx, call.npc, selector, Read::Conversation)
        } else {
            status(ctx, call.npc, selector, Read::Conversation)
        }
        .map_err(InterfaceError::Refused)?;
        let text = conversation_reply(ctx, call.npc, selector, name == "quest.offer", text)
            .map_err(InterfaceError::Refused)?;
        Ok(InterfaceReply::text(text))
    }

    fn npc_context(&self, w: &World, npc: ObjId) -> Option<String> {
        if definitions(w, npc).is_empty() {
            return None;
        }
        Some("You may offer optional authored quests and follow-ups. Use quest.status or quest.offer for the current asker's verified availability, saved path, instructions, closure and bonus. Let conversation lead into it and respect refusal. Completed work and chosen paths are reliable service facts even when your conversational recall is imperfect. The ordinary services work without any quest or prescribed order; explain those commands when useful. Replies are private. Never infer acceptance, choices, work, payment or completion from dialogue. Only returned +quest commands advance or close the journal. Acceptance fixes a path; declining leaves alternatives open; abandonment closes the quest without rewards or penalties and keeps the chosen path. Never promise a restart, another quest or another reward.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_conversation_summaries_preserve_whole_commands_and_closure_terms() {
        let parts: Vec<_> = (0..model::MAX_QUESTS)
            .map(|n| format!("{}\nRead: +quest #42=example/quest.{n}", "é".repeat(128)))
            .collect();
        let reply = compact_reply(ObjId(42), parts);
        assert!(reply.len() <= mush_core::MAX_INTERFACE_REPLY_BYTES);
        assert!(reply.contains("Declined or abandoned quests cannot restart"));
        assert!(reply.contains("Read: +quest #42=example/quest.0\n"));
        assert!(reply.contains("Full details: +quest #42."));
        assert_eq!(InterfaceReply::text(reply.clone()).text, reply);
    }
}
