//! The NPC's content entry kinds (14-spatial-implementation-plan §3.1): what a pack
//! says about the people in a place, and how that becomes an agent NPC.
//!
//! - `npc` — a character with an [`AgentProfile`] and an [`NpcKnowledge`] document,
//!   standing in a room this pack or an explicitly required pack made. The object is created once and **kept**
//!   across pack revisions: an update rewrites the source-owned fields (name, tier,
//!   profile, knowledge) in place, and never the NPC's memories, its location, or what
//!   it carries (doc 14 §3.1: live state is never in a pack's diff).
//! - `lore` — a shared [`LorePool`], persisted under this module's namespace exactly as
//!   `@lore/set` persists one, and put into the host's RAM pools so it is live at once.
//!
//! Applying either makes **no LLM call** (doc 14 P9): a profile is an attribute, and
//! the agent wakes only on a stimulus in play.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_content::{Appliers, EntryApplier, Pack, PackError, RawEntry};
use mush_core::{
    Attr, CanonicalId, Ctx, Flags, GOD, ModuleOp, ModuleWrite, ObjId, ObjKind, World, WorldEvent,
};
use serde::Deserialize;
use smol_str::SmolStr;

use crate::knowledge::{KNOWLEDGE_ATTR, LoreEntry, LorePool, NpcKnowledge, Secrecy, pool_key};
use crate::profile::{AgentProfile, KnowledgeBounds, NS, NpcTier, PROFILE_ATTR};
use crate::runtime::NpcHostState;

/// The phase both kinds run in: after rooms (7) and beside items (9), before exits.
pub const PHASE: u8 = 9;

/// Register the `npc` and `lore` appliers under this module's name.
///
/// # Errors
///
/// [`PackError::KindOwnedElsewhere`] when another owner already declared a kind.
pub fn register_appliers(appliers: &mut Appliers) -> Result<(), PackError> {
    for kind in ["npc", "lore"] {
        let kind = CanonicalId::new(kind).map_err(|e| PackError::EntryDecode {
            kind: kind.to_string(),
            detail: e.to_string(),
        })?;
        appliers.register(NS, Arc::new(NpcApplier { kind }))?;
    }
    Ok(())
}

/// One lore fact, as authored.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoreDef {
    /// `public`, `guarded` or `secret`.
    #[serde(default)]
    pub tier: Secrecy,
    /// Who or what it is about.
    #[serde(default)]
    pub subject: String,
    /// The fact.
    pub fact: String,
    /// How the NPC knows.
    #[serde(default)]
    pub source: String,
}

impl LoreDef {
    fn to_entry(&self) -> LoreEntry {
        LoreEntry {
            subject: self.subject.clone(),
            fact: self.fact.clone(),
            secrecy: self.tier,
            source: self.source.clone(),
        }
    }
}

/// The profile, as authored.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ProfileDef {
    /// Personality.
    pub persona: String,
    /// History.
    pub background: String,
    /// Register.
    pub speech_style: String,
    /// Motivations.
    pub goals: Vec<String>,
    /// Facts the NPC knows.
    pub knows: Vec<String>,
    /// Facts the NPC must not claim.
    pub does_not_know: Vec<String>,
    /// Canned lines for when the model is off or refused.
    pub fallback: Vec<String>,
    /// Seconds between turns (tier default when absent).
    pub cooldown_s: Option<u32>,
    /// Daily token budget override.
    pub daily_tokens: Option<u64>,
    /// Whether the agent is on (default on).
    pub enabled: Option<bool>,
    /// Subjects of interest.
    pub watch: Vec<String>,
    /// S6: the interface-ability providers the NPC fronts, by id.
    pub interfaces: Vec<String>,
    /// Registered command names this NPC can explain to the player.
    pub command_refs: Vec<String>,
}

/// An `npc` entry.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NpcDef {
    /// The name.
    pub name: String,
    /// The room entry the NPC initially stands in.
    pub room: String,
    /// An explicitly required pack owning that room; absent means this pack.
    #[serde(default)]
    pub room_pack: Option<String>,
    /// The tier (default minor).
    #[serde(default)]
    pub tier: NpcTier,
    /// The profile.
    #[serde(default)]
    pub profile: ProfileDef,
    /// Facts authored on this NPC.
    #[serde(default)]
    pub lore: Vec<LoreDef>,
    /// Shared pools it draws on.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// A `lore` entry.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LorePoolDef {
    /// The tag.
    pub tag: String,
    /// The facts.
    pub entries: Vec<LoreDef>,
}

