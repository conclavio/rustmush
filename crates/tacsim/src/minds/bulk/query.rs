//! Source-free, own-character result hydration; no action is accepted here.
use super::{
    artifacts::{Artifacts, Key, Part},
    model::{RunId, Status, WorkKind},
    owner::BulkStore,
};
use mush_core::{ObjId, QueryHandler, World};
use mush_proto::{OutputEvent, PanelFrameKind, RichText};
use serde::Deserialize;
use serde_json::{Value, json};
pub(crate) const FEED: &str = "mind.bulk";
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Params {
    run: Option<u64>,
    page: Option<usize>,
}
pub(crate) struct BulkQuery;
impl QueryHandler for BulkQuery {
    fn query(&self, w: &World, actor: ObjId, params: &Value) -> Result<OutputEvent, RichText> {
        let missing = || RichText::plain("No such owned proving result.");
        let p = if params.is_null() {
            Params::default()
        } else {
            serde_json::from_value::<Params>(params.clone()).map_err(|_| missing())?
        };
        let account = w
            .module_state::<mush_sim::AccountOracle>()
            .and_then(|o| o.account_of(actor))
            .ok_or_else(missing)?;
        let owner = w.module_state::<BulkStore>().ok_or_else(missing)?;
        let rows = match owner.runs(account, actor) {
            Ok(rows) => rows,
            Err(super::BulkError::Missing) => Vec::new(),
            Err(_) => return Err(missing()),
        };
        let rows=rows.into_iter().filter(|r|r.plan.kind==WorkKind::Proving).rev().take(100).map(|r|json!({
            "run":r.id.0,"state":r.status,"games":r.plan.games,"scenario":r.plan.scenario,
            "per_side":r.plan.per_side,"charged_unit_ms":r.charged_unit_ms,"finished_ms":r.finished_ms,
            "can_cancel":!r.status.terminal(),"can_forget":r.status.terminal()&&r.pins==0,
        })).collect::<Vec<_>>();
        let report = if let Some(run) = p.run {
            let row = owner
                .run(account, actor, RunId(run))
                .map_err(|_| missing())?;
            if row.plan.kind != WorkKind::Proving || row.status != Status::Complete {
                return Err(missing());
            }
            let manifest = w
                .module_state::<Artifacts>()
                .ok_or_else(missing)?
                .get(Key {
                    account,
                    run: RunId(run),
                    part: Part::Result,
                })
                .map_err(|_| missing())?;
            let pages = manifest.summary.len().div_ceil(15).max(1);
            let page = p.page.unwrap_or(1);
            if page == 0 || page > pages {
                return Err(missing());
            }
            // Exact decimal strings preserve the full seed range in browsers.
            let mut context = serde_json::to_value(&manifest.context).map_err(|_| missing())?;
            if let (Some(source), Some(object)) = (&manifest.context, context.as_object_mut()) {
                object.insert(
                    "pair_seeds".into(),
                    json!(
                        source
                            .pair_seeds
                            .iter()
                            .map(u64::to_string)
                            .collect::<Vec<_>>()
                    ),
                );
            }
            Some(
                json!({"run":run,"page":page,"pages":pages,"context":context,"lines":manifest.summary.iter().skip((page-1)*15).take(15).collect::<Vec<_>>() }),
            )
        } else {
            if p.page.is_some() {
                return Err(missing());
            }
            None
        };
        Ok(OutputEvent::Panel {
            feed: FEED.into(),
            kind: PanelFrameKind::Keyframe,
            data: json!({"schema":{"id":FEED,"version":1},"runs":rows,"report":report}),
        })
    }
}
