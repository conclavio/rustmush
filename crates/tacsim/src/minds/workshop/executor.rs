//! Dedicated bounded authoring executor. No live pool, world, process execution
//! cache or persistence handle crosses this boundary. Pending results count
//! against the same slots as queued and running work.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use mush_core::{AccountId, ObjId};
use mush_roll::{
    EditBase, Envelope, PreparedSave, RevisionAdapter, RollError, RollStore, SourceAdapter,
    WorkingCopy,
};

use super::invalid;
use mush_roll::commerce::authored_access::{AuthoredDraftAccess, AuthoredJobKind};

pub(crate) const WORKERS: usize = 2;
pub(crate) const MAX_JOBS: usize = 8;
const MAX_SESSIONS: usize = 4096;
const RESULT_TTL: Duration = Duration::from_secs(120);

/// Current authenticated character and account plus an ephemeral session lease.
/// The constructor is available only to the world command adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Principal {
    actor: ObjId,
    account: AccountId,
    session: u64,
}
impl Principal {
    pub(crate) fn actor(self) -> ObjId {
        self.actor
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct JobId(u64);
impl JobId {
    pub(crate) fn number(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Picture {
    Empty,
    Hostile450,
    HurtTwo,
}
pub(crate) const PICTURES: [(Picture, bool); 6] = [
    (Picture::Empty, false),
    (Picture::Empty, true),
    (Picture::Hostile450, false),
    (Picture::Hostile450, true),
    (Picture::HurtTwo, false),
    (Picture::HurtTwo, true),
];

/// Only bounded observations of the isolated think are returned; memory and
/// source are never returned through a result or a diagnostic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PictureReport {
    pub picture: Picture,
    pub armed: bool,
    pub operations: u64,
    pub fees: u64,
    pub intents: Vec<String>,
    pub fault: Option<String>,
    pub station_refusals: Vec<String>,
    pub class_violations: Vec<String>,
}

/// The concrete provider constructs only the six content-backed private
/// pictures, uses the declared class row and a disposable bounded sandbox, and
/// returns exactly one report for each PICTURES entry in order. It is installed
/// at production composition, never selected by a request or a source script.
pub(crate) trait CheckPictures: Send + Sync {
    fn run(&self, normalized: &WorkingCopy) -> Result<Vec<PictureReport>, RollError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Diagnostic {
    pub field: String,
    pub message: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

#[derive(Clone, Debug)]
pub(crate) enum JobKind {
    Check,
    Save {
        notes: String,
        version: Option<String>,
    },
}

impl JobKind {
    pub(super) fn authored_kind(&self) -> AuthoredJobKind {
        match self {
            Self::Check => AuthoredJobKind::Check,
            Self::Save { .. } => AuthoredJobKind::Save,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DraftOrigin {
    Original,
    Authored,
}

struct Binding {
    origin: DraftOrigin,
    principal: Principal,
    name: String,
    base: EditBase,
    original: WorkingCopy,
}
struct Request {
    binding: Binding,
    kind: JobKind,
}

/// Nonserializable, nonconstructible outside this worker module. The receipt
/// binds the exact draft, base, actor, session and complete canonical envelope.
pub(crate) struct ValidatedDraft {
    binding: Binding,
    normalized: String,
}

impl ValidatedDraft {
    /// No compilation on the world pulse. Equality against the sealed receipt
    /// is the only adapter path; no transport can supply a custom validator.
    pub(crate) fn prepare_save<S: SourceAdapter>(
        self,
        principal: Principal,
        roll: &RollStore,
        sources: &S,
        notes: &str,
        version: Option<&str>,
    ) -> Result<PreparedSave<S::Token>, RollError> {
        if principal != self.binding.principal || self.binding.origin != DraftOrigin::Original {
            return Err(RollError::Missing);
        }
        let mind = super::authoring_mind(roll, principal.actor, &self.binding.name)?;
        self.prepare_exact(principal, roll, sources, notes, version, mind)
    }
    pub(super) fn prepare_authored_save<S: SourceAdapter>(
        self,
        principal: Principal,
        access: &AuthoredDraftAccess,
        roll: &RollStore,
        sources: &S,
        notes: &str,
        version: Option<&str>,
    ) -> Result<PreparedSave<S::Token>, RollError> {
        if principal != self.binding.principal
            || self.binding.origin != DraftOrigin::Authored
            || access.actor() != principal.actor
            || access.job_kind() != Some(AuthoredJobKind::Save)
        {
            return Err(RollError::Missing);
        }
        let mind = roll.mind(principal.actor, &self.binding.name)?;
        if mind != access.mind() {
            return Err(RollError::StaleEdit);
        }
        self.prepare_exact(principal, roll, sources, notes, version, mind)
    }
    fn prepare_exact<S: SourceAdapter>(
        self,
        principal: Principal,
        roll: &RollStore,
        sources: &S,
        notes: &str,
        version: Option<&str>,
        mind: &mush_roll::Mind,
    ) -> Result<PreparedSave<S::Token>, RollError> {
        if mind.base() != self.binding.base || mind.draft() != &self.binding.original {
            return Err(RollError::StaleEdit);
        }
        let adapter = ReceiptAdapter {
            original: &self.binding.original,
            normalized: &self.normalized,
        };
        roll.prepare_save(
            principal.actor,
            &self.binding.name,
            self.binding.base,
            notes,
            version,
            &adapter,
            sources,
        )
    }
}

struct ReceiptAdapter<'a> {
    original: &'a WorkingCopy,
    normalized: &'a str,
}
impl RevisionAdapter for ReceiptAdapter<'_> {
    fn normalize_source(&self, source: &str) -> Result<String, RollError> {
        if source == self.original.source || source == self.normalized {
            Ok(self.normalized.to_owned())
        } else {
            Err(RollError::StaleEdit)
        }
    }
    fn validate(&self, source: &str, envelope: &Envelope) -> Result<(), RollError> {
        if source == self.normalized && envelope == &self.original.envelope {
            Ok(())
        } else {
            Err(RollError::StaleEdit)
        }
    }
}

pub(crate) struct Completion {
    expected: (Principal, String, EditBase, DraftOrigin),
    pub kind: JobKind,
    pub reports: Vec<PictureReport>,
    pub diagnostic: Option<Diagnostic>,
    pub validated: Option<ValidatedDraft>,
}

impl Completion {
    pub(super) fn check_authored(
        &self,
        principal: Principal,
        access: &AuthoredDraftAccess,
    ) -> Result<(), RollError> {
        if self.expected.0 != principal
            || self.expected.3 != DraftOrigin::Authored
            || access.actor() != principal.actor
            || access.job_kind() != Some(self.kind.authored_kind())
        {
            return Err(RollError::Missing);
        }
        if access.mind().name() != self.expected.1 || access.mind().base() != self.expected.2 {
            return Err(RollError::StaleEdit);
        }
        Ok(())
    }
    pub(crate) fn check_current(
        &self,
        principal: Principal,
        roll: &RollStore,
    ) -> Result<(), RollError> {
        if self.expected.0 != principal || self.expected.3 != DraftOrigin::Original {
            return Err(RollError::Missing);
        }
        if super::authoring_mind(roll, principal.actor, &self.expected.1)?.base() != self.expected.2
        {
            return Err(RollError::StaleEdit);
        }
        Ok(())
    }
}

enum Status {
    Queued(Option<Box<Request>>),
    Running,
    Done(Option<Box<Completion>>),
    CancelledRunning,
}
struct Entry {
    principal: Principal,
    started: Instant,
    status: Status,
}
#[derive(Default)]
struct State {
    stopped: bool,
    next: u64,
    sessions: BTreeMap<ObjId, Principal>,
    queue: VecDeque<JobId>,
    jobs: BTreeMap<JobId, Entry>,
}
struct Shared {
    state: Mutex<State>,
    ready: Condvar,
    pictures: Arc<dyn CheckPictures>,
}

/// Cheap cloneable process service. The final service handle signals shutdown;
/// already running bounded work finishes and drops its data without re-entry.
#[derive(Clone)]
pub(crate) struct WorkshopExecutor {
    shared: Arc<Shared>,
    _lifetime: Arc<Stop>,
}
struct Stop(Arc<Shared>);
impl Drop for Stop {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.state.lock() {
            state.stopped = true;
            state.queue.clear();
            state.jobs.retain(|_, entry| {
                matches!(entry.status, Status::Running | Status::CancelledRunning)
            });
        }
        self.0.ready.notify_all();
    }
}

impl WorkshopExecutor {
    pub(crate) fn new(pictures: Arc<dyn CheckPictures>) -> Result<Self, RollError> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            ready: Condvar::new(),
            pictures,
        });
        let service = Self {
            shared: shared.clone(),
            _lifetime: Arc::new(Stop(shared.clone())),
        };
        for index in 0..WORKERS {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name(format!("mind-author-{index}"))
                .spawn(move || worker(&shared))
                .map_err(|_| invalid("workshop", "cannot start authoring worker"))?;
        }
        Ok(service)
    }

    /// Called only after the world adapter checked its account oracle and live
    /// connection. Linkdead/disconnected hooks MUST call cancel_character.
    pub(super) fn principal(
        &self,
        actor: ObjId,
        account: AccountId,
    ) -> Result<Principal, RollError> {
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| invalid("workshop", "executor unavailable"))?;
        if state.stopped {
            return Err(invalid("workshop", "executor stopped"));
        }
        if let Some(principal) = state.sessions.get(&actor) {
            if principal.account == account {
                return Ok(*principal);
            }
            return Err(RollError::Missing);
        }
        if state.sessions.len() >= MAX_SESSIONS {
            return Err(invalid("workshop", "session capacity reached"));
        }
        state.next = state
            .next
            .checked_add(1)
            .ok_or(RollError::Exhausted("author session"))?;
        let principal = Principal {
            actor,
            account,
            session: state.next,
        };
        state.sessions.insert(actor, principal);
        Ok(principal)
    }

    pub(crate) fn submit(
        &self,
        principal: Principal,
        roll: &RollStore,
        name: &str,
        base: EditBase,
        kind: JobKind,
    ) -> Result<JobId, RollError> {
        let mind = super::authoring_mind(roll, principal.actor, name)?;
        self.submit_exact(principal, mind, base, kind, DraftOrigin::Original)
    }

    pub(super) fn submit_authored(
        &self,
        principal: Principal,
        access: &AuthoredDraftAccess,
        base: EditBase,
        kind: JobKind,
    ) -> Result<JobId, RollError> {
        if access.actor() != principal.actor || access.job_kind() != Some(kind.authored_kind()) {
            return Err(RollError::Missing);
        }
        self.submit_exact(principal, access.mind(), base, kind, DraftOrigin::Authored)
    }

    fn submit_exact(
        &self,
        principal: Principal,
        mind: &mush_roll::Mind,
        base: EditBase,
        kind: JobKind,
        origin: DraftOrigin,
    ) -> Result<JobId, RollError> {
        if mind.base() != base {
            return Err(RollError::StaleEdit);
        }
        mind.draft().validate_bounds()?;
        if let JobKind::Save { notes, version } = &kind {
            if notes.len() > mush_roll::MAX_NOTES_BYTES || notes.contains('\0') {
                return Err(invalid("notes", "maximum 2 KiB; NUL is forbidden"));
            }
            if let Some(version) = version {
                mush_roll::canonical_name(version)?;
            }
        }
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| invalid("workshop", "executor unavailable"))?;
        reap(&mut state);
        if state.stopped || state.sessions.get(&principal.actor) != Some(&principal) {
            return Err(RollError::Missing);
        }
        if state.jobs.len() >= MAX_JOBS {
            return Err(invalid(
                "workshop",
                "authoring queue is full; try again later",
            ));
        }
        if state
            .jobs
            .values()
            .any(|entry| entry.principal.account == principal.account)
        {
            return Err(invalid(
                "workshop",
                "one outstanding author job per account",
            ));
        }
        state.next = state
            .next
            .checked_add(1)
            .ok_or(RollError::Exhausted("author job"))?;
        let id = JobId(state.next);
        let request = Request {
            binding: Binding {
                origin,
                principal,
                name: mind.name().to_owned(),
                base,
                original: mind.draft().clone(),
            },
            kind,
        };
        state.jobs.insert(
            id,
            Entry {
                principal,
                started: Instant::now(),
                status: Status::Queued(Some(Box::new(request))),
            },
        );
        state.queue.push_back(id);
        self.shared.ready.notify_one();
        Ok(id)
    }

