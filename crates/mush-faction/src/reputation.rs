//! Public standing is derived from immutable, cause-keyed grants. It is never
//! membership, money, or model-authored belief. Private adjudicated incidents
//! affect nothing until a separate publication lands with its grant and canon.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{Ctx, Deferred, ModError, ModuleCtx, ModuleOp, ModuleWrite, ObjId, World};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::{Faction, FactionId, FactionState, NS};

const SCHEMA: u16 = 1;
const MAX_CAUSE: usize = 96;
const MAX_REASON: usize = 512;
const MAX_EFFECTS: usize = 8;
const MAX_DOCUMENT: usize = 4096;
const MAX_RECORDS: usize = 16_384;
const HISTORY: usize = 10;
/// The authored penalty for a publicly adjudicated attack, once per incident.
pub const PUBLIC_ATTACK_PENALTY: i16 = -20;

/// One authored consequence. Producers must establish their receipt's authority
/// before entering this owner API; the NPC action vocabulary has no such seam.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StandingEffect {
    pub faction: FactionId,
    pub delta: i16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Grant {
    schema: u16,
    cause: String,
    subject: ObjId,
    effects: Vec<StandingEffect>,
    reason: String,
    at: i64,
}

/// A private report, not a public fact or standing consequence. The first cut
/// is staff adjudication; it does not infer guilt from arbitrary combat events.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttackIncident {
    pub schema: u16,
    pub cause: String,
    pub victim: ObjId,
    pub faction: FactionId,
    pub attacker: Option<ObjId>,
    pub reason: String,
    pub reported_by: ObjId,
    pub at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Publication {
    schema: u16,
    cause: String,
    canon: u64,
    published_by: ObjId,
    at: i64,
}

/// Owner-held immutable records and a rebuildable score index. Receipts are
/// never evicted: a full store refuses new consequences rather than forgetting
/// that an old cause was already applied.
#[derive(Clone, Debug, Default)]
pub(crate) struct StandingStore {
    grants: BTreeMap<String, Grant>,
    incidents: BTreeMap<String, AttackIncident>,
    publications: BTreeMap<String, Publication>,
    scores: BTreeMap<(FactionId, ObjId), i64>,
}

/// A public faction's standing toward a character. No membership is implied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandingView {
    pub score: i16,
    pub band: &'static str,
    pub name: String,
}

fn valid_cause(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_CAUSE
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._:/-".contains(&b))
}

fn valid_reason(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= MAX_REASON && !value.chars().any(char::is_control)
}

fn check_grant(grant: &Grant) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    if grant.schema != SCHEMA
        || !valid_cause(&grant.cause)
        || !valid_reason(&grant.reason)
        || grant.at < 0
        || grant.subject == ObjId(0)
        || grant.effects.is_empty()
        || grant.effects.len() > MAX_EFFECTS
        || grant
            .effects
            .iter()
            .any(|e| e.delta == 0 || !(-100..=100).contains(&e.delta) || !seen.insert(e.faction))
        || grant.effects.windows(2).any(|pair| {
            pair.first()
                .zip(pair.get(1))
                .is_some_and(|(a, b)| a.faction >= b.faction)
        })
    {
        return Err("Invalid standing grant.".into());
    }
    Ok(())
}

fn check_incident(row: &AttackIncident) -> Result<(), String> {
    if row.schema != SCHEMA
        || !valid_cause(&row.cause)
        || row.cause.len() > MAX_CAUSE - "incident:".len()
        || !valid_reason(&row.reason)
        || row.at < 0
        || row.victim == ObjId(0)
        || row.reported_by == ObjId(0)
        || row.attacker == Some(ObjId(0))
        || row.attacker == Some(row.victim)
    {
        return Err("Invalid attack incident.".into());
    }
    Ok(())
}

fn key(kind: &str, cause: &str) -> SmolStr {
    SmolStr::new(format!("standing:{kind}:{cause}"))
}

fn prepared<T: Serialize>(kind: &str, cause: &str, value: &T) -> Result<ModuleWrite, String> {
    let value = serde_json::to_value(value).map_err(|_| "Standing serialization failed.")?;
    if serde_json::to_vec(&value)
        .map_err(|_| "Standing serialization failed.")?
        .len()
        > MAX_DOCUMENT
    {
        return Err("Standing document exceeds its byte bound.".into());
    }
    Ok(ModuleWrite::JsonSet {
        key: key(kind, cause),
        value,
    })
}