struct NpcApplier {
    kind: CanonicalId,
}

fn decode<T: serde::de::DeserializeOwned>(id: &CanonicalId, entry: &RawEntry) -> Result<T, String> {
    entry.decode::<T>().map_err(|e| format!("{id}: {e}"))
}

fn profile_of(def: &NpcDef) -> Result<AgentProfile, String> {
    let profile = AgentProfile {
        tier: def.tier,
        persona: def.profile.persona.clone(),
        background: def.profile.background.clone(),
        speech_style: def.profile.speech_style.clone(),
        goals: def.profile.goals.clone(),
        knowledge: KnowledgeBounds {
            knows: def.profile.knows.clone(),
            does_not_know: def.profile.does_not_know.clone(),
        },
        cooldown_s: def.profile.cooldown_s,
        daily_tokens: def.profile.daily_tokens,
        enabled: def.profile.enabled.unwrap_or(true),
        fallback_lines: def.profile.fallback.clone(),
        principal: None,
        public_faction: None,
        loyal: true,
        watch: def.profile.watch.clone(),
        interfaces: def
            .profile
            .interfaces
            .iter()
            .map(|i| smol_str::SmolStr::new(i.trim().to_ascii_lowercase()))
            .collect(),
        command_refs: def
            .profile
            .command_refs
            .iter()
            .map(|name| SmolStr::new(name.trim().to_ascii_lowercase()))
            .collect(),
    };
    profile.validate().map_err(|e| format!("profile: {e}"))?;
    Ok(profile)
}

fn knowledge_of(def: &NpcDef) -> Result<NpcKnowledge, String> {
    let knowledge = NpcKnowledge {
        lore: def.lore.iter().map(LoreDef::to_entry).collect(),
        tags: def.tags.clone(),
    };
    knowledge.validate().map_err(|e| format!("lore: {e}"))?;
    Ok(knowledge)
}