    /// One-shot read. Expired, foreign or stale-session handles all answer the
    /// same absence; no job existence or source diagnostic crosses characters.
    pub(crate) fn take(
        &self,
        principal: Principal,
        id: JobId,
    ) -> Result<Option<Completion>, RollError> {
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| invalid("workshop", "executor unavailable"))?;
        reap(&mut state);
        if state.sessions.get(&principal.actor) != Some(&principal) {
            return Err(RollError::Missing);
        }
        let entry = state
            .jobs
            .get_mut(&id)
            .filter(|entry| entry.principal == principal)
            .ok_or(RollError::Missing)?;
        let Status::Done(completion) = &mut entry.status else {
            return Ok(None);
        };
        let completion = completion.take().map(|completion| *completion);
        state.jobs.remove(&id);
        Ok(completion)
    }

    /// Poll at most eight ids per world tick; returning an id is not consuming
    /// a result. The world re-authorizes the corresponding character first.
    pub(crate) fn ready(&self) -> Vec<(ObjId, JobId)> {
        let Ok(mut state) = self.shared.state.lock() else {
            return Vec::new();
        };
        reap(&mut state);
        state
            .jobs
            .iter()
            .filter_map(|(id, entry)| {
                matches!(entry.status, Status::Done(_)).then_some((entry.principal.actor, *id))
            })
            .collect()
    }

    /// Cancellation is scoped to the exact authenticated worker session.
    /// Running jobs retain their account/global slot until the worker exits.
    pub(crate) fn cancel(&self, principal: Principal, id: JobId) -> Result<(), RollError> {
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| invalid("workshop", "executor unavailable"))?;
        reap(&mut state);
        let entry = state
            .jobs
            .get_mut(&id)
            .filter(|entry| entry.principal == principal)
            .ok_or(RollError::Missing)?;
        if matches!(entry.status, Status::Running | Status::CancelledRunning) {
            entry.status = Status::CancelledRunning;
        } else {
            state.jobs.remove(&id);
        }
        state.queue.retain(|queued| *queued != id);
        Ok(())
    }

    pub(crate) fn cancel_character(&self, actor: ObjId) {
        let Ok(mut state) = self.shared.state.lock() else {
            return;
        };
        state.sessions.remove(&actor);
        state.jobs.retain(|_, entry| {
            if entry.principal.actor != actor {
                return true;
            }
            if matches!(entry.status, Status::Running | Status::CancelledRunning) {
                entry.status = Status::CancelledRunning;
                true
            } else {
                false
            }
        });
        let remaining = state
            .jobs
            .keys()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        state.queue.retain(|id| remaining.contains(id));
    }
}

