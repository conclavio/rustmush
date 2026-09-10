use std::{
    collections::VecDeque,
    fs::File,
    io::{BufRead, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use russh::ChannelMsg;
use serde::Serialize;
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
  /probe <file.json>     Ask and grade a structured evidence question (pauses)\n\
  /send <command>        Send a game command (pauses the bot)\n\
  /knowledge <query>     Search bundled help and supplied documents\n\
  /recall <query|#id>    Retrieve older experiences and their evidence\n\
  /remember <text>       Supply durable prior character knowledge\n\
  /route <place|#dbref>  Find a previously travelled route from here\n\
  /character            Inspect the authored character profile\n\
  /attention            Inspect recent speech and attention cues\n\
  /listen               Listen and respond without pursuing a goal (/pause stops)\n\
  /model [list]         Show the active model or available profiles\n\
  /model use <profile>  Switch model and pause; memory and budget are retained\n\
  /cost                 Show session and all-session costs by model and purpose\n\
  /budget [load|save <json>] Inspect session limits or load/save a policy\n\
  /prompt [load|save <json>]  Inspect effective prompts or load/save a profile\n\
  /inspect [preview [question]] Show last request, or prepare a free preview\n\
  /group                Show this actor's authored group affiliations\n\
  /character-state [set <json>|clear] Inspect or change a temporary condition\n\
  /context [load|save <json>] Inspect context controls or load/save settings\n\
  /debug [load|save <json>]   Inspect breakpoints/watches or load/save settings\n\
  /capture <new.json>    Save the last exact model request for offline replay\n\
  /step model           Produce a decision and hold it before applying\n\
  /step command | /commit Apply the held decision if observations are unchanged\n\
  /step observation     Pause and wait for current command output to settle\n\
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
}

pub enum Control {
    Line(String),
    Prompts(Prompts),
    Context(Context),
    Debug(debugger::Settings),
    Budget(crate::budget::Settings),
}

pub struct Frontend {
    pub controls: mpsc::Receiver<Control>,
    pub events: mpsc::Sender<Value>,
}

#[derive(Clone, Debug, Serialize)]
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
    pub simulated_model: bool,
    pub probe_results: VecDeque<crate::probe::Grade>,
    pub probe_results_omitted: usize,
    /// Canonical input record, including defaulted timeouts, for reproducibility.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script: Option<Scenario>,
}