fn write_json(
    ctx: &mut Ctx<'_>,
    npc: ObjId,
    attr: &str,
    value: &impl serde::Serialize,
) -> Result<(), String> {
    let value = serde_json::to_value(value).map_err(|e| e.to_string())?;
    ctx.w
        .set_attr(npc, attr, Attr::json(value, GOD))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn create_npc(
    ctx: &mut Ctx<'_>,
    pack: &Pack,
    id: &CanonicalId,
    entry: &RawEntry,
    placed: &BTreeMap<CanonicalId, ObjId>,
    existing: Option<ObjId>,
) -> Result<Option<ObjId>, String> {
    let def: NpcDef = decode(id, entry)?;
    let name = def.name.trim();
    if name.is_empty()
        || name.len() > 64
        || name.contains('/')
        || name.contains('=')
        || name.contains(';')
    {
        return Err(format!("{id}: {name:?} is not an NPC name"));
    }
    let mut profile = profile_of(&def).map_err(|e| format!("{id}: {e}"))?;
    // Public affiliation is a runtime role binding/staff choice, not an authored
    // pack default. Reapplying prose must not reset that independent decision.
    if let Some(old) = existing.and_then(|npc| crate::module::read_profile(ctx.w, npc)) {
        profile.public_faction = old.public_faction;
    }
    let knowledge = knowledge_of(&def).map_err(|e| format!("{id}: {e}"))?;
    let room = initial_room(ctx.w, pack, &def, placed).map_err(|e| format!("{id}: {e}"))?;
    let npc = match existing.filter(|o| ctx.w.get(*o).is_some_and(|o| o.kind == ObjKind::Npc)) {
        Some(npc) => {
            if ctx.w.get(npc).is_some_and(|o| o.name != name) {
                ctx.w
                    .set_name(npc, name)
                    .map_err(|e| format!("{id}: {e}"))?;
            }
            npc
        }
        None => {
            let npc = ctx
                .w
                .create(ObjKind::Npc, name, GOD, Some(room))
                .map_err(|e| format!("{id}: {e}"))?;
            ctx.emit_world_event(&WorldEvent::ObjCreated {
                obj: npc,
                kind: ObjKind::Npc,
                creator: GOD,
            });
            npc
        }
    };
    write_json(ctx, npc, PROFILE_ATTR, &profile).map_err(|e| format!("{id}: {e}"))?;
    write_json(ctx, npc, KNOWLEDGE_ATTR, &knowledge).map_err(|e| format!("{id}: {e}"))?;
    Ok(Some(npc))
}

fn initial_room(
    w: &World,
    pack: &Pack,
    def: &NpcDef,
    placed: &BTreeMap<CanonicalId, ObjId>,
) -> Result<ObjId, String> {
    let room_id = CanonicalId::new(def.room.trim()).map_err(|e| format!("room: {e}"))?;
    let room = match &def.room_pack {
        None => placed.get(&room_id).copied(),
        Some(name) => {
            let room_pack = CanonicalId::new(name.trim()).map_err(|e| format!("room_pack: {e}"))?;
            let requirement = pack
                .manifest()
                .requires()
                .iter()
                .find(|r| r.id == room_pack)
                .ok_or("room_pack must be an explicit pack dependency")?;
            let content = w
                .module_state::<mush_content::ContentState>()
                .ok_or("content state is unavailable")?;
            let latch = content
                .latches
                .applied(&room_pack)
                .filter(|latch| {
                    !content.latches.is_held(&room_pack)
                        && !latch.is_retired()
                        && latch.anomaly().is_none()
                        && latch.version() == requirement.version
                        && requirement
                            .digest
                            .is_none_or(|digest| digest == latch.digest())
                })
                .ok_or("room_pack has no matching active dependency latch")?;
            latch
                .entries()
                .get(&room_id)
                .filter(|entry| entry.kind().as_str() == "room")
                .and_then(mush_content::AppliedEntry::object)
        }
    };
    room.filter(|room| {
        w.get(*room)
            .is_some_and(|o| o.kind == ObjKind::Room && !o.flags.contains(Flags::GOING))
    })
    .ok_or_else(|| format!("room {} is not a live authored room", def.room))
}

fn create_pool(
    ctx: &mut Ctx<'_>,
    id: &CanonicalId,
    entry: &RawEntry,
) -> Result<Option<ObjId>, String> {
    let def: LorePoolDef = decode(id, entry)?;
    let pool = LorePool {
        tag: def.tag.trim().to_string(),
        entries: def.entries.iter().map(LoreDef::to_entry).collect(),
    };
    pool.validate().map_err(|e| format!("{id}: {e}"))?;
    let value = serde_json::to_value(&pool).map_err(|e| format!("{id}: {e}"))?;
    let key = pool_key(&pool.tag);
    // WS03-S14: an applier runs under the CONTENT module's context, so the
    // checked spelling would refuse this write of the `npc` module's own slot
    // and the pool would land in the store but not in RAM. Same crate, same
    // module, only the context's name differs — the unchecked world accessor,
    // and no `state-slot-owners.toml` row (that file baselines a crate writing
    // another module's slot).
    if let Some(host) = ctx.w.module_state_mut::<NpcHostState>() {
        host.lore_pools
            .set(pool)
            .map_err(|e| format!("{id}: {e}"))?;
    }
    ctx.persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op: ModuleWrite::JsonSet { key, value },
    });
    Ok(None)
}

/// Whether any NPC in the world, other than those `placed` by the pack, draws on
/// pool `tag`.
fn tag_in_use(ctx: &Ctx<'_>, tag: &str, placed: &BTreeMap<CanonicalId, ObjId>) -> Option<String> {
    let wanted = crate::memory::slug(tag);
    ctx.w
        .iter()
        .filter(|o| o.kind == ObjKind::Npc && !placed.values().any(|p| *p == o.id))
        .find(|o| {
            o.attrs
                .get(KNOWLEDGE_ATTR)
                .and_then(Attr::as_json)
                .and_then(|v| serde_json::from_value::<NpcKnowledge>(v.clone()).ok())
                .is_some_and(|k| k.tags.iter().any(|t| crate::memory::slug(t) == wanted))
        })
        .map(|o| format!("{} ({})", o.name, o.id))
}

/// Validate an authored `npc` or `lore` entry against schema and profile/lore
/// bounds without requiring a live world (doc 34 cut CS.0a).
///
/// # Errors
///
/// Human-readable refusal naming the entry id and field.
pub fn validate_entry(id: &CanonicalId, entry: &RawEntry) -> Result<(), String> {
    match entry.kind().as_str() {
        "npc" => {
            let def: NpcDef = decode(id, entry)?;
            let name = def.name.trim();
            if name.is_empty()
                || name.len() > 64
                || name.contains('/')
                || name.contains('=')
                || name.contains(';')
            {
                return Err(format!("{id}: {name:?} is not an NPC name"));
            }
            CanonicalId::new(def.room.trim()).map_err(|e| format!("{id}: room: {e}"))?;
            if let Some(pack_name) = &def.room_pack {
                CanonicalId::new(pack_name.trim()).map_err(|e| format!("{id}: room_pack: {e}"))?;
            }
            profile_of(&def).map_err(|e| format!("{id}: {e}"))?;
            knowledge_of(&def).map_err(|e| format!("{id}: {e}"))?;
            Ok(())
        }
        "lore" => {
            let def: LorePoolDef = decode(id, entry)?;
            let pool = LorePool {
                tag: def.tag.trim().to_string(),
                entries: def.entries.iter().map(LoreDef::to_entry).collect(),
            };
            pool.validate().map_err(|e| format!("{id}: {e}"))?;
            Ok(())
        }
        other => Err(format!("{id}: unsupported NPC content kind {other}")),
    }
}

