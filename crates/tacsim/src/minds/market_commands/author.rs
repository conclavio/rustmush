//! 17g C2a (C2 plan §2.2): the author's commands. `@mind/rate` starts the
//! existing Registry rating by name; `@mind/file` (reserve, then file),
//! `@mind/update`, `@mind/withdraw`, `@mind/attest` and the paid `@mind/fork`
//! each submit exactly one existing Roll legal action with parameters the
//! server builds from the actor's own state. `@mind/localcopy` stays the
//! local draft copy; `@mind/publish` is the market's (17g C2c,
//! `market::publish`), which reads the filed mind through `own_filed` here.
//!
//! Every handler parses strictly first (a malformed argument prints the
//! command's `HELP` line), then refuses `NOT_OPEN` without the player market,
//! then reads only the actor's own state. Numbers are decimal `u64`, nonzero,
//! with no sign, padding or other characters.

use mush_core::{CmdError, Ctx};
use mush_roll::commerce::command_actions::RollCommandActions;
use mush_roll::commerce::command_reader::{
    OwnFiledView, RollCommandReadRequest, RollCommandReadResult,
};
use mush_roll::commerce::holding_reader::{OriginalReadRequest, OriginalReadResult};
use mush_roll::commerce::{FiledRevisionId, LicenceId};
use serde_json::json;

use super::words::{self, Outcome, Refusal, fill, word};
use crate::minds::workshop::{MindRef, saved_revision};

/// The `@mind` switches this module serves.
pub(crate) const SWITCHES: [&str; 6] = ["rate", "file", "update", "withdraw", "attest", "fork"];

/// The usage lines: exactly the `@mind` `HELP` lines that name these commands.
pub(crate) const USAGE_RATE_FILE: &str = "@mind/rate <name>@<rev>; @mind/file <name>@<rev>[=<charter name>] (Registry rating, then filing)";
pub(crate) const USAGE_FILED: &str =
    "@mind/update <filed> <name>@<rev>; @mind/withdraw <filed>; @mind/attest <filed> [org=<org>]";
pub(crate) const USAGE_FORK: &str =
    "@mind/fork <licence>=<name> (from an Open licence; @mind/localcopy copies your own drafts)";

/// `MAX_CHARTER_NAME_BYTES` (80): the filing's own charter-name bound.
const MAX_CHARTER_NAME: usize = mush_roll::commerce::records::MAX_CHARTER_NAME_BYTES;

fn usage(line: &str) -> CmdError {
    CmdError::msg(line)
}
fn not_open() -> CmdError {
    CmdError::msg(words::NOT_OPEN)
}

/// A decimal `u64`, nonzero, with no sign, padding, leading zero or any
/// other character.
pub(crate) fn number(text: &str) -> Option<u64> {
    if text.is_empty() || text.starts_with('0') || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse::<u64>().ok().filter(|n| *n > 0)
}

/// A library reference that names a saved revision: `<name>@<rev>`.
pub(crate) fn revision_reference(text: &str) -> Option<MindRef> {
    let text = text.trim();
    if !text.contains('@') {
        return None;
    }
    MindRef::parse(text).ok()
}

/// A charter name: the `=` part trimmed, 1 to 80 bytes, no control
/// characters. An absent `=` part takes the mind's name (`None` here).
pub(crate) fn charter_name(text: Option<&str>) -> Result<Option<String>, ()> {
    let Some(text) = text else {
        return Ok(None);
    };
    let name = text.trim();
    if name.is_empty() || name.len() > MAX_CHARTER_NAME || name.chars().any(char::is_control) {
        return Err(());
    }
    Ok(Some(name.to_owned()))
}

/// `@mind/file`'s argument: `<name>@<rev>[=<charter name>]`.
pub(crate) fn file_arguments(text: &str) -> Option<(MindRef, Option<String>)> {
    let (reference, charter) = match text.split_once('=') {
        Some((reference, charter)) => (reference, Some(charter)),
        None => (text, None),
    };
    Some((revision_reference(reference)?, charter_name(charter).ok()?))
}

/// `@mind/update`'s argument: `<filed> <name>@<rev>`.
pub(crate) fn update_arguments(text: &str) -> Option<(u64, MindRef)> {
    let (filed, reference) = text.trim().split_once(' ')?;
    Some((number(filed)?, revision_reference(reference)?))
}

/// `@mind/fork`'s argument: `<licence>=<name>`, the name canonical.
pub(crate) fn fork_arguments(text: &str) -> Option<(u64, String)> {
    let (licence, name) = text.split_once('=')?;
    Some((
        number(licence.trim())?,
        mush_roll::canonical_name(name.trim()).ok()?,
    ))
}

/// `roll-reserve-mind-charter`'s parameters: none (the provider reads the
/// actor and allocates the shell).
pub(crate) fn reserve_parameters() -> serde_json::Value {
    json!({})
}

