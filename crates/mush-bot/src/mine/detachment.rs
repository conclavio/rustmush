//! Hired-crew expedition steps. All inputs are this SSH session's terminal text.
//! Uses the mining lane's buffering, echo suppression, pacing and retry bounds.
use super::{ANSWER_MS, Beat, Kind, Lane, Milestone, Verdict, lines};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub npc: u64,
    pub carrier: u64,
    pub units: Vec<u64>,
    pub operation: String,
    pub contract: String,
    pub chat_received: bool,
    pub wilco_received: bool,
    pub returned: bool,
    pub unseated: bool,
    pub closed: bool,
    pub hold: String,
}
#[derive(Default)]
pub(super) struct Expedition {
    npc: u64,
    actor: u64,
    units: [u64; 2],
    loan: u64,
    loan2: u64,
    carrier: u64,
    hire: String,
    draft: String,
    consent: String,
    operation: String,
    alias: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Lease,
    Loans1,
    ReadLoans1,
    Loans2,
    ReadLoans2,
    Grants,
    Concourse,
    Slack,
    Offers,
    Hire,
    Contract,
    BoardCrew,
    BackConcourse,
    Exchange,
    Passage,
    Hold,
    Bay,
    BoardCaptain,
    Focus,
    Handoff,
    Draft,
    Plans,
    Plan,
    Accept,
    Ready,
    Activate,
    Active,
    Seat,
    Chat,
    SendChat,
    Recall,
    RadioReceived,
    Arrival,
    HomeArrival,
    Launch,
    Field,
    Refresh,
    RefreshAgain,
    Orders,
    Order,
    Reply,
    NpcReturned,
    HoldRead,
    Recover,
    BothReturned,
    LeaveCaptain,
    Unseat,
    BayOut,
    BackPassage,
    Airlock,
    End,
    EndRead,
    Deactivate,
    Closed,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Task {
    pub phase: Phase,
    marker: String,
    poll: bool,
}
const RADIO_CHECK: &str = "Expedition radio check: ready to mine.";

pub(super) fn verdict(task: &Task, opened: u64, text: &str, quiet: bool, now: u64) -> Verdict {
    if matches!(task.phase, Phase::Reply) && lines::has(text, "NEGATIVE received") {
        return Verdict::Fail(format!("NPC refused the mining order: {text}"));
    }
    let complete = if matches!(task.phase, Phase::HoldRead) {
        number_after(text, "metal ore, run-of-mine: ").is_ok() && quiet
    } else if task.marker.is_empty() {
        now >= opened.saturating_add(2_000)
    } else if matches!(task.phase, Phase::Field | Phase::BothReturned) {
        text.matches(&task.marker).count() == 2 && quiet
    } else {
        lines::has(text, &task.marker) && quiet
    };
    if complete {
        return Verdict::Done;
    }
    if task.poll && quiet {
        return Verdict::Retry {
            needle: "awaiting expedition fact",
            wait: 3_000,
            tries: match task.phase {
                Phase::NpcReturned | Phase::Arrival | Phase::HomeArrival => 300,
                Phase::Field | Phase::BothReturned => 80,
                _ => 20,
            },
        };
    }
    Verdict::Wait
}
fn number_after(text: &str, prefix: &str) -> Result<u64, String> {
    let tail = text
        .split_once(prefix)
        .ok_or_else(|| format!("missing {prefix:?} in {text}"))?
        .1;
    tail.chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| format!("invalid number after {prefix:?}"))
}
fn printed(text: &[String], prefix: &str) -> Result<String, String> {
    text.iter()
        .find_map(|l| l.trim().strip_prefix(prefix).map(str::to_owned))
        .ok_or_else(|| format!("missing printed command {prefix:?}: {}", text.join(" ")))
}
impl Lane {
    pub(super) fn expedition_start(&mut self) {
        self.dt(Phase::Lease, "+tv/lease go", lines::LEASE_SIGNED, false, 0);
    }
    pub(super) fn expedition_board(&mut self) {
        self.dt(Phase::Passage, "in", "Central Passage", false, 0);
    }
    pub(super) fn expedition_arrival(&mut self, home: bool) {
        let (phase, marker) = if home {
            (Phase::HomeArrival, "docked Ceres Freeport".to_owned())
        } else {
            (
                Phase::Arrival,
                format!("settled on the pad at {}", self.site.name()),
            )
        };
        self.dt(phase, "+ship", &marker, true, 0);
    }
    pub(super) fn expedition_launch(&mut self) {
        let op = self.dt_state().operation.clone();
        self.dt(
            Phase::Launch,
            &format!("+sortie/launch {op}=all"),
            "",
            false,
            0,
        );
    }
    fn dt_state(&mut self) -> &mut Expedition {
        self.expedition.get_or_insert_default()
    }
    fn dt(&mut self, phase: Phase, command: &str, marker: &str, poll: bool, not_before: u64) {
        if !in_vocabulary(command) {
            self.fail(
                not_before,
                format!("detachment command outside its vocabulary: {command}"),
            );
            return;
        }
        // A committed action's receipt is distinct from its queued notification.
        // Loan requests use their next read to establish the actual committed loans.
        let marker = if marker.is_empty() {
            match phase {
                Phase::Loans1 | Phase::Loans2 => "Occator is recording",
                Phase::Handoff => "You take foreground control",
                Phase::Recall => "Channel message queued for IC transit.",
                _ => "Done.",
            }
        } else {
            marker
        };
        let beat = match phase {
            Phase::Arrival
            | Phase::Passage
            | Phase::Hold
            | Phase::Bay
            | Phase::BoardCaptain
            | Phase::Focus
            | Phase::Handoff
            | Phase::Draft
            | Phase::Plans
            | Phase::Plan
            | Phase::Accept
            | Phase::Ready
            | Phase::Activate
            | Phase::Active
            | Phase::Seat
            | Phase::Chat
            | Phase::SendChat
            | Phase::Recall
            | Phase::RadioReceived => Beat::BoardAndLift,
            Phase::Launch | Phase::Field => Beat::OnPad,
            Phase::Refresh
            | Phase::RefreshAgain
            | Phase::Orders
            | Phase::Order
            | Phase::Reply
            | Phase::NpcReturned
            | Phase::HoldRead => Beat::DriveStakeMine,
            Phase::HomeArrival
            | Phase::Recover
            | Phase::BothReturned
            | Phase::LeaveCaptain
            | Phase::Unseat
            | Phase::BayOut
            | Phase::BackPassage
            | Phase::Airlock
            | Phase::End
            | Phase::EndRead
            | Phase::Deactivate
            | Phase::Closed => Beat::Home,
            _ => Beat::KitUp,
        };
        self.send(
            beat,
            Kind::Detachment(Task {
                phase,
                marker: marker.into(),
                poll,
            }),
            command,
            not_before,
        );
        if let Some(step) = &mut self.step {
            step.timeout_ms = ANSWER_MS;
        }
    }
    pub(super) fn expedition_next(
        &mut self,
        phase: Phase,
        raw: &[String],
        now: u64,
    ) -> Result<(), String> {
        use Phase::*;
        let text = raw.join(" ");
        match phase {
            Arrival => {
                self.milestone(Milestone::ArrivedSite, now);
                self.send(Beat::Witness, Kind::MapSite, "+tv/map site", 0);
            }
            HomeArrival => {
                self.milestone(Milestone::DockedHome, now);
                self.send(Beat::Home, Kind::Leave, "+ship/leave", 0);
            }
            Lease => self.dt(Loans1, "+tv/1/permissions go", "", false, 0),
            Loans1 => self.dt(ReadLoans1, "+tv/1/permissions", "Rig 1 loan", true, 0),
            ReadLoans1 => {
                let line = raw
                    .iter()
                    .find(|l| l.contains("operate_crew allowed now"))
                    .ok_or("rig 1 lacks a current operating loan")?;
                self.dt_state().loan = number_after(line, "Rig 1 loan ")?;
                self.dt(Loans2, "+tv/2/permissions go", "", false, 0);
            }
            Loans2 => self.dt(ReadLoans2, "+tv/2/permissions", "Rig 2 loan", true, 0),
            ReadLoans2 => {
                let line = raw
                    .iter()
                    .find(|l| l.contains("operate_crew allowed now"))
                    .ok_or("rig 2 lacks a current operating loan")?;
                self.dt_state().loan2 = number_after(line, "Rig 2 loan ")?;
                self.dt(Grants, "@unitgrant/list", "\"scope\"", false, 0);
            }
            Grants => {
                let json_text = raw.join("");
                let json_text = json_text
                    .get(json_text.find('{').ok_or("grant JSON missing")?..)
                    .ok_or("grant JSON missing")?;
                let grants: Vec<Value> = serde_json::Deserializer::from_str(json_text)
                    .into_iter::<Value>()
                    .map_while(Result::ok)
                    .collect();
                let loan = self.dt_state().loan;
                let unit = |g: &Value| {
                    g["scope"]["Units"]["units"]
                        .as_array()
                        .filter(|v| v.len() == 1)
                        .and_then(|v| v.first())
                        .and_then(Value::as_u64)
                };
                let first = grants
                    .iter()
                    .find(|g| g["id"].as_u64() == Some(loan))
                    .and_then(unit)
                    .ok_or("operating loan's public unit scope missing")?;
                let loan2 = self.dt_state().loan2;
                let second = grants
                    .iter()
                    .find(|g| g["id"].as_u64() == Some(loan2))
                    .and_then(unit)
                    .filter(|u| *u != first)
                    .ok_or("second rental unit missing")?;
                self.dt_state().units = [first, second];
                self.report.detachment.as_mut().ok_or("evidence")?.units = vec![first, second];
                self.dt(Concourse, "concourse", lines::ROOM_CONCOURSE, false, 0);
            }
            Concourse => self.dt(Slack, "slack", "The Slack Line", false, 0),
            Slack => self.dt(
                Offers,
                "+sortie/crew",
                "These offers have no wage charge",
                false,
                0,
            ),
            Offers => {
                let start = raw
                    .iter()
                    .position(|l| l.starts_with("Ilya Mendes (#"))
                    .ok_or("Ilya Mendes is not available for hire in this port")?;
                let hire = printed(&raw[start..], "Accept: ")?;
                let npc = number_after(&hire, "+sortie/hire #")?;
                let state = self.dt_state();
                state.npc = npc;
                state.hire = hire.clone();
                self.report.detachment.as_mut().ok_or("evidence")?.npc = npc;
                self.dt(Hire, &hire, "", false, 0);
            }
            Hire => {
                let npc = self.dt_state().npc;
                self.dt(
                    Contract,
                    &format!("+sortie/contract #{npc}"),
                    "Contract recorded.",
                    true,
                    0,
                );
            }
            Contract => {
                self.dt_state().actor = number_after(&text, "employer #")?;
                self.report.detachment.as_mut().ok_or("evidence")?.contract = text;
                let npc = self.dt_state().npc;
                self.dt(
                    BoardCrew,
                    &format!("+sortie/board #{npc}"),
                    "under your crew contract",
                    false,
                    0,
                );
            }
            BoardCrew => self.dt(BackConcourse, "concourse", lines::ROOM_CONCOURSE, false, 0),
            BackConcourse => self.dt(Exchange, "exchange", lines::ROOM_EXCHANGE, false, 0),
            Exchange => self.send(Beat::KitUp, Kind::Chart, "+chart", 0),
            Passage => self.dt(Hold, "hold", "Cargo Hold", false, 0),
            Hold => self.dt(Bay, "+tv/bay in", "entered vehicle bay", false, 0),
            Bay => {
                let carrier = number_after(&text, "Carrier #")?;
                self.dt_state().carrier = carrier;
                self.report.detachment.as_mut().ok_or("evidence")?.carrier = carrier;
                let (npc, unit, loan) = {
                    let d = self.dt_state();
                    (d.npc, d.units[0], d.loan)
                };
                // This is an ordinary delegated permission, never a staff bypass.
                self.dt(
                    BoardCaptain,
                    &format!("@unitgrant/issue #{unit}=#{npc} operate_crew parent={loan}"),
                    "",
                    false,
                    0,
                );
            }
            BoardCaptain => self.dt(Focus, "+tv/2 board", "Choose +focus", false, 0),
            Focus => {
                let command = printed(raw, "You are at rig ").and_then(|_| {
                    text.split_once("Choose ")
                        .and_then(|(_, s)| s.split_once(" then "))
                        .map(|(c, _)| c.to_owned())
                        .ok_or("focus command missing".into())
                })?;
                self.dt(Handoff, &command, "", false, 0);
            }
            Handoff => {
                let unit = self.dt_state().units[1];
                self.dt(
                    Draft,
                    &format!("@station/handoff #{unit}/pilot"),
                    "",
                    false,
                    0,
                );
            }
            Draft => {
                let d = self.dt_state();
                let plan = json!({"carrier":d.carrier,"slots":[
                    {"slot":1,"unit":d.units[0],"controller":{"kind":"npc","character":d.npc},"role":"miner","mind":null},
                    {"slot":2,"unit":d.units[1],"controller":{"kind":"player","character":d.actor},"role":"scout","mind":null}],
                    "npc_commander":null,"financial_terms":null});
                self.dt(Plans, &format!("+sortie/draft {plan}"), "", false, 0);
            }
            Plans => self.dt(Plan, "+sortie/plans", "Draft ", true, 0),
            Plan => {
                let id = number_after(&text, "Draft ")?;
                self.dt_state().draft = id.to_string();
                self.dt(
                    Accept,
                    &format!("+sortie/plan {id}"),
                    "+sortie/accept ",
                    false,
                    0,
                );
            }
            Accept => {
                let accept = format!("+sortie/accept {}", printed(raw, "+sortie/accept ")?);
                self.dt_state().consent = accept.trim_start_matches("+sortie/accept ").into();
                self.dt(Ready, &accept, "", false, 0);
            }
            Ready => {
                let fence = self.dt_state().consent.clone();
                self.dt(Activate, &format!("+sortie/ready {fence}"), "", false, 0);
            }
            Activate => {
                let draft = self.dt_state().draft.clone();
                self.dt(
                    Active,
                    &format!("+sortie/activate {draft}:1=mining"),
                    "",
                    false,
                    0,
                );
            }
            Active => {
                let draft = self.dt_state().draft.clone();
                self.dt(
                    Seat,
                    &format!("+sortie/active {draft}"),
                    "Prepared detachment",
                    true,
                    0,
                );
            }
            Seat => {
                let release = printed(raw, "Release: +sortie/deactivate ")?;
                self.dt_state().operation = release.clone();
                self.report.detachment.as_mut().ok_or("evidence")?.operation = release.clone();
                self.dt(
                    Chat,
                    &format!("+sortie/seat {release}=1"),
                    "Launch checks the actual helm lease again",
                    false,
                    0,
                );
            }
            Chat => self.dt(SendChat, "+sortie/chat", "Detachment chat [", false, 0),
            SendChat => {
                let alias = text
                    .split_once("]: ")
                    .and_then(|(_, s)| s.split_whitespace().next())
                    .ok_or("chat alias missing")?;
                if !alias.bytes().all(|c| c.is_ascii_alphanumeric()) {
                    return Err("invalid chat alias".into());
                }
                self.dt_state().alias = alias.into();
                self.dt(
                    Recall,
                    &format!("+sortie/chat ={RADIO_CHECK}"),
                    "",
                    false,
                    0,
                );
            }
            Recall => {
                let alias = self.dt_state().alias.clone();
                self.dt(
                    RadioReceived,
                    &format!("@channel/recall {alias}"),
                    RADIO_CHECK,
                    true,
                    0,
                );
            }
            RadioReceived => {
                self.report
                    .detachment
                    .as_mut()
                    .ok_or("evidence")?
                    .chat_received = true;
                let route = format!("+ship/plan {}", self.site.word());
                self.send(Beat::BoardAndLift, Kind::PlanOut, &route, 0);
            }
            Launch => {
                let op = self.dt_state().operation.clone();
                self.dt(
                    Field,
                    &format!("+sortie/operation {op}"),
                    "last confirmed on the surface",
                    true,
                    0,
                );
            }
            Field => {
                self.milestone(Milestone::Launched, now);
                let op = self.dt_state().operation.clone();
                self.dt(Refresh, &format!("+sortie/refresh {op}"), "", false, 0);
            }
            Refresh => {
                let op = self.dt_state().operation.clone();
                self.dt(
                    RefreshAgain,
                    &format!("+sortie/refresh {op}"),
                    "",
                    false,
                    now + 2_000,
                );
            }
            RefreshAgain => {
                let op = self.dt_state().operation.clone();
                self.dt(
                    Orders,
                    &format!("+sortie/orders {op}"),
                    "Order prefill: ORDER DT:",
                    false,
                    0,
                );
            }
            Orders => {
                let fence = text
                    .split_once("Order prefill: ")
                    .and_then(|(_, s)| s.split_once(" ALL HOLD"))
                    .map(|(s, _)| s.to_owned())
                    .ok_or("order prefill missing")?;
                self.family = Some(super::super::Family::Metals);
                self.report.family = Some("metals".into());
                self.dt(Order, &format!("{fence} 1 MINE metals"), "", false, 0);
            }
            Order => {
                let op = self.dt_state().operation.clone();
                self.dt(
                    Reply,
                    &format!("+sortie/orders {op}"),
                    "WILCO received",
                    true,
                    0,
                );
            }
            Reply => {
                self.report
                    .detachment
                    .as_mut()
                    .ok_or("evidence")?
                    .wilco_received = true;
                let (op, unit) = {
                    let d = self.dt_state();
                    (d.operation.clone(), d.units[0])
                };
                self.dt(
                    NpcReturned,
                    &format!("+sortie/operation {op}"),
                    &format!("Slot 1: unit #{unit} — last confirmed returned"),
                    true,
                    0,
                );
            }
            NpcReturned => self.dt(HoldRead, "+ship/hold", "metal ore", true, 0),
            HoldRead => {
                self.report.mined_t = number_after(&text, "metal ore, run-of-mine: ")?;
                self.report.detachment.as_mut().ok_or("evidence")?.hold = text;
                let op = self.dt_state().operation.clone();
                self.dt(Recover, &format!("+sortie/recover {op}=2"), "", false, 0);
            }
            Recover => {
                let op = self.dt_state().operation.clone();
                self.dt(
                    BothReturned,
                    &format!("+sortie/operation {op}"),
                    "last confirmed returned",
                    true,
                    0,
                );
            }
            BothReturned => {
                self.report.detachment.as_mut().ok_or("evidence")?.returned = true;
                self.milestone(Milestone::Recovered, now);
                self.milestone(Milestone::Unloaded, now);
                self.dt(
                    LeaveCaptain,
                    "+tv/2 leave",
                    "give back your rig billet",
                    true,
                    0,
                );
            }
            LeaveCaptain => {
                let npc = self.dt_state().npc;
                self.dt(
                    Unseat,
                    &format!("+sortie/unseat #{npc}"),
                    "gives back their unit billet",
                    true,
                    0,
                );
            }
            Unseat => {
                self.report.detachment.as_mut().ok_or("evidence")?.unseated = true;
                self.dt(BayOut, "+tv/bay out", "returned to cargo hold", false, 0);
            }
            BayOut => self.dt(BackPassage, "passage", "Central Passage", false, 0),
            BackPassage => self.dt(Airlock, "airlock", "Airlock", false, 0),
            Airlock => {
                let op = self.dt_state().operation.clone();
                self.dt(End, &format!("+sortie/end {op}"), "", false, 0);
            }
            End => {
                let op = self.dt_state().operation.clone();
                self.dt(
                    EndRead,
                    &format!("+sortie/operation {op}"),
                    "Ended operation",
                    true,
                    0,
                );
            }
            EndRead => {
                let op = self.dt_state().operation.clone();
                self.dt(
                    Deactivate,
                    &format!("+sortie/deactivate {op}"),
                    "",
                    false,
                    0,
                );
            }
            Deactivate => {
                let draft = self.dt_state().draft.clone();
                self.dt(
                    Closed,
                    &format!("+sortie/active {draft}"),
                    "Closed detachment",
                    true,
                    0,
                );
            }
            Closed => {
                self.report.detachment.as_mut().ok_or("evidence")?.closed = true;
                self.send(Beat::Home, Kind::PlanHome, "+ship/plan ceres", 0);
            }
        }
        Ok(())
    }
}