impl EntryApplier for NpcApplier {
    fn kinds(&self) -> Vec<CanonicalId> {
        vec![self.kind.clone()]
    }

    fn phase(&self) -> u8 {
        PHASE
    }

    fn check(
        &self,
        ctx: &Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
    ) -> Result<(), String> {
        validate_entry(id, entry)?;
        if self.kind.as_str() == "npc" {
            let def: NpcDef = decode(id, entry)?;
            if def.room_pack.is_some() {
                initial_room(ctx.w, pack, &def, &BTreeMap::new())
                    .map_err(|error| format!("{id}: {error}"))?;
            }
        }
        Ok(())
    }

    fn create(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<Option<ObjId>, String> {
        match self.kind.as_str() {
            "npc" => create_npc(ctx, pack, id, entry, placed, None),
            _ => create_pool(ctx, id, entry),
        }
    }

    fn update(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        existing: Option<ObjId>,
        placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<Option<ObjId>, String> {
        match self.kind.as_str() {
            // In place: memories, location and inventory survive.
            "npc" => create_npc(ctx, pack, id, entry, placed, existing),
            _ => create_pool(ctx, id, entry),
        }
    }

    fn retire_check(
        &self,
        ctx: &Ctx<'_>,
        _pack: &CanonicalId,
        id: &CanonicalId,
        existing: Option<ObjId>,
        placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<(), String> {
        match self.kind.as_str() {
            "npc" => {
                let Some(npc) = existing else {
                    return Ok(());
                };
                if ctx.w.get(npc).is_none() {
                    return Ok(());
                }
                ctx.reg
                    .check_destroy(ctx.w, GOD, npc)
                    .map_err(|reason| format!("{id}: {reason}"))
            }
            _ => {
                // The pool's tag is on the document, not the latch; find it by the
                // entry's own def is impossible here, so the check is by the world:
                // any NPC outside the pack that draws on a pool this pack made.
                let Some(host) = ctx.state::<NpcHostState>() else {
                    return Ok(());
                };
                let tag = id.as_str().strip_prefix("lore.").unwrap_or(id.as_str());
                if host.lore_pools.get(tag).is_some()
                    && let Some(who) = tag_in_use(ctx, tag, placed)
                {
                    return Err(format!("{id}: {who} still draws on lore pool {tag}"));
                }
                Ok(())
            }
        }
    }

    fn retire(
        &self,
        ctx: &mut Ctx<'_>,
        _pack: &CanonicalId,
        id: &CanonicalId,
        existing: Option<ObjId>,
    ) -> Result<(), String> {
        match self.kind.as_str() {
            "npc" => {
                let Some(npc) = existing else {
                    return Ok(());
                };
                match ctx.w.destroy(npc) {
                    Ok(husks) => {
                        for husk in husks {
                            ctx.emit_world_event(&WorldEvent::ObjDestroyed { obj: husk });
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%id, npc = %npc, %error, "npc: retire could not destroy the NPC");
                    }
                }
                Ok(())
            }
            _ => {
                let tag = id
                    .as_str()
                    .strip_prefix("lore.")
                    .unwrap_or(id.as_str())
                    .to_string();
                // WS03-S14, as above: the retire runs under the content
                // module's context and reaches this crate's own slot.
                if let Some(host) = ctx.w.module_state_mut::<NpcHostState>() {
                    host.lore_pools.remove(&tag);
                }
                ctx.persist(ModuleOp {
                    ns: SmolStr::new_static(NS),
                    op: ModuleWrite::Del {
                        key: pool_key(&tag),
                    },
                });
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authored_command_references_normalize_without_changing_personality() {
        let def: NpcDef = serde_json::from_value(serde_json::json!({
            "name": "Clerk", "room": "room.desk", "profile": {
                "persona": "brisk but warm", "speech_style": "numbered steps",
                "command_refs": ["  +COOP  ", "Look"]
            }
        }))
        .expect("authored NPC");
        let profile = profile_of(&def).expect("valid profile");
        assert_eq!(
            profile.command_refs,
            vec![SmolStr::new_static("+coop"), SmolStr::new_static("look")]
        );
        assert_eq!(profile.persona, "brisk but warm");
        assert_eq!(profile.speech_style, "numbered steps");
        assert!(profile.interfaces.is_empty(), "guidance grants no tools");
    }

    #[test]
    fn older_content_defaults_to_no_command_refs_and_invalid_refs_are_rejected() {
        let mut def: NpcDef = serde_json::from_value(serde_json::json!({
            "name": "Clerk", "room": "room.desk", "profile": {"persona": "brisk"}
        }))
        .expect("older authored NPC");
        assert!(
            profile_of(&def)
                .expect("older profile")
                .command_refs
                .is_empty()
        );
        def.profile.command_refs = vec!["+coop/enrol My Ship".into()];
        assert!(
            profile_of(&def).is_err(),
            "a command line is not a reference"
        );
        assert!(def.room_pack.is_none());
    }

    fn external_fixture() -> (World, Pack, NpcDef, ObjId) {
        use mush_content::{Applied, AppliedEntry, ContentState};
        use mush_core::{ClockRef, ClockStamp, DefinitionDigest};
        let mut w = World::new_minimal();
        let room = w.create(ObjKind::Room, "Station bar", GOD, None).unwrap();
        let id = |text| CanonicalId::new(text).unwrap();
        let latch = Applied::new(
            id("station"),
            2,
            DefinitionDigest::from_bytes([3; 32]),
            ClockStamp {
                clock: ClockRef::WorldIc { generation: 1 },
                at: 0,
            },
            BTreeMap::from([(
                id("room.bar"),
                AppliedEntry::new(
                    id("room"),
                    DefinitionDigest::from_bytes([4; 32]),
                    Some(room),
                ),
            )]),
            false,
            None,
        )
        .unwrap();
        let mut content = ContentState::default();
        content.latches.set(latch);
        w.module_state_insert(content);
        let doc = serde_json::json!({
            "manifest": {"id":"travellers", "version":1, "kind":"campaign_seed",
                "digest":"0".repeat(64), "requires":[{"id":"station", "version":2}]},
            "entries": {"npc.visitor": {"kind":"npc", "def": {
                "name":"Visitor", "room_pack":"station", "room":"room.bar",
                "profile":{"persona":"warm"}
            }}}
        });
        let pack = mush_content::parse_pack(&doc.to_string()).unwrap();
        let def = pack.entry(&id("npc.visitor")).unwrap().decode().unwrap();
        (w, pack, def, room)
    }

    #[test]
    fn external_room_requires_an_exact_healthy_live_dependency() {
        let (w, pack, def, room) = external_fixture();
        assert_eq!(
            initial_room(&w, &pack, &def, &BTreeMap::new()).unwrap(),
            room
        );
        let mut undeclared = def.clone();
        undeclared.room_pack = Some("another_station".into());
        assert!(initial_room(&w, &pack, &undeclared, &BTreeMap::new()).is_err());
        let mut local = def.clone();
        local.room_pack = None;
        assert!(initial_room(&w, &pack, &local, &BTreeMap::new()).is_err());
        assert_eq!(
            initial_room(
                &w,
                &pack,
                &local,
                &BTreeMap::from([(CanonicalId::new("room.bar").unwrap(), room)])
            )
            .unwrap(),
            room
        );
        #[derive(Debug)]
        enum Change {
            Old,
            New,
            Retired,
            Anomaly,
            Kind,
            Missing,
            Held,
        }
        for change in [
            Change::Old,
            Change::New,
            Change::Retired,
            Change::Anomaly,
            Change::Kind,
            Change::Missing,
            Change::Held,
        ] {
            let (mut w, pack, def, _) = external_fixture();
            let content = w.module_state_mut::<mush_content::ContentState>().unwrap();
            let mut value = serde_json::to_value(content.latches.all().next().unwrap()).unwrap();
            match change {
                Change::Old => value["version"] = serde_json::json!(1),
                Change::New => value["version"] = serde_json::json!(3),
                Change::Retired => value["retired"] = serde_json::json!(true),
                Change::Anomaly => value["anomaly"] = serde_json::json!("incomplete"),
                Change::Kind => value["entries"]["room.bar"]["kind"] = serde_json::json!("thing"),
                Change::Missing => {
                    value["entries"]["room.bar"]["object"] = serde_json::json!(99999)
                }
                Change::Held => value["version"] = serde_json::json!(0),
            }
            content.latches = mush_content::Latches::load(&[("applied:station".into(), value)]);
            assert!(
                initial_room(&w, &pack, &def, &BTreeMap::new()).is_err(),
                "{change:?}"
            );
        }
        let mut pinned = pack.canonical_value();
        pinned["manifest"]["digest"] = serde_json::json!("0".repeat(64));
        pinned["manifest"]["requires"][0]["digest"] = serde_json::json!("ff".repeat(32));
        let pinned = mush_content::parse_pack(&pinned.to_string()).unwrap();
        assert!(initial_room(&w, &pinned, &def, &BTreeMap::new()).is_err());
        let (mut w, pack, def, room) = external_fixture();
        w.destroy(room).unwrap();
        assert!(initial_room(&w, &pack, &def, &BTreeMap::new()).is_err());
    }

    #[test]
    fn external_npc_pack_reads_dependency_snapshot_during_real_apply() {
        let (mut w, draft, _, room) = external_fixture();
        let mut value = draft.canonical_value();
        value["manifest"]["digest"] = draft.computed_digest().unwrap().to_hex().into();
        let pack = mush_content::load_pack(&value.to_string()).unwrap();
        register_appliers(
            &mut w
                .module_state_mut::<mush_content::ContentState>()
                .unwrap()
                .appliers,
        )
        .unwrap();
        let (_, reg) = mush_testkit::boot_world();
        let mut out = mush_core::Outbox::new();
        let mut ctx = Ctx {
            w: &mut w,
            actor: GOD,
            switches: Default::default(),
            args: Default::default(),
            out: &mut out,
            reg: &reg,
            module: Some(mush_content::NS.into()),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        let (latch, report) = mush_content::apply_pack(&mut ctx, &pack).unwrap();
        assert!(report.anomaly.is_none());
        let npc = latch.entries()[&CanonicalId::new("npc.visitor").unwrap()]
            .object()
            .unwrap();
        assert_eq!(ctx.w.get(npc).unwrap().location, Some(room));
        let latches = &ctx
            .w
            .module_state::<mush_content::ContentState>()
            .unwrap()
            .latches;
        assert!(
            latches
                .applied(&CanonicalId::new("station").unwrap())
                .is_some()
        );
        assert_eq!(latches.applied(pack.manifest().id()), Some(&latch));
        let (_, replay) = mush_content::apply_pack(&mut ctx, &pack).unwrap();
        assert_eq!(replay.created, 0);
        assert_eq!(replay.unchanged, 1);
    }

    #[test]
    fn external_npc_reapply_keeps_identity_location_and_memory() {
        let (mut w, pack, _, _) = external_fixture();
        let (_, reg) = mush_testkit::boot_world();
        let mut out = mush_core::Outbox::new();
        let mut ctx = Ctx {
            w: &mut w,
            actor: GOD,
            switches: Default::default(),
            args: Default::default(),
            out: &mut out,
            reg: &reg,
            module: Some(mush_content::NS.into()),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        let id = CanonicalId::new("npc.visitor").unwrap();
        let entry = pack.entry(&id).unwrap();
        let npc = create_npc(&mut ctx, &pack, &id, entry, &BTreeMap::new(), None)
            .unwrap()
            .unwrap();
        let arrived = ctx
            .w
            .create(ObjKind::Room, "Destination", GOD, None)
            .unwrap();
        ctx.w.move_to(npc, arrived).unwrap();
        let case = ctx
            .w
            .create(ObjKind::Thing, "Personal case", GOD, Some(npc))
            .unwrap();
        ctx.w
            .set_attr(
                npc,
                crate::SUMMARY_ATTR,
                Attr::text("The captain kept a promise.", GOD),
            )
            .unwrap();
        assert_eq!(
            create_npc(&mut ctx, &pack, &id, entry, &BTreeMap::new(), Some(npc)).unwrap(),
            Some(npc)
        );
        assert_eq!(ctx.w.get(npc).unwrap().location, Some(arrived));
        assert_eq!(ctx.w.get(case).unwrap().location, Some(npc));
        assert_eq!(
            ctx.w.attr(npc, crate::SUMMARY_ATTR).unwrap().as_text(),
            Some("The captain kept a promise.")
        );
    }
}
