//! Connection-local owned mind reader and typed card/rule editor.
//!
//! Every mutation requires Enter on a visible form. This module neither
//! parses rule grammar nor replaces source/envelopes: the shared server
//! returns canonical words and remains the only validation authority.

use std::collections::BTreeMap;

use mush_proto::{
    InvokeRequest, InvokeResult, MindDraft, MindEdit, MindEditBase, MindLifecycle, MindRequest,
    MindResponse,
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use serde_json::Value;

use super::keys::Key;
use crate::line::LineBuffer;

const MAX_FORM_BYTES: usize = 4096;
const MAX_LIBRARY: usize = 256;
const MAX_FIELDS: usize = 64;
const MAX_RULES: usize = 6;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Page {
    #[default]
    Library,
    Card,
    Rules,
    Source,
    Flight,
    Trace,
    Report,
}
impl Page {
    fn next(self, back: bool) -> Self {
        let pages = [
            Self::Library,
            Self::Card,
            Self::Rules,
            Self::Source,
            Self::Flight,
            Self::Trace,
            Self::Report,
        ];
        let at = pages.iter().position(|p| *p == self).unwrap_or(0);
        pages[(at + if back { pages.len() - 1 } else { 1 }) % pages.len()]
    }
}

#[derive(Clone, Debug)]
enum Action {
    Create,
    Revision,
    Set { field: String, widget: Value },
    Rule { slot: u16 },
    ProfileAdd,
    ProfileRename,
    ProfileDelete,
    PresetSave,
    PresetApply,
    PresetDelete,
    Check,
    Save,
    Cancel { job: u64 },
    Trace { follow: bool },
    RenameMind,
    RemoveMind,
    RemoveAttributed { name: String },
    VersionRename,
    VersionForget,
    Prune,
}
struct Form {
    profile: Option<String>,
    action: Action,
    title: String,
    line: LineBuffer,
}
struct Loaded {
    name: String,
    revision: Option<String>,
    base: MindEditBase,
    draft: MindDraft,
    rules: BTreeMap<String, Vec<String>>,
    rule_sources: BTreeMap<String, Vec<Option<String>>>,
    fresh: bool,
}
struct Pending {
    id: u64,
    request: MindRequest,
    job: Option<u64>,
}

/// Bounded state owned by exactly one attached SSH connection.
pub(super) struct MindWorkshop {
    pub(super) open: bool,
    page: Page,
    names: Vec<String>,
    removal_requested: Vec<String>,
    selected: usize,
    profile: usize,
    row: usize,
    scroll: usize,
    loaded: Option<Loaded>,
    form: Option<Form>,
    pending: Option<Pending>,
    next_id: u64,
    outbox: Vec<(u64, InvokeRequest)>,
    message: String,
    report: Vec<String>,
    flight_profile: usize,
    pub(super) flight_current: bool,
    trace_unit: Option<u64>,
    trace_follow: bool,
    traces: Vec<String>,
}

impl Default for MindWorkshop {
    fn default() -> Self {
        Self {
            open: false,
            page: Page::Library,
            names: Vec::new(),
            removal_requested: Vec::new(),
            selected: 0,
            profile: 0,
            row: 0,
            scroll: 0,
            loaded: None,
            form: None,
            pending: None,
            next_id: 1 << 62,
            outbox: Vec::new(),
            message: String::new(),
            report: Vec::new(),
            flight_profile: 0,
            flight_current: false,
            trace_unit: None,
            trace_follow: false,
            traces: Vec::new(),
        }
    }
}

impl MindWorkshop {
    /// Clear private source and stale tokens on a session/character boundary.
    /// Do not recycle request IDs: late replies cannot hydrate a new owner.
    pub(super) fn reset(&mut self) {
        let next_id = self.next_id;
        *self = Self {
            next_id,
            ..Self::default()
        };
    }

    pub(super) fn show(&mut self) {
        self.open = true;
        if self.pending.is_none() && self.loaded.is_none() {
            self.send(MindRequest::List {});
        }
    }

    pub(super) fn take_invokes(&mut self) -> Vec<(u64, InvokeRequest)> {
        std::mem::take(&mut self.outbox)
    }

    fn send(&mut self, request: MindRequest) {
        if self.pending.is_some() {
            self.message = "An operation is pending; wait for its result.".into();
            return;
        }
        let removal_name = match &request {
            MindRequest::Remove { name }
            | MindRequest::Lifecycle {
                name,
                edit: MindLifecycle::Remove {},
                ..
            } => Some(name),
            _ => None,
        };
        if removal_name.is_some_and(|name| self.removal_requested.contains(name)) {
            self.message = "Removal already requested; r refreshes without resubmitting.".into();
            return;
        }
        let invoke = InvokeRequest::Mind(request.clone());
        if let Err(error) = invoke.validate() {
            self.message = format!("Request refused: {error}");
            return;
        }
        let Some(next) = self.next_id.checked_add(1) else {
            self.message = "Request IDs exhausted; reconnect.".into();
            return;
        };
        let id = self.next_id;
        self.next_id = next;
        self.pending = Some(Pending {
            id,
            request,
            job: None,
        });
        self.outbox.push((id, invoke));
        self.message = "Waiting for the server...".into();
    }

    /// Ignore unrelated/late replies. Follow-up reads are queued for the output
    /// pump; applying a response never performs I/O under the connection lock.
    pub(super) fn accept(&mut self, id: u64, result: &InvokeResult) {
        let Some(pending) = self.pending.as_ref().filter(|p| p.id == id) else {
            return;
        };
        let InvokeResult::Mind(response) = result else {
            return;
        };
        if let MindResponse::Queued { job } | MindResponse::Pending { job } = response {
            if let Some(p) = self.pending.as_mut() {
                p.job = Some(*job);
            }
            self.message = format!("Job {job} is pending. x opens cancellation.");
            return;
        }
        let request = pending.request.clone();
        self.pending = None;
        match response {
            MindResponse::Trace { data } => {
                let MindRequest::Trace { unit, follow, .. } = request else {
                    self.message = "Unexpected trace response; select a hull again.".into();
                    return;
                };
                self.form = None;
                self.page = Page::Trace;
                self.scroll = 0;
                self.trace_unit = Some(unit);
                self.trace_follow = follow;
                if data.get("unit").and_then(Value::as_u64) != Some(unit)
                    || data.get("available").and_then(Value::as_bool) != Some(true)
                {
                    self.clear_trace();
                } else {
                    self.traces = trace_rows(data);
                    self.message = if follow {
                        "Following this authorized hull. t reads once and stops following.".into()
                    } else {
                        "Trace read once. f follows; t refreshes after Enter.".into()
                    };
                }
            }
            MindResponse::Library { names } => {
                self.names = names.iter().take(MAX_LIBRARY).cloned().collect();
                self.selected = self.selected.min(self.names.len().saturating_sub(1));
                let removed = self
                    .removal_requested
                    .iter()
                    .any(|name| !names.contains(name));
                self.removal_requested.retain(|name| names.contains(name));
                self.message = if removed {
                    "Requested entry no longer appears in your library; retained revisions remain."
                        .into()
                } else if !self.removal_requested.is_empty() {
                    "Removal requested; entry still present. r refreshes without resubmitting."
                        .into()
                } else if names.len() > MAX_LIBRARY {
                    "Library exceeds this client's bound; use the line reader.".into()
                } else {
                    "Enter reads. n creates from Standard. X requests attributed draft removal without source.".into()
                };
            }
            MindResponse::Read {
                name,
                revision,
                base,
                draft,
                rules_text,
                rules_source,
                diagnostics,
            } => {
                // A response must match the requested identity, even though the
                // transport already correlates it by this connection's ID.
                if !matches!(&request, MindRequest::Read { name: n, revision: r } if n == name && r == revision)
                {
                    self.message = "Mismatched read refused; refresh the library.".into();
                    return;
                }
                let same = self.loaded.as_ref().is_some_and(|l| l.name == *name);
                self.loaded = Some(Loaded {
                    name: name.clone(),
                    revision: revision.clone(),
                    base: *base,
                    draft: (**draft).clone(),
                    rules: rules_text.clone(),
                    rule_sources: rules_source.clone(),
                    fresh: true,
                });
                if !same {
                    self.profile = 0;
                    self.row = 0;
                    self.scroll = 0;
                }
                if self.page == Page::Library {
                    self.page = Page::Card;
                }
                if !diagnostics.is_empty() {
                    self.report = diagnostics
                        .iter()
                        .map(|d| {
                            format!(
                                "{}: {}{}",
                                d.field,
                                d.message,
                                d.line
                                    .map(|l| format!(
                                        " (line {l}, column {})",
                                        d.column.unwrap_or(1)
                                    ))
                                    .unwrap_or_default()
                            )
                        })
                        .collect();
                }
                self.message = if revision.is_some() {
                    "Saved revision: read only. r returns to the current working copy.".into()
                } else if diagnostics.is_empty() {
                    "Working copy loaded. Enter edits a selected field/rule; a second Enter sends."
                        .into()
                } else {
                    "Working copy has diagnostics; Report shows them. Source remains readable."
                        .into()
                };
            }
            MindResponse::RemovalRequested { name } => {
                if !matches!(&request,
                    MindRequest::Remove { name: selected }
                    | MindRequest::Lifecycle { name: selected, edit: MindLifecycle::Remove {}, .. }
                    if selected == name)
                {
                    self.message = "Mismatched removal request; refresh the library.".into();
                    return;
                }
                self.form = None;
                if !self.removal_requested.contains(name)
                    && self.removal_requested.len() < MAX_LIBRARY
                {
                    self.removal_requested.push(name.clone());
                }
                self.message = "Removal requested, not yet confirmed. r refreshes your library; current editor is retained.".into();
                self.page = Page::Library;
            }
            MindResponse::Removed { name } => {
                self.form = None;
                if self.loaded.as_ref().is_some_and(|l| &l.name == name) {
                    self.loaded = None;
                }
                self.page = Page::Library;
                self.send(MindRequest::List {});
            }
            MindResponse::Pruned {
                revisions,
                source_candidates_retained,
            } => {
                self.form = None;
                self.page = Page::Report;
                self.scroll = 0;
                self.report = vec![
                    format!("Pruned {revisions} unreferenced revisions."),
                    format!(
                        "Retained {source_candidates_retained} immutable source candidates; source cleanup is deferred."
                    ),
                ];
                self.message =
                    "Prune complete; retained revisions and sortie seals remain available.".into();
            }
            MindResponse::Edited { name, .. } => {
                self.form = None;
                self.send(MindRequest::Read {
                    name: name.clone(),
                    revision: None,
                });
            }
            MindResponse::Saved {
                revision, digest, ..
            } => {
                self.form = None;
                self.report = vec![format!("Saved revision {revision}"), digest.clone()];
                self.page = Page::Report;
                if let Some(loaded) = &self.loaded {
                    self.send(MindRequest::Read {
                        name: loaded.name.clone(),
                        revision: None,
                    });
                }
            }
            MindResponse::Checked {
                reports,
                diagnostics,
                ..
            } => {
                self.form = None;
                self.page = Page::Report;
                self.scroll = 0;
                self.report = reports
                    .iter()
                    .flat_map(|r| {
                        let mut lines = vec![format!(
                            "{} / armed={} / {} ops + {} fees / {}",
                            r.picture,
                            r.armed,
                            r.operations,
                            r.fees,
                            r.fault.as_deref().unwrap_or("ok")
                        )];
                        lines.extend(r.intents.iter().map(|i| format!("  {i}")));
                        lines.extend(
                            r.station_refusals
                                .iter()
                                .chain(&r.class_violations)
                                .map(|i| format!("  refused: {i}")),
                        );
                        lines
                    })
                    .collect();
                self.report.extend(diagnostics.iter().map(|d| {
                    format!(
                        "{}: {} (line {:?}, column {:?})",
                        d.field, d.message, d.line, d.column
                    )
                }));
                self.message = "Check complete; no revision was saved.".into();
            }
            MindResponse::Error { code, message } => {
                if matches!(request, MindRequest::Trace { .. }) {
                    self.clear_trace();
                    self.message = format!("{code}: {message}");
                    return;
                }
                // Keep unsent text, but never automatically retry a mutation.
                if let Some(loaded) = &mut self.loaded {
                    loaded.fresh = false;
                }
                self.message = format!(
                    "{code}: {message}. Ctrl-R refreshes before retry; your input is kept."
                );
            }
            MindResponse::Cancelled { .. } => {
                self.form = None;
                self.message = "Job cancelled.".into();
            }
            _ => {
                self.message = "Unexpected mind response; refresh before editing.".into();
            }
        }
    }

    fn profiles(&self) -> Vec<String> {
        self.loaded
            .as_ref()
            .and_then(|l| l.draft.profiles.as_object())
            .map(|p| p.keys().take(4).cloned().collect())
            .unwrap_or_default()
    }
    fn rule_capacity(&self) -> usize {
        match self
            .loaded
            .as_ref()
            .map(|l| l.draft.declared_class.as_str())
        {
            Some("0") => 1,
            Some("I") => 2,
            Some("II") => 4,
            _ => MAX_RULES,
        }
    }
    fn clear_trace(&mut self) {
        self.trace_follow = false;
        self.traces.clear();
        self.message = "No mind trace is available. Select a permitted hull again.".into();
    }

    /// A feed never establishes a selector. Only a correlated authorized reply
    /// does that; attachment resets clear it and revoke late followed updates.
    pub(super) fn trace_feed(&mut self, data: &Value) {
        if data.get("available").and_then(Value::as_bool) != Some(true) {
            self.clear_trace();
            return;
        }
        if !self.trace_follow
            || self
                .pending
                .as_ref()
                .is_some_and(|p| matches!(p.request, MindRequest::Trace { .. }))
            || data.get("unit").and_then(Value::as_u64) != self.trace_unit
        {
            return;
        }
        self.traces = trace_rows(data);
    }

    /// Flight controls use the current cockpit frame. A selected profile only
    /// returns command-box text; this module never sends cockpit command text.
    pub(super) fn key_with_flight(&mut self, key: Key, cockpit: Option<&Value>) -> Option<String> {
        let cockpit = cockpit.filter(|f| f.get("seated").and_then(Value::as_bool) != Some(false));
        if self.open && self.form.is_none() {
            if matches!(key, Key::Char('t') | Key::Char('f'))
                || (self.page == Page::Trace && matches!(key, Key::Enter | Key::Char('r')))
            {
                let unit = if self.page == Page::Trace {
                    self.trace_unit
                        .or_else(|| cockpit.and_then(|f| f.get("unit")?.as_u64()))
                } else {
                    cockpit
                        .and_then(|f| f.get("unit")?.as_u64())
                        .or(self.trace_unit)
                };
                self.begin(
                    Action::Trace {
                        follow: key == Key::Char('f'),
                    },
                    "Trace hull number [count 1..64]; Enter requests current permission",
                    &unit.map(|n| format!("#{n} 3")).unwrap_or_default(),
                );
                return None;
            }
            if self.page == Page::Flight {
                let profiles = cockpit.and_then(|f| f.get("mind")?.get("profiles")?.as_array());
                let count = profiles.map_or(0, Vec::len).min(4);
                self.flight_profile = self.flight_profile.min(count.saturating_sub(1));
                if matches!(key, Key::Left | Key::Right) {
                    if count != 0 {
                        self.flight_profile = (self.flight_profile
                            + if key == Key::Left { count - 1 } else { 1 })
                            % count;
                        self.scroll = 0;
                    }
                    return None;
                }
                if key == Key::Enter {
                    if let Some(name) = profiles
                        .and_then(|p| p.get(self.flight_profile))
                        .and_then(Value::as_str)
                    {
                        self.open = false;
                        return Some(format!("MIND PROFILE {}", clean(name)));
                    }
                    self.message = "No seated profile is available.".into();
                    return None;
                }
            }
        }
        self.key(key);
        None
    }

    fn profile_name(&self) -> Option<String> {
        self.profiles().get(self.profile).cloned()
    }
    fn fields(&self) -> Vec<&Value> {
        self.loaded
            .as_ref()
            .map(|l| fields(&l.draft.schema))
            .unwrap_or_default()
    }
    fn begin(&mut self, action: Action, title: impl Into<String>, value: &str) {
        let mut line = LineBuffer::new(0);
        for c in value.chars() {
            line.insert(c);
        }
        self.form = Some(Form {
            profile: self.profile_name(),
            action,
            title: title.into(),
            line,
        });
    }
    fn editable(&mut self) -> bool {
        let yes = self
            .loaded
            .as_ref()
            .is_some_and(|l| l.revision.is_none() && l.fresh);
        if !yes {
            self.message = "Read the current working copy with r before editing.".into();
        }
        yes
    }
    fn refresh(&mut self) {
        if self.pending.is_some() {
            return;
        }
        if self.page == Page::Library {
            self.send(MindRequest::List {});
        } else if let Some(l) = &self.loaded {
            self.send(MindRequest::Read {
                name: l.name.clone(),
                revision: None,
            });
        }
    }

    pub(super) fn key(&mut self, key: Key) {
        if !self.open {
            return;
        }
        if key == Key::F(10) {
            self.open = false;
            return;
        }
        if self.form.is_some() {
            self.form_key(key);
            return;
        }
        match key {
            Key::Esc | Key::F(10) => self.open = false,
            Key::Tab | Key::BackTab => {
                self.page = self.page.next(key == Key::BackTab);
                self.row = 0;
                self.scroll = 0;
            }
            Key::Char('r') => self.refresh(),
            Key::Char('x') => {
                if let Some(job) = self.pending.as_ref().and_then(|p| p.job) {
                    self.begin(
                        Action::Cancel { job },
                        format!("Cancel job {job}: Enter confirms"),
                        "",
                    );
                }
            }
            Key::Char('c') if self.editable() => self.begin(
                Action::Check,
                "Check this exact working copy: Enter confirms",
                "",
            ),
            Key::Char('s') if self.editable() => self.begin(
                Action::Save,
                "Save immutable revision: optional notes, Enter sends",
                "",
            ),
            Key::Char('N') if self.editable() => self.begin(
                Action::RenameMind,
                "Rename current owned mind; Enter confirms",
                "",
            ),
            Key::Char('X') if self.page == Page::Library && self.pending.is_none() => {
                if let Some(name) = self.names.get(self.selected).cloned() {
                    if self.removal_requested.contains(&name) {
                        self.message =
                            "Removal already requested; r refreshes without resubmitting.".into();
                    } else {
                        self.begin(
                            Action::RemoveAttributed { name },
                            "Request attributed draft removal; Enter confirms",
                            "",
                        );
                    }
                }
            }
            Key::Char('X') if self.editable() => self.begin(
                Action::RemoveMind,
                "Remove current mind from library; Enter confirms (retained seals/history remain)",
                "",
            ),
            Key::Char('v') if self.editable() => self.begin(
                Action::VersionRename,
                "Rename named version: old=new; Enter confirms",
                "",
            ),
            Key::Char('V') if self.editable() => self.begin(
                Action::VersionForget,
                "Forget named version (revision stays while referenced); Enter confirms",
                "",
            ),
            Key::Char('g') => self.begin(
                Action::Prune,
                "Prune at most 1..128 unreferenced own revisions; Enter confirms",
                "128",
            ),
            Key::Char('o') if self.loaded.is_some() => self.begin(
                Action::Revision,
                "Read saved revision / version / sha selector",
                "",
            ),
            Key::Left | Key::Right if matches!(self.page, Page::Card | Page::Rules) => {
                let count = self.profiles().len();
                if count != 0 {
                    self.profile =
                        (self.profile + if key == Key::Left { count - 1 } else { 1 }) % count;
                }
                self.row = 0;
                self.scroll = 0;
            }
            Key::Up => self.move_row(false, 1),
            Key::Down => self.move_row(true, 1),
            Key::PgUp => self.scroll = self.scroll.saturating_sub(10),
            Key::PgDn => self.scroll = self.scroll.saturating_add(10).min(32768),
            Key::Home => {
                self.row = 0;
                self.scroll = 0;
                self.selected = 0;
            }
            Key::Char('n') if self.page == Page::Library => self.begin(
                Action::Create,
                "New mind name (FROM Standard): Enter creates working copy",
                "",
            ),
            Key::Char('+') if self.page == Page::Card && self.editable() => self.begin(
                Action::ProfileAdd,
                "New profile name (copy selected profile)",
                "",
            ),
            Key::Char('n') if self.page == Page::Card && self.editable() => self.begin(
                Action::ProfileRename,
                "Rename selected profile",
                &self.profile_name().unwrap_or_default(),
            ),
            Key::Char('-') if self.page == Page::Card && self.editable() => self.begin(
                Action::ProfileDelete,
                "Delete selected profile: Enter confirms",
                "",
            ),
            Key::Char('p') if self.page == Page::Card && self.editable() => {
                self.begin(Action::PresetApply, "Apply preset by name", "")
            }
            Key::Char('P') if self.page == Page::Card && self.editable() => {
                self.begin(Action::PresetSave, "Save selected card as preset name", "")
            }
            Key::Char('D') if self.page == Page::Card && self.editable() => {
                self.begin(Action::PresetDelete, "Delete preset by name", "")
            }
            Key::Enter => self.select(),
            _ => {}
        }
    }
    fn move_row(&mut self, down: bool, amount: usize) {
        if matches!(self.page, Page::Card | Page::Rules) {
            self.scroll = 0;
        }
        match self.page {
            Page::Library => {
                self.selected =
                    shift(self.selected, down, amount).min(self.names.len().saturating_sub(1))
            }
            Page::Card => {
                self.row = shift(self.row, down, amount).min(self.fields().len().saturating_sub(1))
            }
            Page::Rules => self.row = shift(self.row, down, amount).min(self.rule_capacity() - 1),
            _ => self.scroll = shift(self.scroll, down, amount).min(32_768),
        }
    }
    fn select(&mut self) {
        let page = self.page;
        match page {
            Page::Library => {
                if let Some(name) = self.names.get(self.selected) {
                    self.send(MindRequest::Read {
                        name: name.clone(),
                        revision: None,
                    });
                }
            }
            Page::Card if self.editable() => {
                let Some(field) = self.fields().get(self.row).copied() else {
                    return;
                };
                let id = string(field, "id").to_owned();
                let widget = field.get("widget").cloned().unwrap_or(Value::Null);
                let value = self
                    .loaded
                    .as_ref()
                    .and_then(|l| l.draft.profiles.get(self.profile_name()?))
                    .and_then(|p| p.get(&id))
                    .map(edit_value)
                    .unwrap_or_default();
                let title = format!(
                    "{}: {} | {}",
                    string(field, "label"),
                    widget_hint(&widget),
                    string(&widget, "help")
                );
                self.begin(Action::Set { field: id, widget }, title, &value);
            }
            Page::Rules if self.editable() => {
                let profile = self.profile_name().unwrap_or_default();
                let text = self
                    .loaded
                    .as_ref()
                    .and_then(|l| l.rule_sources.get(&profile))
                    .and_then(|r| r.get(self.row))
                    .cloned()
                    .flatten()
                    .unwrap_or_default();
                self.begin(
                    Action::Rule {
                        slot: (self.row + 1) as u16,
                    },
                    format!(
                        "S{}: whole replacement rule; type OFF to remove. Enter uses the shared parser",
                        self.row + 1
                    ),
                    &text,
                );
            }
            _ => {}
        }
    }

    fn form_key(&mut self, key: Key) {
        if key == Key::Ctrl('r') {
            self.refresh();
            return;
        }
        if key == Key::Esc {
            self.form = None;
            return;
        }
        if key == Key::Enter {
            self.submit_form();
            return;
        }
        if self.pending.is_some() {
            return;
        }
        let Some(form) = &mut self.form else {
            return;
        };
        match key {
            Key::Char(c) if form.line.text().len() + c.len_utf8() <= MAX_FORM_BYTES => {
                form.line.insert(c);
            }
            Key::Backspace => {
                form.line.backspace();
            }
            Key::Delete => {
                form.line.delete();
            }
            Key::Left => {
                form.line.left();
            }
            Key::Right => {
                form.line.right();
            }
            Key::Home | Key::Ctrl('a') => {
                form.line.home();
            }
            Key::End | Key::Ctrl('e') => {
                form.line.end();
            }
            Key::Ctrl('u') => {
                form.line.kill_to_start();
            }
            Key::Ctrl('w') => {
                form.line.kill_word();
            }
            Key::Up | Key::Down => {
                if let Action::Set { widget, .. } = &form.action
                    && let Some(value) = adjust_value(widget, &form.line.text(), key == Key::Up)
                {
                    form.line.clear();
                    for c in value.chars() {
                        form.line.insert(c);
                    }
                }
            }
            _ => {}
        }
    }

    fn submit_form(&mut self) {
        let Some(form) = &self.form else {
            return;
        };
        let action = form.action.clone();
        let profile = form.profile.clone().unwrap_or_default();
        let text = form.line.text();
        if let Action::Cancel { job } = action {
            // Cancellation uses a fresh correlation ID. The old final is now
            // deliberately ignored; it may have raced successful publication.
            self.pending = None;
            if let Some(l) = &mut self.loaded {
                l.fresh = false;
            }
            self.send(MindRequest::Cancel { job });
            return;
        }
        if self.pending.is_some() {
            self.message = "Wait for the pending operation before sending.".into();
            return;
        }
        if let Action::Prune = action {
            if let Some(limit) = text
                .trim()
                .parse::<u16>()
                .ok()
                .filter(|n| (1..=128).contains(n))
            {
                self.send(MindRequest::Prune { limit });
            } else {
                self.message = "Prune limit must be between 1 and 128.".into();
            }
            return;
        }
        if let Action::Trace { follow } = action {
            let mut words = text.split_whitespace();
            let unit = words
                .next()
                .and_then(|s| s.trim_start_matches('#').parse::<u32>().ok().map(u64::from));
            let count = words
                .next()
                .map_or(Some(3), |s| s.parse::<u8>().ok())
                .filter(|n| (1..=64).contains(n));
            if let (Some(unit), Some(count), None) = (unit, count, words.next()) {
                self.send(MindRequest::Trace {
                    unit,
                    count: Some(count),
                    follow,
                });
            } else {
                self.message = "Enter a hull number and optional count from 1 to 64.".into();
            }
            return;
        }
        if let Action::RemoveAttributed { name } = &action {
            if self.names.contains(name) && !self.removal_requested.contains(name) {
                self.send(MindRequest::Remove { name: name.clone() });
            }
            return;
        }
        if let Action::Create = action {
            self.send(MindRequest::Create {
                name: text,
                from: Some("Standard".into()),
            });
            return;
        }
        if let Action::Revision = action {
            if let Some(l) = &self.loaded {
                self.send(MindRequest::Read {
                    name: l.name.clone(),
                    revision: Some(text),
                });
                self.form = None;
            }
            return;
        }
        if !self.editable() {
            return;
        }
        let Some(l) = &self.loaded else {
            return;
        };
        let name = l.name.clone();
        let base = l.base;
        let request = match action {
            Action::RenameMind => MindRequest::Lifecycle {
                name,
                base,
                edit: MindLifecycle::Rename { new_name: text },
            },
            Action::RemoveMind => MindRequest::Lifecycle {
                name,
                base,
                edit: MindLifecycle::Remove {},
            },
            Action::VersionForget => MindRequest::Lifecycle {
                name,
                base,
                edit: MindLifecycle::VersionForget { version: text },
            },
            Action::VersionRename => {
                let Some((old, new)) = text
                    .split_once('=')
                    .filter(|(old, new)| !old.trim().is_empty() && !new.trim().is_empty())
                else {
                    self.message = "Enter old=new for a named version.".into();
                    return;
                };
                MindRequest::Lifecycle {
                    name,
                    base,
                    edit: MindLifecycle::VersionRename {
                        old: old.trim().into(),
                        new: new.trim().into(),
                    },
                }
            }
            Action::Check => MindRequest::Check { name, base },
            Action::Save => MindRequest::Save {
                name,
                base,
                notes: text,
                version: None,
            },
            edit => MindRequest::Edit {
                name,
                base,
                edit: match edit {
                    Action::Set { field, .. } => MindEdit::Set {
                        profile,
                        field,
                        value: text,
                    },
                    Action::Rule { slot } => {
                        if text.trim().is_empty() {
                            self.message =
                                "Enter a replacement rule, or type OFF to remove this slot.".into();
                            return;
                        }
                        MindEdit::Rule {
                            profile,
                            slot,
                            text: if text.trim().eq_ignore_ascii_case("off") {
                                String::new()
                            } else {
                                text
                            },
                        }
                    }
                    Action::ProfileAdd => MindEdit::ProfileAdd {
                        name: text,
                        from: Some(profile),
                    },
                    Action::ProfileRename => MindEdit::ProfileRename {
                        old: profile,
                        new: text,
                    },
                    Action::ProfileDelete => MindEdit::ProfileDelete { name: profile },
                    Action::PresetSave => MindEdit::PresetSave {
                        profile,
                        preset: text,
                    },
                    Action::PresetApply => MindEdit::PresetApply {
                        profile,
                        preset: text,
                    },
                    Action::PresetDelete => MindEdit::PresetDelete { preset: text },
                    _ => return,
                },
            },
        };
        self.send(request);
    }

    pub(super) fn draw(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        utf8: bool,
        flight: Option<&Value>,
    ) {
        if !self.open || area.width < 4 || area.height < 5 {
            return;
        }
        frame.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .title("Mind workshop · F10 / Esc closes · source is read only");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let [tabs, head, body, status, foot] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(4),
            Constraint::Min(1),
            Constraint::Length(2),
            Constraint::Length(2),
        ])
        .areas(inner);
        let tabs_text = [
            Page::Library,
            Page::Card,
            Page::Rules,
            Page::Source,
            Page::Flight,
            Page::Trace,
            Page::Report,
        ]
        .iter()
        .map(|p| {
            format!(
                "{}{:?}{}",
                if *p == self.page { "[" } else { " " },
                p,
                if *p == self.page { "]" } else { " " }
            )
        })
        .collect::<Vec<_>>()
        .join("  ");
        frame.render_widget(
            Paragraph::new(tabs_text).style(Style::default().fg(Color::Cyan)),
            tabs,
        );
        let heading = self
            .loaded
            .as_ref()
            .map(|l| {
                format!(
                    "{} | Class {} | {} | edit {} / revision {:?}\nProfiles: {} ({} of {}; Left/Right changes)",
                    clean(&l.name),
                    clean(&l.draft.declared_class),
                    l.revision
                        .as_deref()
                        .map_or("working copy".into(), |r| format!(
                            "revision {} (read only)",
                            clean(r)
                        )),
                    l.base.edit,
                    l.base.revision,
                    self.profile_name()
                        .as_deref()
                        .map(clean)
                        .unwrap_or_default(),
                    self.profile + 1,
                    self.profiles().len()
                )
            })
            .unwrap_or_else(|| {
                "Your character's minds. Enter reads; N renames loaded mind; X removes; v/V versions; g prunes."
                    .into()
            });
        frame.render_widget(Paragraph::new(heading).wrap(Wrap { trim: false }), head);
        let (rows, selected) = self.rows(utf8, flight);
        let mut lines = Vec::new();
        let mut selected_line = 0;
        let width = usize::from(body.width.max(1));
        for (i, row) in rows.iter().enumerate() {
            if selected == Some(i) {
                selected_line = lines.len();
            }
            let prefix = if selected == Some(i) { "> " } else { "  " };
            let text = format!("{prefix}{}", clean(row));
            lines.extend(wrap_cells(&text, width).into_iter().map(Line::from));
        }
        let max_scroll = lines.len().saturating_sub(usize::from(body.height));
        let scroll = if selected.is_some() {
            selected_line
                .saturating_sub(usize::from(body.height).saturating_sub(3))
                .saturating_add(self.scroll)
                .min(max_scroll)
        } else {
            self.scroll.min(max_scroll)
        };
        frame.render_widget(
            Paragraph::new(lines).scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0)),
            body,
        );
        frame.render_widget(
            Paragraph::new(clean(&self.message))
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(Color::Yellow)),
            status,
        );
        frame.render_widget(Paragraph::new("Tab: page  Left/Right: profile  arrows/PgUp/PgDn: browse  Enter: edit/prefill  r: refresh\nc: check  s: save  o: revision  Card +/n/-: profiles  p/P/D: presets  t: trace once  f: follow"), foot);
        if let Some(form) = &self.form {
            let h = area.height.min(9);
            let form_area = Rect::new(area.x, area.y + area.height - h, area.width, h);
            frame.render_widget(Clear, form_area);
            let text = format!(
                "{}\n{}\n\nEnter sends this exact draft token; Esc discards unsent text. Ctrl-R refreshes after refusal and keeps it. Up/Down adjusts widgets.\n{}",
                clean(&form.title),
                form_window(
                    &form.line,
                    usize::from(form_area.width.saturating_sub(2)),
                    usize::from(h.saturating_sub(5))
                ),
                clean(&self.message)
            );
            frame.render_widget(
                Paragraph::new(text).wrap(Wrap { trim: false }).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(if self.pending.is_some() {
                            "Sent — waiting for server"
                        } else {
                            "Pending edit — not sent"
                        }),
                ),
                form_area,
            );
        }
    }

    fn rows(&self, utf8: bool, flight: Option<&Value>) -> (Vec<String>, Option<usize>) {
        match self.page {
            Page::Library => (
                self.names.iter().map(|s| clean(s)).collect(),
                Some(self.selected),
            ),
            Page::Card => {
                let values = self
                    .loaded
                    .as_ref()
                    .and_then(|l| l.draft.profiles.get(self.profile_name()?));
                let mut rows: Vec<String> = self
                    .fields()
                    .into_iter()
                    .map(|f| {
                        let section = self
                            .loaded
                            .as_ref()
                            .map(|l| section_label(&l.draft.schema, string(f, "id")))
                            .unwrap_or_default();
                        format!(
                            "{} / {}",
                            section,
                            field_line(f, values.and_then(|p| p.get(string(f, "id"))), utf8)
                        )
                    })
                    .collect();
                if let Some(presets) = self
                    .loaded
                    .as_ref()
                    .and_then(|l| l.draft.presets.as_object())
                {
                    rows.push(format!(
                        "Presets: {}",
                        presets
                            .keys()
                            .map(|s| clean(s))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                if self.fields().is_empty() {
                    rows.insert(0, "This card has no editable fields.".into());
                }
                (rows, Some(self.row))
            }
            Page::Rules => {
                let rules = self
                    .loaded
                    .as_ref()
                    .and_then(|l| l.rules.get(&self.profile_name()?));
                (
                    (0..self.rule_capacity())
                        .map(|i| {
                            let text = rules
                                .and_then(|r| r.get(i))
                                .map(String::as_str)
                                .unwrap_or("(empty)");
                            format!("S{} {} {}", i + 1, rule_glyph(text, utf8), text)
                        })
                        .collect(),
                    Some(self.row),
                )
            }
            Page::Source => (
                self.loaded
                    .as_ref()
                    .map(|l| {
                        l.draft
                            .source
                            .lines()
                            .enumerate()
                            .map(|(n, s)| format!("{:4}  {}", n + 1, s.replace('\t', "    ")))
                            .collect()
                    })
                    .unwrap_or_default(),
                None,
            ),
            Page::Report => (self.report.clone(), None),
            Page::Flight => {
                let rows = flight_rows(flight, utf8);
                let selected = flight
                    .and_then(|m| m.get("profiles")?.as_array())
                    .filter(|p| !p.is_empty())
                    .map(|p| 2 + self.flight_profile.min(p.len().min(4).saturating_sub(1)));
                (rows, selected)
            }
            Page::Trace => {
                let mut rows = vec![self.trace_unit.map_or_else(
                    || "No trace selected. t reads once; f follows after Enter.".into(),
                    |unit| {
                        format!(
                            "Hull #{unit}: {}",
                            if self.trace_follow {
                                "following"
                            } else {
                                "read once"
                            }
                        )
                    },
                )];
                rows.extend(self.traces.iter().cloned());
                if self.traces.is_empty() {
                    rows.push("No recorded thinks are currently available.".into());
                }
                (rows, None)
            }
        }
    }
}

fn shift(at: usize, down: bool, amount: usize) -> usize {
    if down {
        at.saturating_add(amount)
    } else {
        at.saturating_sub(amount)
    }
}

fn wrap_cells(text: &str, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    let mut row = String::new();
    for c in text.chars() {
        let mut next = row.clone();
        next.push(c);
        if Line::from(next.as_str()).width() > width.max(1) && !row.is_empty() {
            rows.push(std::mem::take(&mut row));
        }
        row.push(c);
    }
    rows.push(row);
    rows
}

fn form_window(line: &LineBuffer, width: usize, rows: usize) -> String {
    let chars = line.chars();
    // At least one complete visible input row. A cursor marker makes a
    // long rule's current edit position visible without mutating the text.
    let budget = width.max(1).saturating_mul(rows.max(1));
    let start = line.cursor().saturating_sub(budget / 2);
    let end = chars.len().min(start.saturating_add(budget));
    let mut text = if start != 0 {
        "...".into()
    } else {
        String::new()
    };
    for (i, c) in chars.iter().enumerate().take(end).skip(start) {
        if i == line.cursor() {
            text.push('|');
        }
        text.push(*c);
    }
    if line.cursor() == end {
        text.push('|');
    }
    if end != chars.len() {
        text.push_str("...");
    }
    text
}
fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}
fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}
fn fields(schema: &Value) -> Vec<&Value> {
    schema
        .get("sections")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(16)
        .flat_map(|s| {
            s.get("fields")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .take(MAX_FIELDS)
        .collect()
}
fn section_label(schema: &Value, field: &str) -> String {
    for section in schema["sections"].as_array().into_iter().flatten().take(16) {
        if section["fields"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|f| string(f, "id") == field)
        {
            return format!(
                "{} [{}]",
                string(section, "label"),
                string(section, "station")
            );
        }
    }
    String::new()
}
fn edit_value(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(","),
        _ => value.to_string(),
    }
}
fn scaled(value: i64, scale: u64) -> String {
    if !matches!(scale, 10 | 100 | 1000) {
        return value.to_string();
    }
    let v = i128::from(value);
    let s = i128::from(scale);
    let width = match scale {
        10 => 1,
        100 => 2,
        _ => 3,
    };
    format!(
        "{}{}.{:0width$}",
        if v < 0 { "-" } else { "" },
        v.abs() / s,
        v.abs() % s
    )
}
fn widget_hint(widget: &Value) -> String {
    match string(widget, "kind") {
        "slider" => format!(
            "integer ticks {}..{}, step {}, scale {} {}",
            widget["min"],
            widget["max"],
            widget["step"],
            widget["scale"],
            string(widget, "unit")
        ),
        "choice" => format!(
            "choose {}",
            widget.get("choices").map(edit_value).unwrap_or_default()
        ),
        "toggle" => "true or false".into(),
        "bearing" => "bearing 0..359 degrees".into(),
        "facet_set" => "comma-separated front,left,right,rear,top".into(),
        "contact_class_set" => {
            "comma-separated friendly,hostile,unknown (earned relative contact classes)".into()
        }
        _ => "server validates this field".into(),
    }
}
fn field_line(field: &Value, value: Option<&Value>, utf8: bool) -> String {
    let widget = &field["widget"];
    let value = value.unwrap_or(&Value::Null);
    let rendered = match string(widget, "kind") {
        "slider" => {
            let v = value.as_i64().unwrap_or(0);
            let lo = widget["min"].as_i64().unwrap_or(0);
            let hi = widget["max"].as_i64().unwrap_or(lo);
            let filled = if hi > lo {
                ((i128::from(v) - i128::from(lo)).clamp(0, i128::from(hi) - i128::from(lo)) * 12
                    / (i128::from(hi) - i128::from(lo))) as usize
            } else {
                0
            };
            format!(
                "[{}{}] {} {} ({} ticks)",
                if utf8 { "━" } else { "#" }.repeat(filled),
                if utf8 { "·" } else { "." }.repeat(12 - filled),
                scaled(v, widget["scale"].as_u64().unwrap_or(1)),
                string(widget, "unit"),
                v
            )
        }
        "toggle" => format!(
            "[{}]",
            if value.as_bool() == Some(true) {
                "x"
            } else {
                " "
            }
        ),
        "bearing" => format!("{} deg", edit_value(value)),
        _ => edit_value(value),
    };
    format!(
        "{} ({})  {}",
        string(field, "label"),
        string(field, "id"),
        rendered
    )
}
fn adjust_value(widget: &Value, text: &str, up: bool) -> Option<String> {
    match string(widget, "kind") {
        "slider" => {
            let value = text.parse::<i64>().ok()?;
            let min = widget["min"].as_i64()?;
            let max = widget["max"].as_i64()?;
            let step = widget["step"].as_i64()?;
            if step <= 0 || max < min {
                return None;
            }
            Some(
                if up {
                    value.saturating_add(step)
                } else {
                    value.saturating_sub(step)
                }
                .clamp(min, max)
                .to_string(),
            )
        }
        "bearing" => Some(
            (text.parse::<i64>().ok()?.rem_euclid(360) + if up { 1 } else { 359 })
                .rem_euclid(360)
                .to_string(),
        ),
        "toggle" => Some((text != "true").to_string()),
        "choice" => {
            let choices = widget["choices"].as_array()?;
            if choices.is_empty() {
                return None;
            }
            let at = choices
                .iter()
                .position(|v| v.as_str() == Some(text))
                .unwrap_or(0);
            choices[(at + if up { 1 } else { choices.len() - 1 }) % choices.len()]
                .as_str()
                .map(str::to_owned)
        }
        _ => None,
    }
}
fn rule_glyph(text: &str, utf8: bool) -> &'static str {
    if text.starts_with("LOCK ") || text.starts_with("lock ") {
        if utf8 { "◎" } else { "L" }
    } else if (text.starts_with("FIRE ") && text.contains(" HOLD")) || text.contains(" holds fire")
    {
        if utf8 { "■" } else { "H" }
    } else if text.starts_with("FIRE ") || text.contains(" fires at ") {
        if utf8 { "→" } else { "F" }
    } else {
        "-"
    }
}
fn flight_section_label(schema: &Value, field: &str, channels: Option<&Value>) -> String {
    let label = section_label(schema, field);
    let station = schema["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .take(16)
        .find(|s| {
            s["fields"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|f| string(f, "id") == field)
        })
        .map(|s| string(s, "station"));
    if station.is_some_and(|station| {
        !channels
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|c| c.as_str() == Some(station))
    }) {
        format!("{label} (inert: station unavailable)")
    } else {
        label
    }
}

