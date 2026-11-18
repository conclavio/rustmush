use mush_bot::{
    campaign::{self, Checkpoint, Manifest},
    experiment::{load_json, save_json},
    memory::Event,
    social::{Response, Update},
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[path = "support/campaign_fixture.rs"]
mod fixture;
#[test]
fn social_fixture_process() {
    if let Ok(path) = std::env::var("MUSH_CAMPAIGN_START") {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(fixture::serve(path.into()))
            .unwrap();
    }
}
fn temp() -> PathBuf {
    // Tests in one binary run in parallel under one pid, and the Windows
    // clock ticks in 100 ns steps, so the time alone can repeat: the counter
    // keeps two fixtures from racing to create_dir the same path.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let path = std::env::temp_dir().join(format!(
        "mush-social-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir(&path).unwrap();
    path
}
fn checkpoint(output: &Path, branch: &str) -> Checkpoint {
    Checkpoint::load(&output.join(branch).join("checkpoint.json")).unwrap()
}
fn episode(checkpoint: &Checkpoint, actor: &str, quote: &str) -> u64 {
    checkpoint.bots[actor]
        .memory
        .records
        .iter()
        .find(|r| matches!(&r.event,Event::Episode(e) if e.text.contains(quote)))
        .unwrap()
        .id
}
fn events(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}

#[test]
fn native_renderer_hanging_wrap_preserves_agreements_without_promoting_speech_to_truth() {
    use mush_bot::{
        config::{Memory, Ssh},
        memory::Book,
    };
    let mut book = Book::open(
        &Memory {
            persistent: false,
            ..Default::default()
        },
        &Ssh::default(),
    )
    .unwrap();
    let proposal = "Lead says, \"I propose [long] for Bassist: bring a carefully arranged counterpoint | condition: everyone attends | due: next rehearsal\"";
    let lines = mush_ansi::wrap_indent(proposal, 60, 20).join("\n");
    assert!(lines.lines().count() > 1);
    book.observe(&lines).unwrap();
    book.flush().unwrap();
    book.observe("Bassist says, \"I accept [long].\"").unwrap();
    book.flush().unwrap();
    let state = book.social("Bassist").unwrap();
    assert_eq!(
        state.commitments["long"].task,
        "bring a carefully arranged counterpoint"
    );
    assert_eq!(
        state.commitments["long"].participants["Bassist"].response,
        Response::Accepted
    );
    assert_eq!(state.commitments["long"].history[0].quote, lines);
    let reported = "Lead says, \"I heard a story about Friday and I think the club has actually cancelled the gig.\"";
    book.observe(&mush_ansi::wrap_indent(reported, 60, 20).join("\n"))
        .unwrap();
    book.flush().unwrap();
    let episode = book.stats()["records"].as_u64().unwrap();
    let false_fact = json!({"type":"claim","subject":"gig","predicate":"status","value":"cancelled","attribution":"Lead","stance":"believed","basis":"observed","confidence":"strong","evidence":[{"episode":episode,"quote":"cancelled the gig."}]});
    assert!(
        !book
            .annotate_social(Some(json!([false_fact])), "Bassist", &Default::default())
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn band_negotiates_through_ssh_and_preserves_private_perspectives_across_branches() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = temp();
    let mut manifest = Manifest::load(&root.join("bots/social/band.json")).unwrap();
    manifest.fixture_program = std::env::current_exe().unwrap();
    manifest.fixture_args = vec![
        "--exact".into(),
        "social_fixture_process".into(),
        "--nocapture".into(),
    ];
    // Leave allowance for a further child with model annotations from real episode IDs.
    manifest.max_requests = 100;
    let output = dir.join("campaign");
    let result = campaign::run(
        &manifest,
        &output,
        Path::new(env!("CARGO_BIN_EXE_mush-bot")),
    )
    .await
    .unwrap();
    assert_eq!(result.branches.len(), 4);
    let parent = checkpoint(&output, "negotiated");
    let private = checkpoint(&output, "private-correction");
    let again = checkpoint(&output, "parent-again");
    let withdrawal = checkpoint(&output, "withdrawal");
    for (actor, snapshot) in &parent.bots {
        assert_eq!(snapshot.version, 4);
        let social = snapshot.social.as_ref().unwrap();
        assert_eq!(
            social.commitments["arrangement"].participants["Bassist"].response,
            Response::Accepted,
            "{actor}"
        );
        assert_eq!(
            social.commitments["tuning"].participants["Guitarist"].response,
            Response::Declined,
            "{actor}"
        );
        assert!(
            social.protocol_gaps.is_empty(),
            "{}: {:?}",
            actor,
            social.protocol_gaps
        );
        assert_eq!(snapshot.state.goal, again.bots[actor].state.goal);
        assert_eq!(
            serde_json::to_value(&snapshot.social).unwrap(),
            serde_json::to_value(&again.bots[actor].social).unwrap()
        );
        if actor != "Bassist" {
            let data = serde_json::to_string(snapshot).unwrap();
            assert!(
                !data.contains("She is evaluating the band as much"),
                "private grant leaked to {actor}"
            );
            assert!(
                !data.contains("Eight years of playing have taught her"),
                "private profile leaked to {actor}"
            );
        }
    }
    assert_eq!(
        parent.bots["Lead"]
            .social
            .as_ref()
            .unwrap()
            .interpretations
            .iter()
            .filter(|i| matches!(i.entry.update, Update::Relationship { .. }))
            .count(),
        2
    );
    assert!(
        parent.bots["Guitarist"]
            .social
            .as_ref()
            .unwrap()
            .interpretations
            .iter()
            .all(|i| !matches!(i.entry.update, Update::Relationship { .. }))
    );
    assert_eq!(
        withdrawal.bots["Bassist"]
            .social
            .as_ref()
            .unwrap()
            .commitments["arrangement"]
            .participants["Bassist"]
            .response,
        Response::Withdrawn
    );
    let completion = &private.bots["Bassist"].social.as_ref().unwrap().commitments["arrangement"]
        .participants["Bassist"];
    assert!(completion.completion_report.is_some());
    assert!(completion.assessment.is_none());
    for actor in ["Guitarist", "Drummer"] {
        assert!(
            !serde_json::to_string(&private.bots[actor].memory)
                .unwrap()
                .contains("Correction: Friday is still on")
        );
    }
    assert!(
        !serde_json::to_string(&parent.bots["Drummer"].memory)
            .unwrap()
            .contains("Friday was cancelled")
    );
    let trace = events(&output.join("negotiated/Bassist/trace.jsonl"));
    let captures = trace
        .iter()
        .filter(|e| e["kind"] == "model-input")
        .collect::<Vec<_>>();
    assert_eq!(captures.len(), 2);
    let content = captures[1]["data"]["payload"]["messages"][1]["content"]
        .as_str()
        .unwrap();
    let context: Value = serde_json::from_str(content).unwrap();
    assert_eq!(
        context["social_context"]["accepted_assignments"][0]["id"],
        "arrangement"
    );
    assert!(!content.contains("Tea before rehearsal"));
    assert!(
        context["group_affiliations"]["groups"][0]
            .get("background")
            .is_none()
    );
    assert!(trace.iter().all(|e| e["kind"] != "memory-feedback"));
    let lead_trace = events(&output.join("negotiated/Lead/trace.jsonl"));
    assert!(lead_trace.iter().all(|e| e["kind"] != "memory-feedback"));
    assert!(
        lead_trace.iter().any(|e| e["kind"] == "social"
            && e["data"].as_str().unwrap().contains("Tea before rehearsal"))
    );
    for branch in &result.branches {
        for report in branch.reports.values() {
            if branch.name != "negotiated" {
                assert_eq!(report["costs"]["session"]["totals"]["requests"], 0);
            }
        }
    }
    // Checkpoint schema and derived state cannot be edited independently.
    let mut bad = private.bots["Bassist"].clone();
    bad.social.as_mut().unwrap().version = 99;
    assert!(bad.validate().is_err());
    bad = private.bots["Bassist"].clone();
    bad.social.as_mut().unwrap().commitments.clear();
    assert!(bad.validate().is_err());
    bad = private.bots["Bassist"].clone();
    bad.social = None;
    assert!(bad.validate().is_err());
    // The model summarizes only actual locally observed rumor/correction episodes.
    let mut continuation = manifest.clone();
    continuation.groups.clear();
    continuation.from_checkpoint = Some(output.join("private-correction/checkpoint.json"));
    let mut steps = vec![];
    for (actor, value, quote, attribution) in [
        (
            "Bassist",
            "still on",
            "Correction: Friday is still on. The cancellation was a rumor.",
            "Lead corrected his earlier rumor",
        ),
        (
            "Guitarist",
            "cancelled",
            "Lead told me Friday was cancelled.",
            "Bassist attributed this to Lead",
        ),
    ] {
        let tape = dir.join(format!("{actor}-claim.json"));
        let decision = json!({"action":"done","summary":"Record my local understanding without broadcasting it.","social":[{"type":"claim","subject":"Friday gig","predicate":"status","value":value,"attribution":attribution,"stance":"unverified","basis":"reported","confidence":"tentative","evidence":[{"episode":episode(&private,actor,quote),"quote":quote}]}]});
        save_json(
            &tape,
            &json!({"version":1,"replies":[{"content":decision.to_string()}]}),
        )
        .unwrap();
        steps.push(json!({"actor":actor,"control":format!("/tape {}",tape.display())}));
        steps.push(json!({"actor":actor,"control":"/step","wait":{"kind":"done"}}));
    }
    continuation.branches =
        serde_json::from_value(json!([{"name":"different-beliefs","steps":steps}])).unwrap();
    let child_output = dir.join("beliefs");
    campaign::run(
        &continuation,
        &child_output,
        Path::new(env!("CARGO_BIN_EXE_mush-bot")),
    )
    .await
    .unwrap();
    let inspected = campaign::inspect(&child_output.join("summary.json")).unwrap();
    let perspectives = &inspected["branches"][0]["social_comparison"]["claims"][0]["perspectives"];
    assert_eq!(perspectives["Drummer"]["coverage"], "unestablished");
    assert_eq!(
        perspectives["Bassist"]["records"][0]["entry"]["update"]["value"],
        "still on"
    );
    assert_eq!(
        perspectives["Guitarist"]["records"][0]["entry"]["update"]["value"],
        "cancelled"
    );
    let child = checkpoint(&child_output, "different-beliefs");
    assert!(
        child.bots["Drummer"]
            .social
            .as_ref()
            .unwrap()
            .interpretations
            .iter()
            .all(|i| !matches!(i.entry.update, Update::Claim { .. }))
    );
    assert!(
        !serde_json::to_string(&child.bots["Guitarist"].last_capture)
            .unwrap()
            .contains("Correction: Friday is still on")
    );
    assert_eq!(parent.id, checkpoint(&output, "negotiated").id);
    // Operator JSON exports do not issue model requests and carry a single view.
    let report: Value = load_json(
        &child_output.join("different-beliefs/Bassist/report.json"),
        32_000_000,
    )
    .unwrap();
    assert_eq!(report["social"]["observer"], "Bassist");
    assert_eq!(report["costs"]["session"]["totals"]["requests"], 1);
}