fn persist(ctx: &mut Ctx<'_>, op: ModuleWrite) {
    ctx.out
        .defer(Deferred::Persist(ModuleOp { ns: NS.into(), op }));
}

fn live_actor(w: &World, actor: ObjId) -> bool {
    w.get(actor)
        .is_some_and(|o| o.kind.is_actor() && !o.flags.contains(mush_core::Flags::GOING))
}

fn prepare_grant(w: &World, grant: &Grant) -> Result<bool, String> {
    check_grant(grant)?;
    let store = w
        .module_state::<StandingStore>()
        .ok_or("Standing unavailable.")?;
    if let Some(old) = store.grants.get(&grant.cause) {
        return if old.subject == grant.subject
            && old.effects == grant.effects
            && old.reason == grant.reason
        {
            Ok(false)
        } else {
            Err("Standing cause conflicts with its immutable receipt.".into())
        };
    }
    if store.grants.len() >= MAX_RECORDS {
        return Err("Standing receipt store is full.".into());
    }
    if !live_actor(w, grant.subject) {
        return Err("Standing subject is not a live character.".into());
    }
    let factions = w
        .module_state::<FactionState>()
        .ok_or("Factions unavailable.")?;
    if grant
        .effects
        .iter()
        .any(|e| !factions.factions.contains_key(&e.faction))
    {
        return Err("Standing consequence names a missing faction.".into());
    }
    Ok(true)
}

fn install_grant(store: &mut StandingStore, grant: Grant) {
    for effect in &grant.effects {
        let total = store
            .scores
            .entry((effect.faction, grant.subject))
            .or_default();
        *total += i64::from(effect.delta);
    }
    store.grants.insert(grant.cause.clone(), grant);
}

/// Record public standing from a verified engine-owned work/incident receipt.
/// Identical retries return false; conflicting causes fail without writes. The
/// cause must be stable across retries; the producer supplies authored effects,
/// never numbers proposed by a player or language model. All grants commit in
/// the caller's write batch. Call after producer success, with no fallible work
/// remaining that should undo that success.
pub fn record_standing(
    ctx: &mut Ctx<'_>,
    cause: &str,
    subject: ObjId,
    effects: &[StandingEffect],
    reason: &str,
) -> Result<bool, String> {
    if !valid_cause(cause)
        || !valid_reason(reason)
        || effects.is_empty()
        || effects.len() > MAX_EFFECTS
    {
        return Err("Invalid standing consequence bounds.".into());
    }
    if cause.starts_with("incident:") {
        return Err("Incident causes are reserved for publication.".into());
    }
    let mut effects = effects.to_vec();
    effects.sort_by_key(|effect| effect.faction);
    let grant = Grant {
        schema: SCHEMA,
        cause: cause.into(),
        subject,
        effects,
        reason: reason.into(),
        at: ctx.w.now(),
    };
    if !prepare_grant(ctx.w, &grant)? {
        return Ok(false);
    }
    let op = prepared("grant", cause, &grant)?;
    let store = ctx
        .w
        .module_state_mut::<StandingStore>()
        .ok_or("Standing unavailable.")?;
    install_grant(store, grant);
    persist(ctx, op);
    ctx.out.pin_deferred();
    Ok(true)
}

fn score(w: &World, faction: FactionId, subject: ObjId) -> Option<i16> {
    let store = w.module_state::<StandingStore>()?;
    Some(
        store
            .scores
            .get(&(faction, subject))
            .copied()
            .unwrap_or(0)
            .clamp(-100, 100) as i16,
    )
}

/// Public projection only: hidden and missing factions are indistinguishable.
#[must_use]
pub fn public_standing(w: &World, faction: FactionId, subject: ObjId) -> Option<StandingView> {
    let faction_row = w.module_state::<FactionState>()?.factions.get(&faction)?;
    if faction_row.hidden || !live_actor(w, subject) {
        return None;
    }
    let score = score(w, faction, subject)?;
    let band = match score {
        ..=-50 => "hostile",
        -49..=-10 => "unfriendly",
        -9..=9 => "neutral",
        10..=49 => "friendly",
        _ => "trusted",
    };
    Some(StandingView {
        score,
        band,
        name: faction_row.name.to_string(),
    })
}

