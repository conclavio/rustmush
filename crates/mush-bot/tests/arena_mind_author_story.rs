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
        let InvokeResult::Mind(answer) = records.pop().expect("final result") else {
            panic!("Mind-only author story")
        };
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
// Observe only this PROVE request. A known terminal refusal must not consume
// the long completion guard, and waiting never resubmits the public command.
async fn proving_progress(
    client: &mut rig::Client,
    from: usize,
    expected: &str,
    timeout: Duration,
) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let text = client
            .seen
            .get(from..)
            .expect("the current PROVE transcript");
        for failure in [
            "Proving preparation refused:",
            "Proving run 1 is Failed.",
            "Proving run 1 is Interrupted.",
            "Proving run 1 is Cancelled.",
        ] {
            assert!(
                !text.contains(failure),
                "proving failed explicitly:\n{text}"
            );
        }
        if text.contains(expected) {
            return;
        }
        assert!(!client.closed, "SSH closed before {expected:?}:\n{text}");
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(
            !left.is_zero(),
            "proving exceeded its liveness guard waiting for {expected:?}:\n{text}"
        );
        client.pump(left.min(Duration::from_millis(100))).await;
    }
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
async fn retained_blob(server: &rig::Rig, key: &str, manifest: &Value) -> Vec<u8> {
    let chunks = manifest["chunks"].as_u64().unwrap();
    assert!((1..=513).contains(&chunks));
    let mut bytes = Vec::new();
    for index in 0..chunks {
        let chunk = document(server, &format!("{key}:chunk:{index}")).await;
        bytes.extend_from_slice(chunk.as_str().expect("retained UTF-8 chunk").as_bytes());
        assert!(bytes.len() <= 16 * 1024 * 1024);
    }
    assert_eq!(
        bytes.len() as u64,
        manifest["artifact"]["bytes"].as_u64().unwrap()
    );
    assert_eq!(
        json!(Sha256::digest(&bytes).to_vec()),
        manifest["artifact"]["sha256"],
        "evidence comes from the exact retained artifact"
    );
    bytes
}
fn selected_profile<'a>(seal: &'a Value, expected: &str) -> &'a Value {
    let selected = seal["cards"]["selected"].as_array().unwrap();
    assert_eq!(selected.len(), 1, "one actual selected profile");
    assert_eq!(selected[0]["name"], expected);
    let index = selected[0]["key"]["index"].as_u64().unwrap() as usize;
    let profile = &seal["cards"]["definition"]["profiles"][index];
    assert_eq!(
        profile["name"], expected,
        "binding resolves to its actual card"
    );
    profile
}
const MAX_FROZEN_GAME_BYTES: usize = 16 * 1024 * 1024;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FrozenGameSummary<'a> {
    seed: u64,
    swapped: bool,
    match_id: u64,
    unit_count: u16,
    raw_bytes: u32,
    raw_sha256: &'a str,
    zlib: &'a str,
}
fn frozen_hex(value: &str, maximum: usize) -> Result<Vec<u8>, &'static str> {
    if value.is_empty()
        || value.len() > maximum * 2
        || !value.len().is_multiple_of(2)
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("frozen game canonical hex or byte bound");
    }
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|word| u8::from_str_radix(word, 16).ok())
                .ok_or("frozen game hex")
        })
        .collect()
}
// This is an independent evidence decoder, not a call into the game owner.
// Only one body is expanded at a time; the opaque snapshot string is untouched.
fn frozen_game(row: &Value) -> Result<Value, &'static str> {
    use serde::Deserialize as _;
    let summary = FrozenGameSummary::deserialize(row).map_err(|_| "frozen game summary shape")?;
    let size = usize::try_from(summary.raw_bytes).map_err(|_| "frozen game byte bound")?;
    if !(1..=MAX_FROZEN_GAME_BYTES).contains(&size)
        || summary.match_id == 0
        || !(2..=12).contains(&summary.unit_count)
    {
        return Err("frozen game summary bounds");
    }
    let digest = frozen_hex(summary.raw_sha256, 32)?;
    if digest.len() != 32 {
        return Err("frozen game digest shape");
    }
    let compressed = frozen_hex(summary.zlib, MAX_FROZEN_GAME_BYTES)?;
    // One extra byte detects an inflated body larger than its declared bound.
    // No unbounded read_to_end/flush or allocation from unchecked metadata.
    let mut raw = vec![0; size + 1];
    let mut decoder = flate2::Decompress::new(true);
    let status = decoder
        .decompress(&compressed, &mut raw, flate2::FlushDecompress::Finish)
        .map_err(|_| "frozen game invalid zlib stream")?;
    if status != flate2::Status::StreamEnd
        || decoder.total_in() != compressed.len() as u64
        || decoder.total_out() != u64::from(summary.raw_bytes)
    {
        return Err("frozen game length, truncated stream or trailing bytes");
    }
    raw.truncate(size);
    let actual_digest: [u8; 32] = Sha256::digest(&raw).into();
    if actual_digest.as_slice() != digest.as_slice() {
        return Err("frozen game raw digest differs");
    }
    let body: Value = serde_json::from_slice(&raw).map_err(|_| "frozen game JSON")?;
    const KEYS: [&str; 10] = [
        "seed", "swapped", "format", "match_id", "limit", "engine", "opening", "clock", "seals",
        "sources",
    ];
    // GameBody has precisely the ten original GameStart fields. Checking this
    // independently also prevents silently accepting a different wire version.
    if !body.as_object().is_some_and(|object| {
        object.len() == 10 && object.keys().all(|key| KEYS.contains(&key.as_str()))
    }) {
        return Err("frozen game body shape");
    }
    if body["seed"].as_u64() != Some(summary.seed)
        || body["swapped"].as_bool() != Some(summary.swapped)
        || body["match_id"].as_u64() != Some(summary.match_id)
        || body["format"].as_u64() != Some(1)
        || body["seals"].as_array().map(Vec::len) != Some(usize::from(summary.unit_count))
    {
        return Err("frozen game body differs from summary");
    }
    Ok(body)
}

