#![cfg(test)]

//! 17g C2a: the line-mode command kinds over Roll's real filing fixture (the
//! registered composition, an acknowledged shell, an actual initial filing),
//! read through the registered original-holdings read (C2 plan §2.4
//! fallback 1) as TacSim's command slot reads them.
use super::*;
use crate::commerce::filing_tests::{Fixture, config, token};
use crate::commerce::holding_reader::{OriginalOwnerReader, OriginalReadRequest};
use mush_core::read_service::OwnerReadHandle;
use mush_core::{GOD, ModError, ObjId, Outbox, Registrar, World};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

type TestResult = Result<(), Box<dyn std::error::Error>>;
/// The command answer and the whole result's encoding as it leaves Roll.
type Answer = Result<(RollCommandReadResult, String), OwnerReadError>;
type Answers = Arc<Mutex<Option<Answer>>>;

struct Query {
    reader: OwnerReadHandle<OriginalOwnerReader>,
    answers: Answers,
}
impl mush_core::module::QueryHandler for Query {
    fn query(
        &self,
        _: &World,
        _: ObjId,
        _: &Value,
    ) -> Result<mush_proto::OutputEvent, mush_proto::RichText> {
        Err(mush_proto::RichText::plain("live context required"))
    }
    fn query_with_context(
        &self,
        ctx: &mush_core::module::HookReadContext<'_>,
        _: ObjId,
        args: &Value,
    ) -> Result<mush_proto::OutputEvent, mush_proto::RichText> {
        let request = match args.as_u64().and_then(|n| FiledRevisionId::new(n).ok()) {
            Some(filed) => RollCommandReadRequest::own_filed(filed),
            None => RollCommandReadRequest::open_shell(),
        };
        let answer = ctx
            .read_owner(&self.reader, OriginalReadRequest::command(request))
            .and_then(|result| {
                let encoded =
                    serde_json::to_string(&result).map_err(|_| OwnerReadError::Integrity)?;
                let command = result.into_command().ok_or(OwnerReadError::Integrity)?;
                Ok((command, encoded))
            });
        if let Ok(mut answers) = self.answers.lock() {
            *answers = Some(answer);
        }
        Ok(mush_proto::OutputEvent::Sys(mush_proto::RichText::plain(
            "observed",
        )))
    }
}
/// The fixture's "tacsim" module reads through its one original-holdings
/// grant, as TacSim's command slot does.
fn tacsim_reads(
    reg: &mut Registrar<'_>,
    reader: OwnerReadHandle<OriginalOwnerReader>,
) -> Result<(), ModError> {
    let answers: Answers = Arc::new(Mutex::new(None));
    reg.state(answers.clone());
    reg.query_handler("mind.commands.test", Arc::new(Query { reader, answers }))
}
fn read(f: &mut Fixture, actor: ObjId, args: Value) -> Result<Answer, Box<dyn std::error::Error>> {
    f.engine.step(
        mush_core::Inbound::Query {
            conn: 1,
            actor,
            id: 1,
            name: "mind.commands.test".into(),
            args,
        },
        &mut Outbox::new(),
        &mut mush_core::engine::Effects::new(),
    );
    let answer = f
        .engine
        .world
        .module_state::<Answers>()
        .ok_or("answers")?
        .lock()
        .map_err(|_| "answers poisoned")?
        .take()
        .ok_or("no answer")?;
    Ok(answer)
}