pub(crate) fn policy_score(w: &World, faction: FactionId, subject: ObjId) -> Option<i16> {
    if !live_actor(w, subject)
        || !w
            .module_state::<FactionState>()?
            .factions
            .contains_key(&faction)
    {
        return None;
    }
    score(w, faction, subject)
}

/// Resolve the organization's unique valid faction, or create a public one
/// using its actual authored organization name. Never invent a polity label.
pub fn ensure_organization_faction(
    ctx: &mut Ctx<'_>,
    org: mush_org::OrgId,
) -> Result<FactionId, String> {
    let organization = ctx
        .w
        .module_state::<mush_org::OrgStore>()
        .and_then(|store| store.state().org(org))
        .ok_or("Organization absent.")?;
    let name = organization.name.clone();
    let factions = ctx
        .w
        .module_state::<FactionState>()
        .ok_or("Factions unavailable.")?;
    let mut linked = factions.factions.values().filter(|f| f.org == Some(org));
    if let Some(existing) = linked.next() {
        if linked.next().is_some() {
            return Err("Organization has conflicting faction links.".into());
        }
        crate::backing_org(ctx.w, existing).map_err(|e| e.to_string())?;
        return Ok(existing.id);
    }
    if organization.hidden || organization.status != mush_org::OrgStatus::Active {
        return Err("Only an active public organization can create a public faction.".into());
    }
    if name.trim().is_empty()
        || name.len() > crate::MAX_NAME_LEN
        || name.chars().any(char::is_control)
        || factions.factions.len() >= 4096
        || factions.faction_by_name(&name).is_some()
    {
        return Err("Organization faction name conflicts or exceeds its bounds.".into());
    }
    let id = FactionId(factions.next_faction_id);
    let next = factions
        .next_faction_id
        .checked_add(1)
        .ok_or("Faction ids exhausted.")?;
    let mut faction = Faction::new(id, &name);
    faction.org = Some(org);
    let state = ctx
        .w
        .module_state_mut::<FactionState>()
        .ok_or("Factions unavailable.")?;
    state.next_faction_id = next;
    state.factions.insert(id, faction.clone());
    let next_region = state.next_region_id;
    crate::data::persist_faction(ctx.out, &faction);
    crate::data::persist_meta(ctx.out, next, next_region);
    ctx.out.pin_deferred();
    Ok(id)
}

/// File a private, immutable incident from an adjudicated engine source. This
/// does not establish public knowledge or change standing. The staff command
/// checks faction-admin authority; a future combat producer must establish its
/// own attribution before calling this API.
pub fn report_attack_incident(
    ctx: &mut Ctx<'_>,
    cause: &str,
    faction: FactionId,
    victim: ObjId,
    attacker: Option<ObjId>,
    reason: &str,
) -> Result<bool, String> {
    if !valid_cause(cause) || cause.len() > MAX_CAUSE - "incident:".len() || !valid_reason(reason) {
        return Err("Invalid incident bounds.".into());
    }
    let row = AttackIncident {
        schema: SCHEMA,
        cause: cause.into(),
        victim,
        faction,
        attacker,
        reason: reason.into(),
        reported_by: ctx.actor,
        at: ctx.w.now(),
    };
    check_incident(&row)?;
    let store = ctx
        .w
        .module_state::<StandingStore>()
        .ok_or("Standing unavailable.")?;
    if let Some(old) = store.incidents.get(cause) {
        return if old.faction == faction
            && old.victim == victim
            && old.attacker == attacker
            && old.reason == reason
        {
            Ok(false)
        } else {
            Err("Incident cause conflicts with its immutable report.".into())
        };
    }
    if store.incidents.len() >= MAX_RECORDS {
        return Err("Incident receipt store is full.".into());
    }
    if !ctx
        .w
        .get(victim)
        .is_some_and(|o| o.kind.is_actor() || o.kind == mush_core::ObjKind::Thing)
        || attacker.is_some_and(|a| !live_actor(ctx.w, a))
        || !ctx
            .w
            .module_state::<FactionState>()
            .is_some_and(|s| s.factions.contains_key(&faction))
    {
        return Err("Incident names a missing victim, faction, or attacker.".into());
    }
    let op = prepared("incident", cause, &row)?;
    ctx.w
        .module_state_mut::<StandingStore>()
        .ok_or("Standing unavailable.")?
        .incidents
        .insert(cause.into(), row);
    persist(ctx, op);
    ctx.out.pin_deferred();
    Ok(true)
}

