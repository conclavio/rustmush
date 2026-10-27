//! The NPC's content entry kinds (14-spatial-implementation-plan §3.1): what a pack
//! says about the people in a place, and how that becomes an agent NPC.
//!
//! - `npc` — a character with an [`AgentProfile`] and an [`NpcKnowledge`] document,
//!   standing in a room the same pack made. The object is created once and **kept**
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
use mush_core::{Attr, CanonicalId, Ctx, GOD, ModuleOp, ModuleWrite, ObjId, ObjKind, WorldEvent};
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
    /// The room entry (of the same pack) the NPC stands in.
    pub room: String,
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
    let room_id = CanonicalId::new(def.room.trim()).map_err(|e| format!("{id}: room: {e}"))?;
    let room = placed
        .get(&room_id)
        .copied()
        .filter(|r| ctx.w.get(*r).is_some_and(|o| o.kind == ObjKind::Room))
        .ok_or_else(|| format!("{id}: room {} is not a room this pack has made", def.room))?;
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

impl EntryApplier for NpcApplier {
    fn kinds(&self) -> Vec<CanonicalId> {
        vec![self.kind.clone()]
    }

    fn phase(&self) -> u8 {
        PHASE
    }

    fn create(
        &self,
        ctx: &mut Ctx<'_>,
        _pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<Option<ObjId>, String> {
        match self.kind.as_str() {
            "npc" => create_npc(ctx, id, entry, placed, None),
            _ => create_pool(ctx, id, entry),
        }
    }

    fn update(
        &self,
        ctx: &mut Ctx<'_>,
        _pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        existing: Option<ObjId>,
        placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<Option<ObjId>, String> {
        match self.kind.as_str() {
            // In place: memories, location and inventory survive.
            "npc" => create_npc(ctx, id, entry, placed, existing),
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
    }
}
