use mush_bot::{
    campaign::{self, Checkpoint, Manifest},
    config::{Memory, Ssh},
    experiment::{load_json, revision, save_json},
    memory::{Book, Event, Source},
    playtest::{self, Archive, Job},
    reproduction::{self, Status},
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[path = "support/campaign_fixture.rs"]
mod fixture;
#[test]
fn playtest_fixture_process() {
    if let Ok(path) = std::env::var("MUSH_CAMPAIGN_START") {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(fixture::serve(path.into()))
            .unwrap();
    }
}
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn temp() -> PathBuf {
    // Tests in one binary run in parallel under one pid, and the Windows
    // clock ticks in 100 ns steps, so the time alone can repeat: the counter
    // keeps two fixtures from racing to create_dir the same path.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let p = std::env::temp_dir().join(format!(
        "mush-playtest-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir(&p).unwrap();
    p
}
fn book() -> Book {
    Book::open(
        &Memory {
            persistent: false,
            ..Default::default()
        },
        &Ssh {
            username: "Subject".into(),
            ..Default::default()
        },
    )
    .unwrap()
}
fn observed(b: &mut Book, text: &str) -> u64 {
    b.observe(text).unwrap();
    b.flush().unwrap();
    b.records()
        .iter()
        .rev()
        .find(|r| matches!(&r.event,Event::Episode(e) if e.source==Source::Observed))
        .unwrap()
        .id
}
fn feedback(id: u64) -> Value {
    json!({"ease":2,"discoverability":2,"completion":"partial","summary":"Blocked north exit caused confusion.","claims":[{"id":"travel","category":"navigation","title":"Cannot reach Lounge","expected":"North reaches Lounge","expectation_basis":"player_assumption","observed":"The north exit was blocked.","suggested_change":"Explain alternatives.","evidence":[{"episode":id,"quote":"The exit is blocked."}]}]})
}
fn step(actor: &str, control: &str, kind: Option<&str>) -> campaign::Step {
    campaign::Step {
        actor: actor.into(),
        control: control.into(),
        wait: kind.map(|k| campaign::Wait {
            kind: k.into(),
            contains: None,
        }),
    }
}
fn fixture_manifest() -> Manifest {
    let mut m = Manifest::load(&root().join("bots/adversarial/campaign.json")).unwrap();
    m.fixture_program = std::env::current_exe().unwrap();
    m.fixture_args = vec![
        "--exact".into(),
        "playtest_fixture_process".into(),
        "--nocapture".into(),
    ];
    m
}
fn tape(path: &Path, replies: Vec<Value>) {
    save_json(path,&json!({"version":1,"replies":replies.into_iter().map(|v|json!({"content":v.to_string()})).collect::<Vec<_>>()})).unwrap();
}
fn decision(action: &str, command: Option<&str>) -> Value {
    json!({"action":action,"command":command,"summary":"Follow observed route evidence."})
}

#[tokio::test]
async fn cancelled_review_and_denied_command_leave_no_success_evidence() {
    let dir = temp();
    let mut m = fixture_manifest();
    m.actors.retain(|a| a.name == "Subject");
    let mut config = mush_bot::config::Config::load(&m.actors[0].config).unwrap();
    config.mission = Some(root().join("bots/adversarial/mission.json"));
    let path = dir.join("delayed.json");
    save_json(&path, &json!({"version":1,"replies":[{"delay_ms":10000,"content":"{\"ease\":5,\"discoverability\":5,\"completion\":\"completed\",\"summary\":\"Unsent review.\",\"claims\":[]}"}]})).unwrap();
    config.model_tape = Some(path);
    let path = dir.join("subject.toml");
    std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    m.actors[0].config = path;
    m.branches[0].steps = vec![
        step("Subject", "/feedback", Some("model")),
        step("Subject", "/pause", None),
    ];
    let result = campaign::run(
        &m,
        &dir.join("cancelled"),
        Path::new(env!("CARGO_BIN_EXE_mush-bot")),
    )
    .await
    .unwrap();
    let c = Checkpoint::load(&result.branches[0].checkpoint.path).unwrap();
    assert!(
        c.bots["Subject"]
            .playtest
            .as_ref()
            .unwrap()
            .entries
            .is_empty()
    );
    assert_eq!(c.bots["Subject"].usage.requests, 1);
    let trace =
        std::fs::read_to_string(dir.join("cancelled/encounter/Subject/trace.jsonl")).unwrap();
    assert!(trace.contains("playtest-cancelled"));
    m.branches[0].steps = vec![
        step("Subject", "/mission start", None),
        step("Subject", "/pause", None),
        step("Subject", "/send say outside-permitted-scope", None),
    ];
    assert!(
        campaign::run(
            &m,
            &dir.join("denied"),
            Path::new(env!("CARGO_BIN_EXE_mush-bot"))
        )
        .await
        .is_err()
    );
    let trace = std::fs::read_to_string(dir.join("denied/encounter/Subject/trace.jsonl")).unwrap();
    let events: Vec<Value> = trace
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(
        events.iter().any(|e| e["kind"] == "control-error"
            && e["data"].as_str().unwrap().contains("mission denied"))
    );
    assert!(
        !events
            .iter()
            .any(|e| e["kind"] == "send" && e["data"] == "say outside-permitted-scope")
    );
}

#[tokio::test]
async fn failed_fixture_is_inconclusive_and_exhausted_budget_cannot_start() {
    let dir = temp();
    let mut m = fixture_manifest();
    m.fixture_program = dir.join("does-not-exist.exe");
    let (mut spec, _) =
        reproduction::Spec::load(&root().join("bots/adversarial/reproduce.json")).unwrap();
    spec.repeats = 1;
    spec.max_trials = 2;
    spec.max_requests = 10;
    let r = reproduction::run(
        &spec,
        &m,
        &dir.join("missing"),
        Path::new(env!("CARGO_BIN_EXE_mush-bot")),
        false,
    )
    .await
    .unwrap();
    assert_eq!(r.status, Status::Inconclusive);
    assert!(r.exported_recipe.is_none());
    assert_eq!(r.trials[0].status, Status::Inconclusive);
    assert!(r.trials[0].verify(&spec.oracle).is_err());
    spec.max_requests = 4;
    assert!(
        reproduction::run(
            &spec,
            &m,
            &dir.join("budget"),
            Path::new(env!("CARGO_BIN_EXE_mush-bot")),
            false
        )
        .await
        .is_err()
    );
    assert!(!dir.join("budget/trial-000").exists());
}

#[test]
fn offline_docket_batches_and_disagreement_preserve_all_claims() {
    let dir = temp();
    // The offline commands must dispatch before runtime configuration or dotenv.
    std::fs::write(dir.join("mush-bot.toml"), "not valid TOML [[[").unwrap();
    std::fs::write(dir.join(".env"), "invalid test fixture input").unwrap();
    let mut b = book();
    let id = observed(&mut b, "The exit is blocked.");
    let job = Job::new(
        "Subject".into(),
        "Reach Lounge".into(),
        b.records(),
        None,
        false,
    );
    let mut entries = vec![];
    for i in 0..3 {
        let mut f = feedback(id);
        let claim = f["claims"][0].clone();
        f["claims"] = json!(
            (0..16)
                .map(|j| {
                    let mut c = claim.clone();
                    c["id"] = json!(format!("claim-{j}"));
                    c
                })
                .collect::<Vec<_>>()
        );
        entries.push(
            job.complete(&f.to_string(), format!("revision-{i}"), "local-tape".into())
                .unwrap(),
        );
    }
    let archive = Archive {
        version: 1,
        entries,
    };
    assert_eq!(
        archive
            .dockets()
            .unwrap()
            .iter()
            .map(|d| d.claims.len())
            .sum::<usize>(),
        48
    );
    let path = dir.join("archive.json");
    save_json(&path, &archive).unwrap();
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_mush-bot"))
        .current_dir(&dir)
        .args([
            "--make-refutation-dockets",
            "archive.json",
            "--output",
            "batches",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let index: Value = load_json(&dir.join("batches/index.json"), 128_000).unwrap();
    assert_eq!(index["claims"], 48);
    assert_eq!(index["batches"].as_array().unwrap().len(), 3);
    let reused = std::process::Command::new(env!("CARGO_BIN_EXE_mush-bot"))
        .current_dir(&dir)
        .args([
            "--make-refutation-dockets",
            "archive.json",
            "--output",
            "batches",
        ])
        .output()
        .unwrap();
    assert!(!reused.status.success());
    let docket = archive.dockets().unwrap().remove(0);
    let mut combined = archive.clone();
    for verdict in ["supported", "refuted"] {
        let job = Job::new(
            format!("reviewer-{verdict}"),
            "Challenge claims".into(),
            b.records(),
            Some(docket.clone()),
            false,
        );
        let response = json!({"challenges":docket.claims.iter().map(|c|json!({"claim":c.key,"verdict":verdict,"rationale":"An explicitly simulated disagreement, not proof.","evidence":[{"episode":id,"quote":"The exit is blocked."}],"proposed_test":"Check the other exits in a fresh fixture."})).collect::<Vec<_>>()});
        combined.entries.push(
            job.complete(
                &response.to_string(),
                format!("review-{verdict}"),
                "local-tape".into(),
            )
            .unwrap(),
        );
    }
    save_json(&dir.join("combined.json"), &combined).unwrap();
    save_json(
        &dir.join("study.json"),
        &json!({"version":1,"archives":["combined.json","archive.json"]}),
    )
    .unwrap();
    let report = playtest::consolidate(&dir.join("study.json")).unwrap();
    assert_eq!(report["counts"]["contested"], 16);
    assert_eq!(report["counts"]["needs_refutation"], 32);
    assert_eq!(report["ratings"].as_array().unwrap().len(), 3);
    assert!(report["actionable_issues"].as_array().unwrap().is_empty());
    if let playtest::Review::Refutation { docket, .. } =
        &mut combined.entries.last_mut().unwrap().review
    {
        docket.claims[0].title = "Altered allegation".into();
    }
    save_json(&dir.join("tampered.json"), &combined).unwrap();
    save_json(
        &dir.join("bad-study.json"),
        &json!({"version":1,"archives":["tampered.json"]}),
    )
    .unwrap();
    assert!(playtest::consolidate(&dir.join("bad-study.json")).is_err());
}

#[tokio::test]
async fn additional_adversaries_use_only_public_scene_commands() {
    let dir = temp();
    let fixture = fixture_manifest();
    for role in ["heckler", "withholding", "conflicting"] {
        let mut m =
            Manifest::load(&root().join(format!("bots/adversarial/{role}-campaign.json"))).unwrap();
        m.fixture_program = fixture.fixture_program.clone();
        m.fixture_args = fixture.fixture_args.clone();
        let result = campaign::run(
            &m,
            &dir.join(role),
            Path::new(env!("CARGO_BIN_EXE_mush-bot")),
        )
        .await
        .unwrap();
        let c = Checkpoint::load(&result.branches[0].checkpoint.path).unwrap();
        assert_eq!(c.bots["Subject"].usage.requests, 1);
        assert!(
            c.bots
                .iter()
                .filter(|(n, _)| *n != "Subject")
                .all(|(_, b)| b.usage.requests == 0)
        );
        assert!(c.bots["Subject"].memory.records.iter().any(|r|matches!(&r.event,Event::Episode(e) if e.source==Source::Observed && e.text.contains("Subject says,"))));
    }
}

#[test]
fn reviews_reject_fabricated_future_and_sent_citations_and_cannot_act() {
    let mut b = book();
    let id = observed(&mut b, "The exit is blocked.");
    let job = Job::new(
        "Subject".into(),
        "Reach Lounge".into(),
        b.records(),
        None,
        false,
    );
    let e = job
        .complete(
            &feedback(id).to_string(),
            "request-revision".into(),
            "test-model".into(),
        )
        .unwrap();
    e.validate_local(b.records(), b.records().len() as u64 + 1)
        .unwrap();
    let mut bad = feedback(id);
    bad["claims"][0]["evidence"][0]["quote"] = json!("You arrived.");
    assert!(
        job.complete(&bad.to_string(), "r".into(), "m".into())
            .is_err()
    );
    bad = feedback(id);
    bad["command"] = json!("say I did it");
    assert!(
        job.complete(&bad.to_string(), "r".into(), "m".into())
            .is_err()
    );
    bad = feedback(id + 100);
    assert!(
        job.complete(&bad.to_string(), "r".into(), "m".into())
            .is_err()
    );
    assert!(e.validate_local(b.records(), id).is_err());
    b.sent("The exit is blocked.").unwrap();
    let sent = b.records().last().unwrap().id;
    assert!(
        Job::new("Subject".into(), "Goal".into(), b.records(), None, false)
            .complete(&feedback(sent).to_string(), "r".into(), "m".into())
            .is_err()
    );
    b.save_playtest(e.clone()).unwrap();
    let archive = Archive::project(b.records()).unwrap();
    assert_eq!(archive.entries.len(), 1);
    let docket = archive.docket().unwrap();
    let question = Job::new(
        "Refuter".into(),
        "Challenge travel complaint".into(),
        &[],
        Some(docket.clone()),
        false,
    );
    assert!(!question.prompt().unwrap().contains("request-revision"));
    let missing = json!({"challenges":[]});
    assert!(
        question
            .complete(&missing.to_string(), "r".into(), "m".into())
            .is_err()
    );
    let uncertain = json!({"challenges":[{"claim":docket.claims[0].key,"verdict":"needs_retest","rationale":"I have no local observations.","evidence":[],"proposed_test":"Try north and then the listed east exit."}]});
    question
        .complete(&uncertain.to_string(), "r".into(), "m".into())
        .unwrap();
    // Review allegations stay out of the normal character recall path.
    let recall = b.recall_with_limit("Cannot reach Lounge", 8000);
    assert!(
        !serde_json::to_string(&recall)
            .unwrap()
            .contains("suggested_change")
    );
}

#[test]
fn mission_binds_endpoint_and_retains_permission_allowance() {
    let mut b = book();
    let spec =
        mush_bot::mission::Spec::load(&root().join("bots/adversarial/mission.json")).unwrap();
    assert!(spec.bind(&Ssh::default(), false).is_err());
    spec.bind(&Ssh::default(), true).unwrap();
    let mut s = mush_bot::mission::State::default();
    s.load(spec.clone()).unwrap();
    s.active = true;
    b.save_mission(&s).unwrap();
    assert!(s.reserve("say outside scope").is_err());
    assert!(s.reserve("north").unwrap());
    b.save_mission(&s).unwrap();
    b.sent("north").unwrap();
    s.sent.push(b.records().last().unwrap().id);
    b.save_mission(&s).unwrap();
    observed(&mut b, "The exit is blocked.");
    b.finish_command().unwrap();
    let mut restored = mush_bot::mission::State::project(b.records()).unwrap();
    restored.load(spec.clone()).unwrap();
    assert_eq!(restored.reservations.len(), 1);
    let report = restored.report(b.records());
    assert_eq!(
        report["coverage"]["receipts"][0]["expectations"][0]["outcome"],
        "expectation_not_observed"
    );
    assert_eq!(
        report["coverage"]["receipts"][0]["expectations"][0]["engine_defect_proven"],
        false
    );
    let mut changed = spec;
    changed.max_actions += 1;
    assert!(restored.load(changed).is_err());
    let mut refund = restored.clone();
    refund.reservations.clear();
    refund.sent.clear();
    b.save_mission(&refund).unwrap();
    assert!(mush_bot::mission::State::project(b.records()).is_err());
}

#[tokio::test]
async fn ssh_mission_feedback_refutation_triage_and_restore() {
    let dir = temp();
    let mut m = fixture_manifest();
    m.actors.retain(|a| a.name != "Bystander");
    let mut config = mush_bot::config::Config::load(&m.actors[0].config).unwrap();
    config.mission = Some(root().join("bots/adversarial/mission.json"));
    config.playtest_feedback = true;
    let model_tape = dir.join("explore.json");
    tape(
        &model_tape,
        vec![
            decision("command", Some("north")),
            decision("command", Some("east")),
            decision("done", None),
            json!({"ease":3,"discoverability":3,"completion":"completed","summary":"Found the side door after north was blocked.","claims":[]}),
        ],
    );
    config.model_tape = Some(model_tape);
    let config_path = dir.join("subject.toml");
    std::fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();
    m.actors[0].config = config_path;
    m.branches[0].steps = vec![
        step("Guide", "/send close-door", Some("command-result")),
        step("Subject", "/mission start", Some("playtest-result")),
    ];
    let first = dir.join("exploration");
    let result = campaign::run(&m, &first, Path::new(env!("CARGO_BIN_EXE_mush-bot")))
        .await
        .unwrap_or_else(|e| panic!("{e}: {}", first.display()));
    let checkpoint = Checkpoint::load(&result.branches[0].checkpoint.path).unwrap();
    let subject = &checkpoint.bots["Subject"];
    assert_eq!(subject.version, 4);
    assert_eq!(subject.mission.as_ref().unwrap().sent.len(), 2);
    assert_eq!(subject.playtest.as_ref().unwrap().entries.len(), 1);
    assert_eq!(subject.resume_status, "done");
    assert_eq!(subject.usage.requests, 4);
    let trace = std::fs::read_to_string(first.join("encounter/Subject/trace.jsonl")).unwrap();
    assert!(trace.contains("playtest_feedback"));
    assert!(!trace.contains("control-error"));
    let obs = |needle: &str| {
        subject.memory.records.iter().find(|r|matches!(&r.event,Event::Episode(e) if e.source==Source::Observed && e.text.contains(needle))).unwrap().id
    };
    let mut report = feedback(obs("The exit is blocked."));
    report["claims"].as_array_mut().unwrap().push(json!({"id":"help-side-door","category":"discoverability","title":"Help omits the side-door directions","expected":"Help should list the movement commands usable here.","expectation_basis":"observed_documentation","observed":"Help lists north and south but omits east and west; look lists the side door.","suggested_change":"Include east and west in fixture help and explain blocked exits.","evidence":[{"episode":obs("Owned rehearsal fixture:"),"quote":"north, south, time."}]}));
    let feedback_tape = dir.join("feedback.json");
    tape(&feedback_tape, vec![report]);
    m.from_checkpoint = Some(result.branches[0].checkpoint.path.clone());
    m.branches[0].name = "review".into();
    m.branches[0].steps = vec![
        step(
            "Subject",
            &format!("/tape {}", feedback_tape.display()),
            None,
        ),
        step("Subject", "/feedback", Some("playtest-result")),
    ];
    let reviewed = campaign::run(
        &m,
        &dir.join("review"),
        Path::new(env!("CARGO_BIN_EXE_mush-bot")),
    )
    .await
    .unwrap();
    let c = Checkpoint::load(&reviewed.branches[0].checkpoint.path).unwrap();
    let archive = c.bots["Subject"].playtest.as_ref().unwrap();
    let docket = archive.docket().unwrap();
    assert_eq!(docket.claims.len(), 2);
    let refute = dir.join("refute.json");
    tape(
        &refute,
        vec![
            json!({"challenges":[{"claim":docket.claims[0].key,"verdict":"refuted","rationale":"East successfully reached the Lounge; north being blocked does not mean every route fails.","evidence":[{"episode":obs("Lounge(#"),"quote":"Lounge(#3)"}],"proposed_test":"No further test needed for the claim that no route exists."},{"claim":docket.claims[1].key,"verdict":"supported","rationale":"The observed help omitted east and west while the room listed the side door.","evidence":[{"episode":obs("Owned rehearsal fixture:"),"quote":"north, south, time."}],"proposed_test":"Have an operator compare help with the displayed exits and triage the omission."}]}),
        ],
    );
    let docket_path = dir.join("docket.json");
    save_json(&docket_path, &docket).unwrap();
    m.from_checkpoint = Some(reviewed.branches[0].checkpoint.path.clone());
    m.branches[0].name = "refutation".into();
    m.branches[0].steps = vec![
        step("Subject", &format!("/tape {}", refute.display()), None),
        step(
            "Subject",
            &format!("/refute {}", docket_path.display()),
            Some("playtest-result"),
        ),
    ];
    let challenged = campaign::run(
        &m,
        &dir.join("refutation"),
        Path::new(env!("CARGO_BIN_EXE_mush-bot")),
    )
    .await
    .unwrap();
    let c = Checkpoint::load(&challenged.branches[0].checkpoint.path).unwrap();
    let archive = c.bots["Subject"].playtest.as_ref().unwrap();
    assert_eq!(archive.entries.len(), 3);
    assert_eq!(
        c.bots["Subject"].mission.as_ref().unwrap().sent.len(),
        2,
        "reviews must never send commands or reset allowance"
    );
    let archive_path = dir.join("archive.json");
    save_json(&archive_path, archive).unwrap();
    let study_path = dir.join("study.json");
    save_json(
        &study_path,
        &json!({"version":1,"archives":["archive.json"]}),
    )
    .unwrap();
    let backlog = playtest::consolidate(&study_path).unwrap();
    assert!(
        backlog["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["status"] == "refuted")
    );
    assert!(
        !backlog["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["status"] == "actionable")
    );
    let triaged = dir.join("triaged.json");
    save_json(&triaged,&json!({"version":1,"archives":["archive.json"],"dispositions":[{"claim":docket.claims[1].key,"operator":"test-fixture-operator","rationale":"Test disposition: the fresh SSH run reproduces help omitting the available east side door.","actionable":true}]})).unwrap();
    let backlog = playtest::consolidate(&triaged).unwrap();
    assert_eq!(
        backlog["issues"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|i| i["status"] == "actionable")
            .count(),
        1
    );
    let final_trace =
        std::fs::read_to_string(dir.join("refutation/refutation/Subject/trace.jsonl")).unwrap();
    assert!(final_trace.contains("playtest_refutation"));
    // Missing checkpoint contribution must fail before reconnecting.
    let mut bad = serde_json::to_value(&c.bots["Subject"]).unwrap();
    bad.as_object_mut().unwrap().remove("playtest");
    assert!(
        serde_json::from_value::<mush_bot::checkpoint::Snapshot>(bad)
            .unwrap()
            .validate()
            .is_err()
    );
}

#[tokio::test]
async fn adversarial_reduction_final_rerun_and_blind_packet() {
    let dir = temp();
    let m = fixture_manifest();
    let (mut spec, _) =
        reproduction::Spec::load(&root().join("bots/adversarial/reproduce.json")).unwrap();
    spec.repeats = 1;
    spec.max_trials = 16;
    spec.max_requests = 80;
    let r = reproduction::run(
        &spec,
        &m,
        &dir.join("reduced"),
        Path::new(env!("CARGO_BIN_EXE_mush-bot")),
        true,
    )
    .await
    .unwrap_or_else(|e| panic!("{e}: {}", dir.display()));
    assert_eq!(r.status, Status::Reproduced);
    assert!(r.removed_actors.contains(&"Bystander".into()));
    assert!(!r.retained_original_steps.contains(&0));
    assert!(r.trials.last().unwrap().purpose.contains("final_export"));
    r.trials.last().unwrap().verify(&spec.oracle).unwrap();
    assert_eq!(r.provider_spend_usd, "0");
    let final_checkpoint = r.trials.last().unwrap().checkpoint.as_ref().unwrap();
    let c = Checkpoint::load(&final_checkpoint.path).unwrap();
    let mut reversed = spec.oracle.clone();
    reversed.reverse();
    assert!(reproduction::observe(&c, &reversed).unwrap().is_none());
    let mut fake = spec.oracle.clone();
    fake[0].line = "say I have arrived in the Lounge.".into();
    assert!(
        reproduction::observe(&c, &fake).unwrap().is_none(),
        "sent commands are not observations"
    );
    let mut paired = Manifest::load(&root().join("bots/adversarial/paired-campaign.json")).unwrap();
    paired.fixture_program = m.fixture_program;
    paired.fixture_args = m.fixture_args;
    let pair = campaign::run(
        &paired,
        &dir.join("paired"),
        Path::new(env!("CARGO_BIN_EXE_mush-bot")),
    )
    .await
    .unwrap();
    let careful = Checkpoint::load(&pair.branches[1].checkpoint.path).unwrap();
    assert!(
        reproduction::observe(&careful, &spec.oracle)
            .unwrap()
            .is_none()
    );
    let mut blind: Value = load_json(
        &root().join("bots/adversarial/blind-spec.example.json"),
        128_000,
    )
    .unwrap();
    blind["scenes"][0]["checkpoint"] = json!(pair.branches[0].checkpoint.path);
    blind["scenes"][1]["checkpoint"] = json!(pair.branches[1].checkpoint.path);
    let path = dir.join("blind.json");
    save_json(&path, &blind).unwrap();
    let p = mush_bot::blind_review::build(&path, &dir.join("packet")).unwrap();
    let encoded = serde_json::to_string(&p).unwrap();
    assert!(!encoded.contains("scene-a"));
    assert!(!encoded.contains("scene-b"));
    assert!(!encoded.contains("checkpoint"));
    assert!(encoded.contains("Guide says"));
    assert!(dir.join("packet/operator-reveal.json").exists());
    let incomplete = mush_bot::blind_review::Ratings {
        version: 1,
        packet: p.id.clone(),
        rubric_revision: revision(&serde_json::to_vec(&p.rubric).unwrap()),
        reviewer: "fixture-validator-not-human-rating".into(),
        complete: false,
        scores: vec![],
    };
    incomplete.validate(&p).unwrap();
}