/// Explicitly publish an adjudicated report and its fixed consequence. Unknown
/// attackers remain unattributed. No ordinary combat or canon event calls this.
pub fn publish_attack_incident(ctx: &mut Ctx<'_>, cause: &str) -> Result<bool, String> {
    if !valid_cause(cause) {
        return Err("Invalid incident cause.".into());
    }
    let store = ctx
        .w
        .module_state::<StandingStore>()
        .ok_or("Standing unavailable.")?;
    let incident = store
        .incidents
        .get(cause)
        .cloned()
        .ok_or("No such incident.")?;
    if store.publications.contains_key(cause) {
        return Ok(false);
    }
    let faction = ctx
        .w
        .module_state::<FactionState>()
        .and_then(|s| s.factions.get(&incident.faction))
        .filter(|f| !f.hidden)
        .ok_or("Incident cannot publish a missing or hidden faction.")?;
    let name = faction.name.to_string();
    let grant_cause = format!("incident:{cause}");
    let grant = incident.attacker.map(|subject| Grant {
        schema: SCHEMA,
        cause: grant_cause.clone(),
        subject,
        effects: vec![StandingEffect {
            faction: incident.faction,
            delta: PUBLIC_ATTACK_PENALTY,
        }],
        reason: incident.reason.clone(),
        at: ctx.w.now(),
    });
    let grant_op = if let Some(grant) = &grant {
        if !prepare_grant(ctx.w, grant)? {
            return Err("Incident publication is missing its receipt; repair required.".into());
        }
        Some(prepared("grant", &grant_cause, grant)?)
    } else {
        None
    };
    // Freeze and serialize every owner write before canon's terminal append.
    let canon = ctx
        .w
        .module_state::<mush_canon::CanonState>()
        .ok_or("Canon unavailable; report remains private.")?
        .next_canon_id;
    canon
        .checked_add(1)
        .ok_or("Canon ids exhausted; report remains private.")?;
    let publication = Publication {
        schema: SCHEMA,
        cause: cause.into(),
        canon,
        published_by: ctx.actor,
        at: ctx.w.now(),
    };
    if publication.at < incident.at {
        return Err("Invalid publication time.".into());
    }
    let publication_op = prepared("publication", cause, &publication)?;
    let attribution = incident.attacker.map_or_else(
        || "attacker unknown".to_owned(),
        |a| format!("attacker {a}"),
    );
    let mut entities = vec![incident.victim];
    entities.extend(incident.attacker);
    let ic_time = ctx.w.game_now_unix();
    mush_canon::commit(ctx, mush_canon::CanonDraft {
        kind: "faction.attack_published".into(),
        summary: format!("{name} publishes an adjudicated attack on {}; {attribution}. {}", incident.victim, incident.reason),
        entities, scope: mush_canon::CanonScope::Public, ic_time,
        source: format!("faction.incident:{cause}"),
        detail: serde_json::json!({"faction":incident.faction,"victim":incident.victim,"attacker":incident.attacker,"incident":cause}),
    }).ok_or("Canon refused publication; report remains private.")?;
    // Registration guarantees this slot; no further fallible operation follows
    // canon. All owner writes join the same pinned write batch.
    if let Some(store) = ctx.w.module_state_mut::<StandingStore>() {
        if let Some(grant) = grant {
            install_grant(store, grant);
        }
        store.publications.insert(cause.into(), publication);
    }
    if let Some(op) = grant_op {
        persist(ctx, op);
    }
    persist(ctx, publication_op);
    ctx.out.pin_deferred();
    Ok(true)
}

pub(crate) fn incident(ctx: &Ctx<'_>, cause: &str) -> Option<(AttackIncident, bool)> {
    let store = ctx.w.module_state::<StandingStore>()?;
    Some((
        store.incidents.get(cause)?.clone(),
        store.publications.contains_key(cause),
    ))
}

pub(crate) fn history(w: &World, faction: FactionId, subject: ObjId) -> Vec<String> {
    if public_standing(w, faction, subject).is_none() {
        return Vec::new();
    }
    let Some(store) = w.module_state::<StandingStore>() else {
        return Vec::new();
    };
    let mut grants: Vec<_> = store
        .grants
        .values()
        .filter(|g| g.subject == subject)
        .filter_map(|g| {
            g.effects
                .iter()
                .find(|e| e.faction == faction)
                .map(|e| (g, e.delta))
        })
        .collect();
    grants.sort_by(|(a, _), (b, _)| b.at.cmp(&a.at).then_with(|| b.cause.cmp(&a.cause)));
    grants
        .into_iter()
        .take(HISTORY)
        .map(|(g, delta)| format!("  {delta:+}: {} [{}]", g.reason, g.cause))
        .collect()
}

