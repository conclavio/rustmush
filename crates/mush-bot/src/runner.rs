use std::{
    collections::VecDeque,
    fs::File,
    io::{BufRead, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use russh::ChannelMsg;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{sync::mpsc, task::JoinHandle, time::Instant};

use crate::{
    Result,
    attention::{Attention, is_communication},
    character::Character,
    config::Config,
    costs::{Ledger, Outcome, PromptType},
    debugger,
    experiment::{Context, Prompts, save_json},
    knowledge::Knowledge,
    memory::Book,
    model::{Action, Brain, Capture, Decision, Reply},
    models::{Catalog, Selection},
    tail,
    terminal::{Decoder, plain},
    transport::{Session, validate_command},
};

// Preserve the original public scenario API for JSON command scripts.
pub use crate::script::{Scenario, ScenarioStep};

pub const REPL_HELP: &str = "Local controls:\n\
  /state                 Show role, goal, memory, recent output and limits\n\
  /goal <text>           Replace the goal and reset its run budget\n\
  /role <text>           Change the role (pauses the bot)\n\
  /note <text>           Add a runner instruction (pauses the bot)\n\
  /pause | /resume       Stop or start autonomous turns\n\
  /step                  Take exactly one model turn while paused\n\
  /ask <question>        Ask about state without acting (pauses the bot)\n\
  /feedback | /refute <docket.json>  Review goal usability or challenge claims\n\
  /playtest [save new.json]  Inspect/export operator reviews\n\
  /mission [load file.json|start|stop|save new.json]  Bounded testing\n\
  /probe <file.json>     Ask and grade a structured evidence question (pauses)\n\
  /send <command>        Send a game command (pauses the bot)\n\
  /knowledge <query>     Search bundled help and supplied documents\n\
  /recall <query|#id>    Retrieve older experiences and their evidence\n\
  /remember <text>       Supply durable prior character knowledge\n\
  /route <place|#dbref>  Find a previously travelled route from here\n\
  /character            Inspect the authored character profile\n\
  /attention            Inspect recent speech and attention cues\n\
  /conversation [load|save <json>] Inspect or change conversation policy\n\
  /recap <new.json>     Save a cited recap of this session's observed episodes\n\
  /bookmark <new.json>  Save a paused, settled bot bookmark (no world restore)\n\
  /listen               Listen and respond without pursuing a goal (/pause stops)\n\
  /model [list]         Show the active model or available profiles\n\
  /model use <profile>  Switch model and pause; memory and budget are retained\n\
  /cost                 Show session and all-session costs by model and purpose\n\
  /budget [load|save <json>] Inspect session limits or load/save a policy\n\
  /prompt [load|save <json>]  Inspect effective prompts or load/save a profile\n\
  /inspect [preview [question]] Show last request, or prepare a free preview\n\
  /group                Show this actor's authored group affiliations\n\
  /social [query|save <new.json>] Inspect social memory or export this perspective\n\
  /map                  Inspect traveled, reported and invalidated routes\n\
  /learning [query|save <new.json>] Inspect lessons and development proposals\n\
  /development [set <json>] Inspect or set learned-development policy\n\
  /activity [load|save <json>|on|off|retry|advance <ms>] Inspect/control routines\n\
  /character-state [set <json>|clear] Inspect or change a temporary condition\n\
  /context [load|save <json>] Inspect context controls or load/save settings\n\
  /debug [load|save <json>]   Inspect breakpoints/watches or load/save settings\n\
  /capture <new.json>    Save the last exact model request for offline replay\n\
  /step model           Produce a decision and hold it before applying\n\
  /step command | /commit Apply the held decision if observations are unchanged\n\
  /step observation     Pause and wait for current command output to settle\n\
  /arena [red|blue [n]|stop]  Fly the simulator: claim, board and fight n matches; bare shows the picture\n\
  /help | /quit          Show these controls or disconnect\n\
An unprefixed line is a game command, just like /send. Use SSH config for login.";

#[derive(Default)]
pub struct Options {
    pub headless: bool,
    pub paused: bool,
    pub transcript: Option<PathBuf>,
    pub report: Option<PathBuf>,
    pub scenario: Option<PathBuf>,
    pub json_events: bool,
    pub test_controls: bool,
    pub managed_floor: bool,
    pub campaign_epoch: Option<String>,
    pub restore_bot: Option<PathBuf>,
    pub campaign_quota: Option<usize>,
    /// `--arena <red|blue>`: fly the simulator for that side (cut AJ.2).
    pub arena: Option<crate::arena::Side>,
    /// `--matches <n>`: how many matches `--arena` plays (1..=100); else
    /// `[arena].matches`.
    pub matches: Option<u32>,
    /// `--mine <site>`: play the mining loop at that site (doc 20 cut
    /// SM.12c-bot).
    pub mine: Option<crate::mine::Site>,
}

pub enum Control {
    Line(String),
    Prompts(Prompts),
    Context(Context),
    Debug(debugger::Settings),
    Budget(crate::budget::Settings),
    Conversation(crate::conversation::Settings),
}

pub struct Frontend {
    pub controls: mpsc::Receiver<Control>,
    pub events: mpsc::Sender<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct State {
    pub status: String,
    pub role: String,
    pub goal: String,
    pub memory: String,
    pub instructions: String,
    pub recent: String,
    pub model_calls: usize,
    pub commands_sent: usize,
    pub scenario_steps_passed: usize,
    pub outcome: String,
}

#[derive(Serialize)]
pub struct Report {
    pub state: State,
    pub character: Character,
    pub affiliations: Option<crate::organization::Affiliations>,
    pub social: crate::social::Ledger,
    pub activity: crate::activity::State,
    pub mission: Value,
    pub playtest: crate::playtest::Archive,
    pub learning: crate::learning::State,
    pub map: crate::navigation::Map,
    pub character_state: crate::character::Condition,
    pub elapsed_ms: u128,
    pub success: bool,
    pub memory_store: Value,
    pub model: Option<Selection>,
    pub costs: crate::costs::Report,
    pub prompts: Prompts,
    pub context: Context,
    pub debug: debugger::Settings,
    pub budget: Value,
    pub conversation: Value,
    pub provenance: Value,
    pub simulated_model: bool,
    pub probe_results: VecDeque<crate::probe::Grade>,
    pub probe_results_omitted: usize,
    pub inherited_probe_results: usize,
    /// Canonical input record, including defaulted timeouts, for reproducibility.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script: Option<Scenario>,
    /// The simulator pilot's matches (cut AJ.2), when it flew.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arena: Option<crate::arena::report::Report>,
    /// The mining pilot's loop (doc 20 cut SM.12c-bot), when it played.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mine: Option<crate::mine::report::Report>,
}

struct Log {
    file: Option<File>,
    sequence: u64,
    echo: bool,
    json_events: bool,
    events: Option<mpsc::Sender<Value>>,
    request: Option<u64>,
    command_sequence: Option<u64>,
    provenance: Value,
    started: Instant,
}

impl Log {
    fn new(path: Option<&PathBuf>) -> Result<Self> {
        // A misspelled/reused artifact path must not silently destroy a prior run.
        let file = path
            .map(|p| File::options().write(true).create_new(true).open(p))
            .transpose()?;
        Ok(Self {
            file,
            sequence: 0,
            echo: true,
            json_events: false,
            events: None,
            request: None,
            command_sequence: None,
            provenance: Value::Null,
            started: Instant::now(),
        })
    }

    fn record(&mut self, kind: &str, data: Value) -> Result<()> {
        self.sequence += 1;
        let timestamp_ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
        let event = json!({"seq":self.sequence, "timestamp_ms":timestamp_ms,
            "elapsed_ms":self.started.elapsed().as_millis(), "provenance":self.provenance,
            "request_id":self.request, "command_sequence":self.command_sequence, "kind":kind, "data":data});
        if let Some(file) = &mut self.file {
            serde_json::to_writer(&mut *file, &event)?;
            writeln!(file)?;
            file.flush()?;
        }
        if self.json_events {
            println!("{event}");
            std::io::stdout().flush()?;
        }
        // The display may lag; the file/JSON event stream remains complete.
        if let Some(events) = &self.events {
            let _ = events.try_send(event);
        }
        Ok(())
    }

    fn event(&mut self, kind: &str, text: &str) -> Result<()> {
        if self.echo {
            println!("\n[bot/{kind}] {}", plain(text));
        }
        self.record(kind, json!(text))
    }
}

struct Pending {
    command: Option<String>,
    text: String,
    deadline: Instant,
    last_output: Instant,
    scenario: Option<ScenarioStep>,
}

impl Pending {
    fn response(&self) -> String {
        // The PTY echoes input. Echoes are not evidence that a command worked.
        self.text
            .lines()
            .filter(|line| self.command.as_deref() != Some(line.trim_end()))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

struct App {
    activity: crate::activity::State,
    activity_tick: Instant,
    mission: crate::mission::State,
    pending_playtest: Option<crate::playtest::Job>,
    playtest: crate::playtest::Archive,
    checkpoint: Option<Boundary>,
    inherited_usage: crate::budget::Usage,
    resume_status: Option<String>,
    config: Config,
    options: Options,
    session: Session,
    knowledge: Knowledge,
    character: Character,
    affiliations: Option<crate::organization::Affiliations>,
    character_state: crate::character::Condition,
    attention: Attention,
    attention_enabled: bool,
    noticed_attention: u64,
    model_snapshot: u64,
    social_turn: bool,
    social_followup: bool,
    social_started: Option<Instant>,
    memories: Book,
    recall_query: Option<String>,
    belief_feedback: Vec<String>,
    brain: Option<Brain>,
    models: Catalog,
    selected_model: Option<Selection>,
    costs: Arc<Mutex<Ledger>>,
    request_id: Option<u64>,
    state: State,
    log: Log,
    pending: Option<Pending>,
    model_task: Option<JoinHandle<Result<Reply>>>,
    asking: bool,
    pending_probe: Option<crate::probe::Spec>,
    probe_results: VecDeque<crate::probe::Grade>,
    probe_results_omitted: usize,
    inherited_probe_results: usize,
    single_step: bool,
    bootstrap: VecDeque<String>,
    scenario: Option<VecDeque<ScenarioStep>>,
    next_turn: Instant,
    goal_started: Instant,
    started: Instant,
    quit: bool,
    controls: Option<mpsc::Receiver<Control>>,
    prompts: Prompts,
    context_settings: Context,
    debug: debugger::Settings,
    budget: crate::budget::Settings,
    last_capture: Option<Capture>,
    capture_observation: Option<u64>,
    staged: Option<(Decision, u64)>,
    observation_revision: u64,
    stage_next: bool,
    await_observation: bool,
    last_snapshot: Instant,
    tape: Option<Arc<crate::tape::Tape>>,
    ready: bool,
    conversation: crate::conversation::Tracker,
    speech: Option<PreparedSpeech>,
    next_intent: u64,
    floor_fault: Option<&'static str>,
    /// The simulator pilot's lane (cut AJ.2): while it flies, every tick is its.
    arena: Option<crate::arena::Lane>,
    /// The lane's result has been written into the state.
    arena_reported: bool,
    /// When the last `arena-picture` record was written.
    arena_picture_ms: u64,
    /// The arena commander's client (cut AJ.3), when a commander model
    /// resolves: `[arena.commander_model]`, or the session's own only with
    /// `commander_from_session`. It shares the session's tape.
    commander: Option<Brain>,
    /// That model and where it came from; its selection is the one the cost
    /// ledger records.
    commander_model: Option<crate::arena::commander::Resolved>,
    /// The commander's one call in flight, beside `model_task`.
    commander_task: Option<JoinHandle<Result<Reply>>>,
    /// Its ledger request.
    commander_request: Option<u64>,
    /// The session's budget has blocked a commander call: it holds for the
    /// rest of the session, so every later lane starts with its commander off.
    commander_budget_off: bool,
    /// The mining pilot's lane (doc 20 cut SM.12c-bot): while it plays, every
    /// tick is its.
    mine: Option<crate::mine::Lane>,
    /// The mining lane's result has been written into the state.
    mine_reported: bool,
}

/// The pilot's name for itself (cut AJ.2): the character part of an
/// `account+Character` SSH username, else the character sheet's name. The lane
/// replaces it with the name in `Connected as {name} (` when it reads one.
fn arena_me(username: &str, character: &Character) -> String {
    match username.split_once('+') {
        Some((_, name)) if !name.trim().is_empty() => name.trim().to_owned(),
        _ => character.name.clone(),
    }
}

/// The refusal for an arena pilot under a mission contract.
const ARENA_MISSION: &str =
    "the arena pilot does not run under a mission contract; use a memory namespace without one";
/// The refusal for an arena pilot in a session that sends its own lines.
const ARENA_SESSION: &str = "--arena cannot be combined with --script, --paused, --managed-floor, \
                             --replay, --studio, --harness or a campaign session";
/// The refusal for a second arena pilot while one flies (`/arena`, and the goal
/// loop's `arena` action, which declines with it).
const ARENA_FLYING: &str = "the arena pilot is already flying; /arena stop first";
/// The refusal for a mining pilot under a mission contract (doc 20 cut
/// SM.12c-bot).
const MINE_MISSION: &str =
    "the mining pilot does not run under a mission contract; use a memory namespace without one";
/// The refusal for a mining pilot in a session that sends its own lines
/// (`main.rs` repeats it byte for byte).
const MINE_SESSION: &str = "--mine cannot be combined with --script, --paused, --managed-floor, \
                            --replay, --studio, --harness or a campaign session";
/// The refusal for a mining pilot with a control input (doc 20 cut
/// SM.12c-bot): the lane is a headless command-line lane (no `/mine`, no
/// pause), so no REPL line or frontend control can start another lane, pause
/// the run or take its ticks while it plays.
const MINE_HEADLESS: &str = "the mining pilot plays headless; add --headless";

/// **Whether a run starts `running`**: not paused, and something plays at
/// once — a goal, a script, the arena pilot or the mining pilot.
fn active_at_start(paused: bool, goal: bool, scripted: bool, arena: bool, mine: bool) -> bool {
    !paused && (goal || scripted || arena || mine)
}

/// **Whether this tick is the mining lane's** (doc 20 cut SM.12c-bot), given
/// a lane: the run is running and the lane has no result yet, or its result is
/// not yet written into the state.
fn mine_due_for(running: bool, result: bool, reported: bool) -> bool {
    if result { !reported } else { running }
}

/// Unix milliseconds now (the runner may read the clock; the mining lane may
/// not, so its epoch arrives as data).
fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

/// What the arena lane's pause does this tick (cut AM.2).
#[derive(Debug, PartialEq, Eq)]
enum PauseAction {
    Pause,
    Resume,
}

/// **The lane's pause follows the run's status** (cut AM.2, 16c Q-AM2): a lane
/// that flies, is not stopped and is not yet paused pauses while the run is
/// not `running`; a paused lane resumes once it is. A stopped lane is never
/// paused, so a failure (which stops it) is never reported as a pause.
fn pause_action(flying: bool, stopped: bool, paused: bool, running: bool) -> Option<PauseAction> {
    if flying && !stopped && !paused && !running {
        Some(PauseAction::Pause)
    } else if paused && running {
        Some(PauseAction::Resume)
    } else {
        None
    }
}

/// **Whether this tick is the lane's** (cut AJ.2; cut AO.2): it flies and
/// the run is running, or it flies **paused** and is not stopped — a paused
/// lane is ticked so it keeps reading and settles (its stale tail flushed,
/// its picture settled), typing nothing — or its one `SIM LEAVE` is due, or
/// its result is not yet written.
fn arena_due_for(
    flying: bool,
    running: bool,
    paused: bool,
    stopped: bool,
    leave_pending: bool,
    unreported_result: bool,
) -> bool {
    (flying && (running || (paused && !stopped))) || leave_pending || unreported_result
}

/// **Whether the lane's tick is the runner's whole tick** (cut AO.2): a
/// running lane's is (no speech, scenario, activity, social or goal model turn
/// runs while it flies); a **paused** lane's never is — it reads and settles,
/// types nothing, and the runner's tick goes on past it exactly as it did
/// before the lane was ticked while paused (a `/step` still takes its one goal
/// turn, a pending speech or scenario arm still runs).
fn arena_tick_ends_tick(paused: bool) -> bool {
    !paused
}

struct Boundary {
    path: PathBuf,
    resume_status: String,
    sent: bool,
    received: bool,
    output: String,
    deadline: Instant,
    saved: bool,
}

struct PreparedSpeech {
    decision: Decision,
    id: u64,
    observation: u64,
    due_ms: u64,
    advertised: bool,
    granted: bool,
    directed: bool,
    request: Option<u64>,
}

pub async fn run(config: Config, options: Options) -> Result<Report> {
    run_controlled(config, options, None).await
}

pub async fn run_controlled(
    mut config: Config,
    mut options: Options,
    frontend: Option<Frontend>,
) -> Result<Report> {
    let restore = options
        .restore_bot
        .as_deref()
        .map(crate::checkpoint::Snapshot::load)
        .transpose()?;
    if options.campaign_epoch.is_some()
        && (options.managed_floor
            || options.scenario.is_some()
            || options.headless
            || !options.json_events
            || config.model_tape.is_none())
    {
        return Err("campaign checkpoint adapter requires JSON controls, local model tape and no script or managed floor".into());
    }
    if restore.is_some() && options.campaign_epoch.is_none() {
        return Err("bot restore requires a certified campaign session".into());
    }
    if let Some(snapshot) = &restore {
        if snapshot.actor != config.ssh.username {
            return Err("checkpoint actor does not match SSH login".into());
        }
        if snapshot.tape.is_none() {
            return Err(
                "restored campaign requires a local tape; live provider admission is unsupported"
                    .into(),
            );
        }
        config.agent = snapshot.agent.clone();
        config.agent.bootstrap.clear();
    }
    config.validate()?;
    let tape_source = config
        .model_tape
        .as_deref()
        .map(crate::tape::Source::load)
        .transpose()?;
    let tape = tape_source
        .clone()
        .map(|s| Arc::new(crate::tape::Tape::new(s)));
    if tape.is_some() && config.model.is_none() && config.model_profiles.is_none() {
        config.model = Some(serde_json::from_value(
            json!({"endpoint":"http://127.0.0.1", "name":"test/tape"}),
        )?);
    }
    let prompts = config
        .prompts
        .as_deref()
        .map(Prompts::load)
        .transpose()?
        .unwrap_or_default();
    let context_settings = config
        .context
        .as_deref()
        .map(Context::load)
        .transpose()?
        .unwrap_or_default();
    let debug = config
        .debug
        .as_deref()
        .map(debugger::Settings::load)
        .transpose()?
        .unwrap_or_default();
    let budget = config
        .budget
        .as_deref()
        .map(crate::budget::Settings::load)
        .transpose()?
        .unwrap_or_default();
    let scenario = options
        .scenario
        .as_deref()
        .map(Scenario::load)
        .transpose()?;
    let conversation_settings = config
        .conversation
        .as_deref()
        .map(crate::conversation::Settings::load)
        .transpose()?
        .unwrap_or_default();
    if options.managed_floor && (!options.json_events || options.headless || scenario.is_some()) {
        return Err("managed floor requires interactive JSON event controls and no script".into());
    }
    if scenario.is_some() {
        options.headless = true;
    }
    let models = Catalog::from_config(&config)?;
    let selected_model = models.initial()?;
    if options.headless
        && scenario.is_none()
        && options.arena.is_none()
        && options.mine.is_none()
        && (config.agent.goal.is_empty() || selected_model.is_none())
    {
        return Err(
            "headless play requires a goal and [model], --arena, --mine, or --script".into(),
        );
    }
    if options.mine.is_some()
        && (scenario.is_some()
            || options.paused
            || options.managed_floor
            || options.campaign_epoch.is_some()
            || restore.is_some())
    {
        return Err(MINE_SESSION.into());
    }
    if options.mine.is_some() && !options.headless {
        return Err(MINE_HEADLESS.into());
    }
    if options.arena.is_some()
        && (scenario.is_some()
            || options.paused
            || options.managed_floor
            || options.campaign_epoch.is_some()
            || restore.is_some())
    {
        return Err(ARENA_SESSION.into());
    }
    let arena_matches = options.matches.unwrap_or(config.arena.matches);
    if !(1..=100).contains(&arena_matches) {
        return Err("--matches takes a number from 1 to 100".into());
    }
    if options.headless && options.paused {
        return Err("--paused and headless/script mode cannot be combined".into());
    }
    // Preflight paths, documents and API credentials before opening the game.
    let report_file = options
        .report
        .as_ref()
        .map(|path| File::options().write(true).create_new(true).open(path))
        .transpose()?;
    let knowledge = Knowledge::load(&config.knowledge)?;
    let character = config
        .character
        .as_deref()
        .map(Character::load)
        .transpose()?
        .unwrap_or_else(|| Character::for_login(&config.ssh.username));
    let mut memories = Book::open(&config.memory, &config.ssh)?;
    let affiliations = config
        .affiliations
        .as_deref()
        .map(crate::organization::Affiliations::load)
        .transpose()?;
    if let Some(snapshot) = &restore {
        memories.restore(snapshot.memory.clone())?;
    }
    for fact in character
        .prior_facts()
        .into_iter()
        .filter(|_| restore.is_none())
    {
        memories.seed_prior(&fact)?;
    }
    if let Some(affiliations) = &affiliations
        && restore.is_none()
    {
        for fact in affiliations.prior_facts() {
            memories.seed_prior(&fact)?;
        }
        for group in &affiliations.groups {
            for fact in &group.background {
                memories.seed_background(
                    &group.id,
                    fact,
                    &config.ssh.username,
                    &conversation_settings.scope,
                )?;
            }
        }
    }
    // Validate durable social versions/evidence before connecting or bootstrapping.
    memories.social(&config.ssh.username)?;
    memories.learning(&config.ssh.username)?;
    for record in memories.records() {
        if let crate::memory::Event::Navigation(report) = &record.event {
            report.validate(memories.records(), record.id)?;
        }
    }
    let mut mission = crate::mission::State::project(memories.records())?;
    let playtest = crate::playtest::Archive::project(memories.records())?;
    if playtest
        .entries
        .iter()
        .any(|e| e.actor != config.ssh.username)
    {
        return Err("playtest journal belongs to another actor".into());
    }
    if restore.is_none()
        && let Some(path) = &config.mission
    {
        let spec = crate::mission::Spec::load(path)?;
        spec.bind(
            &config.ssh,
            options.campaign_epoch.is_some() && options.test_controls,
        )?;
        mission.load(spec)?;
        memories.save_mission(&mission)?;
    }
    if let Some(spec) = &mission.spec {
        spec.bind(
            &config.ssh,
            options.campaign_epoch.is_some() && options.test_controls,
        )?;
        if options.arena.is_some() {
            return Err(ARENA_MISSION.into());
        }
        if options.mine.is_some() {
            return Err(MINE_MISSION.into());
        }
    }
    let mut activity = memories.activity()?;
    if restore.is_none() {
        activity.reconnect();
        if let Some(path) = &config.activities {
            activity.install(crate::activity::Plan::load(path)?, memories.records())?;
        }
    }
    if activity
        .plan
        .as_ref()
        .is_some_and(|p| p.clock == crate::activity::Clock::ManualTest)
        && !options.test_controls
    {
        return Err("manual_test activities require explicit --test-controls".into());
    }
    let attention = Attention::new(config.attention.clone(), &character, &config.ssh.username);
    let attention_enabled = config.attention.enabled;
    let brain = if scenario.is_none() {
        selected_model
            .as_ref()
            .map(|s| Brain::with_tape(&s.settings, tape.clone()))
            .transpose()?
    } else {
        None
    };
    // The arena commander's model (cut AJ.3), its credentials checked here
    // with the goal model's; a script never flies.
    let commander_model = if scenario.is_none() {
        crate::arena::commander::resolve(&config.arena, selected_model.as_ref())
    } else {
        None
    };
    let commander = commander_model
        .as_ref()
        .map(|r| crate::arena::commander::brain(r, tape.clone()))
        .transpose()?;
    let costs = Arc::new(Mutex::new(Ledger::open(
        &config.costs,
        &config.ssh,
        &config.memory.namespace,
    )?));
    let mut log = Log::new(options.transcript.as_ref())?;
    log.echo = frontend.is_none() && !options.json_events;
    log.json_events = options.json_events;
    log.provenance = json!({"scope":conversation_settings.scope,
        "session":format!("{:x}-{:x}", SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos(), std::process::id()),
        "actor":config.ssh.username,"clock":"session_monotonic_elapsed_ms"});
    let conversation =
        crate::conversation::Tracker::new(conversation_settings, &config.ssh.username);
    let controls = frontend.map(|f| {
        log.events = Some(f.events);
        f.controls
    });
    if let Some(source) = &tape_source {
        log.record("model-tape", serde_json::to_value(source)?)?;
        log.event("test-mode", "Local model tape enabled. Model responses are simulated; no provider requests are sent.")?;
    }
    log.record("prompts", prompts.effective())?;
    log.record("context-settings", serde_json::to_value(&context_settings)?)?;
    log.record("debug-settings", serde_json::to_value(&debug)?)?;
    log.record(
        "budget-settings",
        json!({"settings":budget,"revision":budget.revision()}),
    )?;
    log.event("memory", &serde_json::to_string(&memories.stats())?)?;
    log.record("character", serde_json::to_value(&character)?)?;
    log.record("group-affiliations", serde_json::to_value(&affiliations)?)?;
    log.record("model-selected", serde_json::to_value(&selected_model)?)?;
    if let Some(script) = &scenario {
        log.record("script", serde_json::to_value(script)?)?;
    }
    log.event(
        "connect",
        &format!(
            "{}@{}:{} (SSH line mode)",
            config.ssh.username, config.ssh.host, config.ssh.port
        ),
    )?;
    let session = Session::connect(&config.ssh).await?;
    // The game shows the character's room on connect, before any command.
    memories.expect_arrival();
    let now = Instant::now();
    let started = log.started;
    let active = active_at_start(
        options.paused,
        !config.agent.goal.is_empty(),
        scenario.is_some(),
        options.arena.is_some(),
        options.mine.is_some(),
    );
    let state = State {
        status: if active {
            "running"
        } else if !options.paused
            && attention_enabled
            && (brain.is_some()
                || conversation.settings.mode == crate::conversation::Mode::Spectator)
        {
            "listening"
        } else {
            "paused"
        }
        .into(),
        role: config.agent.role.clone(),
        goal: config.agent.goal.clone(),
        memory: memories.working_notes().to_owned(),
        instructions: String::new(),
        recent: String::new(),
        model_calls: 0,
        commands_sent: 0,
        scenario_steps_passed: 0,
        outcome: String::new(),
    };
    if let Some(scenario) = &scenario {
        log.event("scenario", &scenario.name)?;
    }
    let script = scenario.clone();
    let mut app = App {
        activity,
        activity_tick: now,
        checkpoint: None,
        inherited_usage: Default::default(),
        resume_status: None,
        // A replay sends exactly the commands in its source record. Login output
        // is still collected, but agent bootstrap belongs to autonomous play.
        // The arena pilot types its own `look`; the mining pilot its own walk.
        bootstrap: if script.is_some() || options.arena.is_some() || options.mine.is_some() {
            VecDeque::new()
        } else {
            config.agent.bootstrap.clone().into()
        },
        pending: Some(Pending {
            command: None,
            text: String::new(),
            deadline: now + Duration::from_millis(config.agent.observation_ms),
            last_output: now,
            scenario: None,
        }),
        config,
        options,
        session,
        knowledge,
        character,
        attention,
        attention_enabled,
        noticed_attention: 0,
        model_snapshot: 0,
        social_turn: false,
        social_followup: false,
        social_started: None,
        memories,
        recall_query: None,
        belief_feedback: Vec::new(),
        brain,
        models,
        selected_model,
        costs,
        request_id: None,
        state,
        log,
        model_task: None,
        asking: false,
        pending_probe: None,
        pending_playtest: None,
        playtest,
        mission,
        probe_results: Default::default(),
        probe_results_omitted: 0,
        inherited_probe_results: 0,
        single_step: false,
        scenario: scenario.map(|s| s.steps.into()),
        next_turn: now,
        started,
        goal_started: now,
        quit: false,
        controls,
        prompts,
        context_settings,
        debug,
        budget,
        last_capture: None,
        capture_observation: None,
        affiliations,
        character_state: Default::default(),
        staged: None,
        observation_revision: 0,
        stage_next: false,
        await_observation: false,
        last_snapshot: now,
        tape,
        ready: false,
        conversation,
        speech: None,
        next_intent: 1,
        floor_fault: None,
        arena: None,
        arena_reported: false,
        arena_picture_ms: 0,
        commander,
        commander_model,
        commander_task: None,
        commander_request: None,
        commander_budget_off: false,
        mine: None,
        mine_reported: false,
    };
    if let Some(snapshot) = restore {
        app.restore(snapshot)?;
    }
    if let Some(side) = app.options.arena {
        app.arena_start(side, arena_matches)?;
    }
    if let Some(site) = app.options.mine {
        app.mine_start(site)?;
    }
    let result = app.drive().await;
    // The one `SIM LEAVE`, when it is due, before the session closes.
    let arena_result = if result.is_ok() {
        app.arena_finish().await
    } else {
        Ok(())
    };
    let result = result.and(arena_result);
    let memory_result = app.memories.flush();
    let cancel_result = app.cancel_model();
    app.session.close().await;
    if let Err(error) = result.and(memory_result).and(cancel_result) {
        app.state.status = "error".into();
        app.state.outcome = error.to_string();
        app.log.event("error", &error.to_string())?;
    }
    let report = Report {
        success: app.state.status == "done",
        memory_store: app.memories.stats(),
        state: app.state.clone(),
        character: app.character.clone(),
        model: app.selected_model.clone(),
        costs: app.cost_report()?,
        elapsed_ms: app.started.elapsed().as_millis(),
        script,
        prompts: app.prompts.clone(),
        affiliations: app.affiliations.clone(),
        social: app.memories.social(&app.config.ssh.username)?,
        learning: app.memories.learning(&app.config.ssh.username)?,
        map: app.memories.map(),
        activity: app.activity.clone(),
        mission: app.mission.report(app.memories.records()),
        playtest: app.playtest.clone(),
        character_state: app.character_state.clone(),
        context: app.context_settings.clone(),
        debug: app.debug.clone(),
        budget: app.budget_report()?,
        conversation: app.conversation_snapshot(),
        provenance: app.log.provenance.clone(),
        simulated_model: app.tape.is_some(),
        probe_results: app.probe_results.clone(),
        probe_results_omitted: app.probe_results_omitted,
        inherited_probe_results: app.inherited_probe_results,
        arena: app.arena.as_ref().map(crate::arena::Lane::report),
        mine: app.mine.as_ref().map(crate::mine::Lane::report),
    };
    app.log.record("result", serde_json::to_value(&report)?)?;
    if let Some(file) = report_file {
        serde_json::to_writer_pretty(file, &report)?;
    }
    Ok(report)
}

fn stdin_lines() -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel(32);
    // Tokio stdin's blocking read can hold runtime shutdown open. A dedicated
    // thread may remain blocked and is intentionally not joined on exit.
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else {
                break;
            };
            if tx.blocking_send(line).is_err() {
                break;
            }
        }
    });
    rx
}

impl App {
    fn restore(&mut self, value: crate::checkpoint::Snapshot) -> Result<()> {
        self.config.playtest_feedback = value.playtest_feedback.unwrap_or(false);
        self.mission = value.mission.unwrap_or_default();
        self.activity = value.activity.unwrap_or_default();
        self.activity.cancel_unsent();
        self.character = value.character;
        self.affiliations = value.affiliations;
        self.character_state = value.condition;
        self.attention = Attention::restore(value.attention)?;
        self.knowledge = value.knowledge;
        let scope = self.conversation.settings.scope.clone();
        self.conversation = value.conversation;
        // Preserve scheduling time while giving this child a distinct trace session.
        self.conversation.settings.scope = scope;
        self.log.provenance["scope"] = serde_json::to_value(&self.conversation.settings.scope)?;
        self.log.provenance["scheduler_parent_elapsed_ms"] = json!(value.watermark.elapsed_ms);
        self.models = value.models;
        self.selected_model = value.model;
        self.tape = value.tape.map(|t| Arc::new(crate::tape::Tape::new(t)));
        self.brain = self
            .selected_model
            .as_ref()
            .map(|m| Brain::with_tape(&m.settings, self.tape.clone()))
            .transpose()?;
        self.prompts = value.prompts;
        self.context_settings = value.context;
        self.debug = value.debug;
        self.budget = value.budget;
        self.inherited_usage = value.usage;
        self.state = value.state;
        self.resume_status = Some(value.resume_status);
        self.recall_query = value.recall_query;
        self.belief_feedback = value.belief_feedback;
        self.attention_enabled = value.attention_enabled;
        self.noticed_attention = value.noticed_attention;
        self.social_followup = value.social_followup;
        let now = Instant::now();
        let ago = |ms| {
            now.checked_sub(Duration::from_millis(ms))
                .ok_or_else(|| -> crate::Error { "checkpoint timer out of range".into() })
        };
        self.started = ago(value.watermark.elapsed_ms)?;
        self.goal_started = ago(value.goal_elapsed_ms)?;
        self.social_started = value.social_elapsed_ms.map(ago).transpose()?;
        self.next_turn = now + Duration::from_millis(value.next_turn_after_ms);
        self.next_intent = value.next_intent;
        self.observation_revision = value.watermark.observation;
        self.probe_results = value.probe_results;
        self.inherited_probe_results = self.probe_results.len();
        self.probe_results_omitted = value.probe_results_omitted;
        self.last_capture = value.last_capture;
        self.log.record("restored", json!({"parent_epoch":value.epoch,"parent_watermark":value.watermark,
            "inherited_usage":self.inherited_usage,"pending_actions":"invalidated; recompute on resume"}))?;
        Ok(())
    }

    fn checkpoint_snapshot(&self, resume_status: String) -> Result<crate::checkpoint::Snapshot> {
        let value = crate::checkpoint::Snapshot {
            version: 4,
            actor: self.config.ssh.username.clone(),
            epoch: self
                .options
                .campaign_epoch
                .clone()
                .ok_or("campaign epoch missing")?,
            watermark: self.watermark(),
            state: self.state.clone(),
            resume_status,
            character: self.character.clone(),
            affiliations: self.affiliations.clone(),
            condition: self.character_state.clone(),
            memory: self.memories.checkpoint()?,
            social: Some(self.memories.social(&self.config.ssh.username)?),
            learning: Some(self.memories.learning(&self.config.ssh.username)?),
            activity: Some(self.activity.clone()),
            mission: Some(self.mission.clone()),
            playtest_feedback: Some(self.config.playtest_feedback),
            playtest: Some(self.playtest.clone()),
            attention: self.attention.checkpoint()?,
            knowledge: self.knowledge.clone(),
            conversation: self.conversation.clone(),
            models: self.models.clone(),
            model: self.selected_model.clone(),
            tape: self.tape.as_ref().map(|t| t.snapshot()).transpose()?,
            prompts: self.prompts.clone(),
            context: self.context_settings.clone(),
            debug: self.debug.clone(),
            budget: self.budget.clone(),
            usage: self.total_usage()?,
            agent: self.config.agent.clone(),
            recall_query: self.recall_query.clone(),
            belief_feedback: self.belief_feedback.clone(),
            attention_enabled: self.attention_enabled,
            noticed_attention: self.noticed_attention,
            social_followup: self.social_followup,
            social_elapsed_ms: self
                .social_started
                .map(|at| at.elapsed().as_millis() as u64),
            goal_elapsed_ms: self.goal_started.elapsed().as_millis() as u64,
            next_turn_after_ms: self
                .next_turn
                .saturating_duration_since(Instant::now())
                .as_millis() as u64,
            next_intent: self.next_intent,
            last_capture: self.last_capture.clone(),
            probe_results: self.probe_results.clone(),
            probe_results_omitted: self.probe_results_omitted,
        };
        value.validate()?;
        Ok(value)
    }

    async fn checkpoint_tick(&mut self) -> Result<bool> {
        let Some(boundary) = &mut self.checkpoint else {
            return Ok(false);
        };
        if boundary.saved {
            return Ok(true);
        }
        if Instant::now() > boundary.deadline {
            return Err("checkpoint timed out: no verified frozen-world output watermark".into());
        }
        if self.pending.is_some() || !self.bootstrap.is_empty() {
            return Ok(false);
        }
        if !boundary.sent {
            self.session
                .send(&format!(
                    "__campaign_fence {}",
                    self.options.campaign_epoch.as_ref().unwrap()
                ))
                .await?;
            boundary.sent = true;
            return Ok(true);
        }
        if boundary.received {
            let path = boundary.path.clone();
            let status = boundary.resume_status.clone();
            self.memories.finish_command()?;
            self.attention.flush_if_quiet(0);
            self.memories.save_activity(&self.activity)?;
            let snapshot = self.checkpoint_snapshot(status)?;
            save_json(&path, &snapshot)?;
            let checked = crate::checkpoint::Snapshot::load(&path)?;
            self.log.record("checkpoint-ready", json!({"path":path,"sha256":checked.digest()?,"epoch":checked.epoch,"watermark":checked.watermark}))?;
            self.checkpoint.as_mut().unwrap().saved = true;
        }
        Ok(true)
    }

    fn total_usage(&self) -> Result<crate::budget::Usage> {
        let mut usage = self
            .costs
            .lock()
            .map_err(|_| "cost ledger lock poisoned")?
            .session_usage()?;
        usage.requests = usage
            .requests
            .checked_add(self.inherited_usage.requests)
            .ok_or("usage overflow")?;
        usage.unknown_cost_requests += self.inherited_usage.unknown_cost_requests;
        for (currency, cost) in &self.inherited_usage.provider_cost_by_currency {
            usage
                .provider_cost_by_currency
                .entry(currency.clone())
                .or_default()
                .add(*cost)?;
        }
        Ok(usage)
    }
    fn watermark(&self) -> crate::scene::Watermark {
        crate::scene::Watermark {
            session: self.log.provenance["session"]
                .as_str()
                .unwrap_or_default()
                .into(),
            scope: self.conversation.settings.scope.clone(),
            elapsed_ms: self.elapsed_ms(),
            output_sequence: self.log.sequence,
            observation: self.observation_revision,
            speech: self.attention.snapshot(),
        }
    }

    fn bookmark(&self) -> Result<crate::scene::Bookmark> {
        if self.state.status != "paused"
            || self.pending.is_some()
            || self.model_task.is_some()
            || self.speech.is_some()
            || self.staged.is_some()
            || self.single_step
            || !self.bootstrap.is_empty()
        {
            return Err("bookmark requires /pause, settled output and no staged decision; the world continues running".into());
        }
        crate::scene::Bookmark::new(self.watermark(), [
            ("runner".into(), json!({"state":self.state,"recall_query":self.recall_query,"belief_feedback":self.belief_feedback,
                "goal_elapsed_ms":self.goal_started.elapsed().as_millis(),"next_turn_after_ms":self.next_turn.saturating_duration_since(Instant::now()).as_millis()})),
            ("character".into(), json!({"profile":self.character,"affiliations":self.affiliations,"condition":self.character_state})),
            ("memory".into(), self.memories.bookmark()?),
            ("social".into(), serde_json::to_value(self.memories.social(&self.config.ssh.username)?)?),
            ("learning".into(), serde_json::to_value(self.memories.learning(&self.config.ssh.username)?)?),
            ("activity".into(), serde_json::to_value(&self.activity)?),
            ("mission".into(), self.mission.report(self.memories.records())),
            ("playtest".into(), serde_json::to_value(self.playtest.clone())?),
            ("attention".into(), self.attention.bookmark()),
            ("conversation".into(), self.conversation_snapshot()),
            ("configuration".into(), json!({"prompts":self.prompts,"context":self.context_settings,"debug":self.debug,
                "budget":self.budget,"model":self.selected_model,"models":self.models.list(),"knowledge":self.knowledge.bookmark(),
                "agent":self.config.agent,"attention_enabled":self.attention_enabled,"test_controls":self.options.test_controls,"floor_fault":self.floor_fault})),
            ("accounting".into(), json!({"costs":self.cost_report()?,"budget":self.budget_report()?,
                "restoration_policy":"reference append-only ledger; never refund or reset spend"})),
        ].into_iter().collect())
    }
    fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    fn conversation_snapshot(&self) -> Value {
        json!({"state":self.conversation.snapshot(),"managed":self.options.managed_floor,
            "intent":self.speech.as_ref().map(|s| json!({"id":s.id,"observation_watermark":s.observation,
                "due_elapsed_ms":s.due_ms,"advertised":s.advertised,"granted":s.granted,
                "request_id":s.request,"command":s.decision.command,"summary":s.decision.summary})),
            "phase":if self.speech.is_some() {"waiting"} else if self.model_task.is_some() {"preparing"}
                else if self.state.status == "paused" {"paused"} else if self.conversation.declined {"declining"}
                else if self.attention.has_pending() {"attending"} else {"listening"}})
    }

    fn snapshot(&self) -> Result<Value> {
        Ok(
            json!({"state":self.state, "memory_store":self.memories.stats(),
            "model":self.selected_model, "models":self.models.list(), "costs":self.cost_report()?,
            "prompts":self.prompts, "context":self.context_settings, "debug":self.debug,
            "budget":self.budget_report()?,
            "conversation":self.conversation_snapshot(),"provenance":self.log.provenance,
            "observation_watermark":self.observation_revision,"output_watermark":self.log.sequence,
            "group_affiliations":self.affiliations,
            "social":self.memories.social(&self.config.ssh.username)?,
            "learning":self.memories.learning(&self.config.ssh.username)?,"activity":self.activity,"map":self.memories.map(),
            "mission":self.mission.report(self.memories.records()),
            "playtest":self.playtest.view(),
            "pending_playtest":self.pending_playtest.is_some(),
            "pending_probe":self.pending_probe.as_ref().map(|p| &p.id),
            "last_probe":self.probe_results.back(), "probe_results_omitted":self.probe_results_omitted,
            "character_state":self.character_state,
            "capture_stale":self.capture_observation != Some(self.observation_revision),
            "staged_decision":self.staged.as_ref().map(|(d,_)| d),
            "attention":self.attention.context(),
            "attention_enabled":self.attention_enabled, "pending_attention":self.attention.has_pending(),
            "pending_command":self.pending.as_ref().and_then(|p| p.command.as_deref()),
            "model_in_flight":self.model_task.is_some(), "max_steps":self.config.agent.max_steps,
            "max_seconds":self.config.agent.max_seconds, "goal_elapsed_seconds":self.goal_started.elapsed().as_secs()}),
        )
    }

    async fn frontend_control(&mut self, control: Control) -> Result<()> {
        if let Control::Line(line) = control {
            return self.control(&line).await;
        }
        self.configure(control)
    }

    fn configure(&mut self, control: Control) -> Result<()> {
        match control {
            Control::Line(_) => return Err("expected a settings control".into()),
            Control::Prompts(prompts) => {
                prompts.validate()?;
                self.pause()?;
                self.prompts = prompts;
                self.log.record("prompts", self.prompts.effective())?;
                self.log
                    .event("paused", "Prompt profile changed; use /resume or /step.")?;
            }
            Control::Context(context) => {
                context.validate()?;
                self.pause()?;
                self.context_settings = context;
                self.log.record(
                    "context-settings",
                    serde_json::to_value(&self.context_settings)?,
                )?;
                self.log.event(
                    "paused",
                    "Context settings changed; the experience journal is retained.",
                )?;
            }
            Control::Debug(debug) => {
                debug.validate()?;
                self.pause()?;
                self.debug = debug;
                self.log
                    .record("debug-settings", serde_json::to_value(&self.debug)?)?;
                self.log
                    .event("paused", "Breakpoints and watches updated.")?;
            }
            Control::Budget(budget) => {
                budget.validate()?;
                self.pause()?;
                self.budget = budget;
                self.log.record(
                    "budget-settings",
                    json!({"settings":self.budget,"revision":self.budget.revision()}),
                )?;
                self.log.event("paused", "Session limits changed; recorded requests and costs are retained. Use /resume or /step to continue.")?;
            }
            Control::Conversation(settings) => {
                settings.validate()?;
                if settings.scope != self.conversation.settings.scope {
                    return Err("scene provenance is fixed for a session; start a new session to change scope".into());
                }
                self.pause()?;
                self.conversation
                    .configure(settings, &self.config.ssh.username);
                self.log
                    .record("conversation-policy", json!(self.conversation.settings))?;
            }
        }
        self.capture_observation = None;
        Ok(())
    }

    async fn receive_decision(&mut self, decision: Decision) -> Result<()> {
        let revision = self
            .capture_observation
            .unwrap_or(self.observation_revision);
        if self.stage_next || self.debug.command_break(decision.command.as_deref()) {
            self.pause()?;
            self.log
                .record("staged-decision", serde_json::to_value(&decision)?)?;
            self.log.event("breakpoint", "Decision held before applying memory or sending a command. /commit applies it; /resume replans.")?;
            self.staged = Some((decision, revision));
            Ok(())
        } else {
            self.prepare_action(decision, revision).await
        }
    }

    async fn commit(&mut self) -> Result<()> {
        let (decision, revision) = self
            .staged
            .take()
            .ok_or("no held decision; use /step model")?;
        if revision != self.observation_revision {
            return Err("game output changed after the decision; held action discarded, use /step model to reconsider".into());
        }
        if self.pending.is_some() {
            return Err("still collecting command output; use /step model after it settles".into());
        }
        if !self.state.goal.is_empty()
            && self.goal_started.elapsed() >= Duration::from_secs(self.config.agent.max_seconds)
        {
            return self.fail("Run time limit reached");
        }
        self.single_step = true;
        self.prepare_action(decision, revision).await
    }

    async fn prepare_action(&mut self, decision: Decision, revision: u64) -> Result<()> {
        if !decision.command.as_deref().is_some_and(is_communication) {
            return self.apply(decision).await;
        }
        if decision.command.as_deref().unwrap().chars().count()
            > self.conversation.settings.max_utterance_chars
        {
            return self.fail("Prepared utterance exceeds the conversation character limit; revise the goal or policy");
        }
        let id = self.next_intent;
        self.next_intent += 1;
        let directed = self.attention.records().any(|s| {
            s.id > self.attention.handled_through()
                && s.id <= self.model_snapshot
                && !s.self_echo
                && s.cues
                    .iter()
                    .any(|c| c == "name_or_alias" || c == "direct_message")
        });
        self.speech = Some(PreparedSpeech {
            decision,
            id,
            observation: revision,
            due_ms: self.elapsed_ms() + self.conversation.delay(id),
            advertised: false,
            granted: false,
            directed,
            request: self.log.request,
        });
        self.conversation.reason = "prepared reply held for pacing and fresh observations".into();
        self.log
            .record("speech-prepared", self.conversation_snapshot())?;
        if revision != self.observation_revision {
            self.cancel_speech("output changed during preparation")?;
        }
        Ok(())
    }

    fn cancel_speech(&mut self, reason: &str) -> Result<()> {
        if let Some(speech) = self.speech.take() {
            self.conversation.cancelled_replies += 1;
            self.conversation.reason = reason.into();
            self.log.record(
                "speech-cancelled",
                json!({"id":speech.id,"reason":reason,"request_id":speech.request}),
            )?;
            if speech.advertised {
                self.log.record(
                    "speech-complete",
                    json!({"id":speech.id,"outcome":"cancelled"}),
                )?;
            }
            self.single_step = false;
        }
        Ok(())
    }

    async fn dispatch_speech(&mut self) -> Result<()> {
        let Some(speech) = &self.speech else {
            return Ok(());
        };
        if speech.observation != self.observation_revision {
            return self.cancel_speech("observations changed before dispatch");
        }
        let now = self.elapsed_ms();
        if now < speech.due_ms || !self.conversation.speech_gate(now) {
            self.conversation.reason =
                "waiting for pacing, speech quiet or utterance window".into();
            return Ok(());
        }
        let speech = self.speech.as_mut().unwrap();
        if self.options.managed_floor && !speech.granted {
            if !speech.advertised {
                speech.advertised = true;
                self.log.record(
                    "speech-intent",
                    json!({"id":speech.id,"directed":speech.directed}),
                )?;
            }
            self.conversation.reason = "waiting for managed speaking floor".into();
            return Ok(());
        }
        let speech = self.speech.take().unwrap();
        self.apply(speech.decision).await?;
        self.conversation.spoke(now);
        self.log.record(
            "speech-dispatched",
            json!({"id":speech.id,"managed":self.options.managed_floor,
            "observation_watermark":speech.observation,"request_id":speech.request}),
        )?;
        if speech.advertised {
            self.log
                .record("speech-complete", json!({"id":speech.id,"outcome":"sent"}))?;
        }
        Ok(())
    }

    fn cancel_request(&mut self) -> Result<()> {
        if let Some(task) = self.model_task.take() {
            task.abort();
        }
        if let Some(id) = self.request_id.take() {
            self.costs
                .lock()
                .map_err(|_| "cost ledger lock poisoned")?
                .cancel(id)?;
            self.log
                .record("model-cancelled", json!({"request_id":id}))?;
            self.log.request = None;
        }
        Ok(())
    }

    fn cancel_model(&mut self) -> Result<()> {
        self.activity.cancel_unsent();
        if self.pending_playtest.take().is_some() {
            self.log
                .event("playtest-cancelled", "Review cancelled; usage retained.")?;
        }
        self.cancel_speech("runner cancelled pending reply")?;
        if let Some(probe) = self.pending_probe.take() {
            self.log.record(
                "probe-cancelled",
                json!({"probe_id":probe.id,"request_id":self.request_id}),
            )?;
        }
        self.staged = None;
        self.stage_next = false;
        self.asking = false;
        self.single_step = false;
        self.cancel_commander()?;
        self.cancel_request()
    }

    /// Abort the arena commander's call in flight and cancel its ledger
    /// request, as `cancel_request` does the goal loop's (cut AJ.3). The lane
    /// hears `Cancelled` and asks again when it next may.
    fn cancel_commander(&mut self) -> Result<()> {
        if let Some(task) = self.commander_task.take() {
            task.abort();
        }
        if let Some(id) = self.commander_request.take() {
            self.costs
                .lock()
                .map_err(|_| "cost ledger lock poisoned")?
                .cancel(id)?;
            self.commander_record(id, "model-cancelled", json!({"request_id":id}))?;
            let now = self.elapsed_ms();
            if let Some(lane) = self.arena.as_mut() {
                lane.commander_reply(now, crate::arena::CommanderReply::Cancelled);
                let events = lane.events();
                self.arena_events(events)?;
            }
        }
        Ok(())
    }

    fn pause(&mut self) -> Result<()> {
        self.activity.cancel_unsent();
        self.social_followup = false;
        self.social_started = None;
        self.state.status = "paused".into();
        self.cancel_model()
    }

    fn cost_report(&self) -> Result<crate::costs::Report> {
        self.costs
            .lock()
            .map_err(|_| "cost ledger lock poisoned")?
            .report()
    }

    fn budget_status(&self) -> Result<crate::budget::Status> {
        let usage = self.total_usage()?;
        let currency = self
            .selected_model
            .as_ref()
            .and_then(|m| m.settings.currency());
        self.budget.evaluate(&usage, currency.as_deref())
    }

    fn budget_record(&self, status: &crate::budget::Status) -> Value {
        json!({"settings":self.budget,"revision":self.budget.revision(),"status":status})
    }

    fn budget_report(&self) -> Result<Value> {
        Ok(self.budget_record(&self.budget_status()?))
    }

    fn remember(&mut self, label: &str, text: &str) {
        self.state.recent.push_str(&format!("\n[{label}] {text}\n"));
        self.state.recent = tail(&self.state.recent, self.config.agent.context_bytes).to_owned();
    }

    fn context(&self, question: Option<&str>) -> Result<Value> {
        let query = format!(
            "{} {} {} {}",
            self.state.role,
            self.state.goal,
            self.state.memory,
            tail(&self.state.recent, 4000)
        );
        let engagement = self.attention.query();
        let recall_query = question
            .or(self.recall_query.as_deref())
            .or((!engagement.is_empty()).then_some(engagement.as_str()))
            .unwrap_or(&self.state.goal);
        let mut context = json!({"runner_role":self.state.role, "runner_goal":self.state.goal,
            "character_profile": self.character,
            "group_affiliations": self.affiliations.as_ref().map(crate::organization::Affiliations::prompt_view),
            "social_context": self.memories.social(&self.config.ssh.username)?.context(recall_query,self.context_settings.social_bytes),
            "learning_context": self.memories.learning(&self.config.ssh.username)?.context(recall_query,self.context_settings.learning_bytes),
            "activity_context": self.activity.context(),
            "character_state": self.character_state,
            "recent_speech": self.attention.context(),
            "conversation_policy": {"max_utterance_chars":self.conversation.settings.max_utterance_chars,
                "guidance":"Reply only when engaged or when you have a useful contribution. Silence (done) is valid. Address an open question before changing topic. Read all available recent speech; do not repeat a reply already made. A speaking floor grants timing permission only, never knowledge or social authority."},
            "mission_context":{"active":self.mission.active,"contract":self.mission.spec,"remaining":self.mission.spec.as_ref().map(|s|s.max_actions.saturating_sub(self.mission.reservations.len()))},
            "decision_purpose": if question.is_some() {"runner_question"} else if self.social_turn {"conversation"} else {"goal"},
            "runner_instructions":self.state.instructions, "memory":self.state.memory,
            "terminal_history":self.state.recent, "reference_documents":self.knowledge.search(&query),
            "remembered_experience": self.memories.recall_with(recall_query,self.context_settings.recall_bytes,&self.context_settings.recall),
            "route_lookup": self.memories.route(recall_query),
            "belief_feedback": self.belief_feedback,
            "model_calls_remaining":self.config.agent.max_steps.saturating_sub(self.state.model_calls)});
        self.context_settings.apply(&mut context);
        Ok(context)
    }

    async fn drive(&mut self) -> Result<()> {
        let mut decoder = Decoder::default();
        let mut stdin = if self.options.headless || self.controls.is_some() {
            None
        } else {
            self.log.event("help", REPL_HELP)?;
            Some(stdin_lines())
        };
        let mut timer = tokio::time::interval(Duration::from_millis(50));
        while !self.quit {
            tokio::select! {
                biased;
                _ = tokio::signal::ctrl_c() => {
                    self.pause()?;
                    self.state.outcome = "Interrupted by runner".into();
                    self.quit = true;
                }
                input = async { self.controls.as_mut().unwrap().recv().await }, if self.controls.is_some() => {
                    if let Some(control) = input {
                        if let Err(error) = self.frontend_control(control).await { self.log.event("control-error", &error.to_string())?; }
                    } else { self.quit = true; }
                }
                input = async { stdin.as_mut().unwrap().recv().await }, if stdin.is_some() => {
                    if let Some(line) = input {
                        if let Err(error) = self.control(&line).await { self.log.event("control-error", &error.to_string())?; }
                    } else {
                        // Interactive EOF closes. Headless mode never reads stdin.
                        self.state.outcome = "Runner input closed".into();
                        self.quit = true;
                    }
                }
                // A continuously chatty game must not starve the run deadline.
                _ = timer.tick() => self.tick().await?,
                message = self.session.channel.wait() => {
                    match message {
                        Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                            let mut text = decoder.feed(&data);
                            if let Some(boundary) = &mut self.checkpoint && boundary.sent && !text.is_empty() {
                                self.log.record("checkpoint-wire",json!(text))?;
                                text = crate::checkpoint::fenced_output(&mut boundary.output,&mut boundary.received,&text,self.options.campaign_epoch.as_ref().unwrap())?;
                            }
                            if !text.is_empty() {
                                if self.log.echo { print!("{text}"); std::io::stdout().flush()?; }
                                self.observation_revision += 1;
                                self.cancel_speech("new game output invalidated the unsent reply")?;
                                // The arena pilot's polls and acks stay out of memory, attention
                                // and the recent window; the transcript keeps the raw text whole.
                                let now = self.elapsed_ms();
                                let seen = match (self.arena.as_mut(), self.mine.as_mut()) {
                                    (Some(lane), _) => {
                                        let journal = lane.feed(now, &text).journal;
                                        if journal.is_empty() { journal } else { journal + "\n" }
                                    }
                                    // Doc 20 cut SM.12c-bot: the mining lane reads it the same way.
                                    (None, Some(lane)) => {
                                        let journal = lane.feed(now, &text).journal;
                                        if journal.is_empty() { journal } else { journal + "\n" }
                                    }
                                    (None, None) => text.clone(),
                                };
                                let watch = if seen.is_empty() { None } else { self.debug.observation_match(&self.state.recent, &seen).map(str::to_owned) };
                                self.log.record("observation", json!(text))?;
                                if !seen.is_empty() {
                                    self.memories.observe(&seen)?;
                                    self.attention.feed(&seen);
                                    self.notice_attention()?;
                                    self.state.recent.push_str(&seen);
                                    self.state.recent = tail(&self.state.recent, self.config.agent.context_bytes).to_owned();
                                }
                                if let Some(pending) = &mut self.pending {
                                    if pending.text.len() + text.len() > 1_000_000 { return Err("single observation exceeded 1 MB".into()); }
                                    pending.text.push_str(&text);
                                    pending.last_output = Instant::now();
                                }
                                if let Some(watch) = watch {
                                    self.pause()?;
                                    self.log.event("watch", &format!("Observed {watch:?}; paused before further action."))?;
                                }
                            }
                        }
                        None | Some(ChannelMsg::Close) | Some(ChannelMsg::Eof) => return Err("SSH session closed before the runner finished".into()),
                        Some(ChannelMsg::Failure) => return Err("server rejected the SSH PTY or shell request".into()),
                        _ => {}
                    }
                }
                reply = async { self.commander_task.as_mut().unwrap().await }, if self.commander_task.is_some() => {
                    self.commander_task = None;
                    let request = self.commander_request.take();
                    self.commander_answer(request, reply.map_err(|e| -> crate::Error { e.into() }).and_then(|r| r))?;
                }
                reply = async { self.model_task.as_mut().unwrap().await }, if self.model_task.is_some() => {
                    self.model_task = None;
                    self.request_id = None;
                    let answer = self.asking;
                    self.asking = false;
                    let probe = self.pending_probe.take();
                    let playtest = self.pending_playtest.take();
                    match reply.map_err(|e| -> crate::Error { e.into() }).and_then(|r| r) {
                        Ok(reply) => {
                            self.log.record("model-reply", json!({"content":reply.content,"error":reply.error}))?;
                            self.log.record("usage", reply.usage)?;
                            self.log.record("cost-receipt", serde_json::to_value(&reply.receipt)?)?;
                            if let Some(error) = reply.error {
                                if let Some(probe) = &probe { self.log.record("probe-error",json!({"probe_id":probe.id,"reason":"provider_error"}))?; }
                                self.fail(&format!("Model request failed: {error}"))?;
                            }
                            else if let Some(job) = playtest {
                                let capture = self.last_capture.as_ref().ok_or("review response has no prepared request")?;
                                let model = self.selected_model.as_ref().ok_or("review model missing")?.settings.name.clone();
                                match job.complete(&reply.content, capture.request_revision.clone(), model) {
                                    Ok(entry) => {
                                        self.memories.save_playtest(entry.clone())?;
                                        self.playtest = crate::playtest::Archive::project(self.memories.records())?;
                                        self.log.record("playtest-result", serde_json::to_value(&entry)?)?;
                                        self.log.event("playtest", &serde_json::to_string_pretty(&entry.review)?)?;
                                        if job.finish_goal { self.state.status = "done".into(); }
                                    }
                                    Err(error) => self.fail(&format!("Invalid playtest review: {error}"))?,
                                }
                            }
                            else if let Some(probe) = probe {
                                let capture = self.last_capture.as_ref().ok_or("probe response has no prepared request")?;
                                let grade = crate::probe::grade(&probe,capture,&reply.content)?;
                                let value = serde_json::to_value(&grade)?;
                                // A bounded review history; the transcript retains every result.
                                if self.probe_results.len() == 128 {
                                    self.probe_results.pop_front();
                                    self.inherited_probe_results = self.inherited_probe_results.saturating_sub(1);
                                    self.probe_results_omitted += 1;
                                }
                                self.probe_results.push_back(grade);
                                self.log.record("probe-result",value.clone())?;
                                self.log.event("probe",&serde_json::to_string_pretty(&value)?)?;
                            }
                            else if answer { self.log.event("answer", &reply.content)?; }
                            else {
                                match Decision::parse(&reply.content) {
                                    Ok(decision) => self.receive_decision(decision).await?,
                                    Err(error) => self.fail(&format!("Invalid model decision: {error}"))?,
                                }
                            }
                        }
                        Err(error) => {
                            if let Some(probe) = &probe { self.log.record("probe-error",json!({"probe_id":probe.id,"reason":"request_error"}))?; }
                            self.fail(&format!("Model request failed: {error}"))?;
                        },
                    }
                }
            }
            if self.options.headless
                && matches!(self.state.status.as_str(), "done" | "blocked" | "error")
            {
                self.quit = true;
            }
        }
        Ok(())
    }

    // -- the simulator pilot (cut AJ.2) -----------------------------------------

    /// **The lane's pause follows the run's status** (cut AM.2, 16c Q-AM2):
    /// every way the status leaves `running` (`/pause`, `/send`, a bare line,
    /// `/goal`, `/role`, `/note`, a single `/step`, `/checkpoint`, …) pauses
    /// a flying lane, and every way it comes back (`/resume`, `/mission
    /// start`, …) resumes it, with no wiring in any control arm. The pause and
    /// resume sentences reach the transcript at once.
    fn arena_pause_sync(&mut self) -> Result<()> {
        let now = self.elapsed_ms();
        let running = self.state.status == "running";
        let Some(lane) = self.arena.as_mut() else {
            return Ok(());
        };
        match pause_action(lane.flying(), lane.stopped(), lane.paused(), running) {
            Some(PauseAction::Pause) => lane.pause(now),
            Some(PauseAction::Resume) => lane.resume(now),
            None => return Ok(()),
        }
        let events = lane.events();
        self.arena_events(events)
    }

    /// Whether this tick is the lane's ([`arena_due_for`]): it flies and the
    /// run is running, it flies paused (cut AO.2), its one `SIM LEAVE` is due,
    /// or its result is not yet written.
    fn arena_due(&self) -> bool {
        let now = self.elapsed_ms();
        self.arena.as_ref().is_some_and(|lane| {
            arena_due_for(
                lane.flying(),
                self.state.status == "running",
                lane.paused(),
                lane.stopped(),
                lane.leave_pending(now),
                !self.arena_reported && !lane.stopped() && lane.result().is_some(),
            )
        })
    }

    /// The lane is paused (cut AO.2): its tick never ends the runner's.
    fn arena_paused(&self) -> bool {
        self.arena.as_ref().is_some_and(|lane| lane.paused())
    }

    /// The lane's tick: its lines, its events, its picture record and its
    /// result. No speech, scenario, activity, social or goal model turn runs
    /// while it flies, except while it is paused (cut AO.2: then the runner's
    /// tick goes on past it). The commander asks nothing while paused
    /// ([`crate::arena::Lane::commander_ready`]). A blocker read while paused
    /// types its owed `SIM LEAVE` and writes its result while paused, as the
    /// lane already does.
    async fn arena_tick(&mut self) -> Result<()> {
        let now = self.elapsed_ms();
        let Some(lane) = self.arena.as_mut() else {
            return Ok(());
        };
        let sends = lane.tick(now);
        let events = lane.events();
        let result = (!self.arena_reported && !lane.stopped())
            .then(|| lane.result().cloned())
            .flatten();
        let running = matches!(
            lane.picture().phase,
            crate::arena::picture::Phase::Running { .. }
        );
        let picture = (running && now >= self.arena_picture_ms.saturating_add(5_000))
            .then(|| lane.picture_json());
        for send in sends {
            self.arena_send(send).await?;
        }
        self.arena_events(events)?;
        self.commander_tick()?;
        if let Some(picture) = picture {
            self.arena_picture_ms = now;
            self.log.record("arena-picture", picture)?;
        }
        if let Some(result) = result {
            self.arena_reported = true;
            self.state.status = result.status.clone();
            self.state.outcome = result.outcome.clone();
            self.log.event(&result.status, &result.outcome)?;
        }
        Ok(())
    }

    /// Type one lane line. Walk and lobby lines are echoed to the terminal;
    /// the cockpit's are only recorded. None of them is a memory.
    async fn arena_send(&mut self, send: crate::arena::Send) -> Result<()> {
        self.session.send(&send.line).await?;
        self.state.commands_sent += 1;
        self.log.record(
            "arena-send",
            json!({"line": send.line, "purpose": send.purpose, "lane_ms": send.lane_ms}),
        )?;
        if matches!(
            send.purpose,
            crate::arena::Purpose::Walk | crate::arena::Purpose::Lobby
        ) {
            self.log.event("send", &send.line)?;
        }
        Ok(())
    }

    fn arena_events(&mut self, events: Vec<crate::arena::LaneEvent>) -> Result<()> {
        for event in events {
            match event {
                crate::arena::LaneEvent::Said(text) => self.log.event("arena", &text)?,
                crate::arena::LaneEvent::State(record) => self
                    .log
                    .record("arena-state", serde_json::to_value(&record)?)?,
                // Each match has the whole run-time limit to itself.
                crate::arena::LaneEvent::Claimed => self.goal_started = Instant::now(),
                // The commander's `arena-doctrine`, `commander-stale`,
                // `commander-invalid`, `commander-error` and `commander-off`
                // (cut AJ.3), `commander-overruled` and `doctrine-lapsed`
                // (cut AK.2).
                crate::arena::LaneEvent::Record { kind, data } => self.log.record(&kind, data)?,
            }
        }
        Ok(())
    }

    /// At the run's end: stop the lane and let it type its one `SIM LEAVE`,
    /// waiting at most [`crate::arena::Lane::stop_bound_ms`] (cut AM.2: a
    /// claim or a start in flight is answered first, then 3 s for the
    /// leave's own answer).
    async fn arena_finish(&mut self) -> Result<()> {
        let Some(lane) = self.arena.as_mut() else {
            return Ok(());
        };
        lane.stop();
        let deadline = Instant::now() + Duration::from_millis(lane.stop_bound_ms());
        let mut decoder = Decoder::default();
        loop {
            let now = self.elapsed_ms();
            let (sends, events) = match self.arena.as_mut() {
                Some(lane) => (lane.tick(now), lane.events()),
                None => return Ok(()),
            };
            for send in sends {
                self.arena_send(send).await?;
            }
            self.arena_events(events)?;
            let now = self.elapsed_ms();
            if !self
                .arena
                .as_ref()
                .is_some_and(|lane| lane.leave_pending(now))
                || Instant::now() >= deadline
            {
                return Ok(());
            }
            match tokio::time::timeout(Duration::from_millis(50), self.session.channel.wait()).await
            {
                Ok(Some(ChannelMsg::Data { data }))
                | Ok(Some(ChannelMsg::ExtendedData { data, .. })) => {
                    let text = decoder.feed(&data);
                    if !text.is_empty() {
                        self.log.record("observation", json!(text))?;
                        let now = self.elapsed_ms();
                        if let Some(lane) = self.arena.as_mut() {
                            lane.feed(now, &text);
                        }
                    }
                }
                Ok(None) | Ok(Some(ChannelMsg::Close)) | Ok(Some(ChannelMsg::Eof)) => {
                    return Ok(());
                }
                _ => {}
            }
        }
    }

    /// `/arena red|blue [n]`, `/arena stop`, bare `/arena`.
    fn arena_control(&mut self, arg: &str) -> Result<()> {
        let words: Vec<&str> = arg.split_whitespace().collect();
        match words.as_slice() {
            [] => {
                let lane = self
                    .arena
                    .as_ref()
                    .ok_or("no arena pilot yet; /arena red or /arena blue starts one")?;
                let lines = lane.picture().summary_lines(self.elapsed_ms());
                self.log.event("arena", &lines.join("\n"))
            }
            ["stop"] => {
                let lane = self.arena.as_mut().ok_or("no arena pilot is flying")?;
                lane.stop();
                let events = lane.events();
                self.arena_events(events)?;
                self.pause()?;
                self.log
                    .event("arena", "Arena pilot stopped; the tank is yours.")
            }
            [side, rest @ ..] if rest.len() <= 1 => {
                let usage =
                    "/arena takes red or blue, then an optional number of matches from 1 to 100";
                let side = crate::arena::Side::parse(side).ok_or(usage)?;
                let matches = match rest.first() {
                    Some(n) => n
                        .parse::<u32>()
                        .ok()
                        .filter(|n| (1..=100).contains(n))
                        .ok_or(usage)?,
                    None => self.config.arena.matches,
                };
                if self.mission.spec.is_some() {
                    return Err(ARENA_MISSION.into());
                }
                if self.scenario.is_some()
                    || self.options.campaign_epoch.is_some()
                    || self.options.managed_floor
                {
                    return Err(ARENA_SESSION.into());
                }
                if self.arena.as_ref().is_some_and(crate::arena::Lane::flying) {
                    return Err(ARENA_FLYING.into());
                }
                self.cancel_model()?;
                self.arena_start(side, matches)?;
                self.state.status = "running".into();
                self.state.outcome.clear();
                self.goal_started = Instant::now();
                Ok(())
            }
            _ => Err("use /arena, /arena red|blue [n] or /arena stop".into()),
        }
    }

    // -- the mining pilot (doc 20 cut SM.12c-bot) --------------------------------

    /// Start the mining lane for `site` (`--mine`): the hull's name from the
    /// SSH username, the epoch of the lane's clock in unix milliseconds, and
    /// the character's name as [`arena_me`] reads it.
    fn mine_start(&mut self, site: crate::mine::Site) -> Result<()> {
        let hull = crate::mine::hull_name(&self.config.ssh.username)?;
        let epoch_unix_ms = unix_now_ms().saturating_sub(self.elapsed_ms());
        let me = arena_me(&self.config.ssh.username, &self.character);
        let mut lane = crate::mine::Lane::new(
            site,
            crate::mine::Settings {
                hull,
                epoch_unix_ms,
            },
            me,
        );
        let events = lane.events();
        self.mine = Some(lane);
        self.mine_reported = false;
        self.mine_events(events)
    }

    /// Whether this tick is the mining lane's: it plays and the run is
    /// running with no result yet, or its result is not yet written.
    fn mine_due(&self) -> bool {
        self.mine.as_ref().is_some_and(|lane| {
            mine_due_for(
                self.state.status == "running",
                lane.result().is_some(),
                self.mine_reported,
            )
        })
    }

    /// The mining lane's tick: its lines, its events and its result.
    async fn mine_tick(&mut self) -> Result<()> {
        let now = self.elapsed_ms();
        let Some(lane) = self.mine.as_mut() else {
            return Ok(());
        };
        let sends = lane.tick(now);
        let events = lane.events();
        let result = (!self.mine_reported)
            .then(|| lane.result().cloned())
            .flatten();
        for send in sends {
            self.session.send(&send.line).await?;
            self.state.commands_sent += 1;
            self.log.record(
                "mine-send",
                json!({"line": send.line, "beat": send.beat, "lane_ms": send.lane_ms}),
            )?;
            self.log.event("send", &send.line)?;
        }
        self.mine_events(events)?;
        if let Some(result) = result {
            self.mine_reported = true;
            self.state.status = result.status.clone();
            self.state.outcome = result.outcome.clone();
            self.log.event(&result.status, &result.outcome)?;
        }
        Ok(())
    }

    /// The mining lane's events as transcript records: `mine` sentences,
    /// `mine-beat`, `mine-milestone` and `mine-heard`.
    fn mine_events(&mut self, events: Vec<crate::mine::LaneEvent>) -> Result<()> {
        let epoch = self
            .mine
            .as_ref()
            .map_or(0, |lane| lane.settings().epoch_unix_ms);
        for event in events {
            match event {
                crate::mine::LaneEvent::Said(text) => self.log.event("mine", &text)?,
                crate::mine::LaneEvent::Beat {
                    beat,
                    edge,
                    lane_ms,
                } => self.log.record(
                    "mine-beat",
                    json!({"beat": beat, "edge": edge, "lane_ms": lane_ms,
                        "at_unix_ms": epoch.saturating_add(lane_ms)}),
                )?,
                crate::mine::LaneEvent::Milestone { name, lane_ms } => self.log.record(
                    "mine-milestone",
                    json!({"name": name, "lane_ms": lane_ms,
                        "at_unix_ms": epoch.saturating_add(lane_ms)}),
                )?,
                crate::mine::LaneEvent::Heard { sentence, lane_ms } => self.log.record(
                    "mine-heard",
                    json!({"sentence": sentence, "lane_ms": lane_ms,
                        "at_unix_ms": epoch.saturating_add(lane_ms)}),
                )?,
            }
        }
        Ok(())
    }

    /// Start a lane for `side` and `matches` (`--arena`, `/arena`, the goal
    /// loop's `arena` action): its first sentence, then the commander's.
    fn arena_start(&mut self, side: crate::arena::Side, matches: u32) -> Result<()> {
        let me = arena_me(&self.config.ssh.username, &self.character);
        let mut lane = crate::arena::Lane::new(
            side,
            crate::arena::settings::Settings {
                matches,
                commander: self.commander.is_some(),
                ..self.config.arena.clone()
            },
            me,
        );
        let events = lane.events();
        self.arena = Some(lane);
        self.arena_reported = false;
        self.arena_picture_ms = 0;
        self.arena_events(events)?;
        self.log.event(
            "arena",
            &crate::arena::commander::start_line(self.commander_model.as_ref()),
        )?;
        // A budget block holds for the session: a later lane's commander is
        // off from its start (its `commander-off`, no second `budget-blocked`).
        if self.commander_budget_off
            && self.commander.is_some()
            && let Some(lane) = self.arena.as_mut()
        {
            lane.commander_off(crate::arena::commander::OFF_BUDGET);
            let events = lane.events();
            self.arena_events(events)?;
        }
        Ok(())
    }

    /// **The commander's call** (cut AJ.3), when the lane has an ask due and
    /// none is in flight. A call the lane no longer waits for is aborted
    /// first. The budget is checked in the commander model's own currency; a
    /// block is recorded once and switches the commander off for the lane.
    /// Commander calls are not goal turns: they never count against
    /// `agent.max_steps` or `state.model_calls`, and the cost ledger files them
    /// under `arena_commander`.
    fn commander_tick(&mut self) -> Result<()> {
        if self.commander_task.is_some()
            && !self
                .arena
                .as_ref()
                .is_some_and(crate::arena::Lane::commander_in_flight)
        {
            self.cancel_commander()?;
        }
        if self.commander_task.is_some() {
            return Ok(());
        }
        let now = self.elapsed_ms();
        let (Some(brain), Some(resolved), Some(lane)) = (
            self.commander.as_ref(),
            self.commander_model.as_ref(),
            self.arena.as_mut(),
        ) else {
            return Ok(());
        };
        // The budget before the ask is taken, so a refused ask is never
        // counted as a call.
        if !lane.commander_ready(now) {
            let events = lane.events();
            return self.arena_events(events);
        }
        let mut brain = brain.clone();
        let selection = resolved.selection.clone();
        let status = crate::arena::commander::budget_status(
            &self.budget,
            &self.total_usage()?,
            &selection.settings,
        )?;
        if !status.allowed {
            self.log
                .record("budget-blocked", self.budget_record(&status))?;
            self.commander_budget_off = true;
            if let Some(lane) = self.arena.as_mut() {
                lane.commander_off(crate::arena::commander::OFF_BUDGET);
                let events = lane.events();
                self.arena_events(events)?;
            }
            return Ok(());
        }
        let Some(lane) = self.arena.as_mut() else {
            return Ok(());
        };
        let Some(view) = lane.commander_due(now) else {
            let events = lane.events();
            return self.arena_events(events);
        };
        let capture = match crate::arena::commander::capture(&selection.settings, &view) {
            Ok(capture) => capture,
            Err(error) => {
                // Nothing was sent: an error with its cause
                // (`commander-error`), not a call.
                if let Some(lane) = self.arena.as_mut() {
                    lane.commander_reply(
                        now,
                        crate::arena::CommanderReply::Unsent(error.to_string()),
                    );
                    let events = lane.events();
                    self.arena_events(events)?;
                }
                return Ok(());
            }
        };
        let request = crate::arena::commander::admit(&self.costs, selection, &capture)?;
        brain.reserve_tape()?;
        self.commander_record(request.id, "model-input", serde_json::to_value(&capture)?)?;
        self.commander_record(request.id, "model-request", serde_json::to_value(&request)?)?;
        self.commander_request = Some(request.id);
        self.commander_task = Some(tokio::spawn(crate::arena::commander::call(
            brain,
            capture,
            self.costs.clone(),
            request.id,
        )));
        if let Some(lane) = self.arena.as_mut() {
            let events = lane.events();
            self.arena_events(events)?;
        }
        Ok(())
    }

    /// The commander's call came back: its records as the goal loop's are
    /// written, under its own request's envelope, then the lane hears it (a
    /// failure's cause reaches the transcript as `commander-error`).
    fn commander_answer(&mut self, request: Option<u64>, reply: Result<Reply>) -> Result<()> {
        let reply = match reply {
            Ok(reply) => {
                if let Some(id) = request {
                    self.commander_record(
                        id,
                        "model-reply",
                        json!({"content":reply.content,"error":reply.error}),
                    )?;
                    self.commander_record(id, "usage", reply.usage)?;
                    self.commander_record(
                        id,
                        "cost-receipt",
                        serde_json::to_value(&reply.receipt)?,
                    )?;
                }
                match reply.error {
                    Some(error) => crate::arena::CommanderReply::Error(error),
                    None => crate::arena::CommanderReply::Answer(reply.content),
                }
            }
            Err(error) => crate::arena::CommanderReply::Error(error.to_string()),
        };
        let now = self.elapsed_ms();
        if let Some(lane) = self.arena.as_mut() {
            lane.commander_reply(now, reply);
            let events = lane.events();
            self.arena_events(events)?;
        }
        Ok(())
    }

    /// A commander model record (`model-input`, `model-request`,
    /// `model-reply`, `usage`, `cost-receipt`, `model-cancelled`) under its own
    /// request's envelope, as `start_model` stamps the goal loop's; the goal
    /// loop's `log.request` is put back after, so a trace never files a
    /// commander call under a goal decision, nor a goal record under a
    /// commander call.
    fn commander_record(&mut self, request: u64, kind: &str, data: Value) -> Result<()> {
        let goal = self.log.request.replace(request);
        let result = self.log.record(kind, data);
        self.log.request = goal;
        result
    }

    fn fail(&mut self, message: &str) -> Result<()> {
        self.pause()?;
        self.state.status = "blocked".into();
        self.state.outcome = message.to_owned();
        // A stopped lane still types its one `SIM LEAVE` when it is due.
        if let Some(lane) = self.arena.as_mut() {
            lane.stop();
        }
        self.log.event("blocked", message)
    }

    fn notice_attention(&mut self) -> Result<()> {
        let now = self.elapsed_ms();
        let speech_changed = self.conversation.observe(self.attention.records(), now);
        if speech_changed && self.social_turn && self.model_task.is_some() && !self.asking {
            self.cancel_request()?;
            self.next_turn =
                Instant::now() + Duration::from_millis(self.conversation.settings.debounce_ms);
            self.log.event("attention", "New speech cancelled a social decision; collecting the exchange before reconsidering.")?;
        }
        let revision = self.attention.revision();
        if !self.attention_enabled
            || self.scenario.is_some()
            || self.asking
            || (self.state.status == "paused" && !self.single_step)
            || matches!(self.state.status.as_str(), "blocked" | "error")
            || revision <= self.noticed_attention
        {
            return Ok(());
        }
        self.noticed_attention = revision;
        if self.model_task.is_some() && revision > self.model_snapshot {
            // Preserve the last completed memory/decision and /step state. A
            // new addressed utterance must be seen before an unsent action acts.
            self.cancel_request()?;
            self.log.event("attention", "New engagement arrived; refreshing the pending decision with current speech and memory.")?;
        }
        self.next_turn = Instant::now() + Duration::from_millis(self.config.attention.cooldown_ms);
        Ok(())
    }

    async fn send(
        &mut self,
        command: Option<String>,
        scenario: Option<ScenarioStep>,
    ) -> Result<()> {
        if let Some(command) = &command {
            if command.starts_with("__campaign_") {
                return Err(
                    "campaign protocol commands are reserved for coordinator quiescence".into(),
                );
            }
            let reserved = self.mission.reserve(command)?;
            if reserved {
                self.memories.save_mission(&self.mission)?;
            }
            self.session.send(command).await?;
            self.memories.sent(command)?;
            let sent_id = self
                .memories
                .records()
                .last()
                .ok_or("sent journal missing")?
                .id;
            if reserved {
                self.mission.sent.push(sent_id);
                self.memories.save_mission(&self.mission)?;
            }
            if self
                .activity
                .pending
                .as_ref()
                .is_some_and(|p| p.sent.is_none() && &p.command == command)
            {
                let id = sent_id;
                self.activity.dispatched(id);
                self.memories.save_activity(&self.activity)?;
            }
            self.state.commands_sent += 1;
            self.log.event("send", command)?;
            self.log.command_sequence = Some(self.log.sequence);
            self.remember("command", command);
        }
        let now = Instant::now();
        let timeout = scenario
            .as_ref()
            .map_or(self.config.agent.observation_ms, |s| s.timeout_ms);
        self.pending = Some(Pending {
            command,
            text: String::new(),
            deadline: now + Duration::from_millis(timeout),
            last_output: now,
            scenario,
        });
        Ok(())
    }

    async fn tick(&mut self) -> Result<()> {
        self.arena_pause_sync()?;
        let activity_now = Instant::now();
        let delta = activity_now.duration_since(self.activity_tick).as_millis() as u64;
        self.activity_tick = activity_now;
        if self.checkpoint.is_none()
            && self
                .activity
                .plan
                .as_ref()
                .is_some_and(|p| p.clock == crate::activity::Clock::SessionMonotonic)
        {
            self.activity.advance(delta)?;
        }
        if (self.log.events.is_some() || self.options.json_events)
            && self.last_snapshot.elapsed() >= Duration::from_millis(500)
        {
            self.log.record("snapshot", self.snapshot()?)?;
            self.last_snapshot = Instant::now();
        }
        self.memories.flush_if_quiet(self.config.agent.settle_ms)?;
        self.attention.flush_if_quiet(self.config.agent.settle_ms);
        self.notice_attention()?;
        let now = Instant::now();
        if (self.state.status == "running"
            || (self.single_step && !self.state.goal.trim().is_empty())
            || self.scenario.is_some())
            && self.goal_started.elapsed() >= Duration::from_secs(self.config.agent.max_seconds)
        {
            return self.fail("Run time limit reached");
        }
        if self.checkpoint.is_none()
            && self.social_started.is_some_and(|at| {
                at.elapsed() >= Duration::from_secs(self.config.agent.max_seconds)
            })
        {
            return self.fail("Conversation time limit reached");
        }
        if let Some(pending) = &self.pending {
            let quiet = now.duration_since(pending.last_output)
                >= Duration::from_millis(self.config.agent.settle_ms);
            if let Some(step) = &pending.scenario {
                let response = pending.response();
                if let Some(forbidden) = step.reject.iter().find(|s| response.contains(s.as_str()))
                {
                    return Err(format!(
                        "scenario step {} rejected output {forbidden:?}",
                        self.state.scenario_steps_passed + 1
                    )
                    .into());
                }
                let matched = step.expect.iter().all(|s| response.contains(s));
                if !matched {
                    if now >= pending.deadline {
                        return Err(format!(
                            "scenario step {} timed out waiting for {:?}; observed: {}",
                            self.state.scenario_steps_passed + 1,
                            step.expect,
                            tail(&response, 4000)
                        )
                        .into());
                    }
                    return Ok(());
                }
                if !quiet && now < pending.deadline {
                    return Ok(());
                }
            } else if (!quiet || pending.text.is_empty()) && now < pending.deadline {
                // **A pending command never holds the pilot** (cut AM.2): the
                // lane's own polls keep the output from going quiet, so a
                // due lane ticks now and the command stays pending, finished
                // at its own quiet or deadline on a later tick.
                if self.arena_due() {
                    return self.arena_tick().await;
                }
                // Doc 20 cut SM.12c-bot: nor the mining pilot.
                if self.mine_due() {
                    return self.mine_tick().await;
                }
                return Ok(());
            }
            let pending = self.pending.take().unwrap();
            self.log.record(
                "command-result",
                json!({"command":pending.command,"output":pending.response()}),
            )?;
            self.log.command_sequence = None;
            if self.await_observation {
                self.await_observation = false;
                self.log.event(
                    "observation-step",
                    "Current output settled; the bot remains paused.",
                )?;
            }
            self.memories.finish_command()?;
            if self.activity.reconcile(self.memories.records(), true) {
                self.memories.save_activity(&self.activity)?;
                self.log
                    .record("activity", serde_json::to_value(&self.activity)?)?;
            }
            if let Some(command) = &pending.command
                && (command == "help" || command.starts_with("help "))
            {
                self.knowledge
                    .add(&format!("live {command}"), &pending.response())?;
            }
            if pending.scenario.is_some() {
                self.state.scenario_steps_passed += 1;
                self.log.event(
                    "assert",
                    &format!("step {} passed", self.state.scenario_steps_passed),
                )?;
            }
        }
        if let Some(command) = self.bootstrap.pop_front() {
            return self.send(Some(command), None).await;
        }
        if !self.ready {
            self.ready = true;
            self.log.record("ready", self.snapshot()?)?;
        }
        if self.checkpoint_tick().await? {
            return Ok(());
        }
        // **A paused lane's tick never ends the runner's** (cut AO.2): it
        // reads and settles, types nothing, and the tick goes on exactly as
        // before (a `/step` still takes its one goal turn, a pending speech or
        // scenario arm still runs); a running lane's tick is the whole tick.
        if self.arena_due() {
            let ends = arena_tick_ends_tick(self.arena_paused());
            self.arena_tick().await?;
            if ends {
                return Ok(());
            }
        }
        // Doc 20 cut SM.12c-bot: a playing mining lane's tick is the whole tick.
        if self.mine_due() {
            self.mine_tick().await?;
            return Ok(());
        }
        if self.speech.is_some() {
            self.dispatch_speech().await?;
            return Ok(());
        }
        if let Some(scenario) = &mut self.scenario {
            if let Some(step) = scenario.pop_front() {
                if self.state.scenario_steps_passed >= self.config.agent.max_steps {
                    return self.fail("Scenario step limit reached");
                }
                return self.send(step.command.clone(), Some(step)).await;
            }
            self.state.status = "done".into();
            self.state.outcome = "All scenario assertions passed".into();
            return self.log.event("done", &self.state.outcome);
        }
        if self.state.status == "listening"
            && self.model_task.is_none()
            && self.staged.is_none()
            && !self.attention.has_pending()
            && !self.social_followup
            && self.conversation.settings.mode != crate::conversation::Mode::Spectator
        {
            // A speech intent may have been invalidated by ambient output.
            self.activity.cancel_unsent();
            let before = (
                self.activity.decisions,
                self.activity.suspended,
                self.activity.active.is_some(),
            );
            let command = self.activity.choose(&self.memories);
            if command.is_some()
                || before
                    != (
                        self.activity.decisions,
                        self.activity.suspended,
                        self.activity.active.is_some(),
                    )
            {
                self.memories.save_activity(&self.activity)?;
                self.log
                    .record("activity", serde_json::to_value(&self.activity)?)?;
            }
            if let Some(command) = command {
                let decision = Decision {
                    action: Action::Command,
                    command: Some(command),
                    summary: "Performing the next scheduled activity step.".into(),
                    memory: None,
                    wait_ms: 0,
                    query: None,
                    side: None,
                    beliefs: None,
                    social: None,
                    learning: None,
                    navigation: None,
                };
                return self.receive_decision(decision).await;
            }
        }
        if self.model_task.is_none()
            && now >= self.next_turn
            && (self.state.status == "running"
                || self.single_step
                || (self.attention_enabled
                    && self.brain.is_some()
                    && (self.state.status == "listening"
                        || (self.state.status == "done"
                            && self.config.attention.listen_when_idle))
                    && (self.attention.has_pending() || self.social_followup)))
        {
            if self.state.goal.trim().is_empty()
                && !self.attention.has_pending()
                && !self.social_followup
            {
                return self.fail("Set a goal with /goal before starting");
            }
            if self.conversation.settings.mode == crate::conversation::Mode::Spectator {
                if self.state.status == "running" || self.single_step {
                    return self.fail("Spectator mode does not run autonomous goals; use local /send to position the observer");
                }
                self.conversation.reason = "spectator; observing without model calls".into();
            } else if self.state.status == "running"
                && !self.single_step
                && !self.conversation.burst_ready(self.elapsed_ms())
            {
                self.conversation.reason =
                    "collecting speech burst before the next goal decision".into();
            } else if self.state.status == "running"
                || self.single_step
                || self.conversation.model_gate(self.elapsed_ms())
            {
                self.start_model(None)?;
            }
        }
        Ok(())
    }

    fn start_model(&mut self, question: Option<String>) -> Result<()> {
        if let Some(quota) = self.options.campaign_quota
            && self
                .costs
                .lock()
                .map_err(|_| "cost ledger lock poisoned")?
                .session_usage()?
                .requests
                >= quota
        {
            return self.fail("Campaign reserved request allowance exhausted");
        }
        // Session accounting is independent of the current goal's resettable
        // turn counter. A denied attempt never creates a ledger request/capture.
        let budget = self.budget_status()?;
        if !budget.allowed {
            self.log
                .record("budget-blocked", self.budget_record(&budget))?;
            return self.fail(&budget.message());
        }
        if self.state.model_calls >= self.config.agent.max_steps {
            return self.fail("Model turn limit reached");
        }
        let Some(mut brain) = self.brain.clone() else {
            return self.fail("Configure [model] to enable autonomous play and /ask");
        };
        self.memories.flush()?;
        self.social_turn = question.is_none()
            && self.state.status != "running"
            && (!self.single_step || self.state.goal.trim().is_empty());
        if self.social_turn && self.social_started.is_none() {
            self.social_started = Some(Instant::now());
        }
        self.model_snapshot = self.attention.snapshot();
        let context = self.context(
            self.pending_probe
                .as_ref()
                .map(|p| p.question.as_str())
                .or(question.as_deref()),
        )?;
        let capture = match brain.prepare(context, question, &self.prompts) {
            Ok(capture) => capture,
            Err(error) => return self.fail(&format!("Prompt preparation failed: {error}")),
        };
        self.recall_query = None;
        self.belief_feedback.clear();
        self.asking = capture.question;
        let prompt_type = if let Some(job) = &self.pending_playtest {
            if job.docket.is_some() {
                PromptType::PlaytestRefutation
            } else {
                PromptType::PlaytestFeedback
            }
        } else if self.pending_probe.is_some() {
            PromptType::StructuredProbe
        } else if self.asking {
            PromptType::RunnerQuestion
        } else if self.social_turn {
            PromptType::Conversation
        } else {
            PromptType::GoalDecision
        };
        let request = self
            .costs
            .lock()
            .map_err(|_| "cost ledger lock poisoned")?
            .start(
                self.selected_model
                    .clone()
                    .ok_or("no active model profile")?,
                prompt_type,
                capture.prompt_revision.clone(),
            )?;
        self.log.request = Some(request.id);
        brain.reserve_tape()?;
        self.log
            .record("budget-check", self.budget_record(&budget))?;
        self.log
            .record("model-input", serde_json::to_value(&capture)?)?;
        self.last_capture = Some(capture.clone());
        self.capture_observation = Some(self.observation_revision);
        self.log
            .record("model-request", serde_json::to_value(&request)?)?;
        self.request_id = Some(request.id);
        self.state.model_calls += 1;
        if self.social_turn {
            self.conversation.requested(self.elapsed_ms());
        }
        self.log.event(
            "model",
            if let Some(job) = &self.pending_playtest {
                if job.docket.is_some() {
                    "Challenging playtest claims…"
                } else {
                    "Reviewing goal usability…"
                }
            } else if self.pending_probe.is_some() {
                "Answering structured probe…"
            } else if self.asking {
                "Answering runner question…"
            } else {
                "Choosing next action…"
            },
        )?;
        let costs = self.costs.clone();
        self.model_task = Some(tokio::spawn(async move {
            let started = Instant::now();
            let result = brain.dispatch(&capture).await;
            let (outcome, receipt) = match &result {
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
            // Synchronous receipt persistence before delivering the decision
            // prevents a simultaneous pause from losing an already-known cost.
            costs
                .lock()
                .map_err(|_| "cost ledger lock poisoned")?
                .finish(
                    request.id,
                    outcome,
                    receipt,
                    started.elapsed().as_millis() as u64,
                )?;
            result
        }));
        Ok(())
    }

    async fn apply(&mut self, decision: Decision) -> Result<()> {
        let routine_action = self.activity.pending.as_ref().is_some_and(|p| {
            p.sent.is_none() && decision.command.as_deref() == Some(p.command.as_str())
        });
        self.capture_observation = None;
        if !routine_action && self.character_state.advance() {
            self.log
                .record("character-state-expired", json!(self.character_state))?;
        }
        self.log
            .record("decision", serde_json::to_value(&decision)?)?;
        self.log.event("summary", &decision.summary)?;
        self.memories.flush()?;
        // A deterministic step is not a model/social turn. It must not inherit
        // a prior exchange's follow-up state, acknowledge speech or age a mood.
        if routine_action {
            if self.single_step {
                self.state.status = "paused".into();
                self.single_step = false;
            }
            return self.send(decision.command, None).await;
        }
        self.belief_feedback = self.memories.annotate(decision.beliefs)?;
        self.belief_feedback.extend(self.memories.annotate_social(
            decision.social,
            &self.config.ssh.username,
            &self.conversation.settings.scope,
        )?);
        self.belief_feedback.extend(
            self.memories
                .annotate_learning(decision.learning, &self.config.ssh.username)?,
        );
        self.belief_feedback
            .extend(self.memories.annotate_navigation(decision.navigation)?);
        for feedback in &self.belief_feedback {
            self.log.event("memory-feedback", feedback)?;
        }
        if let Some(memory) = decision.memory {
            self.memories.notes(&memory)?;
            self.state.memory = memory;
        }
        let spoke = decision.command.as_deref().is_some_and(is_communication);
        self.social_followup = self.social_turn
            && (decision.action == Action::Recall
                || (decision.action == Action::Command && !spoke));
        // Keep the question pending through a social recall/inspection chain;
        // the eventual answer still needs its retrieval query and partner label.
        if decision.action != Action::Recall && !self.social_followup {
            self.attention.acknowledge(self.model_snapshot, spoke);
            self.conversation.handled(self.model_snapshot);
        }
        if !self.social_followup {
            self.social_started = None;
        }
        if self.single_step {
            self.state.status = "paused".into();
            self.single_step = false;
            self.social_started = None;
        }
        match decision.action {
            Action::Command => self.send(decision.command, None).await?,
            Action::Wait => {
                self.conversation.reason = "waiting for further engagement".into();
                self.next_turn = Instant::now() + Duration::from_millis(decision.wait_ms)
            }
            Action::Recall => {
                self.conversation.reason = "retrieving evidence for the next decision".into();
                self.recall_query = decision.query;
                self.next_turn = Instant::now();
                self.log.event(
                    "recall",
                    &serde_json::to_string_pretty(&self.memories.recall_with(
                        self.recall_query.as_deref().unwrap(),
                        self.context_settings.recall_bytes,
                        &self.context_settings.recall,
                    ))?,
                )?;
            }
            Action::Arena => {
                // A social turn never starts the lane: declined, as `done` is.
                if self.social_turn {
                    self.conversation.declined = true;
                    self.conversation.reason = decision.summary.clone();
                    self.log.event("conversation", &decision.summary)?;
                    return Ok(());
                }
                if self.mission.spec.is_some() {
                    return self.fail(ARENA_MISSION);
                }
                if self.options.campaign_epoch.is_some() || self.options.managed_floor {
                    return self.fail(ARENA_SESSION);
                }
                let side = decision
                    .side
                    .as_deref()
                    .and_then(crate::arena::Side::parse)
                    .ok_or("arena action requires side red or blue")?;
                // A lane still flying (reached through `/step`) or still
                // typing its `SIM LEAVE` is never replaced: its slot, its
                // report and its commander are its own. Declined, as `/arena`
                // refuses it; the lane flies on.
                let now = self.elapsed_ms();
                if self
                    .arena
                    .as_ref()
                    .is_some_and(|lane| lane.flying() || lane.leave_pending(now))
                {
                    return self.log.event("arena", ARENA_FLYING);
                }
                let matches = self.options.matches.unwrap_or(self.config.arena.matches);
                // The goal stays running; the lane's result ends it as
                // `--arena`'s does.
                self.arena_start(side, matches)?;
            }
            Action::Done | Action::Blocked => {
                if self.social_turn {
                    self.conversation.declined = true;
                    self.conversation.reason = decision.summary.clone();
                    self.log.event("conversation", &decision.summary)?;
                    return Ok(());
                }
                self.state.status = if decision.action == Action::Done {
                    "done"
                } else {
                    "blocked"
                }
                .into();
                self.state.outcome = decision.summary;
                self.log.event(&self.state.status, &self.state.outcome)?;
                if decision.action == Action::Done && self.config.playtest_feedback {
                    self.begin_playtest(None, true)?;
                }
            }
        }
        Ok(())
    }

    fn begin_playtest(
        &mut self,
        docket: Option<crate::playtest::Docket>,
        finish_goal: bool,
    ) -> Result<()> {
        if self.pending.is_some() {
            return Err("wait for the current game command to settle before reviewing".into());
        }
        if self.playtest.entries.len() >= 128 {
            return Err("playtest archive full".into());
        }
        self.pause()?;
        self.memories.flush()?;
        let job = crate::playtest::Job::new(
            self.config.ssh.username.clone(),
            if self.state.goal.trim().is_empty() {
                "Review the observed session".into()
            } else {
                self.state.goal.clone()
            },
            self.memories.records(),
            docket,
            finish_goal,
        );
        let question = job.prompt()?;
        self.pending_playtest = Some(job);
        self.start_model(Some(question))
    }

    async fn control(&mut self, line: &str) -> Result<()> {
        // Harness fences carry a runner-owned ID. A started event marks the
        // boundary before any effects, and applied acknowledges success.
        if let Some(rest) = line.strip_prefix("/harness ") {
            let (id, inner) = rest
                .split_once(' ')
                .ok_or("harness control needs ID and command")?;
            if !crate::organization::identifier(id) || inner.starts_with("/harness") {
                return Err("invalid harness control ID/nesting".into());
            }
            self.log.record("control-started", json!({"id":id}))?;
            self.control_inner(inner).await?;
            self.log.record("control-applied", json!({"id":id}))?;
            return Ok(());
        }
        self.control_inner(line).await
    }

    async fn control_inner(&mut self, line: &str) -> Result<()> {
        let line = line.trim();
        if line.is_empty() {
            return Ok(());
        }
        if line.len() > 16_000 || line.chars().any(char::is_control) {
            return Err("control input exceeds 16000 bytes or contains control characters".into());
        }
        let (verb, arg) = line
            .split_once(char::is_whitespace)
            .map_or((line, ""), |(v, a)| (v, a.trim()));
        if self.checkpoint.is_some() && !["/quit", "/state", "/cost"].contains(&verb) {
            return Err(
                "checkpoint ingress is fenced; resume in a new isolated campaign branch".into(),
            );
        }
        // Validate before recording input, so in-band credentials never reach logs.
        if verb == "/send" {
            validate_command(arg)?;
        } else if !verb.starts_with('/') {
            validate_command(line)?;
        }
        self.memories.flush()?;
        self.log.record("control", json!(line))?;
        match verb {
            "/checkpoint" => {
                if self.options.campaign_epoch.is_none() || arg.is_empty() {
                    return Err(
                        "checkpoint requires an owned campaign adapter and a new JSON path".into(),
                    );
                }
                let resume_status = self
                    .resume_status
                    .take()
                    .unwrap_or_else(|| self.state.status.clone());
                self.cancel_model()?;
                self.state.status = "paused".into();
                self.checkpoint = Some(Boundary {
                    path: arg.into(),
                    resume_status,
                    sent: false,
                    received: false,
                    output: String::new(),
                    deadline: Instant::now() + Duration::from_secs(30),
                    saved: false,
                });
                self.log.event(
                    "quiescing",
                    "Invalidated unsent work; draining SSH before the common world fence.",
                )?;
            }
            "/continue" => {
                self.state.status = self
                    .resume_status
                    .take()
                    .ok_or("no restored execution status; use /resume")?;
                self.log.event(
                    "continued",
                    "Restored unfinished work resumed with retained timers and usage.",
                )?;
            }
            "/tape" if self.options.campaign_epoch.is_some() => {
                let source = crate::tape::Source::load(std::path::Path::new(arg))?;
                self.pause()?;
                self.tape = Some(Arc::new(crate::tape::Tape::new(source)));
                self.brain = self
                    .selected_model
                    .as_ref()
                    .map(|m| Brain::with_tape(&m.settings, self.tape.clone()))
                    .transpose()?;
                self.log.event(
                    "tape-changed",
                    "Explicit branch response-tape intervention applied.",
                )?;
            }
            "/turns" if self.options.campaign_epoch.is_some() => {
                let turns: usize = arg.parse()?;
                if !(1..=10_000).contains(&turns) {
                    return Err("branch turn allowance must be 1..10000".into());
                }
                self.pause()?;
                self.config.agent.max_steps = self
                    .state
                    .model_calls
                    .checked_add(turns)
                    .ok_or("turn allowance overflow")?;
                self.log.event("turns-changed", "Remaining goal turns changed explicitly; session and campaign request usage are retained.")?;
            }
            "/floor" => {
                if !self.options.managed_floor {
                    return Err("floor controls require a managed cast session".into());
                }
                let (action, id) = arg
                    .split_once(' ')
                    .ok_or("floor control requires action and intent ID")?;
                let id: u64 = id.parse()?;
                if id == 0 || !["grant", "revoke"].contains(&action) {
                    return Err("invalid floor control".into());
                }
                if self.floor_fault == Some("controls")
                    || (action == "grant" && self.floor_fault == Some("grants"))
                {
                    return self
                        .log
                        .record("floor-fault", json!({"ignored":action,"id":id}));
                }
                if self
                    .speech
                    .as_ref()
                    .is_some_and(|s| s.id == id && s.advertised)
                {
                    if action == "revoke" {
                        self.cancel_speech("speaking lease revoked")?;
                    } else {
                        self.speech.as_mut().unwrap().granted = true;
                    }
                } else {
                    // A cancellation can precede a queued grant/revoke. Acknowledge only that ID.
                    self.log.record(
                        "speech-complete",
                        json!({"id":id,"outcome":"already_cancelled"}),
                    )?;
                }
            }
            "/recap" => {
                if arg.is_empty() {
                    return Err("use /recap <new.json>".into());
                }
                let recap = self.memories.observed_recap(self.watermark());
                save_json(std::path::Path::new(arg), &recap)?;
                self.log.record("scene-recap", json!({"path":arg,"entries":recap.entries.len(),"omitted":recap.omitted_episodes,"provider_requests":0,"scope":recap.scope}))?;
                self.log.event("recap", "Cited local observation recap saved; no model request. Coverage is limited to this session's SSH view.")?;
            }
            "/bookmark" => {
                if arg.is_empty() {
                    return Err("use /bookmark <new.json>".into());
                }
                let bookmark = self.bookmark()?;
                // Validate the serialized representation as well as the in-memory value.
                let encoded = serde_json::to_vec(&bookmark)?;
                let decoded: crate::scene::Bookmark = serde_json::from_slice(&encoded)?;
                decoded.validate()?;
                save_json(std::path::Path::new(arg), &decoded)?;
                self.log.record(
                    "scene-bookmark",
                    json!({"path":arg,"id":bookmark.id,"watermark":bookmark.watermark,"world_restorable":false}),
                )?;
                self.log.event("bookmark", "Bot bookmark saved and round-trip validated. The game world was not frozen and cannot be restored from this artifact.")?;
            }
            "/feedback" => {
                if !arg.is_empty() {
                    return Err("use /feedback (current goal)".into());
                }
                self.begin_playtest(None, false)?;
            }
            "/refute" => {
                let docket = crate::playtest::Docket::load(std::path::Path::new(arg))?;
                self.begin_playtest(Some(docket), false)?;
            }
            "/playtest" => {
                let archive = self.playtest.clone();
                if let Some(path) = arg.strip_prefix("save ") {
                    save_json(std::path::Path::new(path), &archive)?;
                } else if !arg.is_empty() {
                    return Err("use /playtest [save new.json]".into());
                }
                self.log
                    .event("playtest", &serde_json::to_string_pretty(&archive.view())?)?;
            }
            "/mission" => {
                if let Some(path) = arg.strip_prefix("load ") {
                    let spec = crate::mission::Spec::load(std::path::Path::new(path))?;
                    spec.bind(
                        &self.config.ssh,
                        self.options.campaign_epoch.is_some() && self.options.test_controls,
                    )?;
                    self.pause()?;
                    self.mission.load(spec)?;
                    self.memories.save_mission(&self.mission)?;
                } else if arg == "start" {
                    if self.pending.is_some() {
                        return Err(
                            "wait for current command to settle before starting a mission".into(),
                        );
                    }
                    let spec = self.mission.spec.as_ref().ok_or("load a mission first")?;
                    spec.bind(
                        &self.config.ssh,
                        self.options.campaign_epoch.is_some() && self.options.test_controls,
                    )?;
                    let goal = spec.goal.clone();
                    self.pause()?;
                    self.mission.active = true;
                    self.memories.save_mission(&self.mission)?;
                    self.state.goal = goal;
                    self.state.status = "running".into();
                    self.goal_started = Instant::now();
                } else if arg == "stop" {
                    self.pause()?;
                    self.mission.active = false;
                    self.memories.save_mission(&self.mission)?;
                } else if let Some(path) = arg.strip_prefix("save ") {
                    save_json(
                        std::path::Path::new(path),
                        &self.mission.report(self.memories.records()),
                    )?;
                } else if !arg.is_empty() {
                    return Err("use /mission [load file.json|start|stop|save new.json]".into());
                }
                self.log.event(
                    "mission",
                    &serde_json::to_string_pretty(&self.mission.report(self.memories.records()))?,
                )?;
            }
            "/probe" => {
                if arg.is_empty() {
                    return Err("use /probe <file.json>".into());
                }
                let probe = crate::probe::Spec::load(std::path::Path::new(arg))?;
                let question = probe.question_prompt()?;
                self.pause()?;
                // Keep the rubric in the local trace, outside the character's context.
                self.log
                    .record("probe-definition", serde_json::to_value(&probe)?)?;
                self.pending_probe = Some(probe);
                self.start_model(Some(question))?;
            }
            "/character-state" => {
                if !arg.is_empty() {
                    let state = if arg == "clear" {
                        crate::character::Condition::default()
                    } else if let Some(source) = arg.strip_prefix("set ") {
                        serde_json::from_str(source)?
                    } else {
                        return Err("use /character-state, /character-state set <json> or /character-state clear".into());
                    };
                    state.validate()?;
                    self.pause()?;
                    self.capture_observation = None;
                    self.character_state = state;
                }
                self.log.event(
                    "character-state",
                    &serde_json::to_string_pretty(&self.character_state)?,
                )?;
            }
            "/group" => self
                .log
                .event("group", &serde_json::to_string_pretty(&self.affiliations)?)?,
            "/social" => {
                self.memories.flush()?;
                let ledger = self.memories.social(&self.config.ssh.username)?;
                let view = if let Some(path) = arg.strip_prefix("save ") {
                    save_json(std::path::Path::new(path.trim()), &ledger)?;
                    json!({"saved":path.trim(),"observer":ledger.observer,"mode":"in_world_negotiation"})
                } else if arg.is_empty() {
                    serde_json::to_value(ledger)?
                } else {
                    ledger.context(arg, self.context_settings.social_bytes)
                };
                self.log
                    .event("social", &serde_json::to_string_pretty(&view)?)?;
            }
            "/map" => {
                self.memories.flush()?;
                self.log
                    .event("map", &serde_json::to_string_pretty(&self.memories.map())?)?;
            }
            "/learning" => {
                self.memories.flush()?;
                let state = self.memories.learning(&self.config.ssh.username)?;
                let view = if let Some(path) = arg.strip_prefix("save ") {
                    save_json(std::path::Path::new(path.trim()), &state)?;
                    json!({"saved":path.trim()})
                } else if arg.is_empty() {
                    serde_json::to_value(state)?
                } else {
                    state.context(arg, self.context_settings.learning_bytes)
                };
                self.log
                    .event("learning", &serde_json::to_string_pretty(&view)?)?;
            }
            "/development" => {
                if let Some(source) = arg.strip_prefix("set ") {
                    let policy: crate::learning::Policy = serde_json::from_str(source)?;
                    policy.validate()?;
                    self.pause()?;
                    self.memories
                        .learning_policy(policy, &self.config.ssh.username)?;
                } else if !arg.is_empty() {
                    return Err("use /development or /development set <json>".into());
                }
                self.log.event(
                    "development",
                    &serde_json::to_string_pretty(
                        &self.memories.learning(&self.config.ssh.username)?,
                    )?,
                )?;
            }
            "/activity" => {
                if let Some(path) = arg.strip_prefix("load ") {
                    if self.pending.is_some() {
                        return Err(
                            "wait for the pending command before replacing activities".into()
                        );
                    }
                    let plan = crate::activity::Plan::load(std::path::Path::new(path.trim()))?;
                    if plan.clock == crate::activity::Clock::ManualTest
                        && !self.options.test_controls
                    {
                        return Err("manual_test clock requires --test-controls".into());
                    }
                    let mut next = self.activity.clone();
                    next.install(plan, self.memories.records())?;
                    self.pause()?;
                    self.activity = next;
                    self.memories.save_activity(&self.activity)?;
                } else if let Some(path) = arg.strip_prefix("save ") {
                    save_json(
                        std::path::Path::new(path.trim()),
                        self.activity.plan.as_ref().ok_or("no activity plan")?,
                    )?;
                } else if let Some(delta) = arg.strip_prefix("advance ") {
                    if !self.options.test_controls
                        || self
                            .activity
                            .plan
                            .as_ref()
                            .is_none_or(|p| p.clock != crate::activity::Clock::ManualTest)
                    {
                        return Err("advance requires a manual_test scheduler and --test-controls; it does not change the game clock".into());
                    }
                    self.activity.advance(delta.parse()?)?;
                    self.memories.save_activity(&self.activity)?;
                } else if matches!(arg, "on" | "off" | "retry") {
                    if self.activity.plan.is_none() {
                        return Err("load an activity plan first".into());
                    }
                    if arg == "retry" {
                        if self.pending.is_some() {
                            return Err("wait for the pending command before retry".into());
                        }
                        self.activity.suspended = false;
                        self.activity.reason = "runner authorized retry after inspection".into();
                    } else {
                        self.activity.enabled = arg == "on";
                        if arg == "off" {
                            self.pause()?;
                        } else if self.state.status == "paused" {
                            self.state.status = "listening".into();
                        }
                    }
                    self.memories.save_activity(&self.activity)?;
                } else if !arg.is_empty() {
                    return Err("use /activity [load|save <json>|on|off|retry|advance <ms>]".into());
                }
                self.log
                    .event("activity", &serde_json::to_string_pretty(&self.activity)?)?;
            }
            "/inspect" => {
                let inspection = if arg == "preview" || arg.starts_with("preview ") {
                    let question = arg.strip_prefix("preview").unwrap().trim();
                    let question = (!question.is_empty()).then(|| question.to_owned());
                    let context = self.context(question.as_deref())?;
                    let capture = self
                        .brain
                        .as_ref()
                        .ok_or("configure a model before previewing")?
                        .prepare(context, question, &self.prompts)?;
                    json!({"preview":true,"observation_revision":self.observation_revision,"capture":capture})
                } else if arg.is_empty() {
                    json!({"preview":false,"stale":self.capture_observation != Some(self.observation_revision),
                        "capture":self.last_capture.as_ref().ok_or("no model request yet; use /inspect preview")?})
                } else {
                    return Err("use /inspect or /inspect preview [question]".into());
                };
                self.log.event(
                    "prompt-inspection",
                    &serde_json::to_string_pretty(&inspection)?,
                )?;
            }
            "/help" => self.log.event("help", REPL_HELP)?,
            "/arena" => self.arena_control(arg)?,
            "/state" => self
                .log
                .event("state", &serde_json::to_string_pretty(&self.snapshot()?)?)?,
            "/pause" => {
                self.pause()?;
                self.log.event("paused", "Pending model decision cancelled; an already sent game command may still finish.")?;
            }
            "/resume" | "/step" => {
                if verb == "/step" && arg == "command" {
                    return self.commit().await;
                }
                if verb == "/step" && arg == "observation" {
                    self.pause()?;
                    self.await_observation = self.pending.is_some();
                    return self.log.event(
                        "observation-step",
                        if self.await_observation {
                            "Waiting for current output to settle."
                        } else {
                            "No command is awaiting output."
                        },
                    );
                }
                if verb == "/step" && !arg.is_empty() && arg != "model" {
                    return Err("use /step, /step model, /step command or /step observation".into());
                }
                self.cancel_model()?;
                self.state.outcome.clear();
                self.state.status = if verb == "/step" {
                    "paused"
                } else if self.state.goal.trim().is_empty()
                    && self.attention_enabled
                    && !self.arena.as_ref().is_some_and(crate::arena::Lane::flying)
                {
                    "listening"
                } else {
                    "running"
                }
                .into();
                self.single_step = verb == "/step";
                self.stage_next = verb == "/step" && arg == "model";
                self.next_turn = Instant::now();
            }
            "/goal" | "/role" | "/note" => {
                if arg.is_empty() {
                    return Err(format!("{verb} requires text").into());
                }
                self.pause()?;
                self.capture_observation = None;
                self.recall_query = None;
                match verb {
                    "/goal" => {
                        self.state.goal = arg.to_owned();
                        self.state.outcome.clear();
                        self.state.model_calls = 0;
                        self.goal_started = Instant::now();
                    }
                    "/role" => self.state.role = arg.to_owned(),
                    _ => {
                        self.state.instructions.push_str(&format!("\n{arg}"));
                        self.state.instructions = tail(&self.state.instructions, 16_000).to_owned();
                    }
                }
                self.log
                    .event("paused", "Updated. Use /resume or /step to continue.")?;
            }
            "/ask" => {
                if arg.is_empty() {
                    return Err("/ask requires a question".into());
                }
                self.pause()?;
                self.start_model(Some(arg.to_owned()))?;
            }
            "/knowledge" => self.log.event("knowledge", &self.knowledge.search(arg))?,
            "/character" => self
                .log
                .event("character", &serde_json::to_string_pretty(&self.character)?)?,
            "/attention" => self.log.event(
                "attention",
                &serde_json::to_string_pretty(&self.attention.context())?,
            )?,
            "/cost" => self
                .log
                .event("cost", &serde_json::to_string_pretty(&self.cost_report()?)?)?,
            "/commit" => self.commit().await?,
            "/capture" => {
                let capture = self
                    .last_capture
                    .as_ref()
                    .ok_or("no model request captured yet")?;
                save_json(std::path::Path::new(arg), capture)?;
                self.log.event("capture", "Exact model request saved.")?;
            }
            "/prompt" | "/context" | "/debug" | "/budget" | "/conversation" => {
                if arg.is_empty() || (verb == "/budget" && arg == "show") {
                    let value = match verb {
                        "/prompt" => self.prompts.effective(),
                        "/context" => serde_json::to_value(&self.context_settings)?,
                        "/budget" => self.budget_report()?,
                        "/conversation" => self.conversation_snapshot(),
                        _ => serde_json::to_value(&self.debug)?,
                    };
                    self.log
                        .event(&verb[1..], &serde_json::to_string_pretty(&value)?)?;
                } else if let Some(path) = arg.strip_prefix("save ") {
                    let value = match verb {
                        "/prompt" => serde_json::to_value(&self.prompts)?,
                        "/context" => serde_json::to_value(&self.context_settings)?,
                        "/budget" => serde_json::to_value(&self.budget)?,
                        "/conversation" => serde_json::to_value(&self.conversation.settings)?,
                        _ => serde_json::to_value(&self.debug)?,
                    };
                    save_json(std::path::Path::new(path.trim()), &value)?;
                    self.log
                        .event("saved", "JSON profile saved to a new file.")?;
                } else {
                    let control = if let Some(path) = arg.strip_prefix("load ") {
                        let path = std::path::Path::new(path.trim());
                        match verb {
                            "/prompt" => Control::Prompts(Prompts::load(path)?),
                            "/context" => Control::Context(Context::load(path)?),
                            "/budget" => Control::Budget(crate::budget::Settings::load(path)?),
                            "/conversation" => {
                                Control::Conversation(crate::conversation::Settings::load(path)?)
                            }
                            _ => Control::Debug(debugger::Settings::load(path)?),
                        }
                    } else if let Some(source) = arg.strip_prefix("set ") {
                        match verb {
                            "/prompt" => Control::Prompts(serde_json::from_str(source)?),
                            "/context" => Control::Context(serde_json::from_str(source)?),
                            "/budget" => Control::Budget(serde_json::from_str(source)?),
                            "/conversation" => Control::Conversation(serde_json::from_str(source)?),
                            _ => Control::Debug(serde_json::from_str(source)?),
                        }
                    } else {
                        return Err("use load <json>, save <new.json>, or set <JSON object>".into());
                    };
                    // These variants do not recurse through the line parser.
                    self.configure(control)?;
                }
            }
            "/model" => {
                if arg.is_empty() {
                    self.log.event(
                        "model-selected",
                        &serde_json::to_string_pretty(&self.selected_model)?,
                    )?;
                } else if arg == "list" {
                    self.log.event(
                        "models",
                        &serde_json::to_string_pretty(&self.models.list())?,
                    )?;
                } else if let Some(name) = arg.strip_prefix("use ") {
                    let selection = self.models.select(name.trim())?;
                    // Fully validate credentials/client setup before changing state.
                    let brain = Brain::with_tape(&selection.settings, self.tape.clone())?;
                    self.pause()?;
                    self.brain = Some(brain);
                    self.selected_model = Some(selection);
                    self.log.event(
                        "model-selected",
                        &serde_json::to_string_pretty(&self.selected_model)?,
                    )?;
                    self.capture_observation = None;
                    self.log.event("paused", "Model switched. Memory, goal and remaining budget are retained. Use /resume, /listen or /step.")?;
                } else {
                    return Err("use /model, /model list or /model use <profile>".into());
                }
            }
            "/listen" => {
                if self.brain.is_none()
                    && self.conversation.settings.mode != crate::conversation::Mode::Spectator
                {
                    return Err("Configure [model] before listening for conversation".into());
                }
                self.pause()?;
                self.attention_enabled = true;
                self.state.status = "listening".into();
                self.state.outcome.clear();
                self.next_turn = Instant::now();
                self.log.event("listening", "Listening for engagement. The goal is retained but is not being pursued; /pause stops listening.")?;
            }
            "/recall" => self.log.event(
                "recall",
                &serde_json::to_string_pretty(&self.memories.recall_with(
                    arg,
                    self.context_settings.recall_bytes,
                    &self.context_settings.recall,
                ))?,
            )?,
            "/route" => self.log.event(
                "route",
                &serde_json::to_string_pretty(&self.memories.route(arg))?,
            )?,
            "/remember" => {
                self.pause()?;
                self.capture_observation = None;
                let id = self.memories.prior(arg)?;
                self.log.event(
                    "remembered",
                    &format!(
                        "Prior character knowledge saved as episode {id}. Use /resume to continue."
                    ),
                )?;
            }
            "/quit" => {
                self.cancel_model()?;
                if !matches!(self.state.status.as_str(), "done" | "blocked" | "error") {
                    self.state.status = "paused".into();
                    self.state.outcome = "Runner disconnected".into();
                }
                self.quit = true;
            }
            "/fault" if self.options.test_controls && arg == "disconnect" => {
                self.log.event(
                    "fault",
                    "Closing the SSH transport to test connection-loss handling.",
                )?;
                self.session.close().await;
            }
            "/fault"
                if self.options.test_controls
                    && self.options.managed_floor
                    && ["floor-grants", "floor-controls"].contains(&arg) =>
            {
                self.floor_fault = Some(if arg == "floor-grants" {
                    "grants"
                } else {
                    "controls"
                });
                self.log.event(
                    "fault",
                    "Simulating a stalled speaking-floor control receiver (test controls only).",
                )?;
            }
            "/send" => {
                self.pause()?;
                if self.pending.is_some() {
                    return Err("still collecting game output; retry the command shortly".into());
                }
                self.send(Some(arg.to_owned()), None).await?;
            }
            _ if verb.starts_with('/') => return Err("unknown local control; use /help".into()),
            _ => {
                self.pause()?;
                if self.pending.is_some() {
                    return Err("still collecting game output; retry the command shortly".into());
                }
                self.send(Some(line.to_owned()), None).await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arena_pilot_names_itself_from_the_ssh_username_else_the_sheet() {
        let sheet = Character {
            name: "Rosebot".into(),
            ..Character::default()
        };
        assert_eq!(arena_me("rose+Rose", &sheet), "Rose");
        assert_eq!(arena_me("rose+ Rose ", &sheet), "Rose");
        assert_eq!(arena_me("rose", &sheet), "Rosebot");
        assert_eq!(arena_me("rose+ ", &sheet), "Rosebot");
        assert_eq!(arena_me("rose+", &sheet), "Rosebot");
    }

    /// **The lane's pause follows the run's status** (cut AM.2): every one of
    /// the sixteen cases of its four inputs.
    #[test]
    fn pause_action_follows_the_status() {
        use PauseAction::{Pause, Resume};
        for flying in [false, true] {
            for stopped in [false, true] {
                for paused in [false, true] {
                    for running in [false, true] {
                        let expected = if flying && !stopped && !paused && !running {
                            Some(Pause)
                        } else if paused && running {
                            Some(Resume)
                        } else {
                            None
                        };
                        assert_eq!(
                            pause_action(flying, stopped, paused, running),
                            expected,
                            "flying {flying}, stopped {stopped}, paused {paused}, running {running}"
                        );
                    }
                }
            }
        }
        // The cases that matter, by name: a flying lane pauses when the run
        // stops running, and only once; it resumes when the run runs again; a
        // stopped or finished lane never pauses.
        assert_eq!(pause_action(true, false, false, false), Some(Pause));
        assert_eq!(pause_action(true, false, true, false), None, "once");
        assert_eq!(pause_action(true, false, true, true), Some(Resume));
        assert_eq!(pause_action(true, false, false, true), None, "flying on");
        assert_eq!(pause_action(true, true, false, false), None, "stopped");
        assert_eq!(pause_action(false, false, false, false), None, "finished");
    }

    /// **A paused flying lane is due** (cut AO.2): `arena_due_for` over all
    /// sixty-four cases of its six inputs, then the cases that matter by name.
    #[test]
    fn a_paused_flying_lane_is_due() {
        let both = [false, true];
        for flying in both {
            for running in both {
                for paused in both {
                    for stopped in both {
                        for leave in both {
                            for result in both {
                                let expected = (flying && running)
                                    || (flying && paused && !stopped)
                                    || leave
                                    || result;
                                assert_eq!(
                                    arena_due_for(flying, running, paused, stopped, leave, result),
                                    expected,
                                    "flying {flying}, running {running}, paused {paused}, \
                                     stopped {stopped}, leave {leave}, result {result}"
                                );
                            }
                        }
                    }
                }
            }
        }
        // Paused and flying: due, the run not running.
        assert!(arena_due_for(true, false, true, false, false, false));
        // Paused and stopped: not due, unless a leave or a result is owed.
        assert!(!arena_due_for(true, false, true, true, false, false));
        assert!(
            arena_due_for(true, false, true, true, true, false),
            "a leave"
        );
        assert!(
            arena_due_for(true, false, true, true, false, true),
            "a result"
        );
        // Running and flying: due.
        assert!(arena_due_for(true, true, false, false, false, false));
        // Neither running nor paused: not due.
        assert!(!arena_due_for(true, false, false, false, false, false));
        assert!(!arena_due_for(false, false, false, false, false, false));
    }

    /// **A mining run starts running and its lane takes the ticks** (doc 20
    /// cut SM.12c-bot): `--mine` alone makes a run active (no goal, no
    /// script, no arena), a paused run never is; the lane is due while the run
    /// runs and it has no result, and once more to write its result — then
    /// never again, whatever the status.
    #[test]
    fn a_mining_run_starts_running_and_its_lane_is_due() {
        assert!(
            active_at_start(false, false, false, false, true),
            "--mine alone"
        );
        assert!(!active_at_start(true, false, false, false, true), "paused");
        assert!(
            !active_at_start(false, false, false, false, false),
            "nothing plays"
        );
        for (goal, scripted, arena) in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            assert!(active_at_start(false, goal, scripted, arena, false));
        }
        assert!(mine_due_for(true, false, false), "running, no result");
        assert!(!mine_due_for(false, false, false), "not running, no result");
        assert!(mine_due_for(true, true, false), "a result to write");
        assert!(
            mine_due_for(false, true, false),
            "a result to write, whatever the status"
        );
        assert!(!mine_due_for(true, true, true), "written");
        assert!(!mine_due_for(false, true, true), "written");
    }

    /// **A paused lane's tick never ends the runner's** (cut AO.2): the
    /// runner's main path goes on past a paused lane's tick (so a `/step`
    /// still takes its goal turn), and a running lane's tick is the whole
    /// tick.
    #[test]
    fn a_paused_lanes_tick_never_ends_the_runners() {
        assert!(!arena_tick_ends_tick(true), "paused: the tick goes on");
        assert!(
            arena_tick_ends_tick(false),
            "running: the lane's tick is the tick"
        );
    }
}