fn reap(state: &mut State) {
    state.jobs.retain(|_, entry| {
        if entry.started.elapsed() <= RESULT_TTL {
            return true;
        }
        if matches!(entry.status, Status::Running | Status::CancelledRunning) {
            entry.status = Status::CancelledRunning;
            true
        } else {
            false
        }
    });
    let remaining = state
        .jobs
        .keys()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    state.queue.retain(|id| remaining.contains(id));
}

fn worker(shared: &Shared) {
    loop {
        let job = {
            let Ok(mut state) = shared.state.lock() else {
                return;
            };
            loop {
                if state.stopped {
                    return;
                }
                if let Some(id) = state.queue.pop_front() {
                    if let Some(entry) = state.jobs.get_mut(&id)
                        && let Status::Queued(request) = &mut entry.status
                        && let Some(request) = request.take()
                    {
                        entry.status = Status::Running;
                        break (id, request);
                    }
                    continue;
                }
                let Ok(next) = shared.ready.wait(state) else {
                    return;
                };
                state = next;
            }
        };
        let (id, request) = job;
        let kind = request.kind.clone();
        let expected = (
            request.binding.principal,
            request.binding.name.clone(),
            request.binding.base,
            request.binding.origin,
        );
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            execute(*request, shared.pictures.as_ref())
        }))
        .unwrap_or_else(|_| Completion {
            expected,
            kind,
            reports: Vec::new(),
            diagnostic: Some(Diagnostic {
                field: "check".into(),
                message: "Authoring worker failed; draft unchanged.".into(),
                line: None,
                column: None,
            }),
            validated: None,
        });
        let Ok(mut state) = shared.state.lock() else {
            return;
        };
        let discard = state.stopped
            || state.jobs.get(&id).is_none_or(|entry| {
                matches!(entry.status, Status::CancelledRunning)
                    || entry.started.elapsed() > RESULT_TTL
            });
        if discard {
            state.jobs.remove(&id);
        } else if let Some(entry) = state.jobs.get_mut(&id) {
            entry.status = Status::Done(Some(Box::new(result)));
        }
    }
}