pub(crate) fn command_standing(ctx: &mut Ctx<'_>) -> Result<(), mush_core::CmdError> {
    let name = ctx.args.raw().trim();
    let state = ctx
        .w
        .module_state::<FactionState>()
        .ok_or_else(|| mush_core::CmdError::msg("Factions unavailable."))?;
    let ids: Vec<_> = if name.is_empty() {
        state
            .factions
            .values()
            .filter(|f| !f.hidden)
            .take(64)
            .map(|f| f.id)
            .collect()
    } else {
        vec![
            state
                .faction_by_name(name)
                .filter(|f| !f.hidden)
                .map(|f| f.id)
                .ok_or_else(|| mush_core::CmdError::msg("No such faction."))?,
        ]
    };
    let show_history = !name.is_empty();
    let mut lines = Vec::new();
    for faction in ids {
        if let Some(view) = public_standing(ctx.w, faction, ctx.actor) {
            lines.push(format!("{}: {} ({:+}).", view.name, view.band, view.score));
            if show_history {
                lines.extend(history(ctx.w, faction, ctx.actor));
            }
        }
    }
    if lines.is_empty() {
        lines.push("No public faction standings are available.".into());
    }
    for line in lines {
        ctx.out.sys_to_actor(ctx.actor, "faction", line);
    }
    Ok(())
}

fn dbref(value: &str) -> Result<ObjId, mush_core::CmdError> {
    value
        .trim()
        .strip_prefix('#')
        .and_then(|id| id.parse::<u32>().ok())
        .map(ObjId)
        .ok_or_else(|| mush_core::CmdError::msg("Use an exact #object id."))
}

pub(crate) fn command_incident(ctx: &mut Ctx<'_>) -> Result<(), mush_core::CmdError> {
    let cause = ctx.args.lhs().trim().to_owned();
    if let Some(rhs) = ctx.args.rhs().map(str::to_owned) {
        let mut parts = rhs.splitn(4, '/').map(str::trim);
        let usage = || {
            mush_core::CmdError::msg(
                "Usage: @faction/incident <cause>=<faction>/<victim #id>/<attacker #id|unknown>/<reason>",
            )
        };
        let faction =
            crate::commands::resolve_faction(ctx, parts.next().ok_or_else(usage)?, false)?.id;
        let victim = dbref(parts.next().ok_or_else(usage)?)?;
        let attacker = match parts.next().ok_or_else(usage)? {
            "unknown" => None,
            value => Some(dbref(value)?),
        };
        let reason = parts.next().ok_or_else(usage)?;
        let fresh = report_attack_incident(ctx, &cause, faction, victim, attacker, reason)
            .map_err(mush_core::CmdError::msg)?;
        ctx.out.sys_to_actor(
            ctx.actor,
            "faction",
            if fresh {
                "Private incident recorded; standing unchanged."
            } else {
                "Identical incident already recorded."
            },
        );
    } else {
        let (row, published) =
            incident(ctx, &cause).ok_or_else(|| mush_core::CmdError::msg("No such incident."))?;
        ctx.out.sys_to_actor(
            ctx.actor,
            "faction",
            format!(
                "Incident {}: faction {}, victim {}, attacker {}; reported by {} at {}; {}. {}",
                row.cause,
                row.faction,
                row.victim,
                row.attacker
                    .map_or_else(|| "unknown".into(), |id| id.to_string()),
                row.reported_by,
                row.at,
                if published {
                    "published"
                } else {
                    "private; standing unchanged"
                },
                row.reason
            ),
        );
    }
    Ok(())
}

pub(crate) fn command_publish(ctx: &mut Ctx<'_>) -> Result<(), mush_core::CmdError> {
    if ctx.args.rhs().is_some() {
        return Err(mush_core::CmdError::msg(
            "Usage: @faction/publish-incident <cause>",
        ));
    }
    let cause = ctx.args.lhs().trim().to_owned();
    let fresh = publish_attack_incident(ctx, &cause).map_err(mush_core::CmdError::msg)?;
    ctx.out.sys_to_actor(
        ctx.actor,
        "faction",
        if fresh {
            "Incident published; its standing consequence is recorded."
        } else {
            "Incident was already published; no further consequence."
        },
    );
    Ok(())
}

