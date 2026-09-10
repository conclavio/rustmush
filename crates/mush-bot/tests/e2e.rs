//! Actual mush-bot child processes against production SSH/world wiring.
//! No live model or Redis. The bot is a player or a scoped builder, never God.
//! Game-crate imports below belong only to the server-under-test fixture. The
//! bot's decisions go through its own HTTP client to a separate wiremock server;
//! mush-llm supplies fake responses only for the game server's @gen command.
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use mush_core::{Engine, Flags, GOD, LIMBO, ObjKind, Powers, World};
use mush_llm::{
    CompletionRequest, CompletionResponse, ContentBlock, LlmError, LlmProvider, LlmService,
    LlmStream, MockProvider, ProviderCaps, ProviderId, RouteConfig, StopReason, Usage,
};
use mush_session::SessionManager;
use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore};
use mushd::{
    accounts::{Accounts, hash_password},
    ipguard::IpGuard,
    ssh_bridge::{SshBridge, start_listener},
    world_task,
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::Notify,
};
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{header, method, path},
};

const PASSWORD: &str = "bot-e2e-secret-42";

#[path = "support/budgets.rs"]
mod budgets;
#[path = "support/cast_debug.rs"]
mod cast_debug;
#[path = "support/lab.rs"]
mod lab;
#[path = "support/probes.rs"]
mod probes;
#[path = "support/studio_cast.rs"]
mod studio_cast;

struct Fixture {
    dir: PathBuf,
    addr: SocketAddr,
    pin: String,
}

impl Fixture {
    async fn boot() -> Self {
        Self::boot_with_builder(false, None).await
    }

    async fn boot_with_builder(builder: bool, llm: Option<Arc<LlmService>>) -> Self {
        Self::boot_world(builder, llm, false).await
    }

    async fn boot_world(builder: bool, llm: Option<Arc<LlmService>>, speaker: bool) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "mush-bot-e2e-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut engine = if let Some(llm) = &llm {
            Engine::with_modules(
                World::new_minimal(),
                world_task::base_registry(),
                vec![mushd::modules::worldgen_module(llm.clone())],
            )
            .unwrap()
        } else {
            world_task::boot_engine()
        };
        engine.world.set_now(mushd::unix_now_s());
        engine.world.set_name(LIMBO, "Market Square").unwrap();
        let shop = engine
            .world
            .create(ObjKind::Room, "Flower Shop", GOD, None)
            .unwrap();
        let exit = engine
            .world
            .create(ObjKind::Exit, "east", GOD, Some(LIMBO))
            .unwrap();
        engine.world.link_exit(exit, Some(shop)).unwrap();
        let exit = engine
            .world
            .create(ObjKind::Exit, "west", GOD, Some(shop))
            .unwrap();
        engine.world.link_exit(exit, Some(LIMBO)).unwrap();
        engine
            .world
            .create(ObjKind::Thing, "rose", GOD, Some(shop))
            .unwrap();
        let player = engine
            .world
            .create(ObjKind::Player, "Rosebot", GOD, Some(LIMBO))
            .unwrap();
        if builder {
            engine.world.set_flag(player, Flags::BUILDER, true).unwrap();
            engine
                .world
                .get_mut(player)
                .unwrap()
                .powers
                .insert(Powers::BUILDER_LLM);
            engine.world.chown(LIMBO, player).unwrap();
        }
        let sessions = Arc::new(SessionManager::new());
        let speaker = speaker.then(|| {
            engine
                .world
                .create(ObjKind::Player, "Marco", GOD, Some(LIMBO))
                .unwrap()
        });
        let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
        mushd::modules::boot_modules(&mut engine, &*store)
            .await
            .unwrap();
        let world = world_task::spawn_persist(
            engine,
            sessions.clone(),
            world_task::ServerDeps {
                store: store.clone(),
                llm,
                ..world_task::ServerDeps::in_memory()
            },
        )
        .handle;
        store
            .create_account(AccountRec {
                id: None,
                name: "rosebot".into(),
                argon2_hash: hash_password(PASSWORD).unwrap(),
                created: 0,
                characters: vec![CharacterRef {
                    id: player,
                    name: "Rosebot".into(),
                }],
                ssh_keys: vec![],
                discord: None,
            })
            .await
            .unwrap();
        if let Some(id) = speaker {
            store
                .create_account(AccountRec {
                    id: None,
                    name: "marco".into(),
                    argon2_hash: hash_password(PASSWORD).unwrap(),
                    created: 0,
                    characters: vec![CharacterRef {
                        id,
                        name: "Marco".into(),
                    }],
                    ssh_keys: vec![],
                    discord: None,
                })
                .await
                .unwrap();
        }
        let accounts = Arc::new(Accounts::new(store, world.clone(), 3));
        let bridge = Arc::new(SshBridge::new(
            accounts,
            sessions,
            world,
            Arc::new(IpGuard::new()),
        ));
        let listener = start_listener(
            "127.0.0.1:0".parse().unwrap(),
            &dir,
            "Bot fixture".into(),
            bridge,
        )
        .await
        .unwrap();
        let key = russh::keys::load_secret_key(dir.join("ssh_host_ed25519"), None).unwrap();
        let pin = key
            .public_key()
            .fingerprint(russh::keys::HashAlg::Sha256)
            .to_string();
        Self {
            dir,
            addr: listener.addr(),
            pin,
        }
    }

    fn config(&self, model: Option<&MockServer>, limit: usize) -> PathBuf {
        let mut config = format!(
            r#"
[ssh]
host = "127.0.0.1"
port = {}
username = "rosebot+Rosebot"
password_env = "MUSH_BOT_E2E_PASSWORD"
host_key_sha256 = "{}"
[agent]
goal = "Find the flower shop and acquire a rose, then verify your inventory"
max_steps = {limit}
max_seconds = 30
settle_ms = 100
observation_ms = 1000
bootstrap = ["help", "look"]
"#,
            self.addr.port(),
            self.pin
        );
        if let Some(model) = model {
            config.push_str(&format!(
                r#"
[model]
endpoint = "{}/v1/chat/completions"
name = "fixture-model"
timeout_secs = 10
"#,
                model.uri()
            ));
        }
        let path = self.dir.join("bot.toml");
        std::fs::write(&path, config).unwrap();
        path
    }

    fn command(&self, config: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_mush-bot"));
        command
            .current_dir(&self.dir)
            .arg("--config")
            .arg(config)
            .env("MUSH_BOT_E2E_PASSWORD", PASSWORD)
            .stdin(Stdio::null())
            .kill_on_drop(true);
        command
    }

    fn scenario(&self, steps: Value) -> PathBuf {
        let path = self.dir.join("scenario.json");
        std::fs::write(
            &path,
            json!({"name":"flower shop", "steps":steps}).to_string(),
        )
        .unwrap();
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn decision(action: &str, command: Option<&str>, summary: &str) -> Value {
    json!({"action":action,"command":command,"summary":summary,
        "memory":"Market Square has an east exit to Flower Shop; west returns.","wait_ms":0})
}

fn reply(decision: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({"choices":[{"message":{"content":decision.to_string()},"finish_reason":"stop"}],"usage":{"total_tokens":42}}))
}

async fn output(command: &mut Command) -> std::process::Output {
    tokio::time::timeout(Duration::from_secs(45), command.output())
        .await
        .expect("bot deadline")
        .unwrap()
}

async fn read_until(
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    marker: &str,
) -> String {
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut seen = String::new();
        while let Some(line) = lines.next_line().await.unwrap() {
            seen.push_str(&line);
            seen.push('\n');
            if line.contains(marker) {
                return seen;
            }
        }
        panic!("bot closed before {marker}: {seen}");
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {marker}"))
}