/// 17g C2a test 6 (C2 plan §2.4). Over the real filing fixture:
/// - `OpenShell`: GOD's own acknowledged reservation reads its request id;
///   another actor reads none; after the filing consumes it, none;
/// - `OwnFiled`: the author reads the filed revision's charter, line, owner
///   licence (the charter's initial filing operation names it), the charter's
///   public name, an original line and an Active owner phase with whole IC
///   days left;
/// - another actor naming that filed number and the author naming a number
///   that does not exist both read `Refused`, identically;
/// - the serialized answers carry no digest, account, rating or source, nor
///   any original-holding field beyond the empty ones.
///
/// The owner phase's Grace, Lapsed and rounding are proven on `phase` below.
/// The other actor is a real second player character (reviewer F4), not a
/// room. A dependent line and a transferred charter need fixtures this crate
/// does not have (a dependent filing's parent rights, an E4 charter
/// transfer); `dependent` is the birth operation's parent and the title check
/// is `charter_transfer::history::current`, the own-holding read's own. Both
/// are a residual the lead owns (the C2d story's charter step).
#[test]
fn command_reader_answers_only_for_the_actors_own_shell_and_filed_minds() -> TestResult {
    let mut f = Fixture::new_with_reads(true, tacsim_reads, true);
    let other = f.engine.world.create(
        mush_core::ObjKind::Player,
        "Other",
        GOD,
        Some(mush_core::LIMBO),
    )?;
    check!(
        f.engine
            .world
            .get(other)
            .is_some_and(|o| o.kind == mush_core::ObjKind::Player && other != GOD),
        "a second live player character"
    );
    let (shell, _) = read(&mut f, GOD, Value::Null)??;
    check_eq!(
        shell.open_shell(),
        Some(Some(RequestUid::from_bytes([3; 16]))),
        "the fixture's own acknowledged reservation"
    );
    check_eq!(
        read(&mut f, other, Value::Null)??.0.open_shell(),
        Some(None),
        "another actor's shell is invisible"
    );
    check!(matches!(
        read(&mut f, GOD, json!(1))?,
        Err(OwnerReadError::Refused)
    ));
    // File through the real registered action.
    let request = f.request();
    let (prepared, _) = f.engine.prepare_registered_composed_authoritative_action(
        &config().initial_filing_action()?,
        &request,
        token(4),
    )?;
    let (_, install) = prepared.into_parts();
    f.engine.install_composed_action(install)?;
    check_eq!(
        read(&mut f, GOD, Value::Null)??.0.open_shell(),
        Some(None),
        "filing consumed the shell"
    );
    let filed = FiledRevisionId::new(1)?;
    let (answer, encoded) = read(&mut f, GOD, json!(1))??;
    check!(
        encoded.starts_with(r#"{"holding":null,"next_after":null,"command":{"kind":"own_filed""#),
        "{encoded}"
    );
    let view = answer.into_own_filed().ok_or("an own filed view")?;
    let record = f
        .engine
        .action_state()
        .get(&f.families.filed, &filed.key())?
        .ok_or("the filed row")?;
    check_eq!(view.filed(), filed);
    check_eq!(view.charter(), record.content().position().charter);
    check_eq!(view.line(), record.content().position().line);
    check_eq!(view.charter().get(), f.shell.0, "the charter is the shell");
    check_eq!(view.owner_licence(), LicenceId::new(1)?);
    check!(!view.dependent(), "an original line");
    check_eq!(view.name(), "Original");
    check_eq!(view.owner_phase(), OwnerPhase::Active { days_left: 90 });
    check_eq!(
        serde_json::to_value(&view)?,
        json!({"filed":"1","charter":view.charter().get().to_string(),"line":"1",
            "owner_licence":"1","dependent":false,"name":"Original",
            "owner_phase":{"phase":"active","days_left":90}})
    );
    for forbidden in ["digest", "account", "payer", "rating", "source", "revision"] {
        check!(!encoded.contains(forbidden), "{forbidden} in {encoded}");
    }
    check!(
        !encoded.contains(&f.revision.reference().digest.to_string()),
        "no revision digest"
    );
    // Foreign and missing read alike.
    let foreign = read(&mut f, other, json!(1))?;
    let missing = read(&mut f, GOD, json!(99))?;
    check_eq!(foreign, Err(OwnerReadError::Refused));
    check_eq!(missing, Err(OwnerReadError::Refused));
    Ok(())
}

/// 17g C2a (C2 plan §2.4 fallback 1): the folded kinds answer only when the
/// player market opens them (`with_mind_commands`). Over the same fixture
/// with the original-holdings grant but without the market, both kinds read
/// `Unavailable`, the actor's own acknowledged shell included.
#[test]
fn command_kinds_are_closed_without_the_player_market() -> TestResult {
    let mut f = Fixture::new_with_reads(true, tacsim_reads, false);
    check_eq!(
        read(&mut f, GOD, Value::Null)?,
        Err(OwnerReadError::Unavailable)
    );
    check_eq!(
        read(&mut f, GOD, json!(1))?,
        Err(OwnerReadError::Unavailable)
    );
    Ok(())
}

/// The owner phase in whole IC days, rounded up, at the term's boundaries.
#[test]
fn owner_phase_rounds_whole_ic_days_up_and_reads_grace_and_lapse() -> TestResult {
    let day = DAY;
    let window = super::super::TermWindow::new(
        WorldIcSeconds(1_000),
        WorldIcSeconds(1_000),
        WorldIcSeconds(1_000 + 90 * day),
        WorldIcSeconds(1_000 + 93 * day),
    )?;
    let at = |offset: i64| WorldIcSeconds(1_000 + offset);
    check_eq!(phase(window, at(0)), OwnerPhase::Active { days_left: 90 });
    check_eq!(phase(window, at(1)), OwnerPhase::Active { days_left: 90 });
    check_eq!(phase(window, at(day)), OwnerPhase::Active { days_left: 89 });
    check_eq!(
        phase(window, at(90 * day - 1)),
        OwnerPhase::Active { days_left: 1 }
    );
    check_eq!(
        phase(window, at(90 * day)),
        OwnerPhase::Grace { days_left: 3 }
    );
    check_eq!(
        phase(window, at(93 * day - 1)),
        OwnerPhase::Grace { days_left: 1 }
    );
    check_eq!(phase(window, at(93 * day)), OwnerPhase::Lapsed);
    Ok(())
}
