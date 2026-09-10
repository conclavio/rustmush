use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exported_studio_cast_reaches_real_players_with_only_their_private_profiles() {
    let fixture = Fixture::boot_world(false, None, true).await;
    let model = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|request: &Request| {
            let context = request_context(request);
            let body = context.to_string();
            assert!(context["terminal_history"].as_str().unwrap().contains("Market Square"), "the bot must observe its actual arrival before concluding readiness");
            assert_eq!(context["group_affiliations"]["groups"][0]["name"], "The Lantern Company");
            assert_eq!(context["group_affiliations"]["groups"][0]["roster"].as_array().unwrap().len(), 2);
            let (name, appearance, role, goal, briefing, excluded) = match context["group_affiliations"]["actor"].as_str().unwrap() {
                "scout" => ("Mira Ash", "MIRA_APPEARANCE_PRIVATE", "SCOUT_ROLE_PRIVATE", "SCOUT_GOAL_PRIVATE", "SCOUT_BRIEFING_PRIVATE", ["Tomas Vale", "TOMAS_APPEARANCE_PRIVATE", "MEDIC_ROLE_PRIVATE", "MEDIC_GOAL_PRIVATE", "MEDIC_BRIEFING_PRIVATE"]),
                "medic" => ("Tomas Vale", "TOMAS_APPEARANCE_PRIVATE", "MEDIC_ROLE_PRIVATE", "MEDIC_GOAL_PRIVATE", "MEDIC_BRIEFING_PRIVATE", ["Mira Ash", "MIRA_APPEARANCE_PRIVATE", "SCOUT_ROLE_PRIVATE", "SCOUT_GOAL_PRIVATE", "SCOUT_BRIEFING_PRIVATE"]),
                actor => panic!("unexpected exported actor {actor}"),
            };
            assert_eq!(context["character_profile"]["name"], name);
            assert!(context["character_profile"]["appearance"].as_str().unwrap().contains(appearance));
            assert!(context["runner_role"].as_str().unwrap().contains(role));
            assert!(context["runner_goal"].as_str().unwrap().contains(goal));
            assert!(body.contains(briefing));
            for secret in excluded { assert!(!body.contains(secret), "another member's private draft leaked into the player prompt: {secret}"); }
            reply(json!({"action":"done","summary":"Observed Market Square on arrival and assessed personal readiness to meet the company."}))
        })
        .expect(2)
        .mount(&model)
        .await;
    let config = fixture.config(Some(&model), 5);
    let mut session = mush_bot::studio::Session::new(serde_json::from_value(json!({
        "version":1,"kind":"group","concept":"Two independent travelers meet at Market Square to prepare for a journey.","count":1
    })).unwrap()).unwrap();
    let draft = json!({
        "title":"The Lantern Company",
        "synopsis":"Two travelers approach their rendezvous with distinct hopes and quiet concerns. They know the meeting place and the company mission, while retaining their own histories and private judgments about the journey ahead. Each intends to assess readiness after observing the square, without assuming the other has already completed any task. ".repeat(3),
        "characters":[
            {"actor":"scout","character":{"name":"Mira Ash","appearance":"MIRA_APPEARANCE_PRIVATE: a weathered green coat and carefully mended gloves.","background":"A patient trail surveyor.","personality":["careful"],"voice":{"style":"Brief, thoughtful observations."}},
             "role":"SCOUT_ROLE_PRIVATE: a cautious surveyor who verifies the surroundings.","goal":"After observing Market Square, assess your own readiness as SCOUT_GOAL_PRIVATE and finish."},
            {"actor":"medic","character":{"name":"Tomas Vale","appearance":"TOMAS_APPEARANCE_PRIVATE: a blue scarf and a tidy field satchel.","background":"A practical traveling healer.","personality":["warm"],"voice":{"style":"Patient, concrete reassurance."}},
             "role":"MEDIC_ROLE_PRIVATE: a healer who keeps personal concerns private.","goal":"After observing Market Square, assess your own readiness as MEDIC_GOAL_PRIVATE and finish."}
        ],
        "group":{"id":"lanterns","name":"The Lantern Company","kind":"traveling party","mission":"Meet in the square and prepare to travel together.",
            "shared_knowledge":["Our agreed rendezvous is Market Square."],"members":[
                {"actor":"scout","role":"pathfinder","briefing":["SCOUT_BRIEFING_PRIVATE: privately check your confidence before proceeding."]},
                {"actor":"medic","role":"healer","briefing":["MEDIC_BRIEFING_PRIVATE: quietly assess your own readiness."]}
            ]}
    });
    session
        .add_response(
            &json!({"candidates":[draft]}).to_string(),
            "Create this two-person company.",
            &"c".repeat(64),
            1,
        )
        .unwrap();
    session.select(1).unwrap();
    let bindings: mush_bot::studio::Bindings =
        serde_json::from_value(json!({"version":1,"actors":[
            {"actor":"scout","config":config},
            {"actor":"medic","config":config,"login":"marco+Marco"}
        ]}))
        .unwrap();
    let export = fixture.dir.join("authored-cast");
    let manifest = session.export(&export, Some(&bindings)).unwrap();
    let plan = mush_bot::harness::Manifest::load(&manifest)
        .unwrap()
        .plan()
        .unwrap();
    assert_eq!(plan["actor_sessions"], 2);
    let directory = fixture.dir.join("authored-results");
    let result = output(
        fixture
            .command(&config)
            .arg("--harness")
            .arg(&manifest)
            .arg("--output")
            .arg(&directory),
    )
    .await;
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let summary: Value =
        serde_json::from_slice(&std::fs::read(directory.join("summary.json")).unwrap()).unwrap();
    assert_eq!(summary["success"], true);
    for (actor, name, own, excluded) in [
        (
            "scout",
            "Mira Ash",
            "SCOUT_BRIEFING_PRIVATE",
            "MEDIC_BRIEFING_PRIVATE",
        ),
        (
            "medic",
            "Tomas Vale",
            "MEDIC_BRIEFING_PRIVATE",
            "SCOUT_BRIEFING_PRIVATE",
        ),
    ] {
        let report: Value = serde_json::from_slice(
            &std::fs::read(directory.join(format!("run-0001/{actor}/report.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(report["character"]["name"], name);
        assert_eq!(report["state"]["status"], "done");
        assert_eq!(report["state"]["model_calls"], 1);
        assert_eq!(
            report["state"]["commands_sent"], 2,
            "only help/look bootstrap commands were required for this readiness goal"
        );
        let affiliations =
            std::fs::read_to_string(directory.join(format!("run-0001/{actor}/affiliations.json")))
                .unwrap();
        assert!(affiliations.contains(own));
        assert!(!affiliations.contains(excluded));
    }
}
