//! Actual committed filing + registered Market read. This reuses the structural
//! rating fixture only to test the owner boundary, never worker certification.
use super::*;
use crate::commerce::sale_reader::{OriginalSaleFacts, OriginalSaleReader, OriginalSaleRequest};
use mush_core::read_service::{OwnerReadError, OwnerReadHandle};

type Answers = Arc<Mutex<Option<Result<OriginalSaleFacts, OwnerReadError>>>>;
pub(super) struct Market;
struct Query {
    reader: OwnerReadHandle<OriginalSaleReader>,
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
        let licence = crate::commerce::LicenceId::new(1).unwrap();
        let request = if args == "own" {
            OriginalSaleRequest::own(licence)
        } else {
            OriginalSaleRequest::listed(GOD, licence)
        };
        *self.answers.lock().unwrap() = Some(ctx.read_owner(&self.reader, request));
        Ok(mush_proto::OutputEvent::Sys(mush_proto::RichText::plain(
            "observed",
        )))
    }
}
impl GameModule for Market {
    fn name(&self) -> &'static str {
        "market"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let reader = OriginalSaleReader::request(reg, &reference("tacsim", manifest_descriptor()))?;
        let answers: Answers = Arc::new(Mutex::new(None));
        reg.state(answers.clone());
        reg.query_handler("original.sale.test", Arc::new(Query { reader, answers }))
    }
}
fn read(engine: &mut Engine, actor: ObjId, own: bool) -> Result<OriginalSaleFacts, OwnerReadError> {
    engine.step(
        mush_core::Inbound::Query {
            conn: 1,
            actor,
            id: 1,
            name: "original.sale.test".into(),
            args: json!(if own { "own" } else { "listed" }),
        },
        &mut Outbox::new(),
        &mut mush_core::engine::Effects::new(),
    );
    engine
        .world
        .module_state::<Answers>()
        .unwrap()
        .lock()
        .unwrap()
        .take()
        .unwrap()
}
fn filed() -> Fixture {
    let mut f = Fixture::new_with_market(true, true);
    let request = f.request();
    let (prepared, _) = f
        .engine
        .prepare_registered_composed_authoritative_action(
            &config().initial_filing_action().unwrap(),
            &request,
            token(4),
        )
        .unwrap();
    let (_, install) = prepared.into_parts();
    f.engine.install_composed_action(install).unwrap();
    f
}

#[test]
fn market_receives_only_exact_original_subject_and_keeps_actual_requester() {
    let mut f = filed();
    let own = read(&mut f.engine, GOD, true).unwrap();
    assert_eq!(own.seller(), GOD);
    assert_eq!(own.requester(), GOD);
    assert_eq!(
        own.revision(),
        RevisionIdentity::capture(&f.revision).unwrap()
    );
    assert_eq!(own.rating(), f.rating);
    assert_eq!(own.charter().object(), f.shell);
    let buyer = f
        .engine
        .world
        .create(
            mush_core::ObjKind::Player,
            "Buyer",
            GOD,
            Some(mush_core::LIMBO),
        )
        .unwrap();
    let quoted = read(&mut f.engine, buyer, false).unwrap();
    assert_eq!(quoted.requester(), buyer);
    assert_eq!(quoted.seller(), GOD);
    assert_eq!(quoted.revision(), own.revision());
    assert_eq!(
        read(&mut f.engine, buyer, true),
        Err(OwnerReadError::Refused)
    );
    assert_eq!(
        read(&mut f.engine, mush_core::LIMBO, false),
        Err(OwnerReadError::Refused)
    );
    let keys: BTreeMap<String, Value> = serde_json::from_value(json!(quoted)).unwrap();
    assert_eq!(
        keys.keys().map(String::as_str).collect::<Vec<_>>(),
        vec![
            "charter",
            "compatibility",
            "declared",
            "definition",
            "filed",
            "line",
            "minimum",
            "observed_ic",
            "owner_licence",
            "owner_term",
            "owner_window",
            "rating",
            "requester",
            "revision",
            "schema",
            "seller",
            "sequence",
            "title_revision"
        ]
    );
    // Source identity is intentionally exact; no source/program/schema/card body
    // or other private revision fields are present in the small projection.
    assert!(serde_json::to_vec(&quoted).unwrap().len() < 4096);
}

#[test]
fn fresh_sale_refuses_withdrawn_or_recalled_rows_and_source_reaudit() {
    let mut f = filed();
    assert!(read(&mut f.engine, GOD, true).is_ok());
    let key = FiledRevisionId::new(1).unwrap().key();
    for (withdrawn, recalled) in [(true, false), (false, true), (true, true), (false, false)] {
        let mut txn = f.engine.action_transaction(&f.families.status).unwrap();
        let mut status = txn.get(&f.families.status, &key).unwrap().unwrap().clone();
        status.withdrawn_by = withdrawn.then(|| token(80));
        status.recalled_by = recalled.then(|| token(81));
        txn.replace(&f.families.status, &key, status).unwrap();
        let (_, install) = txn.prepare().unwrap().into_parts();
        f.engine.install_prepared_action(install).unwrap();
        assert_eq!(
            read(&mut f.engine, GOD, true).is_ok(),
            !withdrawn && !recalled
        );
    }
    f.engine
        .world
        .module_state_mut::<RollStore>()
        .unwrap()
        .audit_sources(&f.sources, &f.sources)
        .unwrap();
    assert_eq!(
        read(&mut f.engine, GOD, true),
        Err(OwnerReadError::Unavailable)
    );
}

#[test]
fn original_sale_reader_requires_explicit_composition_and_actual_market_grant() {
    // A Market requester cannot silently enable a new service in an M6 or
    // ordinary original-filing composition. No broad Roll delegation is added.
    let result = try_engine_with_market(World::new_minimal(), Sources::default(), false, true);
    let Err(error) = result else {
        panic!("unregistered sale reader was granted")
    };
    assert!(error.to_string().contains("owner read"), "{error}");
    assert!(try_engine_with_market(World::new_minimal(), Sources::default(), true, true).is_ok());
}
