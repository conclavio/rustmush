//! R3 service policy. The existing propagation pass remains the only scheduler.
//! Operator authority and the knowledge/canon/front handoff are injected by the game.
use crate::{Envelope, PropagationStore};
use mush_core::{
    ArgShape, CanonicalId, CmdError, CommandSpec, Ctx, ModError, ObjId, Registrar, World,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const MAX_GRANTS: usize = 256;
pub const MAX_RECEIPTS: usize = 512;
pub const MAX_EVIDENCE: usize = 64;
pub const MAX_AUDIT: usize = 128;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    Sovereign,
    Priority,
    #[default]
    Standard,
    Deferred,
}
impl Class {
    pub fn label(self) -> &'static str {
        match self {
            Self::Sovereign => "Sovereign",
            Self::Priority => "Priority",
            Self::Standard => "Standard",
            Self::Deferred => "Deferred",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s {
            "sovereign" => Some(Self::Sovereign),
            "priority" => Some(Self::Priority),
            "standard" => Some(Self::Standard),
            "deferred" => Some(Self::Deferred),
            _ => None,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u16,
    pub bytes_per_unit: u32,
    pub short_message_bytes: u32,
    pub burst_units: u32,
    pub standard_delay_s: i64,
    pub deferred_min_s: i64,
    pub window_period_s: i64,
    pub window_start_s: i64,
    pub window_duration_s: i64,
    pub aging_s: i64,
}
impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || self.bytes_per_unit != 128
            || self.short_message_bytes != 1024
            || !(8..=1_000_000).contains(&self.burst_units)
            || !(0..=86_400).contains(&self.standard_delay_s)
            || !(0..=604_800).contains(&self.deferred_min_s)
            || !(1..=604_800).contains(&self.window_period_s)
            || self.window_start_s < 0
            || self.window_duration_s <= 0
            || self.window_start_s.saturating_add(self.window_duration_s) > self.window_period_s
            || !(1..=86_400).contains(&self.aging_s)
        {
            return Err("invalid relay service policy".into());
        }
        Ok(())
    }
    pub fn window_open(&self, at: i64) -> bool {
        let phase = at.rem_euclid(self.window_period_s);
        phase >= self.window_start_s && phase < self.window_start_s + self.window_duration_s
    }
    pub fn eligible_at(&self, class: Class, queued: i64) -> i64 {
        queued.saturating_add(match class {
            Class::Standard => self.standard_delay_s,
            Class::Deferred => self.deferred_min_s,
            _ => 0,
        })
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ticket {
    pub version: u16,
    pub class: Class,
    pub units: u32,
    pub eligible_at: i64,
    pub declared_s: i64,
    pub actual_ready: Option<i64>,
}
impl Ticket {
    pub fn valid(&self) -> bool {
        self.version == 1
            && (1..=8).contains(&self.units)
            && (0..=86_400).contains(&self.declared_s)
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub envelope: u64,
    pub pair: CanonicalId,
    pub issuer: ObjId,
    pub recipient: ObjId,
    pub reference: Option<String>,
    pub kind: crate::record::DispatchKind,
    pub ticket: Ticket,
    pub queued: i64,
    pub transmitted: i64,
    pub arrived: Option<i64>,
}
impl Receipt {
    pub fn proof(&self) -> bool {
        self.ticket.class == Class::Standard
            && self.ticket.declared_s > 0
            && self
                .ticket
                .actual_ready
                .is_some_and(|at| at < self.ticket.eligible_at)
            && self.transmitted >= self.ticket.eligible_at
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub pair: CanonicalId,
    pub principal: ObjId,
    pub class: Class,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub holder: ObjId,
    pub receipt: Receipt,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Audit {
    pub actor: ObjId,
    pub at: i64,
    pub action: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub config: Config,
    pub grants: Vec<Grant>,
    pub receipts: Vec<Receipt>,
    pub evidence: Vec<Evidence>,
    pub published: std::collections::BTreeMap<CanonicalId, u64>,
    pub audit: Vec<Audit>,
}
impl State {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            grants: vec![],
            receipts: vec![],
            evidence: vec![],
            published: Default::default(),
            audit: vec![],
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        self.config.validate()?;
        let keys: std::collections::BTreeSet<_> =
            self.grants.iter().map(|g| (&g.pair, g.principal)).collect();
        if self.grants.len() > MAX_GRANTS
            || keys.len() != self.grants.len()
            || self.receipts.len() > MAX_RECEIPTS
            || self.evidence.len() > MAX_EVIDENCE
            || self.published.len() > mush_sim::relay::MAX_PAIRS
            || self.audit.len() > MAX_AUDIT
            || self
                .receipts
                .iter()
                .chain(self.evidence.iter().map(|e| &e.receipt))
                .any(|r| {
                    !r.ticket.valid()
                        || r.reference
                            .as_ref()
                            .is_some_and(|s| s.is_empty() || s.len() > 96)
                        || r.queued > r.transmitted
                        || r.transmitted < r.ticket.eligible_at
                        || r.arrived.is_some_and(|at| at < r.transmitted)
                        || r.ticket
                            .actual_ready
                            .is_some_and(|at| at < r.queued || at > r.transmitted)
                })
            || self.evidence.iter().any(|e| !e.receipt.proof())
            || self.audit.iter().any(|a| a.action.len() > 256)
        {
            return Err("invalid or over-bound relay service state".into());
        }
        Ok(())
    }
    pub fn class(&self, pair: &CanonicalId, actor: ObjId) -> Class {
        self.grants
            .iter()
            .find(|g| &g.pair == pair && g.principal == actor)
            .map_or(Class::Standard, |g| g.class)
    }
    pub fn receipt(&mut self, receipt: Receipt) {
        if self.receipts.len() >= MAX_RECEIPTS {
            self.receipts.remove(0);
        }
        self.receipts.push(receipt);
    }
    pub fn audit(&mut self, actor: ObjId, at: i64, action: String) {
        if self.audit.len() >= MAX_AUDIT {
            self.audit.remove(0);
        }
        self.audit.push(Audit { actor, at, action });
    }
}
/// This bridge may use organization, knowledge, canon and front owners. It must
/// preflight all fallible work before mutating, and must never disclose payloads.
pub trait Integration: Send + Sync + std::fmt::Debug {
    fn seed(&self) -> Result<Config, String>;
    fn visible(&self, world: &World, actor: ObjId, receipt: &Receipt) -> bool;
    fn authorized(&self, world: &World, actor: ObjId, pair: &CanonicalId, action: &str) -> bool;
    fn discover(&self, ctx: &mut Ctx<'_>, evidence: &Evidence) -> Result<(), String>;
    fn publish(&self, ctx: &mut Ctx<'_>, evidence: &Evidence) -> Result<u64, String>;
}
fn integration(world: &World) -> Option<Arc<dyn Integration>> {
    world
        .module_state::<crate::routing::RoutingServices>()?
        .1
        .clone()
}
fn current<'a>(ctx: &'a Ctx<'_>) -> Option<&'a State> {
    ctx.w
        .module_state::<PropagationStore>()?
        .transport
        .service
        .as_ref()
}
fn save(ctx: &mut Ctx<'_>, state: State) -> Result<(), CmdError> {
    state.validate().map_err(CmdError::msg)?;
    let book = ctx
        .w
        .module_state_mut::<PropagationStore>()
        .ok_or_else(|| CmdError::msg("Relay service unavailable."))?;
    book.transport.service = Some(state);
    let value = book.transport.to_value();
    ctx.persist(mush_core::ModuleOp {
        ns: crate::NS.into(),
        op: mush_core::ModuleWrite::JsonSet {
            key: crate::routing::STATE_KEY.into(),
            value,
        },
    });
    ctx.gate_on_durable_ack();
    Ok(())
}
fn say(ctx: &mut Ctx<'_>, s: impl Into<String>) {
    ctx.out.sys_to_actor(ctx.actor, crate::NS, s.into());
}
fn pair(ctx: &Ctx<'_>, s: &str) -> Result<CanonicalId, CmdError> {
    let id = CanonicalId::new(s).map_err(|_| CmdError::msg("No such public link."))?;
    if crate::routing::registry(ctx.w).is_none_or(|r| !r.pairs.contains_key(&id)) {
        return Err(CmdError::msg("No such public link."));
    }
    Ok(id)
}
pub fn board(world: &World, actor: ObjId) -> Vec<String> {
    let Some(state) = world
        .module_state::<PropagationStore>()
        .and_then(|s| s.transport.service.as_ref())
    else {
        return vec!["Relay service unavailable.".into()];
    };
    let Some(registry) = crate::routing::registry(world) else {
        return vec!["No public relay service.".into()];
    };
    let cfg = &state.config;
    let mut lines = vec![format!(
        "Relay board v{}: operator-granted service; no payment charged. One message-unit = {} content bytes; short limit {} bytes.",
        cfg.version, cfg.bytes_per_unit, cfg.short_message_bytes
    )];
    for (id, p) in &registry.pairs {
        if p.class_policy != mush_sim::relay::ClassPolicy::Exchange {
            continue;
        }
        lines.push(format!("{id}: {:?}; your class {}; Standard declared wait {} IC seconds. Sovereign/Priority wait for capacity. Deferred: at least {} IC seconds, window {}..{} in each {} IC seconds. Rate {} units/IC second; burst {} units.",p.state(),state.class(id,actor).label(),cfg.standard_delay_s,cfg.deferred_min_s,cfg.window_start_s,cfg.window_start_s+cfg.window_duration_s,cfg.window_period_s,p.capacity_units_per_ic_second,cfg.burst_units.min(p.capacity_units_per_ic_second)));
    }
    lines
}
fn command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let api = integration(ctx.w).ok_or_else(|| CmdError::msg("Relay service unavailable."))?;
    let mut state = current(ctx)
        .cloned()
        .ok_or_else(|| CmdError::msg("Relay service unavailable."))?;
    let viewer = ctx.actor;
    if ctx.switches.has("receipts") {
        let lines: Vec<_> = state
            .receipts
            .iter()
            .rev()
            .filter(|r| {
                r.recipient == viewer && r.arrived.is_some() && api.visible(ctx.w, viewer, r)
            })
            .take(32)
            .map(|r| {
                format!(
                    "{} {:?} {}: queued {}, transmitted {}, arrived {}; {} units.",
                    r.pair,
                    r.kind,
                    r.ticket.class.label(),
                    r.queued,
                    r.transmitted,
                    r.arrived.unwrap_or(r.transmitted),
                    r.ticket.units
                )
            })
            .collect();
        for line in lines {
            say(ctx, line);
        }
        return Ok(());
    }
    if !["grant", "load", "audit", "discover", "publish"]
        .iter()
        .any(|s| ctx.switches.has(s))
    {
        for line in board(ctx.w, ctx.actor) {
            say(ctx, line);
        }
        return Ok(());
    }
    let left = ctx.args.lhs().trim().to_owned();
    let (pair_name, subject) = left
        .split_once('/')
        .map_or((left.as_str(), None), |(a, b)| (a, Some(b)));
    let id = pair(ctx, pair_name)?;
    let action = if ctx.switches.has("grant") {
        "relay.manage"
    } else if ctx.switches.has("publish") {
        "relay.publish"
    } else {
        "relay.inspect"
    };
    if !api.authorized(ctx.w, ctx.actor, &id, action) {
        return Err(CmdError::msg("Permission denied."));
    }
    let at = ctx.w.world_ic_now().at;
    if ctx.switches.has("grant") {
        let actor = subject
            .and_then(|s| s.trim_start_matches('#').parse::<u32>().ok())
            .map(ObjId)
            .filter(|a| ctx.w.get(*a).is_some())
            .ok_or_else(|| {
                CmdError::msg(
                    "Usage: +relay/grant <pair>/<character>=sovereign|priority|standard|deferred",
                )
            })?;
        if ctx
            .w
            .get(actor)
            .is_none_or(|o| o.kind != mush_core::ObjKind::Player)
        {
            return Err(CmdError::msg("Class grants require an IC character."));
        }
        let class = ctx
            .args
            .rhs()
            .and_then(|s| Class::parse(s.trim()))
            .ok_or_else(|| CmdError::msg("Unknown service class."))?;
        if let Some(g) = state
            .grants
            .iter_mut()
            .find(|g| g.pair == id && g.principal == actor)
        {
            g.class = class;
        } else {
            if state.grants.len() >= MAX_GRANTS {
                return Err(CmdError::msg("Class grant table is full."));
            }
            state.grants.push(Grant {
                pair: id.clone(),
                principal: actor,
                class,
            });
        }
        state.audit(
            ctx.actor,
            at,
            format!("grant {id} {actor} {}", class.label()),
        );
        save(ctx, state)?;
        say(ctx, "Class grant recorded; no payment charged.");
        return Ok(());
    }
    if ctx.switches.has("audit") {
        // Audits are operator-scoped by the pair prefix embedded in each action.
        for a in state
            .audit
            .iter()
            .filter(|a| a.action.split_whitespace().nth(1) == Some(id.as_str()))
            .rev()
            .take(32)
        {
            say(ctx, format!("{} {} {}", a.at, a.actor, a.action));
        }
        return Ok(());
    }
    if ctx.switches.has("load") {
        let pending: Vec<_> = ctx
            .w
            .module_state::<PropagationStore>()
            .into_iter()
            .flat_map(|s| s.envelopes())
            .filter(|e| {
                e.transit().is_some_and(|r| {
                    r.queued_at.is_some()
                        && r.legs
                            .get(r.next)
                            .is_some_and(|l| l.pair.as_ref() == Some(&id))
                })
            })
            .collect();
        let oldest = pending
            .iter()
            .filter_map(|e| e.transit()?.queued_at.as_ref().map(|s| s.at))
            .min();
        let bucket = ctx
            .w
            .module_state::<PropagationStore>()
            .and_then(|s| s.transport.capacity.get(&id));
        let spent = bucket.filter(|b| b.at == at).map_or(0, |b| b.units);
        let line = format!(
            "{id}: actual queued {}; oldest IC age {}; spent {} units at {at}; scanned at most {} envelopes; retained receipts {}.",
            pending.len(),
            oldest.map_or(0, |t| at.saturating_sub(t)),
            spent,
            crate::MAX_ENVELOPES,
            state.receipts.iter().filter(|r| r.pair == id).count()
        );
        state.audit(ctx.actor, at, format!("inspect {id}"));
        save(ctx, state)?;
        say(ctx, line);
        return Ok(());
    }
    if ctx.switches.has("discover") {
        let receipt = state
            .receipts
            .iter()
            .rev()
            .find(|r| r.pair == id && r.proof())
            .cloned()
            .ok_or_else(|| CmdError::msg("No retained evidence for this link."))?;
        let evidence = Evidence {
            holder: ctx.actor,
            receipt,
        };
        if state
            .evidence
            .iter()
            .any(|e| e.holder == ctx.actor && e.receipt.pair == id)
        {
            say(ctx, "Evidence already filed.");
            return Ok(());
        }
        if state.evidence.len() >= MAX_EVIDENCE {
            return Err(CmdError::msg("Evidence store full."));
        }
        api.discover(ctx, &evidence).map_err(CmdError::msg)?;
        state.evidence.push(evidence);
        state.audit(ctx.actor, at, format!("discover {id}"));
        save(ctx, state)?;
        say(
            ctx,
            "Evidence filed in your intel; publication requires separate authority.",
        );
        return Ok(());
    }
    if let Some(front) = state.published.get(&id) {
        say(ctx, format!("Evidence already published; front {front}."));
        return Ok(());
    }
    let evidence = state
        .evidence
        .iter()
        .find(|e| e.holder == ctx.actor && e.receipt.pair == id)
        .cloned()
        .ok_or_else(|| CmdError::msg("Discover and retain the evidence before publication."))?;
    let front = api.publish(ctx, &evidence).map_err(CmdError::msg)?;
    state.published.insert(id.clone(), front);
    state.audit(ctx.actor, at, format!("publish {id}"));
    save(ctx, state)?;
    say(
        ctx,
        format!("Evidence published; scandal front {front} is open."),
    );
    Ok(())
}
pub fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    drop(reg.panel_feed("relay.board")?);
    reg.query_handler("relay.board", Arc::new(BoardQuery))?;
    reg.command(
        CommandSpec {
            name: "+relay".into(),
            shape: ArgShape::EqPair,
            switches: ["receipts", "grant", "load", "audit", "discover", "publish"]
                .into_iter().map(Into::into).collect(),
            help: "+relay: public board and your operator-granted class. +relay/receipts: your arrived traffic stamps. +relay/grant pair/character=class: operator grants, not purchases. +relay/load|audit|discover|publish pair: separately permissioned operator actions.".into(),
            ..Default::default()
        },
        Arc::new(command),
    )?;
    Ok(())
}
pub fn units(envelope: &Envelope) -> u32 {
    match envelope.dispatch_ref() {
        Some(d) if d.bytes > crate::dispatch::SHORT_MESSAGE_BYTES => u32::MAX,
        Some(d) => d.bytes.div_ceil(128),
        None => (envelope.payload().len() as u32).div_ceil(128),
    }
}

/// Query hydration uses the exact same player-safe board as line clients.
pub struct BoardQuery;
impl mush_core::QueryHandler for BoardQuery {
    fn query(
        &self,
        world: &World,
        actor: ObjId,
        _params: &serde_json::Value,
    ) -> Result<mush_proto::OutputEvent, mush_proto::RichText> {
        Ok(mush_proto::OutputEvent::Panel {
            feed: "relay.board".into(),
            kind: mush_proto::PanelFrameKind::Keyframe,
            data: serde_json::json!({"lines":board(world,actor)}),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Config {
        Config {
            version: 1,
            bytes_per_unit: 128,
            short_message_bytes: 1024,
            burst_units: 8,
            standard_delay_s: 60,
            deferred_min_s: 7200,
            window_period_s: 86400,
            window_start_s: 0,
            window_duration_s: 3600,
            aging_s: 300,
        }
    }
    #[test]
    fn policy_has_strict_bounds_and_utc_independent_ic_windows() {
        let cfg = config();
        cfg.validate().unwrap();
        assert!(cfg.window_open(0));
        assert!(cfg.window_open(3599));
        assert!(!cfg.window_open(3600));
        assert!(cfg.window_open(86400));
        assert!(!cfg.window_open(-1));
        assert_eq!(cfg.eligible_at(Class::Standard, 7), 67);
        assert_eq!(cfg.eligible_at(Class::Deferred, 7), 7207);
        assert_eq!(cfg.eligible_at(Class::Priority, 7), 7);
        let mut bad = cfg.clone();
        bad.window_period_s = 0;
        assert!(bad.validate().is_err());
        bad = cfg.clone();
        bad.aging_s = 0;
        assert!(bad.validate().is_err());
        bad = cfg.clone();
        bad.bytes_per_unit = 1;
        assert!(bad.validate().is_err());
        bad = cfg;
        bad.window_start_s = i64::MAX;
        assert!(bad.validate().is_err());
    }
    #[test]
    fn receipt_bounds_and_evidence_require_observed_free_capacity() {
        let pair = CanonicalId::new("test").unwrap();
        let mut state = State::new(config());
        let mut receipt = Receipt {
            envelope: 1,
            pair: pair.clone(),
            issuer: ObjId(2),
            recipient: ObjId(3),
            reference: None,
            kind: crate::record::DispatchKind::Mail,
            ticket: Ticket {
                version: 1,
                class: Class::Standard,
                units: 2,
                eligible_at: 60,
                declared_s: 60,
                actual_ready: Some(0),
            },
            queued: 0,
            transmitted: 60,
            arrived: None,
        };
        assert!(receipt.proof());
        receipt.ticket.actual_ready = None;
        assert!(!receipt.proof());
        receipt.ticket.actual_ready = Some(60);
        assert!(!receipt.proof());
        for id in 1..=MAX_RECEIPTS + 1 {
            receipt.envelope = id as u64;
            state.receipt(receipt.clone());
        }
        assert_eq!(state.receipts.len(), MAX_RECEIPTS);
        assert_eq!(state.receipts.first().unwrap().envelope, 2);
        state.validate().unwrap();
        state.grants = vec![
            Grant {
                pair: pair.clone(),
                principal: ObjId(1),
                class: Class::Priority
            };
            2
        ];
        assert!(state.validate().is_err());
    }
}