pub(crate) fn is_key(value: &str) -> bool {
    value.starts_with("standing:")
}

pub(crate) fn load(
    ctx: &mut ModuleCtx<'_>,
    rows: Vec<(SmolStr, serde_json::Value)>,
) -> Result<(), ModError> {
    let mut store = StandingStore::default();
    for (key_text, value) in rows {
        if serde_json::to_vec(&value)
            .map_err(|e| ModError::msg(e.to_string()))?
            .len()
            > MAX_DOCUMENT
        {
            return Err(ModError::msg(
                "faction: oversized standing receipt; refusing boot",
            ));
        }
        let parse_error = |why: String| {
            ModError::msg(format!(
                "faction: invalid standing receipt {key_text}: {why}"
            ))
        };
        if let Some(cause) = key_text.strip_prefix("standing:grant:") {
            let grant: Grant =
                serde_json::from_value(value).map_err(|e| parse_error(e.to_string()))?;
            check_grant(&grant).map_err(parse_error)?;
            if grant.cause != cause || store.grants.contains_key(cause) {
                return Err(ModError::msg("faction: standing grant key conflict"));
            }
            install_grant(&mut store, grant);
        } else if let Some(cause) = key_text.strip_prefix("standing:incident:") {
            let row: AttackIncident =
                serde_json::from_value(value).map_err(|e| parse_error(e.to_string()))?;
            check_incident(&row).map_err(parse_error)?;
            if row.cause != cause || store.incidents.insert(cause.into(), row).is_some() {
                return Err(ModError::msg("faction: incident key conflict"));
            }
        } else if let Some(cause) = key_text.strip_prefix("standing:publication:") {
            let row: Publication =
                serde_json::from_value(value).map_err(|e| parse_error(e.to_string()))?;
            if row.schema != SCHEMA
                || row.cause != cause
                || !valid_cause(cause)
                || row.at < 0
                || store.publications.insert(cause.into(), row).is_some()
            {
                return Err(ModError::msg("faction: publication key conflict"));
            }
        } else {
            return Err(ModError::msg(
                "faction: unknown standing receipt; refusing boot",
            ));
        }
        if store.grants.len() > MAX_RECORDS
            || store.incidents.len() > MAX_RECORDS
            || store.publications.len() > MAX_RECORDS
        {
            return Err(ModError::msg("faction: standing receipts exceed bounds"));
        }
    }
    // Never silently orphan-purge a receipt: doing so would permit old causes
    // to apply again. Deleted subjects/factions keep their historical identity.
    let mut publication_ids = BTreeSet::new();
    for (cause, publication) in &store.publications {
        if publication.canon == u64::MAX
            || publication.published_by == ObjId(0)
            || !publication_ids.insert(publication.canon)
        {
            return Err(ModError::msg(
                "faction: invalid or duplicate publication identity",
            ));
        }
        let row = store
            .incidents
            .get(cause)
            .ok_or_else(|| ModError::msg("faction: publication without incident"))?;
        if publication.at < row.at {
            return Err(ModError::msg("faction: publication predates incident"));
        }
        let grant = store.grants.get(&format!("incident:{cause}"));
        let valid = match (row.attacker, grant) {
            (None, None) => true,
            (Some(actor), Some(g)) => {
                g.subject == actor
                    && g.reason == row.reason
                    && g.at == publication.at
                    && g.effects
                        == vec![StandingEffect {
                            faction: row.faction,
                            delta: PUBLIC_ATTACK_PENALTY,
                        }]
            }
            _ => false,
        };
        if !valid {
            return Err(ModError::msg(
                "faction: incident publication/grant mismatch",
            ));
        }
    }
    for cause in store
        .grants
        .keys()
        .filter_map(|cause| cause.strip_prefix("incident:"))
    {
        if !store.publications.contains_key(cause) {
            return Err(ModError::msg("faction: incident grant without publication"));
        }
    }
    *ctx.w
        .module_state_mut::<StandingStore>()
        .ok_or_else(|| ModError::msg("faction: standing slot unavailable"))? = store;
    Ok(())
}