/// The filing's `RequestWire` (`mush_roll::commerce::filing`): the actor's
/// open shell, its rated revision, the rating and its personal payer.
pub(crate) fn file_parameters(
    shell: mush_core::RequestUid,
    revision: &mush_roll::RevisionRef,
    rating: &mush_roll::commerce::rating::RatingRef,
    payer: mush_economy::AccountId,
    charter: &str,
) -> serde_json::Value {
    json!({
        "shell_request": shell,
        "mind": revision.key.mind,
        "revision": revision.key.number,
        "revision_digest": revision.digest,
        "rating_job": rating.job(),
        "rating_id": rating.id(),
        "rating_digest": rating.receipt_digest(),
        "payer": payer.0,
        "charter_name": charter,
    })
}

/// The line update's `Wire` (`mush_roll::commerce::line_update`): the filed
/// mind's line and the filing's revision, rating and payer fields.
pub(crate) fn update_parameters(
    line: u64,
    revision: &mush_roll::RevisionRef,
    rating: &mush_roll::commerce::rating::RatingRef,
    payer: mush_economy::AccountId,
) -> serde_json::Value {
    json!({
        "line": line,
        "mind": revision.key.mind,
        "revision": revision.key.number,
        "revision_digest": revision.digest,
        "rating_job": rating.job(),
        "rating_id": rating.id(),
        "rating_digest": rating.receipt_digest(),
        "payer": payer.0,
    })
}

/// The withdrawal's wire: the filed revision as a decimal string.
pub(crate) fn withdraw_parameters(filed: u64) -> serde_json::Value {
    json!({"filed": filed.to_string()})
}

/// The owner attestation's wire: the owner licence as a number.
pub(crate) fn attest_parameters(owner_licence: u64) -> serde_json::Value {
    json!({"owner_licence": owner_licence})
}

/// M8c B4: the House attestation's wire: the owner licence and the
/// organisation; without one, exactly `attest_parameters`.
pub(crate) fn attest_parameters_for(
    owner_licence: u64,
    org: Option<mush_org::OrgId>,
) -> serde_json::Value {
    match org {
        None => attest_parameters(owner_licence),
        Some(org) => json!({"owner_licence": owner_licence, "org": org.0}),
    }
}

/// The paid fork's wire: the parent licence as a decimal string
/// (`LicenceId`'s encoding) and the new library name.
pub(crate) fn fork_parameters(parent_licence: u64, name: &str) -> serde_json::Value {
    json!({"parent_licence": parent_licence.to_string(), "name": name})
}

/// Dispatch one author switch. The caller (`WorkshopCommands::dispatch`) has
/// already applied the workshop's principal rule.
pub(crate) fn author(ctx: &mut Ctx<'_>, switch: &str, text: &str) -> Result<(), CmdError> {
    match switch {
        "rate" => rate(ctx, text),
        "file" => file(ctx, text),
        "update" => update(ctx, text),
        "withdraw" => withdraw(ctx, text),
        "attest" => attest(ctx, text),
        "fork" => fork(ctx, text),
        _ => Err(not_open()),
    }
}

/// The resolved own revision and its canonical `name@rev`, or `NO_REVISION`.
fn own_revision(
    ctx: &Ctx<'_>,
    reference: &MindRef,
    typed: &str,
) -> Result<(mush_roll::Revision, String), CmdError> {
    let refuse = || {
        let typed = typed
            .trim()
            .split_once('@')
            .map_or(String::new(), |(_, rev)| rev.to_owned());
        CmdError::msg(fill(
            words::NO_REVISION,
            &[("reference", &format!("{}@{typed}", reference.name()))],
        ))
    };
    let roll = ctx
        .w
        .module_state::<mush_roll::RollStore>()
        .ok_or_else(refuse)?;
    let revision = saved_revision(roll, ctx.actor, reference)
        .map_err(|_| refuse())?
        .clone();
    let shown = format!("{}@{}", reference.name(), revision.reference().key.number);
    Ok((revision, shown))
}

/// The composed Roll actions, or `NOT_OPEN`.
fn actions(ctx: &Ctx<'_>) -> Result<RollCommandActions, Refusal> {
    ctx.reg
        .definitions()
        .map(RollCommandActions::capture)
        .ok_or_else(Refusal::not_open)
}

/// The actor's own filed mind selected by `filed`, or `NO_FILED`: a foreign
/// and a missing number read alike (the Roll read refuses both alike).
/// 17g C2c: `@mind/publish` reads its filed mind through this too.
pub(super) fn own_filed(ctx: &Ctx<'_>, filed: u64) -> Result<OwnFiledView, Refusal> {
    own_filed_for(ctx, filed, None)
}

