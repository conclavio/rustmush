//! M6.10 production composition, including real signed SSH typed authoring.
//! Direct store reads below are test assertions about immutable custody only;
//! all gameplay/author actions go through the authenticated public front door.
use mush_proto::{InvokeResult, MindDraft, MindEditBase, MindRequest, MindResponse};
use mush_store::SshKeyRec;
use russh::{
    ChannelMsg,
    keys::{HashAlg, PrivateKey, PrivateKeyWithHashAlg},
};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[path = "support/arena.rs"]
mod rig;
const SOURCE: &str = include_str!("../../tacsim/content/minds/acceptance/field_pilot.rhai");
const ENVELOPE: &str =
    include_str!("../../tacsim/content/minds/acceptance/field_pilot.envelope.json");
// Publicly committed loopback-only fixture; never used outside isolated tests.
const TEST_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\nQyNTUxOQAAACAimbLmJ8+bw424vILUEi+XY4bI9f/KJQPsh5DzC6fyogAAAKANWMViDVjF\nYgAAAAtzc2gtZWQyNTUxOQAAACAimbLmJ8+bw424vILUEi+XY4bI9f/KJQPsh5DzC6fyog\nAAAEAxj4SkgEcYjlGJjsCyLFctOcsJ/hm6ZWXlqQgTZw5esSKZsuYnz5vDjbi8gtQSL5dj\nhsj1/8olA+yHkPMLp/KiAAAAFmxvb3BiYWNrLXRlc3RAcnVzdG11c2gBAgMEBQYH\n-----END OPENSSH PRIVATE KEY-----\n";
struct Pin(String);
impl russh::client::Handler for Pin {
    type Error = russh::Error;
    async fn check_server_key(
        &mut self,
        key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        Ok(key.public_key().fingerprint(HashAlg::Sha256).to_string() == self.0)
    }
}
async fn register_key(server: &rig::Rig, account: &str) {
    let key = PrivateKey::from_openssh(TEST_KEY).unwrap();
    server
        .store
        .set_account_ssh_keys(
            account,
            &[SshKeyRec {
                openssh: key.public_key().to_openssh().unwrap(),
                fingerprint: key.public_key().fingerprint(HashAlg::Sha256).to_string(),
                added: 0,
            }],
        )
        .await
        .expect("public test key registration");
}
async fn invoke(server: &rig::Rig, user: &str, request: MindRequest) -> (MindResponse, u32) {
    let command = match &request {
        MindRequest::Read { .. } => "mind pull",
        MindRequest::Check { .. } => "mind check",
        _ => "mind push",
    };
    let mut client = russh::client::connect(
        Arc::new(russh::client::Config::default()),
        server.addr,
        Pin(server.pin.clone()),
    )
    .await
    .unwrap();
    let key =
        PrivateKeyWithHashAlg::new(Arc::new(PrivateKey::from_openssh(TEST_KEY).unwrap()), None);
    assert!(
        client
            .authenticate_publickey(user, key)
            .await
            .unwrap()
            .success()
    );
    let mut channel = client.channel_open_session().await.unwrap();
    channel
        .exec(true, command.as_bytes().to_vec())
        .await
        .unwrap();
    let bytes = serde_json::to_vec(&request).unwrap();
    for packet in bytes.chunks(127) {
        channel.data(packet).await.unwrap();
    }
    channel.eof().await.unwrap();
    let answer = tokio::time::timeout(Duration::from_secs(60), async {
        let mut bytes = Vec::new();
        let mut status = None;
        while let Some(message) = channel.wait().await {
            match message {
                ChannelMsg::Data { data } => bytes.extend_from_slice(&data),
                ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
                ChannelMsg::ExtendedData { data, .. } => {
                    assert!(data.is_empty(), "no private text fallback")
                }
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        let mut records = bytes
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice::<InvokeResult>(line).expect("typed NDJSON only"))
            .collect::<Vec<_>>();
        let InvokeResult::Mind(answer) = records.pop().expect("final result");
        assert!(answer.is_final());
        (answer, status.expect("final SSH exit status"))
    })
    .await
    .expect("bounded worker completion");
    let _ = client
        .disconnect(russh::Disconnect::ByApplication, "done", "")
        .await;
    answer
}
fn field_draft() -> MindDraft {
    let mut envelope: Value = serde_json::from_str(ENVELOPE).unwrap();
    envelope
        .as_object_mut()
        .unwrap()
        .insert("source".into(), SOURCE.into());
    serde_json::from_value(envelope).unwrap()
}
async fn read(server: &rig::Rig, name: &str) -> (MindEditBase, Box<MindDraft>) {
    let (answer, status) = invoke(
        server,
        "author+Pilot",
        MindRequest::Read {
            name: name.into(),
            revision: None,
        },
    )
    .await;
    assert_eq!(status, 0);
    let MindResponse::Read { base, draft, .. } = answer else {
        panic!("owned Read expected");
    };
    (base, draft)
}
async fn replace(
    server: &rig::Rig,
    name: &str,
    base: MindEditBase,
    draft: Box<MindDraft>,
) -> MindEditBase {
    let (answer, status) = invoke(
        server,
        "author+Pilot",
        MindRequest::Replace {
            name: name.into(),
            base,
            draft,
        },
    )
    .await;
    assert_eq!(status, 0);
    let MindResponse::Edited { base, .. } = answer else {
        panic!("working edit expected");
    };
    base
}
async fn save(server: &rig::Rig, name: &str, base: MindEditBase) -> (MindEditBase, u64, String) {
    let (answer, status) = invoke(
        server,
        "author+Pilot",
        MindRequest::Save {
            name: name.into(),
            base,
            notes: "M6 acceptance".into(),
            version: None,
        },
    )
    .await;
    assert_eq!(status, 0, "save has a final exit status");
    let MindResponse::Saved {
        base,
        revision,
        digest,
        ..
    } = answer
    else {
        panic!("immutable save expected");
    };
    (base, revision, digest)
}
async fn document(server: &rig::Rig, key: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Some(value) = server.store.module_json_get("tacsim", key).await.unwrap() {
                return value;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("deferred writer persisted document")
}
fn state(doc: &Value) -> Value {
    serde_json::from_str(doc["state"].as_str().expect("canonical state string")).unwrap()
}
fn selected_seal(live: &Value, name: &str) -> (String, Value) {
    let state = state(live);
    state["record"]["minds"]
        .as_object()
        .expect("custody minds")
        .iter()
        .find_map(|(unit, row)| {
            (row["seal"]["requested"]["mind"] == name).then(|| (unit.clone(), row["seal"].clone()))
        })
        .expect("the requested authored mind is sealed")
}
async fn wait_snapshot(server: &rig::Rig, match_id: u64, ready: impl Fn(&Value) -> bool) -> Value {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let doc = document(server, &format!("tacsim:snapshot:{match_id}")).await;
            if ready(&state(&doc)) {
                return doc;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("actual recorded arena state, not assumed wall time")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "full M6 author story and thirty private MUSTER games; explicit acceptance run"]
async fn field_pilot_complete_production_author_story() {
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let config = rig::mind_acceptance_config();
    let server = rig::boot(
        config,
        &[
            ("author", "Pilot"),
            ("observer", "Observer"),
            ("enemy", "Enemy"),
        ],
    )
    .await;
    let mut author = rig::Client::login(&server, "author", rig::PILOT_PASSWORD, "Pilot").await;
    author.say("@newchar Alt", "created Alt").await;
    register_key(&server, "author").await;
    let mut observer =
        rig::Client::login(&server, "observer", rig::PILOT_PASSWORD, "Observer").await;
    let mut enemy = rig::Client::login(&server, "enemy", rig::PILOT_PASSWORD, "Enemy").await;
    let mut god = rig::Client::login(&server, rig::GOD_ACCOUNT, rig::GOD_PASSWORD, "God").await;

    let (created, status) = invoke(
        &server,
        "author+Pilot",
        MindRequest::Create {
            name: "FieldPilot".into(),
            from: None,
        },
    )
    .await;
    assert_eq!(status, 0);
    assert!(matches!(created, MindResponse::Edited { .. }));
    let (base, _) = read(&server, "FieldPilot").await;
    let base = replace(&server, "FieldPilot", base, Box::new(field_draft())).await;
    let (checked, status) = invoke(
        &server,
        "author+Pilot",
        MindRequest::Check {
            name: "FieldPilot".into(),
            base,
        },
    )
    .await;
    assert_eq!(status, 0);
    let MindResponse::Checked {
        reports,
        diagnostics,
        ..
    } = checked
    else {
        panic!("six picture result");
    };
    assert!(diagnostics.is_empty());
    assert_eq!(reports.len(), 6);
    assert!(reports.iter().all(|r| r.fault.is_none()
        && r.class_violations.is_empty()
        && r.station_refusals.is_empty()
        && r.operations + r.fees <= 3000));
    assert_eq!(
        reports.iter().filter(|r| !r.armed).count(),
        3,
        "actual unarmed pictures"
    );
    let (saved_base, revision, definition) = save(&server, "FieldPilot", base).await;
    assert_eq!(revision, 1);
    let (read_base, draft) = read(&server, "FieldPilot").await;
    assert_eq!(read_base, saved_base);
    assert_eq!(draft.source, SOURCE);

    // Same account is not source ownership. Missing and foreign local names
    // produce the same closed response, with no excerpt or owner identifier.
    let (foreign, status) = invoke(
        &server,
        "author+Alt",
        MindRequest::Read {
            name: "FieldPilot".into(),
            revision: None,
        },
    )
    .await;
    assert_eq!(status, 1);
    let (missing, _) = invoke(
        &server,
        "author+Alt",
        MindRequest::Read {
            name: "DoesNotExist".into(),
            revision: None,
        },
    )
    .await;
    assert_eq!(foreign, missing);
    for outsider in [&mut observer, &mut enemy, &mut god] {
        let (answer, text) = outsider
            .answer(
                "@mind/source FieldPilot",
                &["No such owned mind or revision."],
            )
            .await;
        assert_eq!(
            answer,
            Some(0),
            "an actual ownership refusal, not a timeout: {text}"
        );
        assert!(!text.contains("fn think") && !text.contains("const MIND"));
    }

    rig::walk_to_lobby(&mut author).await;
    rig::walk_to_lobby(&mut observer).await;
    rig::walk_to_lobby(&mut enemy).await;
    rig::walk_to_lobby(&mut god).await;
    author.say("SIM TEAM red", "takes red slot 1").await;
    author
        .say("SIM MIND FieldPilot PROFILE Hold", "Your slot will carry")
        .await;
    author.say("SIM START", "is boarding").await;
    author.say("red", "Red Room").await;
    author.say("BOARD", "climb into").await;
    author.expect("Good hunting", rig::STEP_TIMEOUT).await;
    author.say("MIND", "FieldPilot").await;
    let live = document(&server, "tacsim:live:1").await;
    let (unit, sealed) = selected_seal(&live, "FieldPilot");
    assert_eq!(sealed["engine"], "scripted");
    assert_eq!(sealed["class"], "II");
    assert_eq!(sealed["requested"]["origin"]["digest"], definition);
    author.say("EXIT", "You climb out of").await;
    let snapshot = wait_snapshot(&server, 1, |state| {
        state["now_ms"].as_i64().is_some_and(|now| now >= 6000)
            && state["minds"][&unit]["think_seq"]
                .as_u64()
                .is_some_and(|seq| seq >= 2)
    })
    .await;
    // Replay through this observed authored activity, rounded up to a public
    // whole-second BENCH target. Keep the sortie alive beyond it before ending.
    let bench_seconds = (state(&snapshot)["now_ms"].as_i64().unwrap() + 999) / 1000;
    assert!(
        state(&snapshot)["minds"][&unit]["think_seq"]
            .as_u64()
            .unwrap_or(0)
            >= 2,
        "release really schedules the authored runner"
    );

    let unit_selector = format!("#{}", unit.trim_start_matches('#'));
    let trace = author
        .say(&format!("@mind/trace {unit_selector} 3 ONCE"), "Think ")
        .await;
    assert!(trace.contains("ops") && !trace.contains("fn think") && !trace.contains("const MIND"));
    let (denied, denied_trace) = observer
        .answer(
            &format!("@mind/trace {unit_selector} 3 ONCE"),
            &["No mind trace is available."],
        )
        .await;
    assert_eq!(denied, Some(0));
    assert!(!denied_trace.contains("Landing:"));

    // Edit and save while that exact sortie is still running. Both RAM-side
    // readback and deferred persisted custody must retain the first definition.
    let (base, mut changed) = read(&server, "FieldPilot").await;
    changed
        .source
        .push_str("\n// second local revision, after checkout\n");
    changed.profiles["Hold"]["band_m"] = json!(400);
    let base = replace(&server, "FieldPilot", base, changed).await;
    let (_, revision2, definition2) = save(&server, "FieldPilot", base).await;
    assert_eq!(revision2, 2);
    assert_ne!(definition, definition2);
    let still_running_at = 10_000.max(bench_seconds * 1000 + 1000);
    let next = wait_snapshot(&server, 1, |state| {
        state["now_ms"]
            .as_i64()
            .is_some_and(|now| now >= still_running_at)
    })
    .await;
    assert!(state(&next)["now_ms"].as_i64().unwrap() >= still_running_at);
    let (_, still_sealed) = selected_seal(&document(&server, "tacsim:live:1").await, "FieldPilot");
    assert_eq!(
        still_sealed, sealed,
        "editing source/cards cannot rewrite running custody"
    );
    // A stale optimistic edit is refused after the newer save.
    let (stale, status) = invoke(
        &server,
        "author+Pilot",
        MindRequest::Replace {
            name: "FieldPilot".into(),
            base: saved_base,
            draft: Box::new(field_draft()),
        },
    )
    .await;
    assert_eq!(status, 1);
    assert!(matches!(stale, MindResponse::Error { .. }));

    // Close the actual sortie before the private workloads. Concession is a
    // public lead action; durable terminal state, not an old transcript match,
    // establishes that custody and the recording have closed.
    author.say("SIM CONCEDE", "concedes").await;
    let first_result = document(&server, "tacsim:match:1").await;
    author.say("SIM REPORT 1", "Match 1 on").await;
    // The first immutable revision, matching the independent external default,
    // is what the thirty-game acceptance proves. Later edits remain independent.
    author
        .say(
            "@mind/prove FieldPilot@1/Hold VS Standard/Line 30 MUSTER 1v1 SEED 1",
            "Proving preparation queued",
        )
        .await;
    author
        .expect(
            "Proving run 1 retained and queued.",
            Duration::from_secs(60),
        )
        .await;
    author
        .expect("Proving run 1 is Complete.", Duration::from_secs(30 * 60))
        .await;
    let report = author.say("@mind/report 1", "Win rate").await;
    assert!(report.contains("of 30") && report.contains("Wilson"));
    let (denied, observer_report) = observer
        .answer("@mind/report 1", &["No such owned run or bench."])
        .await;
    assert_eq!(denied, Some(0));
    assert!(!observer_report.contains("Win rate"));
    let manifests = server.store.module_json_list("tacsim").await.unwrap();
    let manifest = manifests
        .iter()
        .find_map(|(key, value)| {
            (key.contains(":mindbulk:blob:") && key.ends_with(":1:result")).then_some(value)
        })
        .expect("actual retained proof manifest")
        .clone();
    assert!(
        manifest["context"].is_object(),
        "frozen scenario identity is retained with the result"
    );

    let unit_number = unit.trim_start_matches('#').parse::<u64>().unwrap();
    let callsign = first_result["units"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["unit"].as_u64() == Some(unit_number))
        .unwrap()["callsign"]
        .as_str()
        .unwrap();
    author
        .say(&format!("BENCH LOAD MATCH 1 {callsign}"), "Bench loading")
        .await;
    author
        .expect(
            "Bench ready; verified reference at 0 ms.",
            Duration::from_secs(60),
        )
        .await;
    author
        .say(
            "BENCH STEP 2",
            "Bench step complete; reference hashes matched.",
        )
        .await;
    author
        .say(
            "BENCH BACK 1",
            "Bench step complete; reference hashes matched.",
        )
        .await;
    author
        .say(
            &format!(
                "BENCH RUN TO {}:{:02}",
                bench_seconds / 60,
                bench_seconds % 60
            ),
            "Bench step complete; reference hashes matched.",
        )
        .await;
    let bench_pic = author.say("BENCH PIC", "Bench picture").await;
    let bench_trace = author.say("BENCH TRACE 3", "Think ").await;
    let bench_eval = author.say("BENCH EVAL 1 + 1", "EVAL 2 (").await;
    assert!(!bench_pic.contains("fn think") && !bench_trace.contains("const MIND"));
    let (denied, foreign_bench) = observer
        .answer(
            &format!("BENCH LOAD MATCH 1 {callsign}"),
            &["No such owned run or bench."],
        )
        .await;
    assert_eq!(denied, Some(0));
    assert!(!foreign_bench.contains("Bench ready"));
    author.say("BENCH CLOSE", "Bench closed").await;

    // A source that compiles but faults at execution is a saved revision,
    // never a forged validator receipt. Its third actual think drops to ROM.
    let (created, status) = invoke(
        &server,
        "author+Pilot",
        MindRequest::Create {
            name: "FaultProbe".into(),
            from: None,
        },
    )
    .await;
    assert_eq!(status, 0);
    assert!(matches!(created, MindResponse::Edited { .. }));
    let (base, _) = read(&server, "FaultProbe").await;
    let mut bad = field_draft();
    bad.source = "fn think(v,c) { while true {} }".into();
    let base = replace(&server, "FaultProbe", base, Box::new(bad)).await;
    let _ = save(&server, "FaultProbe", base).await;
    author.say("out", "Simulator Lobby").await;
    author.say("SIM TEAM red", "takes red slot 1").await;
    author
        .say("SIM MIND FaultProbe PROFILE Hold", "Your slot will carry")
        .await;
    author.say("SIM START", "is boarding").await;
    author.say("red", "Red Room").await;
    author.say("BOARD", "climb into").await;
    author.expect("Good hunting", rig::STEP_TIMEOUT).await;
    let (fault_unit, _) = selected_seal(&document(&server, "tacsim:live:2").await, "FaultProbe");
    author.say("EXIT", "You climb out of").await;
    let faulted = wait_snapshot(&server, 2, |state| {
        state["minds"][&fault_unit]["dropped"] == "strikes"
    })
    .await;
    assert_eq!(state(&faulted)["minds"][&fault_unit]["dropped"], "strikes");
    author.say("SIM CONCEDE", "concedes").await;
    let _ = document(&server, "tacsim:match:2").await;

    // Opposing humans preserve the accepted discretion-to-Doctrine substitution.
    author.say("out", "Simulator Lobby").await;
    author.say("SIM TEAM red", "takes red slot 1").await;
    author
        .say("SIM MIND FieldPilot@1 PROFILE Hold", "Your slot will carry")
        .await;
    enemy.say("SIM TEAM blue", "takes blue slot 1").await;
    author.say("SIM START", "is boarding").await;
    author.say("red", "Red Room").await;
    enemy.say("blue", "Blue Room").await;
    author.say("BOARD", "climb into").await;
    enemy.say("BOARD", "climb into").await;
    author.expect("Good hunting", rig::STEP_TIMEOUT).await;
    let substituted_live = state(&document(&server, "tacsim:live:3").await);
    let substitutions = substituted_live["record"]["minds"]
        .as_object()
        .unwrap()
        .values()
        .filter(|row| row["seal"]["fallback"] == tacsim::match_::HUMANS_FALLBACK)
        .map(|row| row["seal"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        substitutions.len(),
        1,
        "only the selected authored hull is substituted"
    );
    let substituted = substitutions.into_iter().next().unwrap();
    assert_eq!(substituted["mind"], tacsim::match_::SEALED_MIND);
    assert_eq!(substituted["fallback"], "humans on the other side");
    assert_ne!(substituted["engine"], "scripted");
    author.say("SIM CONCEDE", "concedes").await;
    let _ = document(&server, "tacsim:match:3").await;

    if let Some(directory) = std::env::var_os("M6_AUTHOR_STORY_OUT") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir(&directory).expect("new acceptance directory");
        let manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        let manifest_sha256 = Sha256::digest(&manifest_bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        std::fs::write(directory.join("proving-manifest.json"), manifest_bytes).unwrap();
        std::fs::write(directory.join("author-story.json"),serde_json::to_vec_pretty(&json!({
            "format":1,"revision":std::env::var("M6_ACCEPTANCE_REVISION").expect("exact tested revision"),
            "run_id":std::env::var("M6_ACCEPTANCE_RUN_ID").expect("one fresh acceptance run id"),
            "proving_manifest_sha256":manifest_sha256,
            "started_unix_ms":started,"first_definition":definition,"second_definition":definition2,
            "finished_unix_ms":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis(),
            "first_seal":sealed,"substituted_seal":substituted,"checks":reports,
            "trace":trace,"bench":{"picture":bench_pic,"trace":bench_trace,"eval":bench_eval},
        })).unwrap()).unwrap();
    }
    author.close().await;
    observer.close().await;
    enemy.close().await;
    god.close().await;
}
