//! Audited link schedules and immutable admission terms. Capacity remains per pair;
//! a completed network admission is not charged again at its operator's next spoke.
use crate::{PropagationStore, service};
use mush_core::{CanonicalId, CmdError, Ctx, ObjId, World};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub standard_delay_s: i64,
    pub deferred_min_s: i64,
    pub window_period_s: i64,
    pub window_start_s: i64,
    pub window_duration_s: i64,
    pub aging_s: i64,
    pub max_bytes: u32,
    pub rate: u32,
    pub burst: u32,
}
impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        if !(0..=86_400).contains(&self.standard_delay_s)
            || !(0..=604_800).contains(&self.deferred_min_s)
            || !(1..=604_800).contains(&self.window_period_s)
            || self.window_start_s < 0
            || self.window_duration_s <= 0
            || self.window_start_s.saturating_add(self.window_duration_s) > self.window_period_s
            || !(1..=86_400).contains(&self.aging_s)
            || !(128..=1024).contains(&self.max_bytes)
            || !self.max_bytes.is_multiple_of(128)
            || !(1..=1_000_000).contains(&self.rate)
            || !(1..=1_000_000).contains(&self.burst)
        {
            return Err("Invalid relay schedule: bounded delays/window, max 128..1024 in 128-byte units, positive rate/burst required.".into());
        }
        Ok(())
    }
    pub fn seeded(config: &service::Config, physical: u32) -> Self {
        Self {
            standard_delay_s: config.standard_delay_s,
            deferred_min_s: config.deferred_min_s,
            window_period_s: config.window_period_s,
            window_start_s: config.window_start_s,
            window_duration_s: config.window_duration_s,
            aging_s: config.aging_s,
            max_bytes: config.short_message_bytes,
            rate: physical,
            burst: config.burst_units.min(physical),
        }
    }
    pub fn window_open(&self, at: i64) -> bool {
        let phase = at.rem_euclid(self.window_period_s);
        phase >= self.window_start_s && phase < self.window_start_s + self.window_duration_s
    }
    pub fn eligible_at(&self, class: service::Class, queued: i64, admitted: bool) -> i64 {
        queued.saturating_add(if admitted {
            0
        } else {
            match class {
                service::Class::Standard => self.standard_delay_s,
                service::Class::Deferred => self.deferred_min_s,
                _ => 0,
            }
        })
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revision {
    pub revision: u64,
    pub actor: ObjId,
    pub at: i64,
    pub reason: String,
    pub settings: Settings,
}
impl Revision {
    pub fn validate(&self) -> Result<(), String> {
        self.settings.validate()?;
        if self.revision == 0 || self.actor.0 == 0 || !valid_reason(&self.reason) {
            return Err("Invalid relay schedule revision".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub revision: u64,
    pub operator: Option<u64>,
    pub settings: Settings,
}
fn valid_reason(reason: &str) -> bool {
    !reason.trim().is_empty() && reason.len() <= 120 && !reason.chars().any(char::is_control)
}
pub fn current(state: Option<&service::State>, id: &CanonicalId, physical: u32) -> (u64, Settings) {
    if let Some(revision) = state.and_then(|s| s.schedules.get(id)) {
        let mut settings = revision.settings.clone();
        // A live remount/content change cannot bypass the installed hardware cap.
        settings.rate = settings.rate.min(physical);
        return (revision.revision, settings);
    }
    (
        0,
        state.map_or_else(
            || Settings {
                standard_delay_s: 0,
                deferred_min_s: 0,
                window_period_s: 1,
                window_start_s: 0,
                window_duration_s: 1,
                aging_s: 60,
                max_bytes: 1024,
                rate: physical,
                burst: physical,
            },
            |s| Settings::seeded(&s.config, physical),
        ),
    )
}
pub fn validate_bindings(w: &World, state: &service::State) -> Result<(), String> {
    if state.schedules.is_empty() {
        return Ok(());
    }
    let registry = crate::routing::registry(w).ok_or("Relay schedules require their registry")?;
    for (id, revision) in &state.schedules {
        // Unmounting a reusable mobile pair legitimately removes its registry
        // entry. Keep its bounded policy and token balance dormant for refit.
        let Some(pair) = registry.pairs.get(id) else {
            continue;
        };
        if revision.settings.rate > pair.capacity_units_per_ic_second {
            return Err("Relay schedule rate exceeds installed capacity".into());
        }
    }
    Ok(())
}
/// Checked typed ingress for operators and future scheduling minds. Compare-and-set
/// prevents stale writers. Settings, attribution, audit and token balance commit once.
pub fn set(
    ctx: &mut Ctx<'_>,
    id: &CanonicalId,
    expected_revision: u64,
    settings: Settings,
    reason: &str,
) -> Result<u64, CmdError> {
    let fail = |s: &str| CmdError::msg(s.to_owned());
    // The same discovery boundary applies to API and command callers.
    service::pair(ctx, id.as_str())?;
    let api = service::integration(ctx.w).ok_or_else(|| fail("Relay service unavailable."))?;
    if !api.authorized(ctx.w, ctx.actor, id, "relay.manage") {
        return Err(fail("Permission denied."));
    }
    settings.validate().map_err(CmdError::msg)?;
    if !valid_reason(reason) {
        return Err(fail("Supply a plain reason of 1..120 bytes."));
    }
    let physical = crate::routing::registry(ctx.w)
        .and_then(|r| r.pairs.get(id))
        .ok_or_else(|| fail("No such public link."))?
        .capacity_units_per_ic_second;
    if settings.rate > physical {
        return Err(fail("Rate exceeds installed link capacity."));
    }
    let mut transport = ctx
        .w
        .module_state::<PropagationStore>()
        .ok_or_else(|| fail("Relay service unavailable."))?
        .transport
        .clone();
    let (previous, old) = current(transport.service.as_ref(), id, physical);
    if previous != expected_revision {
        return Err(fail("Schedule revision changed; read it again."));
    }
    let revision = previous
        .checked_add(1)
        .ok_or_else(|| fail("Schedule revision exhausted."))?;
    let at = ctx.w.world_ic_now().at;
    transport
        .reconfigure(id, at, old.rate, old.burst, settings.burst)
        .map_err(CmdError::msg)?;
    let state = transport
        .service
        .as_mut()
        .ok_or_else(|| fail("Relay service unavailable."))?;
    let action = format!(
        "schedule {id} revision {revision}: {reason}; standard={} deferred={} period={} start={} window={} aging={} max={} rate={} burst={}",
        settings.standard_delay_s,
        settings.deferred_min_s,
        settings.window_period_s,
        settings.window_start_s,
        settings.window_duration_s,
        settings.aging_s,
        settings.max_bytes,
        settings.rate,
        settings.burst
    );
    state.schedules.insert(
        id.clone(),
        Revision {
            revision,
            actor: ctx.actor,
            at,
            reason: reason.into(),
            settings,
        },
    );
    state.audit(ctx.actor, at, action);
    transport.validate().map_err(CmdError::msg)?;
    let value = transport.to_value();
    ctx.w
        .module_state_mut::<PropagationStore>()
        .ok_or_else(|| fail("Relay service unavailable."))?
        .transport = transport;
    ctx.persist(mush_core::ModuleOp {
        ns: crate::NS.into(),
        op: mush_core::ModuleWrite::JsonSet {
            key: crate::routing::STATE_KEY.into(),
            value,
        },
    });
    ctx.gate_on_durable_ack();
    Ok(revision)
}
pub fn line(id: &CanonicalId, revision: u64, s: &Settings) -> String {
    format!(
        "{id}: schedule v{revision}; Standard admission {} IC seconds once per operator; Deferred admission {}, window {}..{} / {}; aging {}; maximum {} bytes; rate {} units/IC second; burst {} units.",
        s.standard_delay_s,
        s.deferred_min_s,
        s.window_start_s,
        s.window_start_s + s.window_duration_s,
        s.window_period_s,
        s.aging_s,
        s.max_bytes,
        s.rate,
        s.burst
    )
}
pub fn command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = service::pair(ctx, ctx.args.lhs().trim())?;
    let physical = crate::routing::registry(ctx.w)
        .and_then(|r| r.pairs.get(&id))
        .ok_or_else(|| CmdError::msg("No such public link."))?
        .capacity_units_per_ic_second;
    let (revision, mut settings) = current(service::current(ctx), &id, physical);
    let Some(rhs) = ctx.args.rhs().map(str::to_owned) else {
        ctx.out
            .sys_to_actor(ctx.actor, crate::NS, line(&id, revision, &settings));
        return Ok(());
    };
    let (fields, reason) = rhs.split_once(';').ok_or_else(|| {
        CmdError::msg("Usage: +relay/schedule pair=standard:60,rate:1,burst:1;reason")
    })?;
    let mut expected_revision = revision;
    let mut seen = std::collections::BTreeSet::new();
    for field in fields.split(',') {
        let (key, value) = field
            .trim()
            .split_once(':')
            .ok_or_else(|| CmdError::msg("Use key:value schedule settings."))?;
        if !seen.insert(key) {
            return Err(CmdError::msg("Duplicate schedule setting."));
        }
        let value: i64 = value
            .trim()
            .parse()
            .map_err(|_| CmdError::msg("Schedule values must be integers."))?;
        let positive = || {
            u32::try_from(value)
                .map_err(|_| CmdError::msg("Schedule size/rate/burst must be positive."))
        };
        match key {
            "expect" => {
                expected_revision = u64::try_from(value)
                    .map_err(|_| CmdError::msg("Expected revision must be nonnegative."))?
            }
            "standard" => settings.standard_delay_s = value,
            "deferred" => settings.deferred_min_s = value,
            "period" => settings.window_period_s = value,
            "start" => settings.window_start_s = value,
            "window" => settings.window_duration_s = value,
            "aging" => settings.aging_s = value,
            "max" => settings.max_bytes = positive()?,
            "rate" => settings.rate = positive()?,
            "burst" => settings.burst = positive()?,
            _ => {
                return Err(CmdError::msg(
                    "Settings: standard, deferred, period, start, window, aging, max, rate, burst, expect.",
                ));
            }
        }
    }
    let revision = set(ctx, &id, expected_revision, settings.clone(), reason.trim())?;
    ctx.out.sys_to_actor(
        ctx.actor,
        crate::NS,
        format!(
            "Schedule revision {revision} recorded. Existing admission terms retained. {}",
            line(&id, revision, &settings)
        ),
    );
    Ok(())
}