/// M8c B4: `own_filed` for a filed mind the organisation `org` owns, to a
/// character acting for it (the read re-asks the grant on the title's
/// scope); a foreign one reads `NO_FILED` exactly as before.
pub(super) fn own_filed_for(
    ctx: &Ctx<'_>,
    filed: u64,
    org: Option<mush_org::OrgId>,
) -> Result<OwnFiledView, Refusal> {
    let refuse = || word!(NO_FILED, &[("filed", &filed.to_string())]);
    let id = FiledRevisionId::new(filed).map_err(|_| refuse())?;
    let service = super::service(ctx)?;
    let request = OriginalReadRequest::command(RollCommandReadRequest::own_filed_for(id, org));
    ctx.read_owner(&service.roll, request)
        .ok()
        .and_then(OriginalReadResult::into_command)
        .and_then(RollCommandReadResult::into_own_filed)
        .ok_or_else(refuse)
}

/// The actor's own finished rating of `revision`, or `NOT_RATED`.
fn rating(
    ctx: &Ctx<'_>,
    revision: &mush_roll::Revision,
    shown: &str,
) -> Result<mush_roll::commerce::rating::RatingRef, CmdError> {
    crate::minds::bulk::registry_service::completed_rating(ctx, revision)
        .ok()
        .flatten()
        .ok_or_else(|| CmdError::msg(fill(words::NOT_RATED, &[("reference", shown)])))
}

/// `@mind/rate <name>@<rev>`: the Registry's own rating job, started by name.
/// It needs no player market (it works wherever `@registry` does).
fn rate(ctx: &mut Ctx<'_>, text: &str) -> Result<(), CmdError> {
    let reference = revision_reference(text).ok_or_else(|| usage(USAGE_RATE_FILE))?;
    let (revision, _) = own_revision(ctx, &reference, text)?;
    let key = revision.reference().key;
    let started = crate::minds::bulk::registry_service::start_revision(ctx, key.mind, key.number);
    let held = || crate::minds::bulk::registry_service::rating_held(ctx.w, ctx.actor);
    started.map_err(|error| CmdError::msg(rate_refusal(&error, held)))
}

/// `@mind/rate`'s refusal (reviewer F2): `RATING_BUSY` only when the Busy is
/// this character's own Registry job (`held`, asked only then); every other
/// error, every other Busy included (bulk recovery, an `@mind/prove` run or
/// the alt's rating on the same login, a full queue), is the existing
/// unavailable line, never a promise of a "finished" line that will not come.
pub(crate) fn rate_refusal(
    error: &crate::minds::bulk::BulkError,
    held: impl FnOnce() -> bool,
) -> &'static str {
    match error {
        crate::minds::bulk::BulkError::Busy if held() => words::RATING_BUSY,
        _ => words::RATING_UNAVAILABLE,
    }
}

/// `@mind/file <name>@<rev>[=<charter name>]`: reserve the charter shell
/// when the actor has none open, else file the rated revision with it.
fn file(ctx: &mut Ctx<'_>, text: &str) -> Result<(), CmdError> {
    let (reference, charter) = file_arguments(text).ok_or_else(|| usage(USAGE_RATE_FILE))?;
    let service = super::service(ctx)?;
    let roll = service.roll.clone();
    let actions = actions(ctx)?;
    let (revision, shown) = own_revision(ctx, &reference, text.split('=').next().unwrap_or(text))?;
    let key = revision.reference().key;
    let subject = format!("{}.{}", key.mind, key.number);
    let open = ctx
        .read_owner(
            &roll,
            OriginalReadRequest::command(RollCommandReadRequest::open_shell()),
        )
        .ok()
        .and_then(OriginalReadResult::into_command)
        .and_then(|result| result.open_shell())
        .ok_or_else(not_open)?;
    let Some(shell) = open else {
        let reserve = actions.reserve().ok_or_else(not_open)?;
        let line = fill(words::RESERVING, &[("arguments", text.trim())]);
        return super::submit(
            ctx,
            "reserve",
            &subject,
            reserve,
            reserve_parameters(),
            &line,
        );
    };
    let rated = rating(ctx, &revision, &shown)?;
    let dependent = revision.require_original_access().is_err();
    let action = actions.file(dependent).ok_or_else(not_open)?;
    let payer = super::personal_account(ctx)?;
    let charter = charter.unwrap_or_else(|| reference.name().to_owned());
    let parameters = file_parameters(shell, &revision.reference(), &rated, payer, &charter);
    let line = fill(
        words::FILING,
        &[("reference", &shown), ("charter", &charter)],
    );
    super::submit(ctx, "file", &subject, action, parameters, &line)
}