struct PlayerConsole {
    child: tokio::process::Child,
    input: tokio::process::ChildStdin,
    lines: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
}

impl PlayerConsole {
    fn spawn(command: &mut Command) -> Self {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        Self {
            input: child.stdin.take().unwrap(),
            lines: BufReader::new(child.stdout.take().unwrap()).lines(),
            child,
        }
    }

    async fn until(&mut self, marker: &str) -> String {
        read_until(&mut self.lines, marker).await
    }

    async fn send(&mut self, line: &str) {
        self.input
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
    }

    async fn speaker(fixture: &Fixture) -> Self {
        let base = fixture.config(None, 10);
        let path = fixture.dir.join("marco.toml");
        std::fs::write(
            &path,
            std::fs::read_to_string(base)
                .unwrap()
                .replace("rosebot+Rosebot", "marco+Marco"),
        )
        .unwrap();
        let mut actor = Self::spawn(fixture.command(&path).arg("--paused"));
        actor.until("[bot/send] look").await;
        actor.until("Exits    east").await;
        // Let bootstrap's ordinary output collection finish before passthrough.
        tokio::time::sleep(Duration::from_millis(200)).await;
        actor
    }

    async fn say(&mut self, text: &str) {
        // Public controls provide an explicit barrier; a fast fake model can
        // answer before this speaker's previous command has finished settling.
        self.send("/step observation").await;
        if self
            .until("[bot/observation-step]")
            .await
            .contains("Waiting for current")
        {
            self.until("Current output settled").await;
        }
        self.send(&format!("say {text}")).await;
        self.until(&format!("Marco says, \"{text}\"")).await;
    }

