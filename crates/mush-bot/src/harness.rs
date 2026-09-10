//! Source-controlled casts and experiment matrices, composed from independent bot processes.
use crate::{
    Result,
    cast_debug::{Controller as DebugController, Session as DebugSession},
    config::Config,
    contracts::{Contract, Matcher, Monitor},
    experiment::{load_json, save_json},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::mpsc,
    time::Instant,
};

fn version() -> u32 {
    1
}
fn timeout() -> u64 {
    60_000
}
fn done() -> String {
    "done".into()
}
fn repeats() -> usize {
    1
}

/// A local observer can display the cast and request graceful cancellation.
#[derive(Clone)]
pub struct Observer {
    pub events: mpsc::Sender<Value>,
    pub cancel: tokio::sync::watch::Receiver<bool>,
}
impl Observer {
    fn stopped(&self) -> bool {
        *self.cancel.borrow() || self.cancel.has_changed().is_err()
    }
}
fn emit(observer: Option<&Observer>, event: Value) {
    if let Some(observer) = observer {
        let _ = observer.events.try_send(event);
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(default = "version")]
    pub version: u32,
    pub name: String,
    pub actors: Vec<Actor>,
    #[serde(default)]
    pub groups: Vec<crate::organization::Group>,
    #[serde(default)]
    pub steps: Vec<Step>,
    #[serde(default)]
    pub assertions: Contract,
    #[serde(default = "timeout")]
    pub timeout_ms: u64,
    pub matrix: Option<Matrix>,
    /// An optional server-under-test process, started afresh for each run.
    pub server: Option<Server>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Actor {
    pub name: String,
    pub config: PathBuf,
    pub login: Option<String>,
    pub goal: Option<String>,
    pub role: Option<String>,
    pub character: Option<PathBuf>,
    pub model: Option<String>,
    pub prompts: Option<PathBuf>,
    pub context: Option<PathBuf>,
    pub debug: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<PathBuf>,
    pub model_tape: Option<PathBuf>,
    pub script: Option<PathBuf>,
    /// First local control after every cast member is ready; omit to remain paused.
    pub start: Option<String>,
    #[serde(default)]
    pub test_controls: bool,
    #[serde(default = "done")]
    pub expected_status: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub name: Option<String>,
    pub actor: Option<String>,
    #[serde(default)]
    pub actors: Vec<String>,
    pub group: Option<String>,
    #[serde(default)]
    pub dispatch: Dispatch,
    pub control: Option<String>,
    /// A local structured question and rubric, kept outside character context.
    pub probe: Option<PathBuf>,
    #[serde(default)]
    pub expect: Vec<Matcher>,
    #[serde(default)]
    pub reject: Vec<Matcher>,
    #[serde(default = "timeout")]
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Matrix {
    pub actor: String,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub prompts: Vec<PathBuf>,
    #[serde(default)]
    pub characters: Vec<PathBuf>,
    #[serde(default)]
    pub contexts: Vec<PathBuf>,
    #[serde(default = "repeats")]
    pub repeats: usize,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Dispatch {
    #[default]
    Parallel,
    Serial,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Server {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub working_directory: Option<PathBuf>,
    /// Readiness is established from this literal stdout substring, never guessed from a delay.
    pub ready_output: String,
    #[serde(default = "timeout")]
    pub timeout_ms: u64,
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Self> {
        let mut manifest: Self = load_json(path, 2_000_000)?;
        let base = std::fs::canonicalize(path)?
            .parent()
            .ok_or("manifest needs a parent directory")?
            .to_path_buf();
        let resolve = |path: &mut PathBuf| {
            if path.is_relative() {
                *path = base.join(&*path);
            }
        };
        for actor in &mut manifest.actors {
            resolve(&mut actor.config);
            for path in actor
                .character
                .iter_mut()
                .chain(actor.prompts.iter_mut())
                .chain(actor.context.iter_mut())
                .chain(actor.debug.iter_mut())
                .chain(actor.budget.iter_mut())
                .chain(actor.model_tape.iter_mut())
                .chain(actor.script.iter_mut())
            {
                resolve(path);
            }
        }
        for step in &mut manifest.steps {
            if let Some(path) = &mut step.probe {
                resolve(path);
            }
        }
        if let Some(matrix) = &mut manifest.matrix {
            for path in matrix
                .prompts
                .iter_mut()
                .chain(&mut matrix.characters)
                .chain(&mut matrix.contexts)
            {
                resolve(path);
            }
        }
        if let Some(server) = &mut manifest.server {
            if let Some(dir) = &mut server.working_directory {
                resolve(dir);
            } else {
                server.working_directory = Some(base);
            }
        }
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<()> {
        if self.version != 1
            || self.name.trim().is_empty()
            || self.name.len() > 200
            || self.actors.is_empty()
            || self.actors.len() > 16
            || self.steps.len() > 1000
            || !(100..=3_600_000).contains(&self.timeout_ms)
        {
            return Err("invalid harness version/name/cast/limits".into());
        }
        let mut names = std::collections::BTreeSet::new();
        for actor in &self.actors {
            if actor.name.is_empty()
                || actor.name == "_harness"
                || actor.name.len() > 64
                || !actor
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                || !names.insert(actor.name.as_str())
            {
                return Err("cast names must be unique ASCII letters/digits/underscores/hyphens, 1..64 bytes".into());
            }
            if !["done", "paused", "listening", "blocked", "error"]
                .contains(&actor.expected_status.as_str())
            {
                return Err("invalid expected actor status".into());
            }
            actor.preflight()?;
        }
        let check_matchers = |contract: &Contract| -> Result<()> {
            contract.validate()?;
            for m in contract
                .eventually
                .iter()
                .chain(&contract.ordered)
                .chain(&contract.forbidden)
            {
                if m.actor.as_deref().is_some_and(|a| !names.contains(a)) {
                    return Err("assertion names an unknown actor".into());
                }
            }
            Ok(())
        };
        check_matchers(&self.assertions)?;
        let mut group_ids = std::collections::BTreeSet::new();
        if self.groups.len() > 8 {
            return Err("harness supports at most 8 groups".into());
        }
        for group in &self.groups {
            group.validate()?;
            if !group_ids.insert(&group.id)
                || group
                    .members
                    .iter()
                    .any(|m| !names.contains(m.actor.as_str()))
            {
                return Err("groups need unique IDs and known cast members".into());
            }
        }
        for actor in &self.actors {
            let affiliations =
                crate::organization::Affiliations::for_actor(&actor.name, &self.groups)?;
            if !affiliations.groups.is_empty()
                && Config::load(&actor.config)?.affiliations.is_some()
            {
                return Err("a grouped harness actor must define affiliations in the manifest, not also its config".into());
            }
        }
        for step in &self.steps {
            let targets = step.targets(self)?;
            if step.control.is_some() && step.probe.is_some() {
                return Err("choose either control or probe for a harness step".into());
            }
            if let Some(path) = &step.probe {
                crate::probe::Spec::load(path)?;
                validate_control_line(&step.control_line())?;
            }
            if step.dispatch == Dispatch::Serial && step.group.is_none() && step.actors.is_empty() {
                return Err("serial dispatch needs a group or actors target".into());
            }
            if ((step.control.is_some() || step.probe.is_some()) && targets.is_empty())
                || !(1..=3_600_000).contains(&step.timeout_ms)
                || (step.control.is_none() && step.probe.is_none() && step.expect.is_empty())
                || step
                    .name
                    .as_ref()
                    .is_some_and(|s| s.trim().is_empty() || s.len() > 200)
            {
                return Err("steps need targets for controls or expectations for a barrier, and bounded name/timeout".into());
            }
            for name in &targets {
                let actor = self
                    .actors
                    .iter()
                    .find(|a| &a.name == name)
                    .ok_or("step names an unknown actor")?;
                if actor.script.is_some() {
                    return Err("script actors cannot receive local controls".into());
                }
                if let Some(control) = &step.control {
                    validate_control(control)?;
                }
            }
            check_matchers(&step.contract(self)?)?;
        }
        if let Some(matrix) = &self.matrix {
            if !names.contains(matrix.actor.as_str()) || matrix.repeats == 0 || matrix.repeats > 100
            {
                return Err("matrix needs a known actor and 1..100 repeats".into());
            }
            let count = [
                matrix.models.len(),
                matrix.prompts.len(),
                matrix.characters.len(),
                matrix.contexts.len(),
            ]
            .iter()
            .try_fold(matrix.repeats, |n, s| n.checked_mul((*s).max(1)))
            .ok_or("matrix size overflow")?;
            if count > 256 {
                return Err("matrix exceeds 256 runs".into());
            }
        }
        if let Some(server) = &self.server
            && (server.program.is_empty()
                || server.ready_output.is_empty()
                || server.ready_output.len() > 4000
                || server.args.len() > 100
                || !(100..=300_000).contains(&server.timeout_ms))
        {
            return Err("server needs a program, readiness output and bounded timeout".into());
        }
        // Validate every model/profile/character combination before launching any player.
        for (actors, _) in self.runs() {
            for actor in actors {
                actor.preflight()?;
            }
        }
        Ok(())
    }

    /// Expand targets and experiment variants without starting processes or reading secrets.
    pub fn plan(&self) -> Result<Value> {
        self.validate()?;
        let phases: Vec<Value> = self.steps.iter().enumerate().map(|(index,step)| {
            let targets = step.targets(self)?;
            Ok(json!({"id":format!("phase-{}",index+1),"name":step.name,"targets":targets,
                "dispatch":step.dispatch,"control":step.control_line(),"probe":step.probe,"timeout_ms":step.timeout_ms,
                "assertions":step.contract(self)?,"requires_acknowledgments":!targets.is_empty(),
                "completion":"All targeted actors acknowledge; all expected observations match; no rejected observation matches."}))
        }).collect::<Result<_>>()?;
        let runs: Vec<Value> = self
            .runs()
            .into_iter()
            .enumerate()
            .map(|(index, (actors, repetition))| {
                json!({"index":index+1,"repetition":repetition,"actors":actors.iter().map(|actor| {
                // Deliberately list selectors and lifecycle only: a plan need not copy
                // biography, private briefings, reference documents, or login secrets.
                json!({"name":actor.name,"config":actor.config,"model":actor.model,
                    "prompts":actor.prompts,"context":actor.context,"character":actor.character,"budget":actor.budget,
                    "script":actor.script,"model_tape":actor.model_tape,
                    "start":actor.start,"expected_status":actor.expected_status})
            }).collect::<Vec<_>>()})
            })
            .collect();
        Ok(
            json!({"version":1,"name":self.name,"run_count":runs.len(),"actor_sessions":runs.len()*self.actors.len(),
            "timeout_ms_per_run":self.timeout_ms,"owns_server":self.server.is_some(),"runs":runs,"phases":phases,
            "assertions":self.assertions,"groups":self.groups.iter().map(|g| json!({"id":g.id,"name":g.name,"kind":g.kind,
                "members":g.members.iter().map(|m|json!({"actor":m.actor,"role":m.role})).collect::<Vec<_>>()})).collect::<Vec<_>>(),
            "notes":["This is a validated execution plan; no game or provider requests were made.",
                "Null profile selectors inherit actor configuration; no credentials are resolved during planning.",
                "Each run uses fresh bot memory. A server process restart does not itself reset the world database.",
                "Game commands remain subject to character permissions and server responses."]}),
        )
    }

    fn runs(&self) -> Vec<(Vec<Actor>, usize)> {
        let Some(matrix) = &self.matrix else {
            return vec![(self.actors.clone(), 1)];
        };
        fn choices<T: Clone>(values: &[T]) -> Vec<Option<T>> {
            if values.is_empty() {
                vec![None]
            } else {
                values.iter().cloned().map(Some).collect()
            }
        }
        let mut runs = Vec::new();
        for model in choices(&matrix.models) {
            for prompts in choices(&matrix.prompts) {
                for character in choices(&matrix.characters) {
                    for context in choices(&matrix.contexts) {
                        for repetition in 1..=matrix.repeats {
                            let mut actors = self.actors.clone();
                            if let Some(actor) = actors.iter_mut().find(|a| a.name == matrix.actor)
                            {
                                if model.is_some() {
                                    actor.model = model.clone();
                                }
                                if prompts.is_some() {
                                    actor.prompts = prompts.clone();
                                }
                                if character.is_some() {
                                    actor.character = character.clone();
                                }
                                if context.is_some() {
                                    actor.context = context.clone();
                                }
                            }
                            runs.push((actors, repetition));
                        }
                    }
                }
            }
        }
        runs
    }
}

fn validate_control(control: &str) -> Result<()> {
    if control
        .split_whitespace()
        .next()
        .is_some_and(|verb| verb == "/probe" || verb == "/harness")
    {
        return Err("use a step's probe field for evaluation; raw /probe and /harness controls are not harness authoring commands".into());
    }
    validate_control_line(control)
}

fn validate_control_line(control: &str) -> Result<()> {
    if control.is_empty() || control.len() > 16_000 || control.chars().any(char::is_control) {
        return Err("harness controls must be nonempty single lines <= 16000 bytes".into());
    }
    if let Some(command) = control.strip_prefix("/send ") {
        crate::transport::validate_command(command)?;
    } else if !control.starts_with('/') {
        crate::transport::validate_command(control)?;
    }
    Ok(())
}

impl Actor {
    fn preflight(&self) -> Result<()> {
        let mut config = Config::load(&self.config)?;
        if let Some(login) = &self.login {
            config.ssh.username = login.clone();
            config.validate()?;
        }
        if let Some(model) = &self.model {
            config.model_profile = Some(model.clone());
        }
        if let Some(model) = crate::models::Catalog::from_config(&config)?.initial()? {
            model.settings.validate()?;
        }
        if let Some(path) = self.character.as_ref().or(config.character.as_ref()) {
            crate::character::Character::load(path)?;
        }
        if let Some(path) = &config.affiliations {
            crate::organization::Affiliations::load(path)?;
        }
        if let Some(path) = self.prompts.as_ref().or(config.prompts.as_ref()) {
            crate::experiment::Prompts::load(path)?;
        }
        if let Some(path) = self.context.as_ref().or(config.context.as_ref()) {
            crate::experiment::Context::load(path)?;
        }
        if let Some(path) = self.debug.as_ref().or(config.debug.as_ref()) {
            crate::debugger::Settings::load(path)?;
        }
        if let Some(path) = self.budget.as_ref().or(config.budget.as_ref()) {
            crate::budget::Settings::load(path)?;
        }
        if let Some(path) = self.model_tape.as_ref().or(config.model_tape.as_ref()) {
            crate::tape::Source::load(path)?;
        }
        if let Some(path) = &self.script {
            crate::script::Scenario::load(path)?;
            if !["done", "blocked", "error"].contains(&self.expected_status.as_str()) {
                return Err("script actors finish as done, blocked or error".into());
            }
        }
        if let Some(start) = &self.start {
            validate_control(start)?;
            if self.script.is_some() {
                return Err("script actors cannot have a start control".into());
            }
        }
        Ok(())
    }
}

impl Step {
    fn control_line(&self) -> String {
        self.probe.as_ref().map_or_else(
            || self.control.clone().unwrap_or_else(|| "/state".into()),
            |path| format!("/probe {}", path.display()),
        )
    }
    fn targets(&self, manifest: &Manifest) -> Result<Vec<String>> {
        if usize::from(self.actor.is_some())
            + usize::from(!self.actors.is_empty())
            + usize::from(self.group.is_some())
            > 1
        {
            return Err("choose one step target: actor, actors or group".into());
        }
        let targets = if let Some(id) = &self.group {
            manifest
                .groups
                .iter()
                .find(|g| &g.id == id)
                .ok_or("step names an unknown group")?
                .members
                .iter()
                .map(|m| m.actor.clone())
                .collect()
        } else if let Some(actor) = &self.actor {
            vec![actor.clone()]
        } else {
            self.actors.clone()
        };
        let mut unique = std::collections::BTreeSet::new();
        if targets
            .iter()
            .any(|a| !unique.insert(a) || !manifest.actors.iter().any(|p| &p.name == a))
        {
            return Err("step targets must be unique known actors".into());
        }
        Ok(targets)
    }

    fn contract(&self, manifest: &Manifest) -> Result<Contract> {
        let targets = self.targets(manifest)?;
        let scoped = self.group.is_some() || !self.actors.is_empty();
        let expand = |matchers: &[Matcher]| -> Result<Vec<Matcher>> {
            let mut result = Vec::new();
            for m in matchers {
                if scoped && m.actor.as_ref().is_some_and(|a| !targets.contains(a)) {
                    return Err(
                        "a group/actors step assertion must refer to one of its targets".into(),
                    );
                }
                if scoped && m.actor.is_none() {
                    for actor in &targets {
                        let mut m = m.clone();
                        m.actor = Some(actor.clone());
                        result.push(m);
                    }
                } else {
                    result.push(m.clone());
                }
            }
            Ok(result)
        };
        let mut expect = self.expect.clone();
        let mut reject = self.reject.clone();
        if let Some(path) = &self.probe {
            let spec = crate::probe::Spec::load(path)?;
            // Probe outcomes are tied to the new control's event fence and its ID.
            let id = format!("\"probe_id\":{}", serde_json::to_string(&spec.id)?);
            // Always scope generated checks, including single-actor phases.
            for actor in &targets {
                expect.push(serde_json::from_value(
                    json!({"actor":actor,"kind":"probe-result","contains":id,
                    "pointer":"/success","equals":true}),
                )?);
                reject.push(serde_json::from_value(
                    json!({"actor":actor,"kind":"probe-result","contains":id,
                    "pointer":"/success","equals":false}),
                )?);
                reject.push(serde_json::from_value(
                    json!({"actor":actor,"kind":"blocked"}),
                )?);
            }
        }
        Ok(Contract {
            eventually: expand(&expect)?,
            ordered: vec![],
            forbidden: expand(&reject)?,
        })
    }
}

struct ActiveStep {
    monitor: Monitor,
    since: Instant,
    id: String,
    targets: std::collections::BTreeSet<String>,
    opened: std::collections::BTreeSet<String>,
    applied: std::collections::BTreeSet<String>,
    queued: std::collections::VecDeque<String>,
    last_dispatched: Option<String>,
}

impl ActiveStep {
    fn feed(&mut self, actor: &str, event: &Value) {
        if self.targets.contains(actor) && event["data"]["id"] == self.id {
            if event["kind"] == "control-started" {
                self.opened.insert(actor.into());
            }
            if event["kind"] == "control-applied" && self.opened.contains(actor) {
                self.applied.insert(actor.into());
            }
        }
        if !self.targets.contains(actor) || self.opened.contains(actor) {
            self.monitor.feed(actor, event);
        }
    }
    fn satisfied(&self) -> bool {
        self.applied == self.targets && self.monitor.satisfied()
    }
    fn report(&self) -> Value {
        json!({"id":self.id,"targets":self.targets,"acknowledged":self.applied,"queued":self.queued,
        "waiting_for":self.targets.difference(&self.applied).collect::<Vec<_>>(),"assertions":self.monitor.report(),"elapsed_ms":self.since.elapsed().as_millis()})
    }
}

struct Player {
    child: Child,
    input: Option<ChildStdin>,
    ready: bool,
    status: String,
    report: PathBuf,
}
struct Incoming {
    actor: String,
    event: Value,
}

#[derive(Default)]
struct ProbeLog {
    entries: Vec<Value>,
    omitted: usize,
    bytes: usize,
    failed: bool,
}
impl ProbeLog {
    fn feed(&mut self, actor: &str, event: &Value) {
        if !["probe-result", "probe-error", "probe-cancelled"]
            .iter()
            .any(|k| event["kind"] == *k)
        {
            return;
        }
        let grade = if event["kind"] == "probe-result" {
            event["data"].clone()
        } else {
            Value::Null
        };
        self.failed |= grade["success"] == false;
        let record =
            json!({"actor":actor,"seq":event["seq"],"outcome":event["kind"],"grade":grade});
        let bytes = record.to_string().len();
        if self.entries.len() >= 4096 || self.bytes.saturating_add(bytes) > 2_000_000 {
            self.omitted += 1;
        } else {
            self.bytes += bytes;
            self.entries.push(record);
        }
    }
}

async fn spawn_actor(
    actor: &Actor,
    directory: &Path,
    sender: mpsc::Sender<Incoming>,
    affiliations: &crate::organization::Affiliations,
    bot_program: Option<&Path>,
) -> Result<Player> {
    std::fs::create_dir(directory)?;
    let report = directory.join("report.json");
    let mut command = Command::new(match bot_program {
        Some(program) => program.to_path_buf(),
        None => std::env::current_exe()?,
    });
    command
        .arg("--config")
        .arg(&actor.config)
        .arg("--json-events")
        .arg("--transcript")
        .arg(directory.join("trace.jsonl"))
        .arg("--report")
        .arg(&report)
        .arg("--isolate-memory")
        .arg(directory.join("memory"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(File::create(directory.join("stderr.txt"))?))
        .kill_on_drop(true);
    if actor.script.is_none() {
        command.arg("--paused");
    }
    if !affiliations.groups.is_empty() {
        let path = directory.join("affiliations.json");
        save_json(&path, affiliations)?;
        command.arg("--affiliations").arg(path);
    }
    for (flag, path) in [
        ("--character", &actor.character),
        ("--prompts", &actor.prompts),
        ("--context", &actor.context),
        ("--debug", &actor.debug),
        ("--budget", &actor.budget),
        ("--model-tape", &actor.model_tape),
        ("--script", &actor.script),
    ] {
        if let Some(path) = path {
            command.arg(flag).arg(path);
        }
    }
    for (flag, text) in [
        ("--login", &actor.login),
        ("--model", &actor.model),
        ("--goal", &actor.goal),
        ("--role", &actor.role),
    ] {
        if let Some(text) = text {
            command.arg(flag).arg(text);
        }
    }
    if actor.test_controls {
        command.arg("--test-controls");
    }
    let mut child = command.spawn()?;
    let input = child.stdin.take();
    let stdout = child.stdout.take().ok_or("missing child stdout")?;
    let name = actor.name.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        loop {
            let event = match lines.next_line().await {
                Ok(Some(line)) if line.len() <= 8_000_000 => {
                    match serde_json::from_str::<Value>(&line) {
                        Ok(event) => event,
                        Err(_) => {
                            json!({"kind":"process-error","data":"bot produced invalid JSON event output"})
                        }
                    }
                }
                Ok(None) => {
                    let _ = sender
                        .send(Incoming {
                            actor: name,
                            event: json!({"kind":"process-eof","data":null}),
                        })
                        .await;
                    break;
                }
                _ => {
                    json!({"kind":"process-error","data":"could not read bounded bot event output"})
                }
            };
            let failure = event["kind"] == "process-error";
            if sender
                .send(Incoming {
                    actor: name.clone(),
                    event,
                })
                .await
                .is_err()
                || failure
            {
                break;
            }
        }
    });
    Ok(Player {
        child,
        input,
        ready: false,
        status: "connecting".into(),
        report,
    })
}

async fn control(players: &mut BTreeMap<String, Player>, actor: &str, line: &str) -> Result<()> {
    let player = players.get_mut(actor).ok_or("unknown actor")?;
    let input = player.input.as_mut().ok_or("actor input is closed")?;
    input.write_all(format!("{line}\n").as_bytes()).await?;
    input.flush().await?;
    Ok(())
}

async fn server_start(spec: &Server, dir: &Path) -> Result<Child> {
    let mut child = Command::new(&spec.program)
        .args(&spec.args)
        .current_dir(spec.working_directory.as_deref().unwrap_or(Path::new(".")))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(File::create(dir.join("server-stderr.txt"))?))
        .kill_on_drop(true)
        .spawn()?;
    let stdout = child.stdout.take().ok_or("server stdout missing")?;
    let mut lines = BufReader::new(stdout).lines();
    let mut file = File::create(dir.join("server-stdout.txt"))?;
    let marker = spec.ready_output.clone();
    tokio::time::timeout(Duration::from_millis(spec.timeout_ms), async {
        let mut bytes = 0;
        loop {
            let line = lines
                .next_line()
                .await?
                .ok_or("server exited before readiness")?;
            bytes += line.len();
            if bytes > 10_000_000 {
                return Err::<(), crate::Error>("server readiness output exceeded 10 MB".into());
            }
            writeln!(file, "{line}")?;
            if line.contains(&marker) {
                return Ok(());
            }
        }
    })
    .await
    .map_err(|_| "server readiness timed out")??;
    tokio::spawn(async move {
        let mut bytes = 0;
        while let Ok(Some(line)) = lines.next_line().await {
            bytes += line.len();
            if bytes <= 10_000_000 {
                let _ = writeln!(file, "{line}");
            }
        }
    });
    Ok(child)
}

fn debug_event(
    joined: &mut File,
    observer: Option<&Observer>,
    run: usize,
    at: Instant,
    event: Value,
) -> Result<()> {
    serde_json::to_writer(
        &mut *joined,
        &json!({"elapsed_ms":at.elapsed().as_millis(),"actor":"_harness","event":event}),
    )?;
    writeln!(joined)?;
    joined.flush()?;
    emit(
        observer,
        json!({"run":run,"actor":"_harness","event":event}),
    );
    Ok(())
}

fn debug_commands(
    controller: &mut Option<&mut DebugController>,
    debugger: &mut Option<DebugSession>,
    joined: &mut File,
    observer: Option<&Observer>,
    run: usize,
    at: Instant,
) -> Result<()> {
    let (Some(controller), Some(debugger)) = (controller.as_deref_mut(), debugger.as_mut()) else {
        return Ok(());
    };
    if controller.commands.is_closed() {
        return Err("harness interrupted".into());
    }
    debugger.refresh(Instant::now());
    if debugger.hold_expired(Instant::now()) {
        return Err("cast debugger hold timed out".into());
    }
    // A producer cannot starve SSH observations or failure checks by continually
    // replenishing the channel while it is drained.
    for _ in 0..32 {
        match controller.commands.try_recv() {
            Ok(command) => {
                let event = debugger.command(command, Instant::now())?;
                debug_event(joined, observer, run, at, event)?;
            }
            Err(mpsc::error::TryRecvError::Empty) => break,
            Err(mpsc::error::TryRecvError::Disconnected) => {
                return Err("harness interrupted".into());
            }
        }
    }
    Ok(())
}

fn debug_gate(
    debugger: &mut Option<DebugSession>,
    next_phase: Option<usize>,
    joined: &mut File,
    observer: Option<&Observer>,
    run: usize,
    at: Instant,
) -> Result<bool> {
    let Some(debugger) = debugger else {
        return Ok(true);
    };
    let (allowed, event) = debugger.gate(next_phase, Instant::now())?;
    if let Some(event) = event {
        debug_event(joined, observer, run, at, event)?;
    }
    Ok(allowed)
}

async fn one(
    manifest: &Manifest,
    actors: &[Actor],
    directory: &Path,
    observer: Option<&Observer>,
    mut debug: Option<&mut DebugController>,
    run_index: usize,
) -> Result<Value> {
    std::fs::create_dir(directory)?;
    let mut joined = File::create(directory.join("joined.jsonl"))?;
    let startup_at = Instant::now();
    let bot_program = debug.as_ref().and_then(|debug| debug.bot_program.clone());
    let mut debugger = debug
        .as_ref()
        .map(|debug| {
            DebugSession::new(
                &debug.settings,
                &debug.state,
                run_index,
                manifest.steps.len(),
            )
        })
        .transpose()?;
    if let Some(debugger) = &debugger {
        debug_event(
            &mut joined,
            observer,
            run_index,
            startup_at,
            debugger.event("run-start", None, None, "run_start"),
        )?;
    }
    // Freeze each rubric for all actors in this run, before any process starts.
    let mut prepared = manifest.clone();
    for (index, step) in prepared.steps.iter_mut().enumerate() {
        if let Some(path) = &step.probe {
            let spec = crate::probe::Spec::load(path)?;
            let frozen = directory.join(format!("probe-{}.json", index + 1));
            save_json(&frozen, &spec)?;
            step.probe = Some(frozen);
        }
    }
    let manifest = &prepared;
    save_json(&directory.join("cast.json"), &actors)?;
    let mut server = if let Some(spec) = &manifest.server {
        let starting = server_start(spec, directory);
        tokio::pin!(starting);
        let server = loop {
            if observer.is_some_and(Observer::stopped) {
                return Err("harness interrupted".into());
            }
            debug_commands(
                &mut debug,
                &mut debugger,
                &mut joined,
                observer,
                run_index,
                startup_at,
            )?;
            tokio::select! {
                server = &mut starting => break server?,
                _ = tokio::signal::ctrl_c() => return Err("harness interrupted".into()),
                _ = tokio::time::sleep(Duration::from_millis(25)) => {},
            }
        };
        Some(server)
    } else {
        None
    };
    let (sender, mut receiver) = mpsc::channel(256);
    let mut players = BTreeMap::new();
    for actor in actors {
        players.insert(
            actor.name.clone(),
            spawn_actor(
                actor,
                &directory.join(&actor.name),
                sender.clone(),
                &crate::organization::Affiliations::for_actor(&actor.name, &manifest.groups)?,
                bot_program.as_deref(),
            )
            .await?,
        );
    }
    drop(sender);
    let mut monitor = Monitor::new(manifest.assertions.clone())?;
    let mut probes = ProbeLog::default();
    let mut steps = Vec::new();
    let mut step: Option<ActiveStep> = None;
    let mut started = false;
    let mut closing = false;
    let mut close_at = None;
    let mut status_before_shutdown = BTreeMap::new();
    let at = Instant::now();
    let mut error = None;
    let mut eof = std::collections::BTreeSet::new();
    loop {
        if observer.is_some_and(Observer::stopped) {
            error = Some("harness interrupted".into());
            break;
        }
        if monitor.failed() {
            error = Some("a forbidden event was observed".into());
        }
        if probes.failed {
            error = Some("a structured probe failed its rubric".into());
        }
        if error.is_some() {
            break;
        }
        if let Err(e) = debug_commands(
            &mut debug,
            &mut debugger,
            &mut joined,
            observer,
            run_index,
            startup_at,
        ) {
            error = Some(e.to_string());
            break;
        }
        let held_duration = debugger.as_ref().map_or(Duration::ZERO, |debugger| {
            debugger.held_duration(Instant::now())
        });
        if at.elapsed().saturating_sub(held_duration) >= Duration::from_millis(manifest.timeout_ms)
        {
            error = Some("harness run timed out".to_owned());
            break;
        }
        if debugger
            .as_ref()
            .is_some_and(|debugger| debugger.hold_expired(Instant::now()))
        {
            error = Some("cast debugger hold timed out".into());
            break;
        }
        if let Some(server) = &mut server
            && server.try_wait()?.is_some()
        {
            error = Some("server-under-test exited during the run".into());
            break;
        }
        if !started
            && players.values().all(|p| p.ready)
            && debug_gate(
                &mut debugger,
                (!manifest.steps.is_empty()).then_some(1),
                &mut joined,
                observer,
                run_index,
                startup_at,
            )?
        {
            started = true;
            for actor in actors {
                if let Some(start) = &actor.start
                    && let Err(e) = control(&mut players, &actor.name, start).await
                {
                    error = Some(e.to_string());
                    break;
                }
            }
        }
        if started
            && step.is_none()
            && steps.len() < manifest.steps.len()
            && error.is_none()
            && debug_gate(
                &mut debugger,
                Some(steps.len() + 1),
                &mut joined,
                observer,
                run_index,
                startup_at,
            )?
        {
            let spec = &manifest.steps[steps.len()];
            let targets = spec.targets(manifest)?;
            let id = format!("phase-{}", steps.len() + 1);
            step = Some(ActiveStep {
                monitor: Monitor::new(spec.contract(manifest)?)?,
                since: Instant::now(),
                id: id.clone(),
                targets: targets.iter().cloned().collect(),
                opened: Default::default(),
                applied: Default::default(),
                queued: targets.iter().cloned().collect(),
                last_dispatched: None,
            });
            serde_json::to_writer(
                &mut joined,
                &json!({"elapsed_ms":startup_at.elapsed().as_millis(),"actor":"_harness",
                "event":{"kind":"phase-start","data":{"id":id,"name":spec.name,"targets":targets,"control":spec.control_line(),"probe":spec.probe,"dispatch":spec.dispatch}}}),
            )?;
            writeln!(joined)?;
            emit(
                observer,
                json!({"run":run_index,"kind":"phase","data":{"id":id,"name":spec.name,"dispatch":spec.dispatch}}),
            );
            if let Some(debugger) = &mut debugger {
                let event = debugger.phase_started(steps.len() + 1)?;
                debug_event(&mut joined, observer, run_index, startup_at, event)?;
            }
        }
        if let Some(active) = &mut step {
            let spec = &manifest.steps[steps.len()];
            while !active.queued.is_empty()
                && !active.monitor.failed()
                && error.is_none()
                && (spec.dispatch == Dispatch::Parallel
                    || active.last_dispatched.as_ref().is_none_or(|actor| {
                        active.applied.contains(actor) && active.monitor.actor_satisfied(actor)
                    }))
            {
                let actor = active.queued.pop_front().unwrap();
                let id = &active.id;
                let command = format!("/harness {id} {}", spec.control_line());
                if let Err(e) = control(&mut players, &actor, &command).await {
                    error = Some(e.to_string());
                    break;
                }
                active.last_dispatched = Some(actor);
                if spec.dispatch == Dispatch::Serial {
                    break;
                }
            }
        }
        if let Some(active) = &step {
            if active.monitor.failed() {
                error = Some(format!(
                    "harness step {} observed forbidden output",
                    steps.len() + 1
                ));
            } else if active.satisfied() {
                serde_json::to_writer(
                    &mut joined,
                    &json!({"elapsed_ms":startup_at.elapsed().as_millis(),"actor":"_harness",
                    "event":{"kind":"phase-complete","data":active.report()}}),
                )?;
                writeln!(joined)?;
                steps.push(active.report());
                step = None;
            } else if active.since.elapsed()
                >= Duration::from_millis(manifest.steps[steps.len()].timeout_ms)
            {
                error = Some(format!("harness step {} timed out", steps.len() + 1));
            }
        }
        if monitor.failed() {
            error = Some("a forbidden event was observed".into());
        }
        if probes.failed {
            error = Some("a structured probe failed its rubric".into());
        }
        if error.is_some() {
            break;
        }
        let final_boundary_open =
            if started && !closing && step.is_none() && steps.len() == manifest.steps.len() {
                debug_gate(
                    &mut debugger,
                    None,
                    &mut joined,
                    observer,
                    run_index,
                    startup_at,
                )?
            } else {
                true
            };
        if started
            && !closing
            && final_boundary_open
            && steps.len() == manifest.steps.len()
            && monitor.satisfied()
            && actors
                .iter()
                .all(|a| players[&a.name].status == a.expected_status)
        {
            closing = true;
            if let Some(debugger) = &mut debugger {
                let event = debugger.begin_cleanup(Instant::now())?;
                debug_event(&mut joined, observer, run_index, startup_at, event)?;
            }
            close_at = Some(Instant::now());
            status_before_shutdown = players
                .iter()
                .map(|(name, p)| (name.clone(), p.status.clone()))
                .collect();
            for actor in actors {
                if actor.script.is_none() && !eof.contains(&actor.name) {
                    let _ = control(&mut players, &actor.name, "/quit").await;
                }
            }
        }
        if closing && eof.len() == players.len() {
            break;
        }
        if close_at.is_some_and(|a| a.elapsed() > Duration::from_secs(10)) {
            error = Some("cast did not shut down within 10 seconds".into());
            break;
        }
        let incoming = tokio::select! {
            _=tokio::signal::ctrl_c()=>{error=Some("harness interrupted".into());break;},
            event=receiver.recv()=>event,
            _=tokio::time::sleep(Duration::from_millis(25))=>continue,
        };
        let Some(Incoming { actor, event }) = incoming else {
            error = Some("cast event stream ended unexpectedly".into());
            break;
        };
        emit(
            observer,
            json!({"run":run_index,"actor":actor,"event":event}),
        );
        serde_json::to_writer(
            &mut joined,
            &json!({"elapsed_ms":startup_at.elapsed().as_millis(),"actor":actor,"event":event}),
        )?;
        writeln!(joined)?;
        joined.flush()?;
        monitor.feed(&actor, &event);
        probes.feed(&actor, &event);
        if let Some(active) = &mut step {
            active.feed(&actor, &event);
        }
        let player = players.get_mut(&actor).ok_or("event from unknown actor")?;
        match event["kind"].as_str().unwrap_or("") {
            "ready" => {
                player.ready = true;
                player.status = event["data"]["state"]["status"]
                    .as_str()
                    .unwrap_or("paused")
                    .into();
            }
            "snapshot" => {
                player.status = event["data"]["state"]["status"]
                    .as_str()
                    .unwrap_or("unknown")
                    .into();
            }
            "done" | "blocked" | "error" => player.status = event["kind"].as_str().unwrap().into(),
            "result" => {
                player.status = event["data"]["state"]["status"]
                    .as_str()
                    .unwrap_or("unknown")
                    .into()
            }
            "process-error" | "control-error" => {
                error = Some(format!("{actor}: {}", event["data"]))
            }
            "process-eof" => {
                eof.insert(actor.clone());
                if !closing
                    && !actors.iter().any(|a| {
                        a.name == actor
                            && (a.script.is_some() || a.expected_status == "error")
                            && a.expected_status == player.status
                    })
                {
                    error = Some(format!("{actor} exited before the cast finished"));
                }
            }
            _ => {}
        }
    }
    // A failing or interrupted held run leaves the hold before cleanup. Failure
    // handling keeps draining actor output and never grants the next phase.
    if !closing && let Some(debugger) = &mut debugger {
        let event = debugger.begin_cleanup(Instant::now())?;
        debug_event(&mut joined, observer, run_index, startup_at, event)?;
    }
    if !closing || error.is_some() {
        // Ask owned runners to persist cancellation and their final reports before
        // resorting to termination. Drain stdout while they finish so a full pipe
        // cannot prevent shutdown or lose the final receipt.
        for actor in actors {
            if actor.script.is_none() && !eof.contains(&actor.name) {
                let _ = tokio::time::timeout(
                    Duration::from_secs(1),
                    control(&mut players, &actor.name, "/quit"),
                )
                .await;
            }
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        // Process exit does not imply its stdout reader has delivered buffered
        // grades and receipts. Each reader sends EOF after its final event.
        while Instant::now() < deadline && eof.len() < players.len() {
            let incoming =
                match tokio::time::timeout(Duration::from_millis(25), receiver.recv()).await {
                    Ok(Some(incoming)) => incoming,
                    Ok(None) => break,
                    Err(_) => continue,
                };
            let Incoming { actor, event } = incoming;
            emit(
                observer,
                json!({"run":run_index,"actor":actor,"event":event}),
            );
            serde_json::to_writer(
                &mut joined,
                &json!({"elapsed_ms":startup_at.elapsed().as_millis(),"actor":actor,"event":event}),
            )?;
            writeln!(joined)?;
            monitor.feed(&actor, &event);
            probes.feed(&actor, &event);
            if event["kind"] == "process-eof" {
                eof.insert(actor);
            }
        }
    }
    let mut reports = BTreeMap::new();
    for (name, mut player) in players {
        if player.child.try_wait()?.is_none() {
            let _ = player.child.kill().await;
        }
        let exit = tokio::time::timeout(Duration::from_secs(5), player.child.wait()).await;
        let exit_code = exit.ok().and_then(|s| s.ok()).and_then(|s| s.code());
        let report = if player.report.exists() {
            load_json::<Value>(&player.report, 8_000_000).ok()
        } else {
            None
        };
        let spec = actors.iter().find(|a| a.name == name).unwrap();
        let expected = &spec.expected_status;
        let expected_exit = if expected == "error" {
            1
        } else if spec.script.is_some() && expected == "blocked" {
            2
        } else {
            0
        };
        let valid = report.as_ref().is_some_and(|r| {
            r["state"]["status"] == expected.as_str()
                || (expected == "listening"
                    && status_before_shutdown.get(&name).map(String::as_str) == Some("listening")
                    && r["state"]["status"] == "paused")
        }) && exit_code == Some(expected_exit);
        if !valid && error.is_none() {
            error = Some(format!(
                "{name} did not finish with expected status {expected}"
            ));
        }
        let before = status_before_shutdown.get(&name);
        reports.insert(
            name,
            json!({"exit_code":exit_code,"expected_status":expected,"status_before_shutdown":before,"report":report}),
        );
    }
    if let Some(mut server) = server {
        let _ = server.kill().await;
        let _ = server.wait().await;
    }
    let mut cast_costs = crate::costs::Breakdown::default();
    let mut group_costs: BTreeMap<String, crate::costs::Breakdown> = manifest
        .groups
        .iter()
        .map(|g| (g.id.clone(), Default::default()))
        .collect();
    let mut missing_costs = Vec::new();
    for (actor, result) in &reports {
        if result["report"]["costs"]["session"].is_null() {
            missing_costs.push(actor.clone());
            continue;
        }
        let session = serde_json::from_value(result["report"]["costs"]["session"].clone())?;
        cast_costs.merge(&session)?;
        for group in &manifest.groups {
            if group.members.iter().any(|m| &m.actor == actor) {
                group_costs.get_mut(&group.id).unwrap().merge(&session)?;
            }
        }
    }
    if let Some(debugger) = &mut debugger {
        let event = debugger.finish(
            Instant::now(),
            error.is_some() || !monitor.satisfied() || probes.failed,
        )?;
        debug_event(&mut joined, observer, run_index, startup_at, event)?;
    }
    let elapsed = at.elapsed();
    let held = debugger.as_ref().map_or(Duration::ZERO, |debugger| {
        debugger.held_duration(Instant::now())
    });
    let debug_report = debugger.as_ref().map_or_else(
        || json!({"enabled":false,"held_ms":0}),
        |debugger| debugger.report(Instant::now(), bot_program.as_deref()),
    );
    Ok(
        json!({"success":error.is_none() && monitor.satisfied() && !probes.failed,"error":error,"elapsed_ms":elapsed.as_millis(),
        "active_elapsed_ms":elapsed.saturating_sub(held).as_millis(),"debug":debug_report,
        "probes":probes.entries,"probes_omitted":probes.omitted,
        "costs":{"cast":cast_costs,"by_group":group_costs,"missing_actor_reports":missing_costs,
            "group_totals_overlap":true,"scope":"This run's actor sessions only; lifetime ledgers are not summed."},
        "assertions":monitor.report(),"steps":steps,"pending_step":step.map(|s| s.report()),"groups":manifest.groups.iter().map(|g| json!({"id":g.id,"name":g.name,"kind":g.kind,"members":g.members.iter().map(|m| &m.actor).collect::<Vec<_>>()})).collect::<Vec<_>>(),"actors":reports}),
    )
}

pub async fn run(manifest: Manifest, directory: &Path) -> Result<Value> {
    run_observed(manifest, directory, None).await
}

pub async fn run_observed(
    manifest: Manifest,
    directory: &Path,
    observer: Option<Observer>,
) -> Result<Value> {
    run_inner(manifest, directory, observer, None).await
}

pub async fn run_debugged(
    manifest: Manifest,
    directory: &Path,
    observer: Option<Observer>,
    controller: DebugController,
) -> Result<Value> {
    run_inner(manifest, directory, observer, Some(controller)).await
}

async fn run_inner(
    manifest: Manifest,
    directory: &Path,
    observer: Option<Observer>,
    mut debug: Option<DebugController>,
) -> Result<Value> {
    manifest.validate()?;
    if let Some(debug) = &mut debug {
        debug.settings.validate_phases(manifest.steps.len())?;
        let program = match &debug.bot_program {
            Some(program) => std::fs::canonicalize(program)?,
            None => std::env::current_exe()?,
        };
        if !program.is_file() {
            return Err("cast debugger bot_program must name a bot executable file".into());
        }
        debug.bot_program = Some(program);
    }
    // A new directory protects earlier artifacts, including traces from failed runs.
    std::fs::create_dir(directory)?;
    let directory = std::fs::canonicalize(directory)?;
    save_json(&directory.join("manifest.json"), &manifest)?;
    if let Some(debug) = &debug {
        save_json(
            &directory.join("cast-debug.json"),
            &json!({"settings":debug.settings,"bot_program":debug.bot_program}),
        )?;
    }
    let mut reports = Vec::new();
    for (index, (actors, repetition)) in manifest.runs().into_iter().enumerate() {
        if observer.is_none() {
            println!("[harness] Run {} ({})", index + 1, manifest.name);
        }
        emit(
            observer.as_ref(),
            json!({"run":index+1,"kind":"cast-start","data":{"name":manifest.name,"run":index+1,
            "actors":actors.iter().map(|a| &a.name).collect::<Vec<_>>()}}),
        );
        let dir = directory.join(format!("run-{:04}", index + 1));
        let report = match one(
            &manifest,
            &actors,
            &dir,
            observer.as_ref(),
            debug.as_mut(),
            index + 1,
        )
        .await
        {
            Ok(report) => report,
            Err(error) => {
                let debug_report = if let Some(debug) = &debug {
                    let mut state = debug.state.borrow().clone();
                    state.mode = crate::cast_debug::Mode::Finished;
                    state.notice = "run_failed";
                    state.revision = state.revision.saturating_add(1);
                    debug.state.send_replace(state);
                    json!({"enabled":true,"held_ms":null,"settings":debug.settings,"bot_program":debug.bot_program})
                } else {
                    json!({"enabled":false,"held_ms":0})
                };
                json!({"success":false,"error":error.to_string(),"active_elapsed_ms":null,"debug":debug_report})
            }
        };
        save_json(&dir.join("run.json"), &report)?;
        let interrupted = report["error"] == "harness interrupted";
        let mut brief = report;
        if let Some(actors) = brief["actors"].as_object_mut() {
            for actor in actors.values_mut() {
                if !actor["report"].is_null() {
                    let report = &actor["report"];
                    actor["report"] = json!({"state":{"status":report["state"]["status"],"model_calls":report["state"]["model_calls"],"commands_sent":report["state"]["commands_sent"]},
                        "model":report["model"],"simulated_model":report["simulated_model"],"memory_store":report["memory_store"],"costs":{"session":report["costs"]["session"]}});
                }
            }
        }
        reports.push(json!({"index":index+1,"repetition":repetition,"cast":actors,"report":dir.join("run.json"),"result":brief}));
        if interrupted {
            break;
        }
    }
    let report = json!({"version":1,"name":manifest.name,"success":reports.iter().all(|r| r["result"]["success"]==true),"runs":reports});
    save_json(&directory.join("summary.json"), &report)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_evaluations_cannot_be_started_as_unawaited_raw_controls() {
        for control in [
            "/probe rubric.json",
            " /probe rubric.json",
            "/harness outer /probe rubric.json",
        ] {
            assert!(validate_control(control).is_err());
        }
        assert!(validate_control_line("/probe path with spaces/rubric.json").is_ok());
        assert!(validate_control("/ask What did you observe?").is_ok());
        let mut probes = ProbeLog {
            entries: vec![Value::Null; 4096],
            ..Default::default()
        };
        probes.feed(
            "scout",
            &json!({"kind":"probe-result","data":{"success":false}}),
        );
        assert!(
            probes.failed,
            "omitted failed grades must still fail the run"
        );
        assert_eq!(probes.omitted, 1);
    }

    #[test]
    fn phase_fences_exclude_queued_events_and_wait_for_every_acknowledgment() {
        let contract = serde_json::from_value(json!({"eventually":[
            {"actor":"scout","kind":"observation","contains":"Rendezvous"},
            {"actor":"medic","kind":"observation","contains":"Rendezvous"}]}))
        .unwrap();
        let mut step = ActiveStep {
            monitor: Monitor::new(contract).unwrap(),
            since: Instant::now(),
            id: "phase-2".into(),
            targets: ["scout".into(), "medic".into()].into(),
            opened: Default::default(),
            applied: Default::default(),
            queued: Default::default(),
            last_dispatched: None,
        };
        let seen = json!({"kind":"observation","data":"Rendezvous","seq":10});
        for actor in ["scout", "medic"] {
            step.feed(actor, &seen); // Old phase's queued output cannot satisfy this one.
            step.feed(
                actor,
                &json!({"kind":"control-started","data":{"id":"phase-2"}}),
            );
        }
        assert!(!step.monitor.satisfied());
        for actor in ["scout", "medic"] {
            step.feed(actor, &seen);
        }
        assert!(step.monitor.satisfied());
        assert!(!step.satisfied());
        step.feed(
            "scout",
            &json!({"kind":"control-applied","data":{"id":"phase-2"}}),
        );
        assert!(!step.satisfied());
        step.feed(
            "medic",
            &json!({"kind":"control-applied","data":{"id":"phase-1"}}),
        );
        assert!(!step.satisfied());
        step.feed(
            "medic",
            &json!({"kind":"control-applied","data":{"id":"phase-2"}}),
        );
        assert!(step.satisfied());
    }
}