/// Only this expedition's player commands; no arbitrary staff verb or model output.
pub fn in_vocabulary(line: &str) -> bool {
    if line.len() > 4096 || line.contains(['\n', '\r']) {
        return false;
    }
    if super::in_vocabulary(line) {
        return true;
    }
    if [
        "concourse",
        "slack",
        "exchange",
        "in",
        "hold",
        "passage",
        "airlock",
        "+ship",
        "@unitgrant/list",
        "+tv/bay in",
        "+tv/bay out",
        "+tv/2 board",
        "+tv/2 leave",
        "+tv/1/permissions go",
        "+tv/1/permissions",
        "+tv/2/permissions go",
        "+tv/2/permissions",
        "+sortie/crew",
        "+sortie/plans",
        "+sortie/chat",
        "+ship/hold",
    ]
    .contains(&line)
    {
        return true;
    }
    if line == format!("+sortie/chat ={RADIO_CHECK}") {
        return true;
    }
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let selector = |s: &str| s.split(':').count() <= 3 && s.split(':').all(digits);
    if let Some(rest) = line.strip_prefix("@channel/recall ") {
        return rest.starts_with("dt")
            && rest.len() <= 8
            && rest.bytes().all(|b| b.is_ascii_alphanumeric());
    }
    for prefix in ["+focus #", "@station/handoff #"] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return rest.strip_suffix("/pilot").is_some_and(digits);
        }
    }
    if let Some(rest) = line.strip_prefix("@unitgrant/issue #") {
        let Some((unit, rest)) = rest.split_once("=#") else {
            return false;
        };
        let Some((actor, parent)) = rest.split_once(" operate_crew parent=") else {
            return false;
        };
        return digits(unit) && digits(actor) && digits(parent);
    }
    if let Some(rest) = line.strip_prefix("ORDER DT:") {
        return rest.strip_suffix(" 1 MINE metals").is_some_and(selector);
    }
    if let Some(rest) = line.strip_prefix("+sortie/draft ") {
        return serde_json::from_str::<Value>(rest).is_ok_and(|p| {
            p["carrier"].as_u64().is_some()
                && p["slots"].as_array().is_some_and(|s| s.len() == 2)
                && p["npc_commander"].is_null()
        });
    }
    for verb in ["hire", "contract", "board", "unseat"] {
        if let Some(rest) = line.strip_prefix(&format!("+sortie/{verb} #")) {
            return if verb == "hire" {
                rest.split_once('=').is_some_and(|(id, hash)| {
                    digits(id) && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
                })
            } else {
                digits(rest)
            };
        }
    }
    for verb in ["accept", "ready"] {
        if let Some(rest) = line.strip_prefix(&format!("+sortie/{verb} ")) {
            return rest.rsplit_once(':').is_some_and(|(ids, hash)| {
                selector(ids) && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
            });
        }
    }
    for verb in [
        "plan",
        "active",
        "refresh",
        "orders",
        "operation",
        "end",
        "deactivate",
        "activate",
        "seat",
        "launch",
        "recover",
    ] {
        if let Some(rest) = line.strip_prefix(&format!("+sortie/{verb} ")) {
            let (id, suffix) = rest
                .split_once('=')
                .map_or((rest, None), |(i, s)| (i, Some(s)));
            return selector(id)
                && match verb {
                    "activate" => suffix == Some("mining"),
                    "seat" => suffix == Some("1"),
                    "launch" => suffix == Some("all"),
                    "recover" => suffix == Some("2"),
                    _ => suffix.is_none(),
                };
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expedition_does_not_admit_staff_commands_or_unfenced_injected_lines() {
        for bad in [
            "@teleport #1=#2",
            "@destroy #1",
            "+tv/1 launch go\n@destroy #1",
            "+sortie/accept 1:2:3:wrong",
            "ORDER DT:1:2:3 ALL MINE metals",
            "@unitgrant/issue #1=#2 operate_crew parent=3 delegate",
            "@channel/recall dt;@destroy",
        ] {
            assert!(!in_vocabulary(bad), "{bad}");
        }
        assert!(in_vocabulary(
            "@unitgrant/issue #1=#2 operate_crew parent=3"
        ));
        assert!(in_vocabulary("ORDER DT:1:2:3 1 MINE metals"));
    }
    #[test]
    fn queued_order_is_never_an_arrived_response_and_both_flights_must_return() {
        let reply = Task {
            phase: Phase::Reply,
            marker: "WILCO received".into(),
            poll: true,
        };
        assert!(!matches!(
            verdict(
                &reply,
                0,
                "Sent slot 1 order 1: submitted; no reply confirmed. Execution unconfirmed.",
                true,
                5000
            ),
            Verdict::Done
        ));
        assert!(matches!(
            verdict(
                &reply,
                0,
                "Sent slot 1 order 1: WILCO received. Execution unconfirmed.",
                true,
                5000
            ),
            Verdict::Done
        ));
        assert!(matches!(
            verdict(
                &reply,
                0,
                "Sent slot 1 order 1: NEGATIVE received.",
                true,
                5000
            ),
            Verdict::Fail(_)
        ));
        let returned = Task {
            phase: Phase::BothReturned,
            marker: "last confirmed returned".into(),
            poll: true,
        };
        assert!(!matches!(
            verdict(
                &returned,
                0,
                "Slot 1 last confirmed returned. Slot 2 last confirmed on the surface.",
                true,
                5000
            ),
            Verdict::Done
        ));
    }
    #[test]
    fn printed_player_consent_keeps_its_command_verb_and_all_fences() {
        let mut lane = Lane::new(
            super::super::super::Site::Ahuna,
            super::super::super::Settings {
                hull: "Expedition Lode".into(),
                epoch_unix_ms: 0,
            },
            "Expedition".into(),
        )
        .with_detachment();
        let printed = format!("+sortie/accept 752:1:1:{}", "a".repeat(64));
        lane.expedition_next(Phase::Accept, std::slice::from_ref(&printed), 0)
            .expect("printed consent");
        assert_eq!(lane.step_line(), Some(printed.as_str()));
        assert!(in_vocabulary(lane.step_line().expect("consent line")));
    }
}