#[test]
fn retained_frozen_game_decoder_preserves_exact_state_and_rejects_stream_tail() {
    use std::io::Write as _;
    let body = json!({"seed":u64::MAX,"swapped":true,"format":1,"match_id":17,
        "limit":MAX_FROZEN_GAME_BYTES,"engine":{},"opening":{"state":"{\"z\":1,\"a\":2}"},
        "clock":{},"seals":[{"unit":100},{"unit":200}],"sources":[]});
    let raw = serde_json::to_vec(&body).unwrap();
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&raw).unwrap();
    let compressed = encoder.finish().unwrap();
    let hex = |bytes: &[u8]| {
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    let row = json!({"seed":u64::MAX,"swapped":true,"match_id":17,"unit_count":2,
        "raw_bytes":raw.len(),"raw_sha256":hex(&Sha256::digest(&raw)),"zlib":hex(&compressed)});
    assert_eq!(frozen_game(&row).unwrap(), body);
    let mut trailing = compressed.clone();
    trailing.extend_from_slice(&compressed);
    for bytes in [
        b"bad".as_slice(),
        &compressed[..compressed.len() - 1],
        &trailing,
    ] {
        let mut bad = row.clone();
        bad["zlib"] = hex(bytes).into();
        assert!(frozen_game(&bad).is_err());
    }
    for (key, value) in [
        ("raw_bytes", json!(raw.len() - 1)),
        ("raw_bytes", json!(MAX_FROZEN_GAME_BYTES + 1)),
        ("raw_sha256", json!("00".repeat(32))),
        ("unit_count", json!(3)),
        ("match_id", json!(18)),
        ("seed", json!(0)),
        ("swapped", json!(false)),
        ("swapped", json!(1)),
        ("zlib", json!(hex(&compressed).to_uppercase())),
    ] {
        let mut bad = row.clone();
        bad[key] = value;
        assert!(frozen_game(&bad).is_err(), "{key} must be validated");
    }
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
async fn warm_cockpit(pilot: &mut rig::Client) {
    let from = pilot.seen.len();
    // Good hunting precedes the asynchronous first cockpit frame. Retry that
    // explicit public not-ready answer; waiting on the same reply cannot help.
    tokio::time::timeout(rig::STEP_TIMEOUT, async {
        loop {
            let (answer, text) = pilot
                .answer(
                    "STATUS",
                    &["m north;", "The panel has told you nothing yet."],
                )
                .await;
            let warmup = &pilot.seen[from..];
            assert!(
                !warmup.contains("DESTROYED"),
                "the live pilot has a working hull: {warmup}"
            );
            match answer {
                Some(0) => break,
                Some(1) => tokio::time::sleep(Duration::from_millis(75)).await,
                _ => panic!("STATUS did not return a ready or explicitly pending panel: {text}"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "first public cockpit frame did not arrive: {}",
            &pilot.seen[from..]
        )
    });
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
    let output = std::env::var_os("M6_AUTHOR_STORY_OUT").map(std::path::PathBuf::from);
    if let Some(directory) = &output {
        std::fs::create_dir(directory).expect("new acceptance directory");
    }
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
    author.create_character("Alt").await;
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
    warm_cockpit(&mut author).await;
    // Author commands accept mixed case; persisted and displayed mind names
    // use Roll's canonical lowercase identity.
    author
        .say("MIND", "Mind: fieldpilot, sealed for this sortie")
        .await;
    let live = document(&server, "tacsim:live:1").await;
    let (unit, sealed) = selected_seal(&live, "fieldpilot");
    assert_eq!(sealed["engine"], "scripted");
    assert_eq!(sealed["class"], "II");
    assert_eq!(
        sealed["source_sha256"],
        sealed["requested"]["source_sha256"]
    );
    assert_eq!(sealed["requested"]["origin"]["digest"], definition);
    let envelope: Value = serde_json::from_str(ENVELOPE).unwrap();
    assert_eq!(sealed["cards"]["definition"]["schema"], envelope["schema"]);
    let hold = selected_profile(&sealed, "Hold");
    assert_eq!(hold["values"], envelope["profiles"]["Hold"]);
    assert_eq!(
        hold["rules"],
        json!([]),
        "the accepted default has no authored rules"
    );
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
    assert_eq!(
        state(&snapshot)["minds"][&unit]["strikes"],
        0,
        "the released Field Pilot must not accrue live execution faults"
    );
    assert!(
        state(&snapshot)["minds"][&unit]["dropped"].is_null(),
        "the released Field Pilot must still be flying its authored seal"
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
    let (_, still_sealed) = selected_seal(&document(&server, "tacsim:live:1").await, "fieldpilot");
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

    // EXIT relinquishes side leadership, but the retained participant may
    // publicly abort. Its normal terminal result/tape closes custody before
    // private workloads and remains available to the reference BENCH below.
    author.say("SIM ABORT", "aborts match 1").await;
    let first_result = document(&server, "tacsim:match:1").await;
    author.say("SIM REPORT 1", "Match 1 on").await;
    // The first immutable revision, matching the independent external default,
    // is what the thirty-game acceptance proves. Later edits remain independent.
    let proving_from = author.seen.len();
    author
        .say(
            "@mind/prove FieldPilot@1/Hold VS Standard/Line 30 MUSTER 1v1 SEED 1",
            "Proving preparation queued",
        )
        .await;
    proving_progress(
        &mut author,
        proving_from,
        "Proving run 1 retained and queued.",
        Duration::from_secs(60),
    )
    .await;
    proving_progress(
        &mut author,
        proving_from,
        "Proving run 1 is Complete.",
        Duration::from_secs(30 * 60),
    )
    .await;
    let report = author.say("@mind/report 1", "Win rate").await;
    assert!(report.contains("of 30") && report.contains("Wilson"));
    let (denied, observer_report) = observer
        .answer("@mind/report 1", &["No such owned run or bench."])
        .await;
    assert_eq!(denied, Some(0));
    assert!(!observer_report.contains("Win rate"));
    let manifests = server.store.module_json_list("tacsim").await.unwrap();
    let (manifest_key, manifest) = manifests
        .iter()
        .find_map(|(key, value)| {
            (key.contains(":mindbulk:blob:") && key.ends_with(":1:result"))
                .then_some((key.clone(), value.clone()))
        })
        .expect("actual retained proof manifest");
    assert!(
        manifest["context"].is_object(),
        "frozen scenario identity is retained with the result"
    );
    let start_key = format!("{}:start", manifest_key.strip_suffix(":result").unwrap());
    let start_manifest = document(&server, &start_key).await;
    let start_bytes = retained_blob(&server, &start_key, &start_manifest).await;
    let result_bytes = retained_blob(&server, &manifest_key, &manifest).await;
    if let Some(directory) = &output {
        // Keep exact immutable bytes before later replay/fault assertions. These
        // artifacts contain only this isolated story's own test mind and stock.
        std::fs::write(directory.join("proving-start.json"), &start_bytes).unwrap();
        std::fs::write(directory.join("proving-result.json"), &result_bytes).unwrap();
        std::fs::write(
            directory.join("proving-start-manifest.json"),
            serde_json::to_vec_pretty(&start_manifest).unwrap(),
        )
        .unwrap();
        std::fs::write(
            directory.join("proving-manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }
    let frozen: Value = serde_json::from_slice(&start_bytes).unwrap();
    assert_eq!(frozen["format"], 2, "current frozen proving wire format");
    assert_eq!(frozen["context"], manifest["context"]);
    let games = frozen["games"].as_array().unwrap();
    assert_eq!(games.len(), 30);
    for (index, encoded_game) in games.iter().enumerate() {
        let game = frozen_game(encoded_game).expect("exact bounded format-2 game body");
        assert_eq!(game["swapped"], index % 2 == 1);
        assert_eq!(game["seed"], frozen["context"]["pair_seeds"][index / 2]);
        let seals = game["seals"].as_array().unwrap();
        assert_eq!(seals.len(), 2, "actual 1v1 frozen roster");
        let candidate = seals
            .iter()
            .find(|seal| seal["requested"]["mind"] == "fieldpilot")
            .expect("actual candidate seal");
        assert_eq!(candidate["engine"], "scripted");
        assert_eq!(candidate["class"], "II");
        assert_eq!(candidate["source_sha256"], sealed["source_sha256"]);
        assert_eq!(candidate["requested"], sealed["requested"]);
        assert_eq!(candidate["requested"]["origin"]["digest"], definition);
        assert_eq!(
            candidate["cards"], sealed["cards"],
            "original immutable card and rules"
        );
        assert_eq!(candidate["card_digest"], sealed["card_digest"]);
        selected_profile(candidate, "Hold");
        let opponent = seals
            .iter()
            .find(|seal| seal["requested"]["mind"] == tacsim::match_::SEALED_MIND)
            .expect("actual stock opponent seal");
        selected_profile(opponent, "Line");
        assert_eq!(opponent["class"], frozen["context"]["opponent"]["class"]);
        assert_eq!(
            opponent["source_sha256"],
            frozen["context"]["opponent"]["source_sha256"]
        );
        let opening = state(&game["opening"]);
        let actual_unit = opening["units"]
            .as_array()
            .unwrap()
            .iter()
            .find(|unit| unit["id"] == candidate["unit"])
            .expect("candidate appears in the actual frozen opening");
        assert_eq!(
            actual_unit["team"],
            if index % 2 == 1 { "blue" } else { "red" }
        );
    }

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

    // Replay one actual retained proving game to its terminal event, through
    // the public bench. Loading a live-sortie tape above proves a different
    // producer and cannot stand in for this private-result custody path.
    let proving_result: Value = serde_json::from_slice(&result_bytes).unwrap();
    assert_eq!(proving_result["context"], frozen["context"]);
    let first_tape = proving_result["tapes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tape| tape["game"] == 0)
        .expect("first proving game");
    assert_eq!(first_tape["recording"]["status"], "complete");
    let mark = &first_tape["recording"]["mark"];
    assert_eq!(mark["status"]["status"], "complete");
    let replay_at = mark["status"]["at"].as_i64().unwrap();
    let replay_events = mark["cursor"]["events"].as_u64().unwrap();
    let replay_hash = mark["status"]["hash"].as_u64().unwrap();
    assert!(replay_at > 0 && replay_events > 0);
    assert_eq!(
        proving_result["report"]["games"][0]["final_hash"],
        replay_hash
    );
    author.say("BENCH LOAD 1/1", "Bench loading").await;
    author
        .expect(
            "Bench ready; verified reference at 0 ms.",
            Duration::from_secs(60),
        )
        .await;
    author
        .send(&format!("BENCH RUN TO {}", replay_at / 1000))
        .await;
    author
        .expect(
            "Bench step complete; reference hashes matched.",
            Duration::from_secs(180),
        )
        .await;
    if replay_at % 1000 != 0 {
        // RUN TO accepts whole seconds. STEP clamps to the retained endpoint,
        // including a fractional final second, and drains its final barriers.
        author.send("BENCH STEP 1000").await;
        author
            .expect(
                "Bench step complete; reference hashes matched.",
                Duration::from_secs(180),
            )
            .await;
    }
    let proving_bench_pic = author
        .say(
            "BENCH PIC",
            &format!("Bench picture at {replay_at} ms, event {replay_events}."),
        )
        .await;
    // The public exact terminal cursor plus the successful reference-check
    // completion establishes that every event, including final hash, replayed.
    let proving_replay = json!({"run":1,"game":1,"at_ms":replay_at,
        "events":replay_events,"final_hash":format!("{replay_hash:016x}"),
        "terminal_cursor_verified":true});
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
    let (fault_unit, _) = selected_seal(&document(&server, "tacsim:live:2").await, "faultprobe");
    author.say("EXIT", "You climb out of").await;
    let faulted = wait_snapshot(&server, 2, |state| {
        state["minds"][&fault_unit]["dropped"] == "strikes"
    })
    .await;
    assert_eq!(state(&faulted)["minds"][&fault_unit]["dropped"], "strikes");
    author.say("SIM ABORT", "aborts match 2").await;
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

    if let Some(directory) = &output {
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
            "proving_start_artifact":start_manifest["artifact"],
            "proving_result_artifact":manifest["artifact"],
            "started_unix_ms":started,"first_definition":definition,"second_definition":definition2,
            "finished_unix_ms":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis(),
            "first_seal":sealed,"substituted_seal":substituted,"checks":reports,
            "trace":trace,"bench":{"picture":bench_pic,"trace":bench_trace,"eval":bench_eval},
            "proving_replay":proving_replay,"proving_bench_picture":proving_bench_pic,
        })).unwrap()).unwrap();
    }
    author.close().await;
    observer.close().await;
    enemy.close().await;
    god.close().await;
}