struct Log {
    file: Option<File>,
    sequence: u64,
    echo: bool,
    json_events: bool,
    events: Option<mpsc::Sender<Value>>,
    request: Option<u64>,
    command_sequence: Option<u64>,
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
        })
    }

    fn record(&mut self, kind: &str, data: Value) -> Result<()> {
        self.sequence += 1;
        let timestamp_ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
        let event = json!({"seq":self.sequence, "timestamp_ms":timestamp_ms,
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
}

pub async fn run(config: Config, options: Options) -> Result<Report> {
    run_controlled(config, options, None).await
}

pub async fn run_controlled(
    mut config: Config,
    mut options: Options,
    frontend: Option<Frontend>,
) -> Result<Report> {
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
    if scenario.is_some() {
        options.headless = true;
    }
    let models = Catalog::from_config(&config)?;
    let selected_model = models.initial()?;
    if options.headless
        && scenario.is_none()
        && (config.agent.goal.is_empty() || selected_model.is_none())
    {
        return Err("headless play requires a goal and [model], or --script".into());
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
    for fact in character.prior_facts() {
        memories.seed_prior(&fact)?;
    }
    if let Some(affiliations) = &affiliations {
        for fact in affiliations.prior_facts() {
            memories.seed_prior(&fact)?;
        }
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
    let costs = Arc::new(Mutex::new(Ledger::open(
        &config.costs,
        &config.ssh,
        &config.memory.namespace,
    )?));
    let mut log = Log::new(options.transcript.as_ref())?;
    log.echo = frontend.is_none() && !options.json_events;
    log.json_events = options.json_events;
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
    let now = Instant::now();
    let active = !options.paused && (!config.agent.goal.is_empty() || scenario.is_some());
    let state = State {
        status: if active {
            "running"
        } else if !options.paused && attention_enabled && brain.is_some() {
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
        // A replay sends exactly the commands in its source record. Login output
        // is still collected, but agent bootstrap belongs to autonomous play.
        bootstrap: if script.is_some() {
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
        probe_results: Default::default(),
        probe_results_omitted: 0,
        single_step: false,
        scenario: scenario.map(|s| s.steps.into()),
        next_turn: now,
        started: now,
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
    };
    let result = app.drive().await;
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
        character_state: app.character_state.clone(),
        context: app.context_settings.clone(),
        debug: app.debug.clone(),
        budget: app.budget_report()?,
        simulated_model: app.tape.is_some(),
        probe_results: app.probe_results.clone(),
        probe_results_omitted: app.probe_results_omitted,
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
    fn snapshot(&self) -> Result<Value> {
        Ok(
            json!({"state":self.state, "memory_store":self.memories.stats(),
            "model":self.selected_model, "models":self.models.list(), "costs":self.cost_report()?,
            "prompts":self.prompts, "context":self.context_settings, "debug":self.debug,
            "budget":self.budget_report()?,
            "group_affiliations":self.affiliations,
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
        }
        self.capture_observation = None;
        Ok(())
    }

    async fn receive_decision(&mut self, decision: Decision) -> Result<()> {
        if self.stage_next || self.debug.command_break(decision.command.as_deref()) {
            self.pause()?;
            self.log
                .record("staged-decision", serde_json::to_value(&decision)?)?;
            self.log.event("breakpoint", "Decision held before applying memory or sending a command. /commit applies it; /resume replans.")?;
            self.staged = Some((decision, self.observation_revision));
            Ok(())
        } else {
            self.apply(decision).await
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
        self.apply(decision).await
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
        self.cancel_request()
    }

    fn pause(&mut self) -> Result<()> {
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
        let usage = self
            .costs
            .lock()
            .map_err(|_| "cost ledger lock poisoned")?
            .session_usage()?;
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

    fn context(&self, question: Option<&str>) -> Value {
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
            "group_affiliations": self.affiliations,
            "character_state": self.character_state,
            "recent_speech": self.attention.context(),
            "decision_purpose": if question.is_some() {"runner_question"} else if self.social_turn {"conversation"} else {"goal"},
            "runner_instructions":self.state.instructions, "memory":self.state.memory,
            "terminal_history":self.state.recent, "reference_documents":self.knowledge.search(&query),
            "remembered_experience": self.memories.recall_with(recall_query,self.context_settings.recall_bytes,&self.context_settings.recall),
            "route_lookup": self.memories.route(recall_query),
            "belief_feedback": self.belief_feedback,
            "model_calls_remaining":self.config.agent.max_steps.saturating_sub(self.state.model_calls)});
        self.context_settings.apply(&mut context);
        context
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
                            let text = decoder.feed(&data);
                            if !text.is_empty() {
                                if self.log.echo { print!("{text}"); std::io::stdout().flush()?; }
                                self.observation_revision += 1;
                                let watch = self.debug.observation_match(&self.state.recent, &text).map(str::to_owned);
                                self.log.record("observation", json!(text))?;
                                self.memories.observe(&text)?;
                                self.attention.feed(&text);
                                self.notice_attention()?;
                                self.state.recent.push_str(&text);
                                self.state.recent = tail(&self.state.recent, self.config.agent.context_bytes).to_owned();
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
                reply = async { self.model_task.as_mut().unwrap().await }, if self.model_task.is_some() => {
                    self.model_task = None;
                    self.request_id = None;
                    let answer = self.asking;
                    self.asking = false;
                    let probe = self.pending_probe.take();
                    match reply.map_err(|e| -> crate::Error { e.into() }).and_then(|r| r) {
                        Ok(reply) => {
                            self.log.record("model-reply", json!({"content":reply.content,"error":reply.error}))?;
                            self.log.record("usage", reply.usage)?;
                            self.log.record("cost-receipt", serde_json::to_value(&reply.receipt)?)?;
                            if let Some(error) = reply.error {
                                if let Some(probe) = &probe { self.log.record("probe-error",json!({"probe_id":probe.id,"reason":"provider_error"}))?; }
                                self.fail(&format!("Model request failed: {error}"))?;
                            }
                            else if let Some(probe) = probe {
                                let capture = self.last_capture.as_ref().ok_or("probe response has no prepared request")?;
                                let grade = crate::probe::grade(&probe,capture,&reply.content)?;
                                let value = serde_json::to_value(&grade)?;
                                // A bounded review history; the transcript retains every result.
                                if self.probe_results.len() == 128 {
                                    self.probe_results.pop_front();
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

    fn fail(&mut self, message: &str) -> Result<()> {
        self.pause()?;
        self.state.status = "blocked".into();
        self.state.outcome = message.to_owned();
        self.log.event("blocked", message)
    }

    fn notice_attention(&mut self) -> Result<()> {
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
            self.session.send(command).await?;
            self.memories.sent(command)?;
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
        if self
            .social_started
            .is_some_and(|at| at.elapsed() >= Duration::from_secs(self.config.agent.max_seconds))
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
            self.start_model(None)?;
        }
        Ok(())
    }

    fn start_model(&mut self, question: Option<String>) -> Result<()> {
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
        let Some(brain) = self.brain.clone() else {
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
        );
        let capture = match brain.prepare(context, question, &self.prompts) {
            Ok(capture) => capture,
            Err(error) => return self.fail(&format!("Prompt preparation failed: {error}")),
        };
        self.recall_query = None;
        self.belief_feedback.clear();
        self.asking = capture.question;
        let prompt_type = if self.pending_probe.is_some() {
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
        self.log.event(
            "model",
            if self.pending_probe.is_some() {
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
        self.capture_observation = None;
        if self.character_state.advance() {
            self.log
                .record("character-state-expired", json!(self.character_state))?;
        }
        self.log
            .record("decision", serde_json::to_value(&decision)?)?;
        self.log.event("summary", &decision.summary)?;
        self.memories.flush()?;
        self.belief_feedback = self.memories.annotate(decision.beliefs)?;
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
                self.next_turn = Instant::now() + Duration::from_millis(decision.wait_ms)
            }
            Action::Recall => {
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
            Action::Done | Action::Blocked => {
                if self.social_turn {
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
            }
        }
        Ok(())
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
        // Validate before recording input, so in-band credentials never reach logs.
        if verb == "/send" {
            validate_command(arg)?;
        } else if !verb.starts_with('/') {
            validate_command(line)?;
        }
        self.memories.flush()?;
        self.log.record("control", json!(line))?;
        match verb {
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
            "/inspect" => {
                let inspection = if arg == "preview" || arg.starts_with("preview ") {
                    let question = arg.strip_prefix("preview").unwrap().trim();
                    let question = (!question.is_empty()).then(|| question.to_owned());
                    let context = self.context(question.as_deref());
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
                } else if self.state.goal.trim().is_empty() && self.attention_enabled {
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
            "/prompt" | "/context" | "/debug" | "/budget" => {
                if arg.is_empty() || (verb == "/budget" && arg == "show") {
                    let value = match verb {
                        "/prompt" => self.prompts.effective(),
                        "/context" => serde_json::to_value(&self.context_settings)?,
                        "/budget" => self.budget_report()?,
                        _ => serde_json::to_value(&self.debug)?,
                    };
                    self.log
                        .event(&verb[1..], &serde_json::to_string_pretty(&value)?)?;
                } else if let Some(path) = arg.strip_prefix("save ") {
                    let value = match verb {
                        "/prompt" => serde_json::to_value(&self.prompts)?,
                        "/context" => serde_json::to_value(&self.context_settings)?,
                        "/budget" => serde_json::to_value(&self.budget)?,
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
                            _ => Control::Debug(debugger::Settings::load(path)?),
                        }
                    } else if let Some(source) = arg.strip_prefix("set ") {
                        match verb {
                            "/prompt" => Control::Prompts(serde_json::from_str(source)?),
                            "/context" => Control::Context(serde_json::from_str(source)?),
                            "/budget" => Control::Budget(serde_json::from_str(source)?),
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
                if self.brain.is_none() {
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