fn execute(request: Request, pictures: &dyn CheckPictures) -> Completion {
    let Request { binding, kind } = request;
    let expected = (
        binding.principal,
        binding.name.clone(),
        binding.base,
        binding.origin,
    );
    let validation = (|| {
        let rules = crate::minds::authoring::RevisionRules;
        let normalized = rules.normalize_source(&binding.original.source)?;
        rules.validate(&normalized, &binding.original.envelope)?;
        let reports = if matches!(kind, JobKind::Check) {
            let draft = WorkingCopy {
                source: normalized.clone(),
                envelope: binding.original.envelope.clone(),
            };
            let reports = pictures.run(&draft)?;
            validate_reports(&reports)?;
            reports
        } else {
            Vec::new()
        };
        Ok::<_, RollError>((normalized, reports))
    })();
    match validation {
        Ok((normalized, reports)) => Completion {
            expected,
            kind,
            reports,
            diagnostic: None,
            validated: Some(ValidatedDraft {
                binding,
                normalized,
            }),
        },
        Err(error) => {
            let (field, message) = match error {
                RollError::Invalid { field, message } => (field, message),
                other => ("check".into(), other.to_string()),
            };
            let position = if field == "source" {
                let sandbox = mush_mind::Sandbox::new(
                    &mush_mind::EngineLimits::default(),
                    crate::minds::register_api,
                );
                sandbox
                    .compile_error_position(&binding.original.source)
                    .and_then(|(line, column)| {
                        Some((u32::try_from(line).ok()?, u32::try_from(column).ok()?))
                    })
            } else {
                None
            };
            Completion {
                expected,
                kind,
                reports: Vec::new(),
                diagnostic: Some(Diagnostic {
                    line: position.map(|(line, _)| line),
                    column: position.map(|(_, column)| column),
                    field: field.chars().take(64).collect(),
                    message: super::display_text(&message).chars().take(1024).collect(),
                }),
                validated: None,
            }
        }
    }
}

