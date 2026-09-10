//! Model-only replay. This module never opens an SSH session or applies decisions.
use crate::{
    Result,
    config::Config,
    costs::{Ledger, Outcome, PromptType},
    model::{Brain, Capture, Decision},
    models::Catalog,
};
use serde_json::{Value, json};
use std::{fs::File, path::Path, time::Instant};

pub async fn run(
    config: &Config,
    path: &Path,
    output: &Path,
    override_model: bool,
) -> Result<Value> {
    let source = Capture::load(path)?;
    let context: Value = serde_json::from_str(
        source.payload["messages"][1]["content"]
            .as_str()
            .unwrap_or("{}"),
    )
    .unwrap_or(Value::Null);
    if context["studio_protocol"] == 1 {
        return Err("--replay validates gameplay decisions; use --studio-session to continue authoring from a saved session".into());
    }
    let budget = config
        .budget
        .as_deref()
        .map(crate::budget::Settings::load)
        .transpose()?
        .unwrap_or_default();
    let tape = config
        .model_tape
        .as_deref()
        .map(crate::tape::Source::load)
        .transpose()?;
    let mut config = config.clone();
    if tape.is_some() && config.model.is_none() && config.model_profiles.is_none() {
        config.model = Some(serde_json::from_value(
            json!({"endpoint":"http://127.0.0.1", "name":"test/tape"}),
        )?);
    }
    let selection = Catalog::from_config(&config)?
        .initial()?
        .ok_or("replay requires a configured model profile")?;
    let brain = Brain::with_tape(
        &selection.settings,
        tape.map(|s| std::sync::Arc::new(crate::tape::Tape::new(s))),
    )?;
    let capture = if override_model {
        source.with_model(&selection.settings)?
    } else {
        source.clone()
    };
    let file = File::options().write(true).create_new(true).open(output)?;
    let mut costs = Ledger::open(&config.costs, &config.ssh, &config.memory.namespace)?;
    let mut actual_selection = selection;
    actual_selection.settings.name = capture.payload["model"].as_str().unwrap().into();
    actual_selection.settings.max_tokens = capture.payload["max_tokens"].as_u64().unwrap() as u32;
    actual_selection.settings.temperature = capture.payload["temperature"].as_f64();
    let currency = actual_selection.settings.currency();
    let budget_before = budget.evaluate(&costs.session_usage()?, currency.as_deref())?;
    if !budget_before.allowed {
        let report = json!({"version":1,"mode":"decision_replay","execution_mode":"model_only_replay","world_restorable":false,"success":false,
            "source_revision":source.request_revision,"request":null,"capture":capture,
            "content":null,"error":budget_before.message(),"decision":null,"commands_sent":0,
            "costs":costs.report()?,"budget":{"settings":budget,"revision":budget.revision(),"status":budget_before}});
        serde_json::to_writer_pretty(file, &report)?;
        return Ok(report);
    }
    let request = costs.start(
        actual_selection,
        PromptType::DecisionReplay,
        capture.prompt_revision.clone(),
    )?;
    let at = Instant::now();
    let reply = brain.dispatch(&capture).await;
    let (outcome, receipt) = match &reply {
        Ok(reply) => (
            if reply.error.is_some() {
                Outcome::Error
            } else {
                Outcome::Response
            },
            Some(reply.receipt.clone()),
        ),
        Err(_) => (Outcome::Error, None),
    };
    costs.finish(
        request.id,
        outcome,
        receipt,
        at.elapsed().as_millis() as u64,
    )?;
    let (content, error, decision) = match reply {
        Ok(reply) => {
            let parsed = if capture.question || reply.error.is_some() {
                None
            } else {
                Some(Decision::parse(&reply.content))
            };
            let error = reply.error.or_else(|| {
                parsed
                    .as_ref()
                    .and_then(|p| p.as_ref().err().map(|e| e.to_string()))
            });
            (Some(reply.content), error, parsed.and_then(|p| p.ok()))
        }
        Err(error) => (None, Some(error.to_string()), None),
    };
    let report = json!({"version":1,"mode":"decision_replay","execution_mode":"model_only_replay","world_restorable":false, "success":error.is_none(),
        "source_revision":source.request_revision,"request":request,"capture":capture,
        "content":content,"error":error,"decision":decision,"commands_sent":0,"costs":costs.report()?,
        "budget_check":budget_before,
        "budget":{"settings":budget,"revision":budget.revision(),"status":budget.evaluate(&costs.session_usage()?,currency.as_deref())?}});
    serde_json::to_writer_pretty(file, &report)?;
    Ok(report)
}
