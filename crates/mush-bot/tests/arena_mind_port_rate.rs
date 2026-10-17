//! Fresh external comparison for the separately authored M6.10 Rhai pilot.
//! A real mush-bot process flies fifteen MUSTER matches on each side. This
//! dev-only rig does not enter the standalone bot's runtime dependency graph.
use serde_json::json;
use sha2::{Digest as _, Sha256};
use std::{
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[path = "support/arena.rs"]
mod arena_rig;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "fresh 30-match real SSH interval; concurrent sides allow 210 minutes, explicit acceptance run only"]
async fn field_pilot_fresh_external_muster_interval() {
    let directory = PathBuf::from(
        std::env::var_os("M6_BOT_INTERVAL_OUT").expect("set a new M6_BOT_INTERVAL_OUT directory"),
    );
    std::fs::create_dir(&directory)
        .expect("new output directory: never overwrite an earlier interval");
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let revision =
        std::env::var("M6_ACCEPTANCE_REVISION").expect("set the exact tested git revision");
    let scenario = include_bytes!("../../tacsim/content/proving/scenarios.json");
    let ground = include_bytes!("../../tacsim/content/arenas/proving_ground.json");
    let atlas = include_bytes!("../../tacsim/content/atlas/proving_ground.json");
    let sha = |bytes: &[u8]| {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    let config = arena_rig::mind_acceptance_config();
    let config_sha = sha(&serde_json::to_vec(&(
        &config.arena,
        &config.match_,
        &config.sensors,
        &config.minds,
    ))
    .unwrap());
    let mut common_ground = None;
    let mut common_opponent = None;
    let mut matches = Vec::new();
    let mut sides = Vec::new();
    let mut counts = [0_u64; 4];
    // Each lane owns its world, store, loopback listener and PID+counter scratch
    // directory. Only the immutable configuration is shared. Fly both sides at
    // once so their full allowed duration fits the six-hour acceptance job;
    // retain deterministic red/blue ordering when collecting the evidence.
    let fly = |side: &'static str| {
        let config = config.clone();
        let directory = directory.clone();
        async move {
            let extra = arena_rig::extra("hold", &std::collections::BTreeSet::new());
            let (flown, documents) = arena_rig::fly_lane_with_config_evidence(
                &arena_rig::LanePlan {
                    name: side,
                    matches: 15,
                    deadline: Duration::from_secs(210 * 60),
                    extra: &extra,
                    env: &[],
                    poses: &[],
                    max_duration_ms: 600_000,
                    max_seconds: arena_rig::MUSTER_MAX_SECONDS,
                },
                side,
                config,
            )
            .await;
            // Preserve this lane as soon as it returns, before its peer finishes
            // or any completeness/identity assertion can reject the evidence.
            // The rig has already removed its disposable server directory.
            std::fs::write(
                directory.join(format!("{side}.jsonl")),
                &flown.partial.transcript_text,
            )
            .unwrap();
            if let Some(raw) = &flown.partial.report_text {
                std::fs::write(directory.join(format!("{side}.json")), raw).unwrap();
            }
            std::fs::write(
                directory.join(format!("{side}.stdout.log")),
                &flown.partial.stdout,
            )
            .unwrap();
            std::fs::write(
                directory.join(format!("{side}.stderr.log")),
                &flown.partial.stderr,
            )
            .unwrap();
            std::fs::write(
                directory.join(format!("{side}.flight.json")),
                serde_json::to_vec_pretty(&json!({
                    "side":side,"exit_code":flown.partial.code,"early":flown.early,
                    "heard":flown.heard,"ends":flown.ends,"refused":flown.refused,
                    "report_present":flown.partial.report_text.is_some(),
                    "report_decoded":flown.partial.report.is_some(),
                }))
                .unwrap(),
            )
            .unwrap();
            std::fs::write(
                directory.join(format!("{side}.documents.json")),
                serde_json::to_vec_pretty(&documents).unwrap(),
            )
            .unwrap();
            (flown, documents)
        }
    };
    let (red, blue) = tokio::join!(fly("red"), fly("blue"));
    for (side, (flown, documents)) in [("red", red), ("blue", blue)] {
        let records = documents
            .iter()
            .filter(|(_, value)| value["record"] == "tacsim.match")
            .collect::<Vec<_>>();
        assert_eq!(records.len(), 15, "one terminal record per actual flight");
        let mut actual_results = std::collections::BTreeMap::new();
        for (_, record) in records {
            let match_id = record["match"].as_u64().unwrap();
            assert!(
                actual_results
                    .insert(
                        match_id,
                        record["outcome"]
                            .as_str()
                            .expect("terminal outcome")
                            .to_owned(),
                    )
                    .is_none(),
                "distinct actual matches within this side"
            );
            let head = documents
                .iter()
                .find(|(key, _)| key == &format!("tacsim:tape:{match_id}:head"))
                .unwrap()
                .1
                .clone();
            let header: tacsim::arena::input_tape::TapeHeader =
                serde_json::from_value(head.clone()).unwrap();
            let ground_sha = header
                .ground_sha256()
                .unwrap()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            if let Some(previous) = common_ground.replace(ground_sha.clone()) {
                assert_eq!(previous, ground_sha);
            }
            let team = record["teams"]
                .as_array()
                .unwrap()
                .iter()
                .find(|team| team["team"] != side)
                .unwrap();
            let units = team["units"].as_array().unwrap();
            assert_eq!(units.len(), 1);
            let seal = head["seals"]
                .as_array()
                .unwrap()
                .iter()
                .find(|seal| seal["unit"] == units[0])
                .unwrap();
            assert_eq!(seal["requested"]["mind"], tacsim::match_::SEALED_MIND);
            let selected = seal["cards"]["selected"].as_array().unwrap();
            assert_eq!(selected.len(), 1, "the actual opponent has one profile");
            let profile = selected[0]["name"].as_str().unwrap();
            assert_eq!(profile, "Line", "the actual opponent carries Line");
            let index = selected[0]["key"]["index"].as_u64().unwrap() as usize;
            assert_eq!(
                seal["cards"]["definition"]["profiles"][index]["name"],
                profile
            );
            let opponent = json!({"class":seal["class"],"profile":profile,"source_sha256":seal["source_sha256"]});
            if let Some(previous) = common_opponent.replace(opponent.clone()) {
                assert_eq!(previous, opponent);
            }
            matches.push(json!({"side":side,"match":match_id,"seed":record["seed"],"ground_sha256":ground_sha,
                "arena_definition_digest":record["arena_definition_digest"],"outcome":record["outcome"],"reason":record["reason"],
                "arena_ms":record["arena_ms"],"ledger":record["ledger"],"opponent":opponent}));
        }
        assert!(
            flown.early.is_none(),
            "{side}: {:?}; {}",
            flown.early,
            flown.partial.stderr
        );
        assert!(flown.refused.is_empty());
        let report = flown
            .partial
            .report
            .as_ref()
            .expect("required side-aware bot report; no red-only lobby fallback");
        assert_eq!(
            report["arena"]["side"], side,
            "the bot actually flew this side"
        );
        let reported = report["arena"]["matches"]
            .as_array()
            .expect("the bot's individual match results");
        assert_eq!(reported.len(), 15);
        let mut reported_ids = std::collections::BTreeSet::new();
        for row in reported {
            let id = row["match"].as_u64().expect("reported actual match id");
            let result = row["result"]
                .as_str()
                .expect("reported actual match result");
            assert!(reported_ids.insert(id), "no repeated reported match");
            assert_eq!(
                actual_results.get(&id).map(String::as_str),
                Some(result),
                "the public bot result agrees with the completed server record"
            );
            assert_eq!(row["won"].as_bool(), Some(result == side));
        }
        for name in ["wins", "losses", "draws", "timeouts", "other"] {
            assert!(
                report["arena"][name].as_u64().is_some(),
                "explicit {name} count"
            );
        }
        let outcome = arena_rig::Outcome::from_report(report);
        assert_eq!(
            outcome.total(),
            15,
            "all outcomes count including draws and timeouts"
        );
        assert_eq!(outcome.other, 0, "unclassified endings must not disappear");
        counts[0] += outcome.won;
        counts[1] += outcome.lost;
        counts[2] += outcome.drew;
        counts[3] += outcome.timed_out;
        sides.push(json!({"side":side,"samples":15,"wins":outcome.won,"losses":outcome.lost,"draws":outcome.drew,"timeouts":outcome.timed_out}));
    }
    let artifact = json!({
        "format":1,"kind":"fresh_external_bot","revision":revision,"started_unix_ms":started,
        "run_id":std::env::var("M6_ACCEPTANCE_RUN_ID").expect("one fresh acceptance run id"),
        "finished_unix_ms":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis(),
        "scenario":"muster","scenario_content_sha256":sha(scenario),"arena_content_sha256":sha(ground),"atlas_content_sha256":sha(atlas),
        "ground":"tacsim.arena.proving_ground","chassis":"tacsim.unit.line_tank","opponent":common_opponent.expect("captured opponent identity"),
        "ground_sha256":common_ground.expect("captured full authored ground"),"config_sha256":config_sha,
        "native_substep_max_ms":config.arena.tempo.substep_max_ms,"matches":matches,
        "startup_ms":config.match_.startup_ms,"max_duration_ms":config.match_.max_duration_ms,"per_side":1,"samples":30,
        "seed_policy":"fresh live seed_for per match; independent from embedded proving pair seeds",
        "knobs":{"opening":"wait","engagement":"hold","band_m":450,"reapproach":false,"follow":true,"lattice":true},
        "wins":counts[0],"losses":counts[1],"draws":counts[2],"timeouts":counts[3],"sides":sides,
        "rounded_wilson_95":arena_rig::wilson_95(counts[0],30),
    });
    std::fs::write(
        directory.join("external-interval.json"),
        serde_json::to_vec_pretty(&artifact).unwrap(),
    )
    .unwrap();
    println!("FRESH_EXTERNAL_M6 {}", artifact);
}