fn validate_reports(reports: &[PictureReport]) -> Result<(), RollError> {
    if reports.len() != PICTURES.len() {
        return Err(invalid("check", "incomplete canned-picture result"));
    }
    for (report, expected) in reports.iter().zip(PICTURES) {
        if (report.picture, report.armed) != expected
            || report.intents.len() > 16
            || report.station_refusals.len() > 16
            || report.class_violations.len() > 16
        {
            return Err(invalid("check", "invalid canned-picture result"));
        }
        if report
            .intents
            .iter()
            .chain(&report.station_refusals)
            .chain(&report.class_violations)
            .chain(report.fault.iter())
            .any(|line| line.len() > 512 || line.chars().any(char::is_control))
        {
            return Err(invalid("check", "invalid canned-picture diagnostic"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validated_worker_receipt_rechecks_owner_provenance_before_save()
    -> Result<(), Box<dyn std::error::Error>> {
        let actor = mush_core::GOD;
        let service = WorkshopExecutor::new(Arc::new(NoPictures))?;
        let mut roll = RollStore::default();
        let change = super::super::create(
            &roll,
            &super::super::lineage_tests::Sources,
            actor,
            "mine",
            None,
        )?;
        roll.publish_with(change, |_| {})?;
        let principal = service.principal(actor, AccountId::mint("lineage-author", 1)?)?;
        let base = roll.mind(actor, "mine")?.base();
        let job = service.submit(
            principal,
            &roll,
            "mine",
            base,
            JobKind::Save {
                notes: String::new(),
                version: None,
            },
        )?;
        let deadline = Instant::now() + Duration::from_secs(10);
        let completion = loop {
            if let Some(completion) = service.take(principal, job)? {
                break completion;
            }
            if Instant::now() >= deadline {
                return Err("worker timed out".into());
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        completion.check_current(principal, &roll)?;
        assert!(completion.diagnostic.is_none());
        let marked = super::super::lineage_tests::marked_store(roll.mind(actor, "mine")?)?;
        assert_eq!(
            completion.check_current(principal, &marked),
            Err(RollError::Missing)
        );
        let validated = completion.validated.ok_or("validated receipt")?;
        assert!(matches!(
            validated.prepare_save(
                principal,
                &marked,
                &super::super::lineage_tests::Sources,
                "",
                None
            ),
            Err(RollError::Missing)
        ));
        assert!(marked.mind(actor, "mine")?.latest().is_none());
        Ok(())
    }

    struct NoPictures;
    impl CheckPictures for NoPictures {
        fn run(&self, _: &WorkingCopy) -> Result<Vec<PictureReport>, RollError> {
            Err(RollError::Missing)
        }
    }

    fn reports() -> Vec<PictureReport> {
        PICTURES
            .into_iter()
            .map(|(picture, armed)| PictureReport {
                picture,
                armed,
                operations: 1,
                fees: 1,
                intents: Vec::new(),
                fault: None,
                station_refusals: Vec::new(),
                class_violations: Vec::new(),
            })
            .collect()
    }

    struct HeldPictures {
        started: std::sync::mpsc::Sender<()>,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl CheckPictures for HeldPictures {
        fn run(&self, _draft: &WorkingCopy) -> Result<Vec<PictureReport>, RollError> {
            self.started
                .send(())
                .map_err(|_| invalid("test", "start channel"))?;
            self.release
                .lock()
                .map_err(|_| invalid("test", "gate"))?
                .recv_timeout(Duration::from_secs(10))
                .map_err(|_| invalid("test", "gate timed out"))?;
            Ok(reports())
        }
    }

    #[test]
    fn queue_results_and_cancelled_workers_share_bounded_account_slots() {
        let (started_tx, started) = std::sync::mpsc::channel();
        let (release, release_rx) = std::sync::mpsc::channel();
        let service = WorkshopExecutor::new(Arc::new(HeldPictures {
            started: started_tx,
            release: Mutex::new(release_rx),
        }))
        .unwrap();
        let mut roll = RollStore::default();
        let draft = WorkingCopy {
            source: "fn think(view, card) {}".into(),
            envelope: Envelope {
                schema: serde_json::json!({"sections":[]}),
                profiles: serde_json::json!({"Default":{}}),
                presets: serde_json::json!({}),
                rules: serde_json::json!({}),
                declared_class: "I".into(),
                mind_abi: crate::minds::MIND_ABI.into(),
            },
        };
        let mut principals = Vec::new();
        let mut ids = Vec::new();
        for index in 0..=MAX_JOBS {
            let actor = ObjId(u32::try_from(index).unwrap() + 100);
            let account = AccountId::mint(&format!("author-{index}"), 1).unwrap();
            let change = roll.prepare_create(actor, "draft", draft.clone()).unwrap();
            roll.publish_with(change, |_| {}).unwrap();
            let principal = service.principal(actor, account).unwrap();
            let base = roll.mind(actor, "draft").unwrap().base();
            let submitted = service.submit(principal, &roll, "draft", base, JobKind::Check);
            if index < MAX_JOBS {
                ids.push(submitted.unwrap());
            } else {
                assert!(submitted.is_err());
            }
            principals.push(principal);
        }
        started.recv_timeout(Duration::from_secs(10)).unwrap();
        started.recv_timeout(Duration::from_secs(10)).unwrap();
        let first = *principals.first().unwrap();
        let base = roll.mind(first.actor, "draft").unwrap().base();
        assert!(
            service
                .submit(first, &roll, "draft", base, JobKind::Check)
                .is_err()
        );
        assert!(matches!(
            service.take(*principals.get(1).unwrap(), *ids.first().unwrap()),
            Err(RollError::Missing)
        ));
        assert_eq!(
            service.cancel(*principals.get(1).unwrap(), *ids.first().unwrap()),
            Err(RollError::Missing),
            "a different principal cannot cancel a job"
        );
        service.cancel(first, *ids.first().unwrap()).unwrap();
        assert!(
            service
                .submit(first, &roll, "draft", base, JobKind::Check)
                .is_err(),
            "per-job cancellation keeps a running worker's account quota"
        );
        service.cancel_character(first.actor);
        let fresh = service.principal(first.actor, first.account).unwrap();
        assert_ne!(first, fresh);
        assert!(
            service
                .submit(fresh, &roll, "draft", base, JobKind::Check)
                .is_err(),
            "cancel cannot oversubscribe still-running compilation"
        );
        for _ in 0..MAX_JOBS {
            release.send(()).unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while service.ready().len() < MAX_JOBS - 1 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(service.ready().len(), MAX_JOBS - 1);
        // No result was consumed; they still occupy their per-account slots.
        let second = *principals.get(1).unwrap();
        let base = roll.mind(second.actor, "draft").unwrap().base();
        assert!(
            service
                .submit(second, &roll, "draft", base, JobKind::Check)
                .is_err()
        );
        let completion = service.take(second, *ids.get(1).unwrap()).unwrap().unwrap();
        assert!(completion.check_current(second, &roll).is_ok());
        let mut changed = draft.clone();
        changed.source.push('\n');
        let change = roll
            .prepare_edit(second.actor, "draft", base, changed)
            .unwrap();
        roll.publish_with(change, |_| {}).unwrap();
        assert_eq!(
            completion.check_current(second, &roll),
            Err(RollError::StaleEdit)
        );
        assert!(
            matches!(
                service.take(second, *ids.get(1).unwrap()),
                Err(RollError::Missing)
            ),
            "one-shot completion"
        );
    }
    #[test]
    fn receipt_checks_every_executable_field() {
        let draft = WorkingCopy {
            source: "fn think(view, card) {}\r\n".into(),
            envelope: Envelope {
                schema: serde_json::json!({"sections":[]}),
                presets: serde_json::json!({}),
                profiles: serde_json::json!({"Default":{}}),
                rules: serde_json::json!({}),
                declared_class: "I".into(),
                mind_abi: crate::minds::MIND_ABI.into(),
            },
        };
        let normalized = "fn think(view, card) {}\n";
        let adapter = ReceiptAdapter {
            original: &draft,
            normalized,
        };
        assert_eq!(adapter.normalize_source(&draft.source).unwrap(), normalized);
        assert!(adapter.normalize_source("different").is_err());
        assert!(adapter.validate(normalized, &draft.envelope).is_ok());
        let mut changed = draft.envelope.clone();
        changed.declared_class = "II".into();
        assert!(adapter.validate(normalized, &changed).is_err());
        changed = draft.envelope.clone();
        changed.rules = serde_json::json!({"Default":[]});
        assert!(adapter.validate(normalized, &changed).is_err());
    }
    #[test]
    fn reports_must_cover_all_six_exact_pictures() {
        let mut reports = PICTURES
            .into_iter()
            .map(|(picture, armed)| PictureReport {
                picture,
                armed,
                operations: 0,
                fees: 0,
                intents: Vec::new(),
                fault: None,
                station_refusals: Vec::new(),
                class_violations: Vec::new(),
            })
            .collect::<Vec<_>>();
        assert!(validate_reports(&reports).is_ok());
        reports.swap(0, 1);
        assert!(validate_reports(&reports).is_err());
        reports.pop();
        assert!(validate_reports(&reports).is_err());
    }
}