/// Whitelist public trace fields rather than rendering an arbitrary JSON object.
/// Limits mirror the server ring and detail ceilings; unknown source/memory keys
/// can never appear in this view even if a future payload adds them.
fn trace_rows(data: &Value) -> Vec<String> {
    fn text(v: &Value, key: &str) -> String {
        clean(string(v, key)).chars().take(120).collect()
    }
    fn number(v: &Value, key: &str) -> String {
        v.get(key)
            .and_then(Value::as_i64)
            .map_or_else(|| "unmeasured".into(), |n| n.to_string())
    }
    fn outcome(v: &Value) -> String {
        let state = text(v, "state");
        let detail = if v.get("reason").is_some() {
            text(v, "reason")
        } else {
            text(v, "fault")
        };
        if detail.is_empty() {
            state
        } else {
            format!("{state}: {detail}")
        }
    }
    fn truth(v: Option<&Value>) -> &'static str {
        match v.and_then(Value::as_bool) {
            Some(true) => "true",
            Some(false) => "false",
            None => "unavailable",
        }
    }
    let mut rows = Vec::new();
    if let Some(n) = data
        .get("omitted_thinks")
        .and_then(Value::as_u64)
        .filter(|n| *n > 0)
    {
        rows.push(format!(
            "{n} older thinks omitted from this bounded picture."
        ));
    }
    for row in data
        .get("traces")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(64)
    {
        let d = &row["diagnostics"];
        rows.push(format!(
            "Think {} at {} ms | {} ops + {} fees | {}{}",
            number(row, "think_seq"),
            number(row, "t_ms"),
            number(d, "ops"),
            number(d, "fees"),
            outcome(&row["outcome"]),
            if row["rom"].as_bool() == Some(true) {
                " | ROM"
            } else {
                ""
            }
        ));
        if let Some(n) = row["landed_ms"].as_i64() {
            rows.push(format!("  Landed at {n} ms"));
        }
        for key in ["fault", "rom_drop"] {
            if row[key].is_string() {
                rows.push(format!("  {key}: {}", text(row, key)));
            }
        }
        for kernel in d["kernels"].as_array().into_iter().flatten().take(32) {
            rows.push(format!(
                "  Kernel {}: {} fee",
                text(kernel, "kernel"),
                number(kernel, "fee")
            ));
        }
        for rule in d["rules"].as_array().into_iter().flatten().take(6) {
            rows.push(format!(
                "  S{} {}: {}",
                number(rule, "slot"),
                text(rule, "description"),
                truth(rule.get("matches"))
            ));
            for term in rule["terms"].as_array().into_iter().flatten().take(4) {
                rows.push(format!(
                    "    {}: {}",
                    text(term, "description"),
                    truth(term.get("value"))
                ));
            }
        }
        for intent in row["intents"].as_array().into_iter().flatten().take(16) {
            rows.push(format!(
                "  {} {}: {}",
                text(intent, "action"),
                text(intent, "detail"),
                outcome(&intent["outcome"])
            ));
        }
        for line in row["author"]
            .as_array()
            .into_iter()
            .flatten()
            .take(8)
            .filter_map(Value::as_str)
        {
            rows.push(format!(
                "  Trace: {}",
                clean(line).chars().take(120).collect::<String>()
            ));
        }
        for key in [
            "author_lines",
            "author_chars",
            "kernel_calls",
            "rules",
            "rule_terms",
            "detail_chars",
            "intents",
        ] {
            if let Some(n) = d["omitted"][key].as_u64().filter(|n| *n > 0) {
                rows.push(format!("  Omitted {key}: {n}"));
            }
        }
    }
    rows
}

