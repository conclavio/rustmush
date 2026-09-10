//! Studio subprocesses use local response tapes or a local HTTP stub. Every
//! config has an unavailable SSH endpoint and an unset SSH login secret.
use std::{
    path::PathBuf,
    process::{Output, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};
use tokio::{io::AsyncWriteExt, process::Command};
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::method};

const SSH_SECRET: &str = "MUSH_BOT_STUDIO_TEST_MISSING_SSH_SECRET";
const MODEL_SECRET: &str = "MUSH_BOT_STUDIO_TEST_MISSING_MODEL_SECRET";

struct Fixture {
    directory: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let directory = std::env::temp_dir().join(format!(
            "mush-bot-studio-test-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir(&directory).unwrap();
        let fixture = Self { directory };
        // An explicit empty file prevents inherited filesystem dotenv lookup.
        fixture.write(".env", "");
        fixture.config(None);
        fixture.json(
            "concept.json",
            json!({
                "version":1,"kind":"character",
                "concept":"She walks dogs during the day while spending her nights studying."
            }),
        );
        fixture
    }

    fn write(&self, name: &str, text: &str) {
        std::fs::write(self.directory.join(name), text).unwrap();
    }

    fn json(&self, name: &str, value: Value) {
        self.write(name, &serde_json::to_string_pretty(&value).unwrap());
    }

    fn read(&self, name: &str) -> Value {
        serde_json::from_slice(&std::fs::read(self.directory.join(name)).unwrap()).unwrap()
    }

    fn config(&self, model: Option<&MockServer>) {
        let endpoint = model.map_or_else(
            || "http://127.0.0.1:1/v1/chat/completions".to_owned(),
            |model| format!("{}/v1/chat/completions", model.uri()),
        );
        let credentials = if model.is_none() {
            format!("api_key_env = '{MODEL_SECRET}'\n")
        } else {
            String::new()
        };
        self.write("bot.toml", &format!(
            "[ssh]\nhost = '127.0.0.1'\nport = 1\nusername = 'studio-test'\npassword_env = '{SSH_SECRET}'\ntimeout_secs = 1\n\
             [model]\nendpoint = '{endpoint}'\nname = 'studio-fixture'\n{credentials}timeout_secs = 10\nmax_tokens = 2048\ncost_currency = 'USD'\n"
        ));
    }

    fn tape(&self, name: &str, responses: Vec<Value>) {
        self.json(
            name,
            json!({"version":1,"replies":responses.into_iter()
            .map(|response| json!({"content":response.to_string()})).collect::<Vec<_>>()}),
        );
    }

    async fn run(&self, args: &[&str], input: &str) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_mush-bot"))
            .args(args)
            .current_dir(&self.directory)
            .env_remove(SSH_SECRET)
            .env_remove(MODEL_SECRET)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(input.as_bytes()).await.unwrap();
        drop(stdin);
        tokio::time::timeout(Duration::from_secs(35), child.wait_with_output())
            .await
            .expect("studio process deadline")
            .unwrap()
    }

    async fn success(&self, args: &[&str], input: &str) -> Output {
        let output = self.run(args, input).await;
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let (Ok(directory), Ok(temporary)) = (
            self.directory.canonicalize(),
            std::env::temp_dir().canonicalize(),
        ) && directory.parent() == Some(temporary.as_path())
            && directory
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("mush-bot-studio-test-"))
        {
            let _ = std::fs::remove_dir_all(directory);
        }
    }
}

fn synopsis() -> &'static str {
    "Mara spends her mornings walking the dogs of commuters who trust her with keys but rarely remember her exam dates. She studies veterinary nursing at night, keeping annotated lecture notes beside a drying rack of muddy leads. Her patience with frightened animals is real, though she can be sharp with people who mistake quiet work for an easy life. She grew up above her aunt's repair shop and still repairs her own boots rather than buying fashionable replacements. She likes local football, old detective novels, and the comfort of a carefully planned lunch. A failed practical exam has shaken her confidence more than she admits. She wants reliable work and an independent future, but taking every offered shift leaves little time for either friendship or study. She speaks plainly, asks specific questions, and remembers useful details. When someone earns her trust she becomes unexpectedly funny. Her first task is to find a quiet public place where she can review her notes between appointments."
}