/// `@mind/update <filed> <name>@<rev>`: file a newer rated revision on the
/// line of a filed mind the actor owns.
fn update(ctx: &mut Ctx<'_>, text: &str) -> Result<(), CmdError> {
    let (filed, reference) = update_arguments(text).ok_or_else(|| usage(USAGE_FILED))?;
    let actions = actions(ctx)?;
    let view = own_filed(ctx, filed)?;
    let typed = text.trim().split_once(' ').map_or("", |(_, rest)| rest);
    let (revision, shown) = own_revision(ctx, &reference, typed)?;
    let rated = rating(ctx, &revision, &shown)?;
    let action = actions.update(view.dependent()).ok_or_else(not_open)?;
    let payer = super::personal_account(ctx)?;
    let reference = revision.reference();
    let parameters = update_parameters(view.line().get(), &reference, &rated, payer);
    let filed = filed.to_string();
    let line = fill(words::UPDATING, &[("reference", &shown), ("filed", &filed)]);
    let subject = format!("{filed}.{}.{}", reference.key.mind, reference.key.number);
    super::submit(ctx, "update", &subject, action, parameters, &line)
}

/// `@mind/withdraw <filed>`: stop new sales of a filed revision the actor
/// authored and owns; the provider rechecks authorship.
/// 17g C3a: `parse_withdraw` then `run_withdraw`; the typed `Withdraw {
/// filed }` runs the same `run_withdraw`.
fn withdraw(ctx: &mut Ctx<'_>, text: &str) -> Result<(), CmdError> {
    super::market::staged(ctx, |ctx| run_withdraw(ctx, parse_withdraw(text)?))
}

/// 17g C3a: `@mind/withdraw`'s argument, the filed number (`number`), else
/// its usage line (`REFUSED`).
pub(super) fn parse_withdraw(text: &str) -> Result<u64, Refusal> {
    number(text.trim()).ok_or_else(|| Refusal::usage(USAGE_FILED))
}

/// 17g C3a: `@mind/withdraw` after its parse: the composed Roll action
/// (`NOT_OPEN`), the actor's own filed mind (`NO_FILED`), then `WITHDRAWING`
/// and the withdrawal.
pub(super) fn run_withdraw(ctx: &mut Ctx<'_>, filed: u64) -> Result<Outcome, Refusal> {
    let actions = actions(ctx)?;
    let view = own_filed(ctx, filed)?;
    let action = actions.withdraw().ok_or_else(Refusal::not_open)?;
    let filed = view.filed().get().to_string();
    let line = fill(words::WITHDRAWING, &[("filed", &filed)]);
    let parameters = withdraw_parameters(view.filed().get());
    super::stage(ctx, "withdraw", &filed, action, parameters, line)
}

/// `@mind/attest <filed> [org=<org>]`: renew the owner's licence of a filed
/// mind the actor owns, once its term is due (the provider decides). M8c
/// B4: with `org=<org>`, the House attestation of a filed mind the
/// organisation owns, by a character acting for it (`NOT_FOR_ORG`
/// otherwise), through the House action.
fn attest(ctx: &mut Ctx<'_>, text: &str) -> Result<(), CmdError> {
    let (text, org) = super::org_argument(text).ok_or_else(|| usage(USAGE_FILED))?;
    let filed = number(text).ok_or_else(|| usage(USAGE_FILED))?;
    let actions = actions(ctx)?;
    let org = super::org_for(ctx, org, super::OrgScope::Title)?;
    let view = own_filed_for(ctx, filed, org)?;
    // The owner's ruling (17g C2h story part 3): a recalled revision, or a withdrawn one the term
    // does not pin, is not attested; it reads as no filed mind, as before the ruling's fix.
    if !view.held() {
        return Err(word!(NO_FILED, &[("filed", &filed.to_string())]).into());
    }
    let action = if org.is_some() {
        actions.attest_house()
    } else {
        actions.attest()
    }
    .ok_or_else(not_open)?;
    let filed = filed.to_string();
    let line = fill(words::ATTESTING, &[("filed", &filed)]);
    let parameters = attest_parameters_for(view.owner_licence().get(), org);
    super::submit(ctx, "attest", &filed, action, parameters, &line)
}

/// `@mind/fork <licence>=<name>`: the paid fork of a mind the actor holds an
/// Open licence for. The licence is a selection only: the provider checks the
/// actor holds it, and a refusal is the driver's one line.
fn fork(ctx: &mut Ctx<'_>, text: &str) -> Result<(), CmdError> {
    let (licence, name) = fork_arguments(text).ok_or_else(|| usage(USAGE_FORK))?;
    super::service(ctx)?;
    let actions = actions(ctx)?;
    let action = actions.fork().ok_or_else(not_open)?;
    let parent = LicenceId::new(licence)
        .map_err(|_| usage(USAGE_FORK))?
        .get();
    let parameters = fork_parameters(parent, &name);
    let licence = parent.to_string();
    let line = fill(words::FORKING, &[("licence", &licence), ("name", &name)]);
    super::submit(ctx, "fork", &licence, action, parameters, &line)
}