fn flight_rows(flight: Option<&Value>, utf8: bool) -> Vec<String> {
    let Some(mind) = flight.filter(|m| m.is_object()) else {
        return vec!["No seated mind is currently visible.".into()];
    };
    let mut rows = vec![
        format!(
            "Sealed flight mind: {} | Class {}",
            string(mind, "name"),
            string(mind, "class")
        ),
        if mind.get("off").and_then(Value::as_bool) == Some(true) || string(mind, "class") == "0" {
            "Flying captured ROM fallback; authored selection remains sealed.".into()
        } else {
            "Left/Right selects; Enter prefills MIND PROFILE; another Enter sends. Card is read only.".into()
        },
    ];
    if let Some(profiles) = mind["profiles"].as_array() {
        for (i, p) in profiles.iter().take(4).enumerate() {
            rows.push(format!(
                "{}Profile {}: {}",
                if mind["active"].as_u64() == Some(i as u64) {
                    "> "
                } else {
                    "  "
                },
                i + 1,
                p.as_str().unwrap_or("?")
            ));
        }
    }
    if let Some(card) = mind.get("card") {
        rows.extend(fields(&card["schema"]).into_iter().map(|f| {
            format!(
                "{} / {}",
                flight_section_label(&card["schema"], string(f, "id"), mind.get("channels")),
                field_line(f, card["values"].get(string(f, "id")), utf8)
            )
        }));
    }
    if let Some(rules) = mind["sealed_rules"].as_array() {
        rows.extend(rules.iter().enumerate().filter_map(|(i, r)| {
            r.as_str()
                .map(|s| format!("S{} {} {}", i + 1, rule_glyph(s, utf8), s))
        }));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn draft() -> MindDraft {
        serde_json::from_value(json!({
            "source":"fn think(view, card, memory) {\n  // OWNED_SOURCE_ONLY\n  memory\n}",
            "schema":{"sections":[{"id":"gunnery","label":"Gunnery","station":"fire","fields":[
                {"id":"odds","label":"Minimum odds","widget":{"kind":"slider","min":0,"max":1000,"step":10,"default":500,"scale":10,"unit":"%","help":"Integer ticks; 10 ticks = 1%."}},
                {"id":"auto","label":"Automatic","widget":{"kind":"toggle","default":true,"help":""}},
                {"id":"face","label":"Facing","widget":{"kind":"bearing","default":359,"help":""}},
                {"id":"kind","label":"Target kind","widget":{"kind":"choice","choices":["fast","slow"],"default":"fast","help":""}},
                {"id":"facets","label":"Facets","widget":{"kind":"facet_set","default":["front","top"],"help":""}},
                {"id":"classes","label":"Contact classes","widget":{"kind":"contact_class_set","default":["hostile","unknown"],"help":""}}
            ]}]},
            "presets":{"Balanced":{"odds":500}},
            "profiles":{
                "Alpha long profile":{"odds":500,"auto":true,"face":359,"kind":"fast","facets":["front","top"],"classes":["hostile","unknown"]},
                "Bravo long profile":{}, "Charlie long profile":{}, "Delta long profile":{}
            },
            "rules":{},"declared_class":"III","mind_abi":"tacsim.mind.v1"
        })).unwrap()
    }

    fn read_response(base: MindEditBase) -> MindResponse {
        MindResponse::Read {
            name: "A rather long authored mind name".into(),
            revision: None,
            base,
            draft: Box::new(draft()),
            rules_text: BTreeMap::from([(
                "Alpha long profile".into(),
                vec!["cannon holds fire when heat is above 80%".into()],
            )]),
            rules_source: BTreeMap::from([(
                "Alpha long profile".into(),
                vec![Some("FIRE cannon HOLD WHEN heat>80%".into())],
            )]),
            diagnostics: vec![],
        }
    }

    fn loaded() -> MindWorkshop {
        let mut state = MindWorkshop {
            open: true,
            ..MindWorkshop::default()
        };
        state.send(MindRequest::Read {
            name: "A rather long authored mind name".into(),
            revision: None,
        });
        let (id, _) = state.take_invokes().pop().unwrap();
        state.accept(
            id,
            &InvokeResult::Mind(read_response(MindEditBase {
                edit: 7,
                revision: Some(3),
            })),
        );
        state
    }

    #[test]
    fn m6_workshop_slider_changes_only_on_enter_and_uses_exact_base() {
        let mut state = loaded();
        state.key(Key::Enter); // open the form, no send
        state.key(Key::Up); // 500 -> 510 local ticks
        assert!(state.take_invokes().is_empty());
        assert_eq!(state.form.as_ref().unwrap().line.text(), "510");
        state.key(Key::Enter);
        let (_, InvokeRequest::Mind(request)) = state.take_invokes().pop().unwrap() else {
            panic!("expected workshop Mind request");
        };
        assert_eq!(
            request,
            MindRequest::Edit {
                name: "A rather long authored mind name".into(),
                base: MindEditBase {
                    edit: 7,
                    revision: Some(3)
                },
                edit: MindEdit::Set {
                    profile: "Alpha long profile".into(),
                    field: "odds".into(),
                    value: "510".into()
                }
            }
        );
    }

    #[test]
    fn m6_workshop_stale_refusal_keeps_input_and_original_profile_target() {
        let mut state = loaded();
        state.key(Key::Enter);
        state.key(Key::Up);
        state.key(Key::Enter);
        let (id, _) = state.take_invokes().pop().unwrap();
        state.accept(
            id,
            &InvokeResult::Mind(MindResponse::Error {
                code: "stale".into(),
                message: "Draft changed".into(),
            }),
        );
        assert_eq!(state.form.as_ref().unwrap().line.text(), "510");
        state.key(Key::Enter);
        assert!(state.take_invokes().is_empty());
        state.key(Key::Ctrl('r'));
        let (id, InvokeRequest::Mind(request)) = state.take_invokes().pop().unwrap() else {
            panic!("expected workshop Mind request");
        };
        assert!(matches!(request, MindRequest::Read { revision: None, .. }));
        let mut response = read_response(MindEditBase {
            edit: 8,
            revision: Some(4),
        });
        if let MindResponse::Read { draft, .. } = &mut response {
            draft
                .profiles
                .as_object_mut()
                .unwrap()
                .insert("A new preceding profile".into(), json!({}));
        }
        state.accept(id, &InvokeResult::Mind(response));
        state.key(Key::Enter);
        let (_, InvokeRequest::Mind(request)) = state.take_invokes().pop().unwrap() else {
            panic!("expected workshop Mind request");
        };
        assert!(
            matches!(request, MindRequest::Edit { base:MindEditBase {edit:8,revision:Some(4)}, edit:MindEdit::Set { profile, value, .. }, .. } if profile == "Alpha long profile" && value == "510")
        );
    }

    #[test]
    fn m6_workshop_rule_prefill_uses_server_grammar_not_english() {
        let mut state = loaded();
        state.page = Page::Rules;
        assert!(state.rows(true, None).0[0].contains("■ cannon holds fire"));
        state.key(Key::Enter);
        assert_eq!(
            state.form.as_ref().unwrap().line.text(),
            "FIRE cannon HOLD WHEN heat>80%"
        );
        assert!(state.take_invokes().is_empty());
        state.key(Key::Enter);
        let (_, InvokeRequest::Mind(request)) = state.take_invokes().pop().unwrap() else {
            panic!("expected workshop Mind request");
        };
        assert!(
            matches!(request, MindRequest::Edit { edit:MindEdit::Rule {slot:1,text,..}, .. } if text == "FIRE cannon HOLD WHEN heat>80%")
        );
    }

    #[test]
    fn m6_workshop_unspellable_rule_never_becomes_an_empty_delete() {
        let mut state = loaded();
        state.page = Page::Rules;
        state
            .loaded
            .as_mut()
            .unwrap()
            .rule_sources
            .insert("Alpha long profile".into(), vec![None]);
        state.key(Key::Enter);
        state.key(Key::Enter);
        assert!(state.take_invokes().is_empty());
        for c in "OFF".chars() {
            state.key(Key::Char(c));
        }
        state.key(Key::Enter);
        let (_, InvokeRequest::Mind(request)) = state.take_invokes().pop().unwrap() else {
            panic!("expected workshop Mind request");
        };
        assert!(
            matches!(request, MindRequest::Edit {edit:MindEdit::Rule {slot:1,text,..},..} if text.is_empty())
        );
    }

    #[test]
    fn m6_library_lifecycle_uses_exact_loaded_base_and_explicit_confirmation() {
        let mut state = loaded();
        let base = state.loaded.as_ref().unwrap().base;
        let name = state.loaded.as_ref().unwrap().name.clone();
        state.key(Key::Char('X'));
        assert!(state.take_invokes().is_empty());
        state.key(Key::Enter);
        let (id, request) = state.take_invokes().pop().unwrap();
        assert_eq!(
            request,
            InvokeRequest::Mind(MindRequest::Lifecycle {
                name: name.clone(),
                base,
                edit: MindLifecycle::Remove {}
            })
        );
        state.accept(
            id,
            &InvokeResult::Mind(MindResponse::Error {
                code: "stale".into(),
                message: "Working copy changed".into(),
            }),
        );
        state.key(Key::Enter);
        assert!(
            state.take_invokes().is_empty(),
            "a stale lifecycle form never resubmits without refresh"
        );
        state.form = None;
        state.key(Key::Char('g'));
        assert!(state.take_invokes().is_empty());
        state.key(Key::Enter);
        let (id, request) = state.take_invokes().pop().unwrap();
        assert_eq!(
            request,
            InvokeRequest::Mind(MindRequest::Prune { limit: 128 })
        );
        state.accept(
            id,
            &InvokeResult::Mind(MindResponse::Pruned {
                revisions: 3,
                source_candidates_retained: 2,
            }),
        );
        assert!(
            state
                .report
                .join("\n")
                .contains("Retained 2 immutable source")
        );
        let mut state = loaded();
        state.key(Key::Char('v'));
        for c in "old=new".chars() {
            state.key(Key::Char(c));
        }
        state.key(Key::Enter);
        let (_, InvokeRequest::Mind(request)) = state.take_invokes().pop().unwrap() else {
            panic!("expected workshop Mind request");
        };
        assert!(
            matches!(request, MindRequest::Lifecycle {base:b,edit:MindLifecycle::VersionRename {old,new},..} if b==base && old=="old" && new=="new")
        );
        let mut state = loaded();
        state.loaded.as_mut().unwrap().revision = Some("1".into());
        for key in [
            Key::Char('N'),
            Key::Char('X'),
            Key::Char('v'),
            Key::Char('V'),
        ] {
            state.key(key);
        }
        assert!(
            state.form.is_none() && state.take_invokes().is_empty(),
            "saved revision is not an editable base"
        );
    }

    #[test]
    fn m6_workshop_source_is_read_only_and_revision_cannot_mutate() {
        let mut state = loaded();
        state.page = Page::Source;
        let before = state.loaded.as_ref().unwrap().draft.clone();
        for key in [
            Key::Char('a'),
            Key::Backspace,
            Key::Delete,
            Key::Enter,
            Key::Up,
        ] {
            state.key(key);
        }
        assert_eq!(state.loaded.as_ref().unwrap().draft, before);
        assert!(state.take_invokes().is_empty());
        assert!(state.form.is_none());
        assert!(
            state
                .rows(true, None)
                .0
                .join("\n")
                .contains("OWNED_SOURCE_ONLY")
        );
        state.loaded.as_mut().unwrap().revision = Some("3".into());
        state.page = Page::Card;
        for key in [Key::Enter, Key::Char('c'), Key::Char('s'), Key::Char('+')] {
            state.key(key);
        }
        assert!(state.form.is_none());
        assert!(state.take_invokes().is_empty());
    }

    fn trace_data(unit: u64, seq: u64) -> Value {
        json!({"available":true,"unit":unit,"omitted_thinks":1,
            "source":"PRIVATE_SOURCE","memory":{"private":"PRIVATE_MEMORY"},
            "traces":[{"think_seq":seq,"t_ms":1000,"landed_ms":1020,"rom":false,
                "outcome":{"state":"applied"}, "fault":null,"rom_drop":null,
                "diagnostics":{"ops":800,"fees":5,"kernels":[{"kernel":"odds","fee":5}],
                    "rules":[{"slot":1,"description":"cannon fires at its lock","matches":true,
                        "terms":[{"description":"identified hostile","value":true}]}],
                    "omitted":{"kernel_calls":2}},
                "intents":[{"action":"fire","detail":"cannon at lock","outcome":{"state":"refused","reason":"cycling"}}],
                "author":["waiting for the gun"]}]})
    }

    #[test]
    fn m6_trace_requires_correlated_selector_and_revokes_late_feed_or_reply() {
        let mut state = loaded();
        let cockpit = json!({"unit":42,"mind":{"profiles":["Line"]}});
        state.key_with_flight(Key::Char('f'), Some(&cockpit));
        assert!(
            state.take_invokes().is_empty(),
            "selection needs explicit Enter"
        );
        state.key_with_flight(Key::Enter, Some(&cockpit));
        let (id, request) = state.take_invokes().pop().unwrap();
        assert_eq!(
            request,
            InvokeRequest::Mind(MindRequest::Trace {
                unit: 42,
                count: Some(3),
                follow: true
            })
        );
        state.trace_feed(&trace_data(42, 1));
        assert!(
            state.traces.is_empty(),
            "unsolicited/pre-reply feed cannot establish authority"
        );
        state.accept(
            id,
            &InvokeResult::Mind(MindResponse::Trace {
                data: trace_data(42, 1),
            }),
        );
        assert!(state.traces.join("\n").contains("Think 1"));
        state.trace_feed(&trace_data(43, 2));
        assert!(!state.traces.join("\n").contains("Think 2"));
        state.trace_feed(&trace_data(42, 3));
        assert!(state.traces.join("\n").contains("Think 3"));
        state.trace_feed(&json!({"available":false,"traces":[]}));
        assert!(state.traces.is_empty());
        state.trace_feed(&trace_data(42, 4));
        assert!(
            state.traces.is_empty(),
            "revocation disables later followed updates"
        );
        state.key_with_flight(Key::Char('f'), Some(&cockpit));
        state.key_with_flight(Key::Enter, Some(&cockpit));
        let (old, _) = state.take_invokes().pop().unwrap();
        state.reset();
        state.trace_feed(&trace_data(42, 5));
        state.accept(
            old,
            &InvokeResult::Mind(MindResponse::Trace {
                data: trace_data(42, 6),
            }),
        );
        assert!(state.traces.is_empty());
        assert_eq!(
            state.trace_unit, None,
            "attachment/character reset forgets selector"
        );
    }

    #[test]
    fn m6_trace_public_rows_and_flight_station_fallback_are_bounded() {
        let data = trace_data(42, 7);
        assert_eq!(
            trace_rows(&data).join("\n"),
            include_str!("testdata/m6-trace-rows.txt").trim_end()
        );
        let rows = trace_rows(&data).join("\n");
        assert!(!rows.contains("PRIVATE_SOURCE") && !rows.contains("PRIVATE_MEMORY"));
        let mut state = loaded();
        state.traces = trace_rows(&data);
        state.trace_unit = Some(42);
        state.page = Page::Trace;
        for width in [120_u16, 160] {
            let backend = ratatui::backend::TestBackend::new(width, 34);
            let mut terminal = ratatui::Terminal::new(backend).unwrap();
            terminal
                .draw(|f| state.draw(f, f.area(), true, None))
                .unwrap();
            let buffer = terminal.backend().buffer();
            let text = (0..34)
                .map(|y| {
                    (0..width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");
            for expected in [
                "Hull #42",
                "Think 7",
                "800 ops + 5 fees",
                "identified hostile: true",
                "refused: cycling",
                "waiting for the gun",
            ] {
                assert!(text.contains(expected), "{width}: missing {expected}");
            }
            assert!(!text.contains("PRIVATE_SOURCE") && !text.contains("PRIVATE_MEMORY"));
        }
        let flight = json!({"name":"Pilot","class":"0","off":true,"active":0,"profiles":["Line"],
            "channels":["helm","sensors"],"card":{"schema":draft().schema,"values":draft().profiles["Alpha long profile"]}});
        let rows = flight_rows(Some(&flight), false).join("\n");
        assert!(rows.contains("captured ROM fallback"));
        assert!(rows.contains("Gunnery [fire] (inert: station unavailable)"));
        assert!(!rows.contains("Navigation [helm] (inert"));
        let mut unmeasured = trace_data(42, 8);
        unmeasured["traces"][0]["diagnostics"]["ops"] = Value::Null;
        unmeasured["traces"][0]["diagnostics"]["fees"] = Value::Null;
        assert!(
            trace_rows(&unmeasured)
                .join("\n")
                .contains("unmeasured ops + unmeasured fees")
        );
    }

    #[test]
    fn m6_workshop_session_reset_forgets_source_and_rejects_late_reply() {
        let mut state = loaded();
        state.refresh();
        let (old, _) = state.take_invokes().pop().unwrap();
        state.reset();
        state.show();
        let (new, _) = state.take_invokes().pop().unwrap();
        assert!(new > old);
        state.accept(
            old,
            &InvokeResult::Mind(read_response(MindEditBase {
                edit: 99,
                revision: None,
            })),
        );
        assert!(state.loaded.is_none());
        assert_eq!(state.pending.as_ref().unwrap().id, new);
    }

    #[test]
    fn m6_workshop_whole_edit_reads_after_success_without_auto_resubmission() {
        let mut state = loaded();
        state.key(Key::Char('s'));
        assert!(state.take_invokes().is_empty());
        state.key(Key::Enter);
        let (id, _) = state.take_invokes().pop().unwrap();
        state.accept(id, &InvokeResult::Mind(MindResponse::Queued { job: 42 }));
        assert!(state.take_invokes().is_empty());
        state.accept(
            id,
            &InvokeResult::Mind(MindResponse::Saved {
                job: 42,
                revision: 4,
                digest: "sha".into(),
                base: MindEditBase {
                    edit: 8,
                    revision: Some(4),
                },
            }),
        );
        let (_, InvokeRequest::Mind(request)) = state.take_invokes().pop().unwrap() else {
            panic!("expected workshop Mind request");
        };
        assert!(matches!(request, MindRequest::Read { revision: None, .. }));
        assert!(state.form.is_none());
    }

    #[test]
    fn m6_workshop_all_widgets_exact_integers_and_rule_cap() {
        let mut state = loaded();
        let rows = state.rows(true, None).0.join("\n");
        assert_eq!(
            rows.trim_end(),
            include_str!("testdata/m6-card-rows.txt").trim_end()
        );
        assert_eq!(scaled(-9007199254740991, 1000), "-9007199254740.991");
        assert_eq!(
            adjust_value(&json!({"kind":"bearing"}), "359", true).as_deref(),
            Some("0")
        );
        assert_eq!(
            adjust_value(
                &json!({"kind":"slider","min":0,"max":9007199254740991_i64,"step":1}),
                "9007199254740990",
                true
            )
            .as_deref(),
            Some("9007199254740991")
        );
        for (class, count) in [("I", 2), ("II", 4), ("III", 6)] {
            state.loaded.as_mut().unwrap().draft.declared_class = class.into();
            state.page = Page::Rules;
            assert_eq!(state.rows(true, None).0.len(), count);
        }
        for _ in 0..3 {
            state.key(Key::Right);
        }
        assert_eq!(state.profile_name().as_deref(), Some("Delta long profile"));
    }

    #[test]
    fn m6_workshop_long_unicode_rules_and_flight_profiles_are_reachable() {
        for width in [120_u16, 160] {
            let mut state = loaded();
            state.page = Page::Flight;
            let flight = json!({"name":"A long sealed mind name", "class":"III", "active":3,
                "profiles":["Alpha long profile","Bravo long profile","Charlie long profile","Delta long profile"],
                "card":{"schema":draft().schema,"values":draft().profiles["Alpha long profile"]},
                "sealed_rules":["lock the nearest identified hostile"]});
            let backend = ratatui::backend::TestBackend::new(width, 34);
            let mut terminal = ratatui::Terminal::new(backend).unwrap();
            terminal
                .draw(|f| state.draw(f, f.area(), true, Some(&flight)))
                .unwrap();
            let buffer = terminal.backend().buffer();
            let text = (0..34)
                .map(|y| {
                    (0..width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");
            for expected in [
                "Alpha long profile",
                "Bravo long profile",
                "Charlie long profile",
                "Delta long profile",
                "Class III",
                "Gunnery",
                "50.0 %",
            ] {
                {
                    assert!(text.contains(expected), "{width}: missing {expected}");
                }
            }
        }
        let long = "界".repeat(200);
        let rows = wrap_cells(&long, 118);
        assert_eq!(rows.concat(), long);
        assert!(rows.iter().all(|r| Line::from(r.as_str()).width() <= 118));
        assert_eq!(rule_glyph("cannon holds fire when hot", false), "H");
    }
    #[test]
    fn attributed_removal_selects_library_metadata_and_waits_for_actual_refresh() {
        let mut state = MindWorkshop::default();
        state.open = true;
        state.names = vec!["marked".into()];
        state.key(Key::Char('X'));
        state.key(Key::Enter);
        let (id, request) = state.take_invokes().pop().expect("name-only request");
        assert_eq!(
            request,
            InvokeRequest::Mind(MindRequest::Remove {
                name: "marked".into()
            })
        );
        assert!(state.loaded.is_none());
        state.accept(
            id,
            &InvokeResult::Mind(MindResponse::RemovalRequested {
                name: "marked".into(),
            }),
        );
        assert_eq!(state.names, vec!["marked"]);
        assert_eq!(state.removal_requested, vec!["marked"]);
        state.key(Key::Char('X'));
        assert!(state.take_invokes().is_empty());
        state.key(Key::Char('r'));
        let (id, request) = state.take_invokes().pop().expect("refresh");
        assert_eq!(request, InvokeRequest::Mind(MindRequest::List {}));
        state.accept(
            id,
            &InvokeResult::Mind(MindResponse::Library { names: vec![] }),
        );
        assert!(state.names.is_empty() && state.removal_requested.is_empty());
        assert!(state.message.contains("no longer appears"));
    }
    #[test]
    fn requested_removal_keeps_loaded_editor_and_refusal_keeps_selection() {
        let mut state = loaded();
        let name = state.loaded.as_ref().expect("loaded").name.clone();
        let before = state.loaded.as_ref().expect("loaded").base;
        state.key(Key::Char('X'));
        state.key(Key::Enter);
        let (id, _) = state.take_invokes().pop().expect("lifecycle request");
        state.accept(
            id,
            &InvokeResult::Mind(MindResponse::RemovalRequested { name: name.clone() }),
        );
        assert_eq!(state.loaded.as_ref().expect("retained").base, before);
        assert_eq!(state.removal_requested, vec![name]);
        // RemovalRequested returns to Library but deliberately retains the
        // fresh editor. Switching to it must not reopen the legacy lifecycle
        // route as a second removal request while custody is unresolved.
        assert_eq!(state.page, Page::Library);
        let next_id = state.next_id;
        state.key(Key::Tab);
        assert_eq!(state.page, Page::Card);
        state.key(Key::Char('X'));
        state.key(Key::Enter);
        assert!(state.take_invokes().is_empty());
        assert!(state.pending.is_none());
        assert_eq!(state.next_id, next_id);
        assert!(state.message.contains("already requested"));
        assert_eq!(state.loaded.as_ref().expect("retained").base, before);
        let mut state = MindWorkshop::default();
        state.open = true;
        state.names = vec!["marked".into()];
        state.key(Key::Char('X'));
        state.key(Key::Enter);
        let (id, _) = state.take_invokes().pop().expect("request");
        state.accept(
            id,
            &InvokeResult::Mind(MindResponse::Error {
                code: "stale".into(),
                message: "Unavailable".into(),
            }),
        );
        assert_eq!(state.names, vec!["marked"]);
        assert!(state.removal_requested.is_empty());
    }
}