fn character(actor: &str, name: &str) -> Value {
    json!({
        "actor":actor,
        "character":{
            "version":1,"name":name,"pronouns":"she/her",
            "appearance":"Tall, with weathered walking boots, cropped dark hair, and a patched raincoat.",
            "identity":"A dog walker studying veterinary nursing.",
            "hometown":"Greyhaven","background":"She grew up above her aunt's repair shop.",
            "personality":["Patient with animals","Dryly funny after trust develops"],
            "motivations":["Finish her practical training without losing her clients"],
            "voice":{"style":"Plain and observant; ask a specific follow-up."},
            "prior_knowledge":["AUTHORED_PRIOR: She studies veterinary nursing." ]
        },
        "role":"A working student with a practical outlook.",
        "goal":"Find a quiet public place to review study notes."
    })
}

fn draft(number: usize) -> Value {
    json!({"title":format!("Mara alternative {number}"),"synopsis":synopsis(),
        "characters":[character("dogwalker", &format!("Mara {number}"))],"group":null})
}

fn response(numbers: std::ops::RangeInclusive<usize>) -> Value {
    json!({"candidates":numbers.map(draft).collect::<Vec<_>>()})
}

fn capture(fixture: &Fixture, name: &str) -> mush_bot::model::Capture {
    mush_bot::model::Capture::load(&fixture.directory.join(name)).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ten_alternatives_refine_with_stable_numbers_and_resume_without_ssh_or_auto_generation() {
    let fixture = Fixture::new();
    fixture.tape("first-tape.json", vec![response(1..=10), response(11..=11)]);
    let refinement = "Blend the background from 8 with the speaking mannerisms from 2, retaining veterinary studies.";
    let output = fixture.success(&[
        "--config","bot.toml","--studio","concept.json","--output","first",
        "--model-tape","first-tape.json",
    ], &format!("8\n{refinement}\n/show 11\n/select 11\n/save saved-session.json\n/export exported\n/quit\n")).await;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[studio/selected] 8:"), "{stdout}");
    assert!(stdout.contains("[studio/candidate] 11:"), "{stdout}");
    let session = fixture.read("first/session.json");
    assert_eq!(session["concept"]["count"], 10);
    assert_eq!(session["selected"], 11);
    assert_eq!(session["candidates"].as_array().unwrap().len(), 11);
    assert_eq!(
        session["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|candidate| candidate["number"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        (1..=11).collect::<Vec<_>>()
    );
    assert_eq!(session["candidates"][10]["instruction"], refinement);
    assert_eq!(session["candidates"][10]["based_on"], 8);
    assert_eq!(fixture.read("saved-session.json"), session);
    assert_eq!(
        fixture.read("exported/selected.json")["candidate"],
        session["candidates"][10]
    );
    let exported = mush_bot::character::Character::load(
        &fixture
            .directory
            .join("exported/characters/character-dogwalker.json"),
    )
    .unwrap();
    assert_eq!(exported.name, "Mara 11");
    assert!(exported.appearance.contains("patched raincoat"));
    assert_eq!(
        exported.prior_knowledge,
        ["AUTHORED_PRIOR: She studies veterinary nursing."]
    );
    assert!(!fixture.directory.join("exported/harness.json").exists());

    for request in ["first/request-0001.json", "first/request-0002.json"] {
        let request = capture(&fixture, request);
        request.validate().unwrap();
        assert_eq!(
            request.payload["max_tokens"], 16384,
            "studio overrides the 2048-token gameplay allowance"
        );
    }
    let refined = capture(&fixture, "first/request-0002.json");
    assert!(refined.payload.to_string().contains(refinement));
    assert!(refined.payload.to_string().contains("Mara 8"));
    assert!(refined.payload.to_string().contains("Mara 2"));
    assert_eq!(
        session["candidates"][10]["request_revision"],
        refined.request_revision
    );
    assert!(fixture.directory.join("first/session-0000.json").is_file());
    assert!(fixture.directory.join("first/result-0002.json").is_file());
    let report = fixture.read("first/report.json");
    assert_eq!(report["costs"]["session"]["totals"]["requests"], 2);
    assert_eq!(
        report["costs"]["session"]["by_prompt_type"]["character_authoring"]["requests"],
        2
    );

    // Headless resume is a read/save operation and needs no provider secret or tape.
    fixture.tape("resume-tape.json", vec![response(12..=12)]);
    fixture
        .success(
            &[
                "--config",
                "bot.toml",
                "--studio-session",
                "saved-session.json",
                "--output",
                "resumed-headless",
                "--headless",
            ],
            "",
        )
        .await;
    assert_eq!(fixture.read("resumed-headless/session.json"), session);
    assert_eq!(
        fixture.read("resumed-headless/report.json")["costs"]["session"]["totals"]["requests"],
        0
    );
    assert!(
        !fixture
            .directory
            .join("resumed-headless/request-0001.json")
            .exists()
    );
    fixture.success(&[
        "--config","bot.toml","--studio-session","saved-session.json","--output","resumed-edit",
        "--model-tape","resume-tape.json",
    ], "/refine Keep option 11 but give her an interest in local football.\n/select 12\n/quit\n").await;
    let resumed = fixture.read("resumed-edit/session.json");
    assert_eq!(resumed["selected"], 12);
    assert_eq!(resumed["candidates"][11]["number"], 12);
    assert_eq!(resumed["candidates"][11]["based_on"], 11);
    assert_eq!(
        &resumed["candidates"].as_array().unwrap()[..11],
        session["candidates"].as_array().unwrap().as_slice()
    );
    assert!(!fixture.directory.join(".mush-bot/memory").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejected_drafts_keep_prior_choices_and_provider_charges_when_generation_is_retried() {
    let fixture = Fixture::new();
    let model = MockServer::start().await;
    fixture.config(Some(&model));
    fixture.json("concept.json", json!({"version":1,"kind":"character","concept":"A dog walker who studies at night.","count":2}));
    let next = Arc::new(AtomicU32::new(0));
    Mock::given(method("POST")).respond_with(move |request: &Request| {
        let body: Value = request.body_json().unwrap();
        assert_eq!(body["max_tokens"], 16384);
        let (content, cost) = match next.fetch_add(1, Ordering::SeqCst) {
            0 => (response(1..=2), "0.1"),
            1 => {
                assert!(body.to_string().contains("Keep candidate two"));
                let mut invalid = draft(999);
                invalid["synopsis"] = "Too short to be a valid synopsis.".into();
                (json!({"candidates":[invalid]}), "0.2")
            }
            2 => (response(3..=4), "0.3"),
            number => panic!("unexpected authoring request {number}"),
        };
        ResponseTemplate::new(200).set_body_json(json!({
            "model":"studio-fixture","choices":[{"message":{"content":content.to_string()}}],
            "usage":{"cost":serde_json::from_str::<Value>(cost).unwrap(),"prompt_tokens":30,"completion_tokens":40}
        }))
    }).expect(3).mount(&model).await;
    let output = fixture.run(&[
        "--config","bot.toml","--studio","concept.json","--output","retry",
    ], "/select 2\n/refine Keep candidate two but refine her study habits.\n/list\n/generate\n/quit\n").await;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("[studio/error]"),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let session = fixture.read("retry/session.json");
    assert_eq!(session["candidates"].as_array().unwrap().len(), 4);
    assert_eq!(
        session["candidates"][0]["draft"]["title"],
        "Mara alternative 1"
    );
    assert_eq!(
        session["candidates"][1]["draft"]["title"],
        "Mara alternative 2"
    );
    assert_eq!(session["candidates"][2]["number"], 3);
    assert_eq!(session["candidates"][3]["number"], 4);
    assert!(!session.to_string().contains("Mara alternative 999"));
    let report = fixture.read("retry/report.json");
    assert_eq!(report["costs"]["session"]["totals"]["requests"], 3);
    assert_eq!(
        report["costs"]["session"]["totals"]["provider_cost_by_currency"]["USD"],
        "0.6"
    );
    assert_eq!(
        report["costs"]["session"]["totals"]["unknown_cost_requests"],
        0
    );
    assert_eq!(
        report["costs"]["session"]["by_prompt_type"]["character_authoring"]["requests"],
        3
    );
    let failed = fixture.read("retry/result-0002.json");
    assert_eq!(failed["success"], false, "{failed}");
    assert!(
        failed
            .to_string()
            .contains("Too short to be a valid synopsis.")
    );
    assert_eq!(fixture.read("retry/result-0003.json")["success"], true);
    for number in 1..=3 {
        capture(&fixture, &format!("retry/request-{number:04}.json"))
            .validate()
            .unwrap();
    }
    assert_eq!(model.received_requests().await.unwrap().len(), 3);
    assert!(!fixture.directory.join(".mush-bot/memory").exists());
}

fn band_draft() -> Value {
    let roles = [
        ("singer", "lead singer and rhythm guitarist"),
        ("guitarist", "lead guitarist"),
        ("bassist", "bassist"),
        ("drummer", "drummer"),
    ];
    json!({"title":"The Trial Period","synopsis":
        "The band has spent years mistaking familiarity for musical agreement. Its singer and drummer have played together since school, and they still settle arguments through jokes nobody else understands. The guitarist joined last year with fast hands and a taste for solos that spill beyond the agreed arrangement. The singer tries to keep the songs recognizable while quietly enjoying some of that recklessness. Their new bassist hears possibilities the others have barely considered: countermelodies, harmonic movement, and space where a root note usually sits. They call this her trial period, unaware that she is deciding whether their loyalty includes room for growth. Onstage the drummer attacks every chorus as if it has personally offended him; afterward he is the calmest person loading the van. All four are in their mid-twenties. They want a reliable local gig, but a productive rehearsal will require them to decide whether listening matters as much as being heard.",
        "characters":roles.iter().map(|(actor,role)| {
            let mut person = character(actor, &format!("Band {actor}"));
            person["character"]["identity"] = format!("The band's {role}.").into();
            person["role"] = (*role).into();
            person["goal"] = "Meet the band and coordinate a rehearsal using ordinary game speech.".into();
            person
        }).collect::<Vec<_>>(),
        "group":{"id":"trial_period","name":"The Trial Period","kind":"four-piece band",
            "mission":"Arrange a rehearsal where everyone gets heard.","shared_knowledge":["The band is seeking a local gig."],
            "members":roles.iter().map(|(actor,role)| json!({"actor":actor,"role":role,
                "briefing":[format!("PRIVATE_{actor}_BRIEFING")]})).collect::<Vec<_>>()}
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn headless_group_draft_exports_a_runnable_plan_and_member_scoped_briefings() {
    let fixture = Fixture::new();
    fixture.json("concept.json", json!({"version":1,"kind":"group","concept":"A four-piece band with competing ambitions.","count":1}));
    fixture.tape("band-tape.json", vec![json!({"candidates":[band_draft()]})]);
    fixture
        .success(
            &[
                "--config",
                "bot.toml",
                "--studio",
                "concept.json",
                "--output",
                "band",
                "--model-tape",
                "band-tape.json",
                "--headless",
            ],
            "",
        )
        .await;
    let session = fixture.read("band/session.json");
    assert_eq!(session["candidates"].as_array().unwrap().len(), 1);
    assert_eq!(
        fixture.read("band/report.json")["costs"]["session"]["by_prompt_type"]["group_authoring"]["requests"],
        1
    );
    fixture.json("bindings.json", json!({"version":1,"actors":(["singer","guitarist","bassist","drummer"]).iter()
        .map(|actor| json!({"actor":actor,"config":"bot.toml","login":format!("band+{actor}")})).collect::<Vec<_>>()}));
    fixture
        .success(
            &[
                "--config",
                "bot.toml",
                "--studio-session",
                "band/session.json",
                "--output",
                "band-export",
            ],
            "/select 1\n/export exported-band bindings.json\n/quit\n",
        )
        .await;
    assert_eq!(
        fixture.read("band-export/report.json")["costs"]["session"]["totals"]["requests"],
        0
    );
    let harness = fixture.read("exported-band/harness.json");
    assert_eq!(harness["actors"].as_array().unwrap().len(), 4);
    assert!(harness.to_string().contains("PRIVATE_bassist_BRIEFING"));
    for actor in ["singer", "guitarist", "bassist", "drummer"] {
        let profile = mush_bot::character::Character::load(
            &fixture
                .directory
                .join(format!("exported-band/characters/character-{actor}.json")),
        )
        .unwrap();
        assert_eq!(profile.name, format!("Band {actor}"));
        let affiliations = mush_bot::organization::Affiliations::load(&fixture.directory.join(
            format!("exported-band/affiliations/affiliations-{actor}.json"),
        ))
        .unwrap();
        assert_eq!(affiliations.actor, actor);
        assert_eq!(
            affiliations.groups[0].briefing,
            [format!("PRIVATE_{actor}_BRIEFING")]
        );
        let serialized = serde_json::to_string(&affiliations).unwrap();
        for other in ["singer", "guitarist", "bassist", "drummer"] {
            if other != actor {
                assert!(!serialized.contains(&format!("PRIVATE_{other}_BRIEFING")));
            }
        }
        let configured = harness["actors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == actor)
            .unwrap();
        assert_eq!(configured["login"], format!("band+{actor}"));
        assert!(!configured["role"].as_str().unwrap().is_empty());
        assert!(!configured["goal"].as_str().unwrap().is_empty());
    }
    let plan = fixture
        .success(
            &[
                "--harness",
                "exported-band/harness.json",
                "--plan-harness",
                "--report",
                "band-plan.json",
            ],
            "",
        )
        .await;
    assert!(plan.status.success());
    let plan = fixture.read("band-plan.json");
    assert!(
        !plan.to_string().contains("PRIVATE_"),
        "offline plans omit member briefings"
    );
    assert!(!fixture.directory.join(".mush-bot/memory").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn studio_budget_zero_and_existing_output_prevent_generation() {
    let fixture = Fixture::new();
    fixture.tape("tape.json", vec![response(1..=10)]);
    fixture.json("budget.json", json!({"version":1,"max_requests":0}));
    let blocked = fixture
        .run(
            &[
                "--config",
                "bot.toml",
                "--studio",
                "concept.json",
                "--output",
                "blocked",
                "--model-tape",
                "tape.json",
                "--budget",
                "budget.json",
                "--headless",
            ],
            "",
        )
        .await;
    assert_eq!(
        blocked.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&blocked.stderr)
    );
    assert!(
        fixture.read("blocked/session.json")["candidates"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(fixture.read("blocked/report.json")["success"], false);
    assert_eq!(
        fixture.read("blocked/report.json")["costs"]["session"]["totals"]["requests"],
        0
    );
    assert!(!fixture.directory.join("blocked/request-0001.json").exists());
    // Empty saved sessions are also resumes, not a fresh generation trigger.
    fixture
        .success(
            &[
                "--config",
                "bot.toml",
                "--studio-session",
                "blocked/session.json",
                "--output",
                "empty-resume",
                "--headless",
            ],
            "",
        )
        .await;
    assert!(
        fixture.read("empty-resume/session.json")["candidates"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        fixture.read("empty-resume/report.json")["costs"]["session"]["totals"]["requests"],
        0
    );
    assert!(
        !fixture
            .directory
            .join("empty-resume/request-0001.json")
            .exists()
    );
    let original = std::fs::read(fixture.directory.join("blocked/session.json")).unwrap();
    let overwrite = fixture
        .run(
            &[
                "--config",
                "bot.toml",
                "--studio",
                "concept.json",
                "--output",
                "blocked",
                "--model-tape",
                "tape.json",
                "--headless",
            ],
            "",
        )
        .await;
    assert_eq!(overwrite.status.code(), Some(1));
    assert_eq!(
        std::fs::read(fixture.directory.join("blocked/session.json")).unwrap(),
        original
    );
    assert!(!fixture.directory.join("blocked/request-0001.json").exists());
    let conflicts = fixture
        .run(
            &[
                "--config",
                "bot.toml",
                "--studio",
                "concept.json",
                "--studio-session",
                "blocked/session.json",
                "--output",
                "conflicting",
                "--model-tape",
                "tape.json",
                "--headless",
            ],
            "",
        )
        .await;
    assert_eq!(conflicts.status.code(), Some(1));
    assert!(!fixture.directory.join("conflicting").exists());
}