    async fn quit(mut self) {
        self.send("/quit").await;
        assert!(
            tokio::time::timeout(Duration::from_secs(5), self.child.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
    }
}

fn conversation_config(fixture: &Fixture, model: &MockServer) -> PathBuf {
    let config = fixture.config(Some(model), 15);
    // Relative profile paths are resolved against the config, not the executable.
    let text = format!(
        "character = \"rose.json\"\n{}\n[attention]\ncooldown_ms = 100\n",
        std::fs::read_to_string(&config).unwrap()
    );
    std::fs::write(&config, text).unwrap();
    std::fs::write(fixture.dir.join("rose.json"), json!({
        "name":"Rosebot", "aliases":["Rosie"], "hometown":"Greyhaven",
        "voice":{"style":"Warm, plain speech; short answers with a relevant follow-up."},
        "opinions":[{"topic":"sports", "stance":"Harbor Rooks supporter", "reason":"A family tradition."}]
    }).to_string()).unwrap();
    config
}

fn request_context(request: &Request) -> Value {
    let body: Value = request.body_json().unwrap();
    serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn addressed_speech_refreshes_unsent_action_with_profile_and_last_completed_memory() {
    const NOTES: &str = "I promised to return a borrowed map after visiting the flower shop.";
    let fixture = Fixture::boot_world(false, None, true).await;
    let mut actor = PlayerConsole::speaker(&fixture).await;
    let model = MockServer::start().await;
    let pending = Arc::new(Notify::new());
    let request_pending = pending.clone();
    let calls = Arc::new(AtomicU32::new(0));
    let next = calls.clone();
    Mock::given(method("POST")).respond_with(move |request: &Request| {
        let context = request_context(request);
        assert_eq!(context["runner_goal"], "Visit Flower Shop.");
        assert_eq!(context["decision_purpose"], "goal");
        assert_eq!(context["character_profile"]["hometown"], "Greyhaven");
        let turn = next.fetch_add(1, Ordering::SeqCst);
        if turn > 0 { assert_eq!(context["memory"], NOTES); }
        match turn {
            0 => reply(json!({"action":"wait", "summary":"Remember my promise.", "memory":NOTES, "wait_ms":100})),
            1 => {
                request_pending.notify_one();
                reply(json!({"action":"command", "command":"goto east", "summary":"Stale departure."}))
                    .set_delay(Duration::from_secs(5))
            }
            2 => {
                let speech = context["recent_speech"]["utterances"].as_array().unwrap();
                assert!(speech.iter().any(|s| s["speaker_label"] == "Marco"
                    && s["text"] == "Rosie, where did you grow up?"
                    && s["cues"].as_array().unwrap().contains(&json!("name_or_alias"))));
                assert!(context["remembered_experience"].to_string().contains("Rosie, where did you grow up?"));
                reply(json!({"action":"command", "command":"say Greyhaven, down by the docks. How about you?", "summary":"Answer the greeting before leaving."}))
            }
            3 => {
                assert!(context["terminal_history"].as_str().unwrap().contains("Rosebot says, \"Greyhaven"));
                assert_eq!(context["recent_speech"]["pending_attention"], false);
                reply(json!({"action":"command", "command":"goto east", "summary":"Continue to the flower shop."}))
            }
            4 => {
                assert!(context["terminal_history"].as_str().unwrap().contains("Flower Shop #"));
                reply(json!({"action":"done", "summary":"Reached the flower shop."}))
            }
            n => panic!("unexpected attention turn {n}"),
        }
    }).expect(5).mount(&model).await;
    let config = conversation_config(&fixture, &model);
    let transcript = fixture.dir.join("attention.jsonl");
    let mut bot = PlayerConsole::spawn(
        fixture
            .command(&config)
            .arg("--goal")
            .arg("Visit Flower Shop.")
            .arg("--transcript")
            .arg(&transcript),
    );
    tokio::time::timeout(Duration::from_secs(10), pending.notified())
        .await
        .unwrap();
    actor.say("Rosie, where did you grow up?").await;
    tokio::time::timeout(
        Duration::from_secs(3),
        bot.until("[bot/send] say Greyhaven"),
    )
    .await
    .expect("addressed speech must refresh the decision before the stale request returns");
    bot.until("[bot/done]").await;
    bot.quit().await;
    actor.quit().await;
    let events: Vec<Value> = std::fs::read_to_string(transcript)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    let commands: Vec<&str> = events
        .iter()
        .filter(|e| e["kind"] == "send")
        .map(|e| e["data"].as_str().unwrap())
        .collect();
    assert_eq!(
        commands,
        [
            "help",
            "look",
            "say Greyhaven, down by the docks. How about you?",
            "goto east"
        ]
    );
    assert!(
        !events
            .iter()
            .any(|e| e["kind"] == "summary" && e["data"] == "Stale departure.")
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        5,
        "cancelled calls count against the budget"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_bot_converses_without_restarting_goal_and_explicit_pause_blocks_attention() {
    let fixture = Fixture::boot_world(false, None, true).await;
    let mut actor = PlayerConsole::speaker(&fixture).await;
    let model = MockServer::start().await;
    let calls = Arc::new(AtomicU32::new(0));
    let next = calls.clone();
    Mock::given(method("POST")).respond_with(move |request: &Request| {
        let context = request_context(request);
        assert_eq!(context["runner_goal"], "Identify this room.");
        let turn = next.fetch_add(1, Ordering::SeqCst);
        if turn > 0 { assert_eq!(context["decision_purpose"], "conversation"); }
        match turn {
            0 => reply(json!({"action":"done", "summary":"This is Market Square.", "memory":"I am in Market Square."})),
            1 => {
                assert_eq!(context["memory"], "I am in Market Square.");
                reply(json!({"action":"recall", "query":"Greyhaven", "summary":"Recall my background before answering."}))
            }
            2 => {
                assert_eq!(context["remembered_experience"]["query"], "Greyhaven");
                assert!(context["remembered_experience"].to_string().contains("my home town is Greyhaven"));
                assert_eq!(context["recent_speech"]["pending_attention"], true);
                reply(json!({"action":"command", "command":"look", "summary":"Check the room before replying."}))
            }
            3 => {
                assert_eq!(context["recent_speech"]["pending_attention"], true,
                    "inspection must not consume the conversation before the reply");
                reply(json!({"action":"command", "command":"say Greyhaven. Where are you from?", "summary":"Share my hometown."}))
            }
            4 => {
                let speech = context["recent_speech"]["utterances"].as_array().unwrap();
                assert!(speech.iter().any(|s| s["text"] == "Which team do you follow?"
                    && s["cues"].as_array().unwrap().contains(&json!("conversation_follow_up"))));
                assert_eq!(context["character_profile"]["opinions"][0]["stance"], "Harbor Rooks supporter");
                // Mentioning our own name must not trigger ourselves again.
                reply(json!({"action":"command", "command":"say The Harbor Rooks. Rosie stays loyal. And you?", "summary":"Share my sporting allegiance."}))
            }
            5 => {
                assert!(context["recent_speech"]["utterances"].as_array().unwrap().iter()
                    .any(|s| s["text"] == "Rosie, are you still there?"));
                reply(json!({"action":"done", "summary":"Remain quiet for now."}))
            }
            n => panic!("unexpected idle conversation turn {n}"),
        }
    }).expect(6).mount(&model).await;
    let config = conversation_config(&fixture, &model);
    let transcript = fixture.dir.join("conversation.jsonl");
    let mut bot = PlayerConsole::spawn(
        fixture
            .command(&config)
            .arg("--goal")
            .arg("Identify this room.")
            .arg("--transcript")
            .arg(&transcript),
    );
    bot.until("[bot/done] This is Market Square.").await;
    actor.say("Rosie, where did you grow up?").await;
    bot.until("Rosebot says, \"Greyhaven. Where are you from?\"")
        .await;
    actor.say("Which team do you follow?").await;
    bot.until("Rosebot says, \"The Harbor Rooks. Rosie stays loyal. And you?\"")
        .await;
    bot.send("/state").await;
    bot.until("\"status\": \"done\"").await;
    bot.send("/pause").await;
    bot.until("[bot/paused]").await;
    actor.say("Rosie, are you still there?").await;
    bot.until("Marco says, \"Rosie, are you still there?\"")
        .await;
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        5,
        "explicit pause suppresses attention calls"
    );
    bot.send("/state").await;
    bot.until("\"pending_attention\": true").await;
    bot.send("/listen").await;
    bot.until("[bot/conversation] Remain quiet for now.").await;
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        6,
        "handled mentions and our own speech cannot spin the loop"
    );
    bot.quit().await;
    actor.quit().await;
    let events: Vec<Value> = std::fs::read_to_string(transcript)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    let state: Value = serde_json::from_str(
        events.iter().find(|e| e["kind"] == "state").unwrap()["data"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(state["state"]["outcome"], "This is Market Square.");
    assert_eq!(state["state"]["goal"], "Identify this room.");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn durable_memory_answers_old_conversations_after_restart_and_accepts_prior_knowledge() {
    let fixture = Fixture::boot_with_builder(true, None).await;
    let config = fixture.config(None, 20);
    // Fixture dialogue is delivered through the same public terminal as ordinary
    // speech; the journal also retains that these were @emit commands in a test.
    let scenario = fixture.scenario(json!([
        {"command":"@emit Marco says, \"I paid 7 silver at the bookstore.\"", "expect":["Marco says, \"I paid 7 silver"]},
        {"command":"@emit Ada says, \"You told me twelve earlier.\"", "expect":["Ada says, \"You told me twelve"]},
        {"command":"say Hello, I will visit the flower shop.", "expect":["Rosebot says"]},
        {"command":"goto east", "expect":["Flower Shop #"]},
        {"command":"get rose", "expect":["You take rose."]},
        {"command":"goto west", "expect":["Market Square #"]}
    ]));
    let first = output(fixture.command(&config).arg("--script").arg(scenario)).await;
    assert!(
        first.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );

    let model = MockServer::start().await;
    let calls = Arc::new(AtomicU32::new(0));
    let next = calls.clone();
    Mock::given(method("POST"))
        .respond_with(move |request: &Request| {
            let body: Value = request.body_json().unwrap();
            let context: Value = serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            let recalled = &context["remembered_experience"];
            let records = recalled["records"].as_array().unwrap();
            assert!(!context["terminal_history"].as_str().unwrap().contains("Marco"), "the answer must come from earlier sessions");
            assert_eq!(recalled["speaker_labels_establish_known_names"], false);
            assert!(body["messages"][0]["content"].as_str().unwrap().contains("Neither does a missing introduction prove you do not know them"));
            let reply_text = match next.fetch_add(1, Ordering::SeqCst) {
                0 => {
                    assert!(recalled["query"].as_str().unwrap().contains("Marco"));
                    assert!(records.iter().any(|r| r["event"]["data"]["source"] == "observed" && r["event"]["data"]["text"].as_str().unwrap().contains("I paid 7 silver")));
                    assert!(recalled.to_string().contains("You told me twelve earlier"));
                    "I recall a speaker labelled Marco claiming to have paid 7 silver. Ada challenged the amount. Name knowledge is unestablished."
                }
                1 => {
                    assert!(records.iter().any(|r| r["event"]["data"]["source"] == "prior_knowledge" && r["event"]["data"]["text"].as_str().unwrap().contains("old friend")));
                    "According to my prior knowledge, I know Marco by name as an old friend."
                }
                2 => {
                    return reply(json!({"action":"recall", "query":"Marco", "summary":"Find the original payment testimony before saving a belief.", "beliefs":[{"kind":"typo"}]}));
                }
                3 => {
                    assert_eq!(recalled["query"], "Marco", "a recall turn must drive retrieval for the following decision");
                    assert!(!context["belief_feedback"].as_array().unwrap().is_empty(), "malformed auxiliary annotations should receive feedback, not halt play");
                    let episode = records.iter().find(|r| r["event"]["data"]["source"] == "observed" && r["event"]["data"]["text"].as_str().unwrap().contains("I paid 7 silver")).unwrap();
                    return reply(json!({"action":"done", "summary":"Saved Marco's payment claim as testimony, not a confirmed purchase.", "beliefs":[{
                        "kind":"transaction", "subject":"Marco's claimed bookstore payment", "predicate":"claimed_paid", "value":"7 silver",
                        "basis":"reported", "confidence":"tentative", "evidence":[{"episode":episode["id"], "quote":"I paid 7 silver at the bookstore."}]
                    }]}));
                }
                n => panic!("unexpected memory model request {n}"),
            };
            ResponseTemplate::new(200).set_body_json(json!({"choices":[{"message":{"content":reply_text}}]}))
        })
        .expect(4)
        .mount(&model).await;
    let config = fixture.config(Some(&model), 20);
    let report = fixture.dir.join("memory-result.json");
    let mut child = fixture
        .command(&config)
        .arg("--paused")
        .arg("--report")
        .arg(&report)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    read_until(&mut lines, "[bot/send] look").await;
    read_until(&mut lines, "Exits    east").await;
    input
        .write_all(b"/ask Does the name Marco ring a bell, and what price did he mention?\n")
        .await
        .unwrap();
    read_until(&mut lines, "[bot/answer] I recall a speaker labelled Marco").await;
    input.write_all(b"/route Flower Shop\n").await.unwrap();
    let route = read_until(&mut lines, "remembered_route").await;
    assert!(route.contains("goto east"));
    input
        .write_all(b"/remember Marco, the bookseller, is an old friend I know by name.\n")
        .await
        .unwrap();
    read_until(&mut lines, "[bot/remembered]").await;
    input
        .write_all(b"/ask Do I know Marco by name?\n")
        .await
        .unwrap();
    read_until(&mut lines, "[bot/answer] According to my prior knowledge").await;
    input
        .write_all(b"/goal Remember Marco's bookstore payment claim as testimony.\n/resume\n")
        .await
        .unwrap();
    read_until(&mut lines, "[bot/done]").await;
    input.write_all(b"/quit\n").await.unwrap();
    let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    let report: Value = serde_json::from_slice(&std::fs::read(report).unwrap()).unwrap();
    assert_eq!(report["memory_store"]["active_beliefs"], 1);
    assert_eq!(
        report["state"]["commands_sent"], 2,
        "only bootstrap; recall and questions never act in the game"
    );
    let journal =
        std::fs::read_to_string(report["memory_store"]["file"].as_str().unwrap()).unwrap();
    assert!(!journal.contains(PASSWORD));
    assert!(journal.contains("claimed_paid"));
    assert!(journal.contains("old friend"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dotenv_credentials_reach_auth_from_parent_and_shell_takes_precedence() {
    let fixture = Fixture::boot().await;
    let model = MockServer::start().await;
    let config = fixture.config(Some(&model), 2);
    let mut config_text = std::fs::read_to_string(&config).unwrap();
    config_text.push_str("api_key_env = \"MUSH_BOT_E2E_MODEL_KEY\"\n");
    std::fs::write(&config, config_text).unwrap();
    std::fs::write(
        fixture.dir.join(".env"),
        format!("MUSH_BOT_E2E_MODEL_KEY=dotenv-test-key\nMUSH_BOT_E2E_PASSWORD={PASSWORD}\n"),
    )
    .unwrap();
    let nested = fixture.dir.join("working");
    std::fs::create_dir(&nested).unwrap();

    for (run, key) in ["dotenv-test-key", "shell-test-key"]
        .into_iter()
        .enumerate()
    {
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", format!("Bearer {key}")))
            .respond_with(reply(decision("done", None, "I can see Market Square.")))
            .expect(1)
            .mount(&model)
            .await;
        let transcript = fixture.dir.join(format!("auth-{run}.jsonl"));
        let report = fixture.dir.join(format!("auth-{run}.json"));
        let mut command = fixture.command(&config);
        command
            .current_dir(&nested)
            .env_remove("MUSH_BOT_E2E_PASSWORD")
            .env_remove("MUSH_BOT_E2E_MODEL_KEY")
            .arg("--headless")
            .arg("--goal")
            .arg("Identify the starting room")
            .arg("--transcript")
            .arg(&transcript)
            .arg("--report")
            .arg(&report);
        if run == 1 {
            command.env("MUSH_BOT_E2E_MODEL_KEY", key);
        }
        let result = output(&mut command).await;
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        for content in [
            String::from_utf8_lossy(&result.stdout).into_owned(),
            String::from_utf8_lossy(&result.stderr).into_owned(),
            std::fs::read_to_string(transcript).unwrap(),
            std::fs::read_to_string(report).unwrap(),
        ] {
            assert!(!content.contains("dotenv-test-key"));
            assert!(!content.contains("shell-test-key"));
            assert!(!content.contains(PASSWORD));
        }
    }
    for request in model.received_requests().await.unwrap() {
        let body = String::from_utf8(request.body).unwrap();
        assert!(!body.contains("dotenv-test-key"));
        assert!(!body.contains("shell-test-key"));
        assert!(!body.contains(PASSWORD));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_dotenv_is_redacted_and_offline_commands_skip_loading_it() {
    let fixture = Fixture::boot().await;
    let config = fixture.config(None, 2);
    std::fs::write(
        fixture.dir.join(".env"),
        "MUSH_BOT_E2E_MODEL_KEY='private-unclosed-value\n",
    )
    .unwrap();
    let result = output(fixture.command(&config).arg("--headless")).await;
    assert_eq!(result.status.code(), Some(1));
    let stderr = String::from_utf8(result.stderr).unwrap();
    assert!(stderr.contains("could not load .env"));
    assert!(!stderr.contains("private-unclosed-value"));
    assert!(result.stdout.is_empty());

    let script = fixture.scenario(json!([{"command":"look","expect":["Market Square"]}]));
    let result = output(
        fixture
            .command(&config)
            .arg("--script")
            .arg(script)
            .arg("--check-script"),
    )
    .await;
    assert!(result.status.success());
    let result = output(fixture.command(&config).arg("--help")).await;
    assert!(result.status.success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_navigates_real_world_and_confirms_inventory_through_bot_binary() {
    let fixture = Fixture::boot().await;
    let model = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(|request: &Request| {
            let body: Value = request.body_json().unwrap();
            let context: Value =
                serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            let history = context["terminal_history"].as_str().unwrap();
            assert!(!history.contains(PASSWORD));
            let action = if history.contains("You are carrying:") {
                decision(
                    "done",
                    None,
                    "Inventory confirms I am carrying the rose from the Flower Shop.",
                )
            } else if history.contains("You take rose.") {
                decision(
                    "command",
                    Some("inventory"),
                    "Verify that the rose is in my inventory.",
                )
            } else if history.contains("Flower Shop #") {
                decision("command", Some("get rose"), "Pick up the visible rose.")
            } else {
                assert!(history.contains("Market Square"), "history: {history}");
                assert!(history.contains("east"));
                decision(
                    "command",
                    Some("goto east"),
                    "Explore the visible east exit.",
                )
            };
            reply(action)
        })
        .expect(4)
        .mount(&model)
        .await;
    let config = fixture.config(Some(&model), 10);
    let report = fixture.dir.join("result.json");
    let transcript = fixture.dir.join("session.jsonl");
    let result = output(
        fixture
            .command(&config)
            .arg("--headless")
            .arg("--report")
            .arg(&report)
            .arg("--transcript")
            .arg(&transcript),
    )
    .await;
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value = serde_json::from_slice(&std::fs::read(report).unwrap()).unwrap();
    assert_eq!(report["state"]["model_calls"], 4);
    assert_eq!(report["success"], true);
    let transcript = std::fs::read_to_string(transcript).unwrap();
    assert!(!transcript.contains(PASSWORD));
    assert!(transcript.contains("You take rose."));
    assert!(transcript.contains("\"kind\":\"result\""));
    for line in transcript.lines() {
        serde_json::from_str::<Value>(line).unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scenarios_assert_fresh_output_and_fail_on_timeout_rejection_and_echo() {
    for (steps, success) in [
        (
            json!([
                {"command":"goto east","expect":["Flower Shop"]},
                {"command":"get rose","expect":["You take rose."]},
                {"command":"inventory","expect":["You are carrying:","rose"]},
                {"command":"goto west","expect":["Market Square"]}
            ]),
            true,
        ),
        (
            json!([{"command":"look","expect":["not a real room"],"timeout_ms":300}]),
            false,
        ),
        (
            json!([{"command":"look","expect":["Market Square"],"reject":["Market Square"]}]),
            false,
        ),
        // The sent command contains the expected text, but it fails in-world.
        (
            json!([{"command":"get nonexistent-unicorn","expect":["nonexistent-unicorn"],"timeout_ms":300}]),
            false,
        ),
        // Initial/bootstrap Market Square output cannot satisfy a later step.
        (
            json!([{"command":"goto east","expect":["Market Square"],"timeout_ms":300}]),
            false,
        ),
    ] {
        let fixture = Fixture::boot().await;
        let config = fixture.config(None, 10);
        let scenario = fixture.scenario(steps);
        let result = output(fixture.command(&config).arg("--scenario").arg(scenario)).await;
        assert_eq!(
            result.status.success(),
            success,
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        if !success {
            assert_eq!(result.status.code(), Some(1));
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn turn_limit_and_invalid_model_commands_stop_without_executing() {
    for (action, limit) in [
        (decision("command", Some("look"), "Look again."), 1),
        (
            decision(
                "command",
                Some("say forbidden\rQUIT"),
                "Invalid multi-line command.",
            ),
            10,
        ),
    ] {
        let fixture = Fixture::boot().await;
        let model = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(reply(action))
            .expect(1)
            .mount(&model)
            .await;
        let config = fixture.config(Some(&model), limit);
        let result = output(fixture.command(&config).arg("--headless")).await;
        assert_eq!(
            result.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        assert!(!String::from_utf8_lossy(&result.stdout).contains("Rosebot says"));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn untrusted_host_is_rejected_before_model_or_commands() {
    let fixture = Fixture::boot().await;
    let config = fixture.config(None, 10);
    let text = std::fs::read_to_string(&config)
        .unwrap()
        .replace(&fixture.pin, "SHA256:wrong");
    std::fs::write(&config, text).unwrap();
    let scenario = fixture.scenario(json!([{"command":"look","expect":["Market Square"]}]));
    let result = output(fixture.command(&config).arg("--scenario").arg(scenario)).await;
    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).contains("SSH host key not trusted"));
    assert!(!String::from_utf8_lossy(&result.stdout).contains("[bot/send]"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repl_can_cancel_a_slow_model_and_replace_goal_without_stale_action() {
    let fixture = Fixture::boot().await;
    let model = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            reply(decision(
                "command",
                Some("say stale-action"),
                "Stale decision.",
            ))
            .set_delay(Duration::from_secs(2)),
        )
        .mount(&model)
        .await;
    let config = fixture.config(Some(&model), 10);
    let transcript = fixture.dir.join("repl.jsonl");
    let mut child = fixture
        .command(&config)
        .arg("--transcript")
        .arg(&transcript)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    tokio::time::timeout(Duration::from_secs(15), async {
        while let Some(line) = lines.next_line().await.unwrap() {
            if line.contains("[bot/model]") {
                break;
            }
        }
    })
    .await
    .unwrap();
    input
        .write_all(b"/pause\n/goal Inspect the market only\n/state\n")
        .await
        .unwrap();
    let mut seen = String::new();
    tokio::time::timeout(Duration::from_secs(2), async {
        while let Some(line) = lines.next_line().await.unwrap() {
            seen.push_str(&line);
            if line.contains("Inspect the market only") {
                break;
            }
        }
    })
    .await
    .expect("REPL must respond before model");
    assert!(seen.contains("Inspect the market only"));
    tokio::time::sleep(Duration::from_millis(2300)).await;
    input.write_all(b"/quit\n").await.unwrap();
    let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
    let transcript = std::fs::read_to_string(transcript).unwrap();
    assert!(!transcript.contains("stale-action"));
    assert!(transcript.contains("Inspect the market only"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_switching_preserves_state_and_accounts_for_costs_across_sessions() {
    const NOTES: &str = "I promised to return the map. The flower shop is east.";
    let fixture = Fixture::boot().await;
    let model = MockServer::start().await;
    let pending = Arc::new(Notify::new());
    let pending_request = pending.clone();
    let calls = Arc::new(AtomicU32::new(0));
    let next = calls.clone();
    Mock::given(method("POST")).respond_with(move |request: &Request| {
        let body: Value = request.body_json().unwrap();
        let context = request_context(request);
        let turn = next.fetch_add(1, Ordering::SeqCst);
        if turn < 4 {
            assert_eq!(context["runner_goal"], "Inspect this room only.");
            assert_eq!(context["runner_role"], "A patient mapmaker.");
            assert!(context["runner_instructions"].as_str().unwrap().contains("Keep the map safe."));
        }
        if turn > 0 { assert_eq!(context["memory"], NOTES); }
        let (content, cost, resolved) = match turn {
            0 => {
                assert_eq!(body["model"], "fixture-fast");
                assert_eq!(body["temperature"], 0.2);
                (json!({"action":"wait", "summary":"Remember my promise.", "memory":NOTES, "wait_ms":100}).to_string(), 0.1, "provider-fast")
            }
            1 => {
                assert_eq!(body["model"], "fixture-careful");
                assert_eq!(body["max_tokens"], 4096);
                ("I remember the map and the flower shop.".into(), 0.2, "provider-careful")
            }
            2 => {
                assert_eq!(body["model"], "fixture-careful");
                pending_request.notify_one();
                return ResponseTemplate::new(200).set_delay(Duration::from_secs(2)).set_body_json(json!({
                    "id":"gen-cancelled", "model":"provider-careful",
                    "choices":[{"message":{"content":json!({"action":"command", "command":"say stale-model-action", "summary":"Stale model action."}).to_string()}}],
                    "usage":{"cost":1.0}
                }));
            }
            3 => {
                assert_eq!(body["model"], "fixture-fast");
                assert_eq!(context["model_calls_remaining"], 7, "switches must not replenish the budget");
                (json!({"action":"done", "summary":"The room is Market Square."}).to_string(), 0.0, "provider-fast")
            }
            4 => {
                assert_eq!(body["model"], "fixture-careful", "CLI selection overrides the JSON default");
                ("The map promise survived this restart.".into(), 0.05, "provider-careful")
            }
            n => panic!("unexpected model switching request {n}"),
        };
        ResponseTemplate::new(200).set_body_json(json!({
            "id":format!("gen-{turn}"), "model":resolved, "provider":"fixture-provider",
            "choices":[{"message":{"content":content},"finish_reason":"stop"}],
            "usage":{"cost":cost,"prompt_tokens":10,"completion_tokens":5,"total_tokens":15,
                "cost_details":{"upstream_inference_cost":0.01}}
        }))
    }).expect(5).mount(&model).await;
    let profiles = fixture.dir.join("models.json");
    std::fs::write(&profiles, json!({"version":1,"default":"fast","profiles":{
        "fast":{"endpoint":format!("{}/v1/chat/completions",model.uri()),"name":"fixture-fast","temperature":0.2,"cost_currency":"USD"},
        "careful":{"endpoint":format!("{}/v1/chat/completions",model.uri()),"name":"fixture-careful","max_tokens":4096,"cost_currency":"USD"}
    }}).to_string()).unwrap();
    let config = fixture.config(None, 10);
    std::fs::write(
        &config,
        format!(
            "model_profiles = \"models.json\"\n{}",
            std::fs::read_to_string(&config).unwrap()
        ),
    )
    .unwrap();
    let report = fixture.dir.join("switch-report.json");
    let transcript = fixture.dir.join("switch-transcript.jsonl");
    let mut bot = PlayerConsole::spawn(
        fixture
            .command(&config)
            .arg("--paused")
            .arg("--report")
            .arg(&report)
            .arg("--transcript")
            .arg(&transcript),
    );
    bot.until("[bot/send] look").await;
    bot.until("Exits    east").await;
    bot.send("/goal Inspect this room only.").await;
    bot.send("/role A patient mapmaker.").await;
    bot.send("/note Keep the map safe.").await;
    bot.send("/step").await;
    bot.until("[bot/summary] Remember my promise.").await;
    bot.send("/model use does-not-exist").await;
    bot.until("[bot/control-error] unknown model profile").await;
    bot.send("/model use careful").await;
    bot.until("[bot/paused] Model switched.").await;
    bot.send("/ask What do you remember?").await;
    bot.until("[bot/answer] I remember the map").await;
    bot.send("/step").await;
    tokio::time::timeout(Duration::from_secs(5), pending.notified())
        .await
        .unwrap();
    bot.send("/model use fast").await;
    bot.until("[bot/paused] Model switched.").await;
    bot.send("/step").await;
    bot.until("[bot/done]").await;
    tokio::time::sleep(Duration::from_millis(2200)).await;
    bot.send("/cost").await;
    bot.until("[bot/cost]").await;
    bot.quit().await;
    let first: Value = serde_json::from_slice(&std::fs::read(report).unwrap()).unwrap();
    let totals = &first["costs"]["session"]["totals"];
    assert_eq!(first["state"]["model_calls"], 4);
    assert_eq!(first["model"]["profile"], "fast");
    assert_eq!(totals["requests"], 4);
    assert_eq!(totals["known_cost_requests"], 3);
    assert_eq!(totals["unknown_cost_requests"], 1);
    assert_eq!(totals["cancelled"], 1);
    assert_eq!(totals["provider_cost_by_currency"]["USD"], "0.3");
    assert_eq!(
        first["costs"]["session"]["by_model"]["provider-careful"]["provider_cost_by_currency"]["USD"],
        "0.2"
    );
    assert_eq!(
        first["costs"]["session"]["by_prompt_type"]["runner_question"]["provider_cost_by_currency"]
            ["USD"],
        "0.2"
    );
    let events = std::fs::read_to_string(transcript).unwrap();
    assert!(!events.contains("stale-model-action"));
    let records: Vec<Value> = events
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        records
            .iter()
            .filter(|e| e["kind"] == "model-request")
            .count(),
        4
    );
    assert!(
        records
            .iter()
            .filter(|e| e["kind"] == "model-request")
            .all(|e| e["data"]["prompt_revision"].as_str().unwrap().len() == 64)
    );

    let report = fixture.dir.join("restart-report.json");
    let mut bot = PlayerConsole::spawn(
        fixture
            .command(&config)
            .arg("--paused")
            .arg("--model")
            .arg("careful")
            .arg("--report")
            .arg(&report),
    );
    bot.until("[bot/send] look").await;
    bot.until("Exits    east").await;
    bot.send("/ask What promise did I make?").await;
    bot.until("[bot/answer] The map promise survived").await;
    bot.quit().await;
    let second: Value = serde_json::from_slice(&std::fs::read(report).unwrap()).unwrap();
    assert_eq!(second["costs"]["sessions_recorded"], 2);
    assert_eq!(
        second["costs"]["session"]["totals"]["provider_cost_by_currency"]["USD"],
        "0.05"
    );
    assert_eq!(
        second["costs"]["all_sessions"]["totals"]["provider_cost_by_currency"]["USD"],
        "0.35"
    );
    assert_eq!(
        second["costs"]["all_sessions"]["totals"]["unknown_cost_requests"],
        1
    );
    assert_eq!(calls.load(Ordering::SeqCst), 5);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_charges_are_recorded_for_truncated_invalid_and_error_responses() {
    for (status, content, finish) in [
        (200, "{}", "length"),
        (200, "not a decision", "stop"),
        (400, "", "stop"),
    ] {
        let fixture = Fixture::boot().await;
        let model = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(status).set_body_json(json!({
            "id":"gen-unusable", "choices":[{"message":{"content":content}, "finish_reason":finish}],
            "usage":{"cost":0.125,"total_tokens":10}
        }))).expect(1).mount(&model).await;
        let config = fixture.config(Some(&model), 2);
        std::fs::write(
            &config,
            format!(
                "{}\ncost_currency = \"USD\"\n",
                std::fs::read_to_string(&config).unwrap()
            ),
        )
        .unwrap();
        let report = fixture.dir.join("failed-response.json");
        let result = output(
            fixture
                .command(&config)
                .arg("--headless")
                .arg("--report")
                .arg(&report),
        )
        .await;
        assert_eq!(result.status.code(), Some(2));
        let report: Value = serde_json::from_slice(&std::fs::read(report).unwrap()).unwrap();
        assert_eq!(report["success"], false);
        assert_eq!(
            report["costs"]["session"]["totals"]["known_cost_requests"],
            1
        );
        assert_eq!(
            report["costs"]["session"]["totals"]["provider_cost_by_currency"]["USD"],
            "0.125"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repl_single_step_question_and_passthrough_share_the_live_session() {
    let fixture = Fixture::boot().await;
    let model = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|request: &Request| {
            let body: Value = request.body_json().unwrap();
            if body["messages"].as_array().unwrap().len() == 3 {
                ResponseTemplate::new(200).set_body_json(json!({"choices":[{"message":{
                    "content":"Your inventory is empty, and you are at Market Square."
                }}]}))
            } else {
                reply(decision("command", Some("inventory"), "Check possessions."))
            }
        })
        .expect(2)
        .mount(&model)
        .await;
    let config = fixture.config(Some(&model), 10);
    let transcript = fixture.dir.join("controls.jsonl");
    let mut child = fixture
        .command(&config)
        .arg("--paused")
        .arg("--transcript")
        .arg(&transcript)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    // A step requested during bootstrap waits for the bootstrap observations.
    input.write_all(b"/step\n").await.unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        while let Some(line) = lines.next_line().await.unwrap() {
            if line.contains("You aren't carrying anything.") {
                return;
            }
        }
        panic!("bot closed before inventory reply");
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    input
        .write_all(b"/ask Where am I and what am I carrying?\n")
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(line) = lines.next_line().await.unwrap() {
            if line.contains("[bot/answer] Your inventory is empty") {
                return;
            }
        }
        panic!("bot closed before answering");
    })
    .await
    .unwrap();
    input.write_all(b"/send goto east\n").await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(line) = lines.next_line().await.unwrap() {
            if line.contains("Flower Shop #") {
                return;
            }
        }
        panic!("passthrough did not move the player");
    })
    .await
    .unwrap();
    input.write_all(b"/quit\n").await.unwrap();
    let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
    let events: Vec<Value> = std::fs::read_to_string(transcript)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let decisions = events.iter().filter(|e| e["kind"] == "decision").count();
    assert_eq!(
        decisions, 1,
        "single-step and questions must not resume autonomous play"
    );
    assert_eq!(events.last().unwrap()["data"]["state"]["model_calls"], 2);
}

fn worldgen_service(enabled: bool) -> (Arc<LlmService>, Arc<MockProvider>) {
    let provider = Arc::new(MockProvider::new("openrouter"));
    (
        worldgen_service_with_provider(enabled, provider.clone()),
        provider,
    )
}

fn worldgen_service_with_provider(
    enabled: bool,
    provider: Arc<dyn LlmProvider>,
) -> Arc<LlmService> {
    let service = LlmService::builder()
        .provider("openrouter", provider)
        .route(
            "worldgen_room",
            RouteConfig {
                model: "test/worldgen".into(),
                max_tokens: 1500,
                timeout_s: 10,
                reasoning: None,
                reasoning_max_tokens: None,
            },
        )
        .global_enabled(enabled)
        .build();
    Arc::new(service)
}

// This gate delays the SERVER's @gen provider, not the bot's model client.
// Release it only after the bot observes the pending job and chooses to wait.
struct DeferredWorldgen {
    provider: Arc<MockProvider>,
    release: Arc<Notify>,
}

#[async_trait::async_trait]
impl LlmProvider for DeferredWorldgen {
    fn id(&self) -> &ProviderId {
        self.provider.id()
    }

    fn caps(&self) -> ProviderCaps {
        self.provider.caps()
    }

    async fn complete(
        &self,
        request: &CompletionRequest,
        timeout: Duration,
    ) -> Result<CompletionResponse, LlmError> {
        self.release.notified().await;
        self.provider.complete(request, timeout).await
    }

    async fn stream(
        &self,
        request: &CompletionRequest,
        timeout: Duration,
    ) -> Result<LlmStream, LlmError> {
        self.release.notified().await;
        self.provider.stream(request, timeout).await
    }
}

fn room_draft(desc: &str) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::ToolUse {
            id: "room-draft".into(),
            name: "submit_room".into(),
            input: json!({"desc":desc}),
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage {
            input_tokens: 100,
            output_tokens: 50,
            ..Usage::default()
        },
        model: "test/worldgen".into(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn omitted_memory_while_waiting_for_worldgen_preserves_notes_and_finishes_goal() {
    const DESCRIPTION: &str = "Secondhand books fill wooden shelves beside a worn armchair.";
    const NOTES: &str = "Bookstore is north of Market Square; south returns to the square.";
    let worldgen = Arc::new(MockProvider::new("openrouter"));
    worldgen.push_response(room_draft(DESCRIPTION));
    let release = Arc::new(Notify::new());
    let llm = worldgen_service_with_provider(
        true,
        Arc::new(DeferredWorldgen {
            provider: worldgen.clone(),
            release: release.clone(),
        }),
    );
    let fixture = Fixture::boot_with_builder(true, Some(llm)).await;
    let model = MockServer::start().await;
    let phase = Arc::new(AtomicU32::new(0));
    let waits = Arc::new(AtomicU32::new(0));
    let next = phase.clone();
    let waiting = waits.clone();
    Mock::given(method("POST"))
        .respond_with(move |request: &Request| {
            let body: Value = request.body_json().unwrap();
            let context: Value = serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            let history = context["terminal_history"].as_str().unwrap();
            let observed = history.rsplit("[command]").next().unwrap();
            let step = next.load(Ordering::SeqCst);
            if step >= 3 {
                assert_eq!(context["memory"], NOTES, "omitted memory must preserve the previous notes");
            }
            let mut action = match step {
                0 => decision("command", Some("@dig Bookstore=north;n,south;s"), "Create the bookstore and return exit."),
                1 => {
                    assert!(observed.contains("Bookstore created with room number"));
                    decision("command", Some("goto north"), "Enter the bookstore.")
                }
                2 => {
                    assert!(observed.contains("Bookstore #"));
                    let mut action = decision("command", Some("@gen here=A tidy secondhand bookstore with wooden shelves and an armchair."), "Request a bookstore draft.");
                    action["memory"] = json!(NOTES);
                    action
                }
                3 => {
                    if !observed.contains("Draft ready.") {
                        assert!(observed.contains("Generating a description for Bookstore"));
                        if waiting.fetch_add(1, Ordering::SeqCst) == 0 {
                            release.notify_one();
                        }
                        // Regression: a real provider omitted memory on this turn.
                        // Omit wait_ms as well to exercise the bounded default.
                        return reply(json!({"action":"wait", "summary":"Wait for the generated draft."}));
                    }
                    assert!(observed.contains(DESCRIPTION));
                    decision("command", Some("@gen/accept desc"), "The draft matches the brief; accept its description.")
                }
                4 => {
                    assert!(observed.contains("Accepted. Wrote: desc. Draft cleared."));
                    decision("command", Some("look"), "Verify the accepted description.")
                }
                5 => {
                    assert!(observed.contains(DESCRIPTION));
                    decision("command", Some("goto south"), "Return to Market Square.")
                }
                6 => {
                    assert!(observed.contains("Market Square #"));
                    decision("done", None, "The bookstore is described and I am back at Market Square.")
                }
                n => panic!("unexpected bookstore turn {n}"),
            };
            if step >= 3 {
                action.as_object_mut().unwrap().remove("memory");
                action.as_object_mut().unwrap().remove("wait_ms");
            }
            next.fetch_add(1, Ordering::SeqCst);
            reply(action)
        })
        .mount(&model)
        .await;
    let config = fixture.config(Some(&model), 20);
    let report = fixture.dir.join("bookstore.json");
    let transcript = fixture.dir.join("bookstore.jsonl");
    let result = output(fixture.command(&config)
        .arg("--headless")
        .arg("--goal").arg("Build a bookstore north of Market Square, generate and accept its description, then return to the square.")
        .arg("--report").arg(&report)
        .arg("--transcript").arg(&transcript)).await;
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(phase.load(Ordering::SeqCst), 7);
    assert!(waits.load(Ordering::SeqCst) >= 1);
    assert_eq!(worldgen.call_count(), 1, "waiting must not duplicate @gen");
    let report: Value = serde_json::from_slice(&std::fs::read(report).unwrap()).unwrap();
    assert_eq!(report["success"], true);
    assert_eq!(report["state"]["memory"], NOTES);
    let events: Vec<Value> = std::fs::read_to_string(transcript)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let wait = events
        .iter()
        .find(|event| event["kind"] == "decision" && event["data"]["action"] == "wait")
        .unwrap();
    assert!(wait["data"]["memory"].is_null());
    assert_eq!(wait["data"]["wait_ms"], 1000);
    let sent: Vec<_> = events
        .iter()
        .filter(|event| event["kind"] == "send")
        .map(|event| event["data"].as_str().unwrap())
        .collect();
    assert_eq!(
        sent,
        [
            "help",
            "look",
            "@dig Bookstore=north;n,south;s",
            "goto north",
            "@gen here=A tidy secondhand bookstore with wooden shelves and an armchair.",
            "@gen/accept desc",
            "look",
            "goto south",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn builder_bot_digs_generates_reviews_retries_accepts_and_walks_both_exits() {
    const FIRST: &str = "Rose beds surround a marble fountain under the summer sun.";
    const REVISED: &str =
        "Moonlit rose beds surround a marble fountain; the south gate leads back to the market.";
    let (llm, worldgen) = worldgen_service(true);
    worldgen.push_response(room_draft(FIRST));
    worldgen.push_response(room_draft(REVISED));
    let fixture = Fixture::boot_with_builder(true, Some(llm)).await;
    let model = MockServer::start().await;
    let phase = Arc::new(AtomicU32::new(0));
    let next = phase.clone();
    Mock::given(method("POST"))
        .respond_with(move |request: &Request| {
            let body: Value = request.body_json().unwrap();
            let policy = body["messages"][0]["content"].as_str().unwrap();
            assert!(policy.contains("World creation is supported"));
            assert!(policy.contains("BUILDER_LLM"));
            let context: Value = serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            assert!(context["runner_goal"].as_str().unwrap().contains("Build a Rose Garden"));
            let history = context["terminal_history"].as_str().unwrap();
            let observed = history.rsplit("[command]").next().unwrap();
            let wait = || {
                let mut d = decision("wait", None, "Wait for the generated draft before reviewing it.");
                d["wait_ms"] = json!(200);
                reply(d)
            };
            let action = match next.load(Ordering::SeqCst) {
                0 => {
                    assert!(observed.contains("Market Square"));
                    decision("command", Some("@dig Rose Garden=north;n,south;s"), "Create the requested garden and linked exits.")
                }
                1 => {
                    assert!(observed.contains("Rose Garden created with room number"));
                    assert!(observed.contains("Opened exit north"));
                    assert!(observed.contains("Opened exit south"));
                    decision("command", Some("goto north"), "Enter the new room through its exit.")
                }
                2 => {
                    assert!(observed.contains("Rose Garden #"));
                    decision("command", Some("@gen here=A moonlit rose garden with a marble fountain and a south gate back to the market."), "Generate a draft for the garden.")
                }
                3 => {
                    if !observed.contains(FIRST) { return wait(); }
                    assert!(observed.contains("Worldgen draft for Rose Garden"));
                    decision("command", Some("look"), "Check that the preview has not changed the room yet.")
                }
                4 => {
                    assert!(observed.contains("Rose Garden #"));
                    assert!(!observed.contains(FIRST), "draft must remain uncommitted");
                    decision("command", Some("@gen/retry Use moonlight instead of the summer sun; include the existing south gate."), "Revise the draft to match the requested night setting.")
                }
                5 => {
                    if !observed.contains(REVISED) { return wait(); }
                    decision("command", Some("@gen/accept desc"), "The revised draft matches the brief and existing exit; commit its description.")
                }
                6 => {
                    assert!(observed.contains("Accepted. Wrote: desc. Draft cleared."));
                    decision("command", Some("look"), "Verify the accepted description in the world.")
                }
                7 => {
                    assert!(observed.contains(REVISED));
                    decision("command", Some("goto south"), "Verify the return exit.")
                }
                8 => {
                    assert!(observed.contains("Market Square #"));
                    decision("command", Some("goto north"), "Verify the entrance and persisted description again.")
                }
                9 => {
                    assert!(observed.contains("Rose Garden #"));
                    assert!(observed.contains(REVISED));
                    decision("done", None, "Created the garden, accepted its reviewed description, and walked both exits.")
                }
                n => panic!("unexpected builder turn {n}"),
            };
            next.fetch_add(1, Ordering::SeqCst);
            reply(action)
        }).mount(&model).await;
    let config = fixture.config(Some(&model), 30);
    let result = output(fixture.command(&config)
        .arg("--headless").arg("--role").arg("A world builder following the setting.")
        .arg("--goal").arg("Build a Rose Garden north of here with a south exit back, generate a moonlit description, review and accept it, and verify both exits.")).await;
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(phase.load(Ordering::SeqCst), 10);
    assert_eq!(
        worldgen.call_count(),
        2,
        "one generation and one reviewed retry"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn world_creation_keeps_server_permission_ownership_and_generation_gates() {
    for (builder, enabled, steps) in [
        (
            false,
            true,
            json!([
                {"command":"@dig Forbidden Garden","expect":["Permission denied"],"reject":["created with room number"]},
                {"command":"@gen here=A garden","expect":["Permission denied"]}
            ]),
        ),
        (
            true,
            true,
            json!([
                {"command":"goto east","expect":["Flower Shop"]},
                {"command":"@dig Forbidden Garden=north,south","expect":["Permission denied"],"reject":["created with room number"]},
                {"command":"@gen here=A garden","expect":["Permission denied"]}
            ]),
        ),
        (
            true,
            false,
            json!([
                {"command":"@gen here=A garden","expect":["LLM features are disabled"]}
            ]),
        ),
    ] {
        let (llm, worldgen) = worldgen_service(enabled);
        let fixture = Fixture::boot_with_builder(builder, Some(llm)).await;
        let config = fixture.config(None, 10);
        let scenario = fixture.scenario(steps);
        let result = output(fixture.command(&config).arg("--scenario").arg(scenario)).await;
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            worldgen.call_count(),
            0,
            "denied generation must not call a provider"
        );
    }
}

fn source_script(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../bots/scripts")
        .join(format!("{name}.json"))
}

async fn replay_and_check_record(fixture: &Fixture, config: &Path, path: &Path, run: usize) {
    let script = mush_bot::script::Scenario::load(path).unwrap();
    let transcript = fixture.dir.join(format!("replay-{run}.jsonl"));
    let report = fixture.dir.join(format!("replay-{run}.json"));
    let result = output(
        fixture
            .command(config)
            .arg("--script")
            .arg(path)
            .arg("--report")
            .arg(&report)
            .arg("--transcript")
            .arg(&transcript),
    )
    .await;
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value = serde_json::from_slice(&std::fs::read(report).unwrap()).unwrap();
    assert_eq!(report["success"], true);
    assert_eq!(report["state"]["scenario_steps_passed"], script.steps.len());
    assert_eq!(report["state"]["model_calls"], 0);
    assert_eq!(report["script"], serde_json::to_value(&script).unwrap());
    let events: Vec<Value> = std::fs::read_to_string(transcript)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let expected: Vec<_> = script
        .steps
        .iter()
        .filter_map(|s| s.command.clone())
        .collect();
    let sent: Vec<_> = events
        .iter()
        .filter(|event| event["kind"] == "send")
        .map(|event| event["data"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        sent, expected,
        "replay must send only the recorded commands, in source order"
    );
    assert_eq!(
        events
            .iter()
            .find(|event| event["kind"] == "script")
            .unwrap()["data"],
        report["script"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn json_source_scripts_replay_the_same_record_repeatedly() {
    // The same read-only record can run repeatedly against the same character.
    let fixture = Fixture::boot().await;
    let config = fixture.config(None, 10);
    for run in 0..3 {
        replay_and_check_record(&fixture, &config, &source_script("smoke"), run).await;
    }

    // World creation replays start from the same disposable world fixture.
    for run in 0..2 {
        let (llm, worldgen) = worldgen_service(true);
        worldgen.push_response(room_draft("Moonlit roses surround a marble fountain."));
        let fixture = Fixture::boot_with_builder(true, Some(llm)).await;
        let config = fixture.config(None, 10);
        replay_and_check_record(&fixture, &config, &source_script("rose-garden"), run).await;
        assert_eq!(worldgen.call_count(), 1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn json_validation_needs_no_config_and_rejects_other_formats_and_partial_scripts() {
    let character =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bots/characters/rosebot.json");
    for (option, check, path) in [
        ("--script", "--check-script", source_script("smoke")),
        ("--character", "--check-character", character.clone()),
    ] {
        let result = output(
            Command::new(env!("CARGO_BIN_EXE_mush-bot"))
                .arg(option)
                .arg(&path)
                .arg(check)
                .arg("--config")
                .arg("this-config-does-not-exist.toml"),
        )
        .await;
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let normalized: Value = serde_json::from_slice(&result.stdout).unwrap();
        let expected = if option == "--script" {
            serde_json::to_value(mush_bot::script::Scenario::load(&path).unwrap()).unwrap()
        } else {
            serde_json::to_value(mush_bot::character::Character::load(&path).unwrap()).unwrap()
        };
        assert_eq!(normalized, expected);
    }

    let fixture = Fixture::boot().await;
    let config = fixture.config(None, 10);
    // Reject non-JSON filenames even when the contents are otherwise valid JSON.
    // Check mode and ordinary startup must agree and never reach SSH.
    for (option, check, source) in [
        ("--script", "--check-script", source_script("smoke")),
        ("--character", "--check-character", character),
    ] {
        for extension in ["md", "markdown", "txt"] {
            let path = fixture.dir.join(format!("legacy.{extension}"));
            std::fs::copy(&source, &path).unwrap();
            for checking in [false, true] {
                let mut command = fixture.command(&config);
                command.arg(option).arg(&path);
                if checking {
                    command.arg(check);
                }
                let result = output(&mut command).await;
                assert_eq!(result.status.code(), Some(1));
                let error = String::from_utf8_lossy(&result.stderr);
                assert!(error.contains("extension must be .json"), "{error}");
                assert!(!String::from_utf8_lossy(&result.stdout).contains("[bot/connect]"));
            }
        }
    }
    let path = fixture.dir.join("invalid.json");
    // A later syntax error must stop the entire record before the valid first
    // step can connect and execute. Validation is not streamed during replay.
    std::fs::write(
        &path,
        r#"{"name":"Invalid","steps":[
        {"command":"say should-not-run","expect":["should-not-run"]},
        {"commmand":"QUIT","expect":["Goodbye"]}
    ]}"#,
    )
    .unwrap();
    let result = output(fixture.command(&config).arg("--script").arg(&path)).await;
    assert_eq!(result.status.code(), Some(1));
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(error.contains("invalid.json"));
    assert!(error.contains("unknown field `commmand`"));
    assert!(!String::from_utf8_lossy(&result.stdout).contains("[bot/connect]"));
}
