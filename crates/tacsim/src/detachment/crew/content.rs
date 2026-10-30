//! Standing consent for persistent authored people, using the passenger-role
//! content pattern. Applying content never hires, moves or replaces a person.
use super::{HireTerms, OFFER_ATTR};
use mush_content::{Appliers, EntryApplier, Pack, PackError, RawEntry};
use mush_core::{Attr, AttrFlags, CanonicalId, Ctx, Flags, GOD, ObjId, ObjKind};
use serde::Deserialize;
use std::{collections::BTreeMap, sync::Arc};

pub const PACK: &str = "tacsim.crew.ceres";
pub const JSON: &str = include_str!("../../../content/crew/ceres.json");
pub fn packs() -> Result<Vec<Pack>, String> {
    mush_content::load_pack(JSON)
        .map(|p| vec![p])
        .map_err(|e| format!("crew pack: {e}"))
}
pub fn register_appliers(appliers: &mut Appliers) -> Result<(), PackError> {
    let kind = CanonicalId::new("crew_offer").map_err(|e| PackError::EntryDecode {
        kind: "crew_offer".into(),
        detail: e.to_string(),
    })?;
    appliers.register(crate::NS, Arc::new(CrewApplier { kind }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OfferDef {
    npc: CanonicalId,
    terms: HireTerms,
}
fn definition(pack: &Pack, id: &CanonicalId, entry: &RawEntry) -> Result<OfferDef, String> {
    let def: OfferDef = entry.decode().map_err(|e| format!("{id}: {e}"))?;
    def.terms.digest()?;
    if !pack
        .entry(&def.npc)
        .is_some_and(|e| e.kind().as_str() == "npc")
    {
        return Err(format!("{id}: crew must name an NPC entry in this pack"));
    }
    for (other, entry) in pack
        .entries()
        .iter()
        .filter(|(_, e)| e.kind().as_str() == "crew_offer")
    {
        let sibling: OfferDef = entry.decode().map_err(|e| format!("{other}: {e}"))?;
        if other != id && sibling.npc == def.npc {
            return Err(format!("{id}: duplicate crew offer for {}", def.npc));
        }
    }
    Ok(def)
}
struct CrewApplier {
    kind: CanonicalId,
}
impl CrewApplier {
    fn bind(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        placed: &BTreeMap<CanonicalId, ObjId>,
        existing: Option<ObjId>,
    ) -> Result<Option<ObjId>, String> {
        let def = definition(pack, id, entry)?;
        let npc = placed
            .get(&def.npc)
            .copied()
            .filter(|npc| {
                ctx.w
                    .get(*npc)
                    .is_some_and(|o| o.kind == ObjKind::Npc && !o.flags.contains(Flags::GOING))
            })
            .ok_or_else(|| format!("{id}: crew NPC has not been created"))?;
        if existing.is_some_and(|old| old != npc) {
            return Err(format!("{id}: a crew offer cannot move to another person"));
        }
        if ctx.w.attr(npc, OFFER_ATTR).is_some_and(|a| a.owner != GOD) {
            return Err(format!("{id}: existing crew consent is not staff authored"));
        }
        let mut attr = Attr::json(
            serde_json::to_value(def.terms).map_err(|e| e.to_string())?,
            GOD,
        );
        attr.flags = AttrFlags::WIZARD_ONLY | AttrFlags::HIDDEN | AttrFlags::NO_INHERIT;
        ctx.w
            .set_attr(npc, OFFER_ATTR, attr)
            .map_err(|e| e.to_string())?;
        // Contracts live in their source family. Reapplication cannot clear a
        // hire, return a person to port, or supply another copy of that person.
        Ok(Some(npc))
    }
}
impl EntryApplier for CrewApplier {
    fn kinds(&self) -> Vec<CanonicalId> {
        vec![self.kind.clone()]
    }
    fn phase(&self) -> u8 {
        mush_npc::content::PHASE + 1
    }
    fn check(
        &self,
        _ctx: &Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
    ) -> Result<(), String> {
        definition(pack, id, entry).map(|_| ())
    }
    fn create(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<Option<ObjId>, String> {
        self.bind(ctx, pack, id, entry, placed, None)
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
        self.bind(ctx, pack, id, entry, placed, existing)
    }
    fn retire_check(
        &self,
        ctx: &Ctx<'_>,
        _pack: &CanonicalId,
        id: &CanonicalId,
        existing: Option<ObjId>,
        _placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<(), String> {
        if existing.is_some_and(|npc| ctx.w.get(npc).is_some()) {
            return Err(format!(
                "{id}: a live crew role cannot be removed through a content pack"
            ));
        }
        Ok(())
    }
    fn retire(
        &self,
        _ctx: &mut Ctx<'_>,
        _pack: &CanonicalId,
        _id: &CanonicalId,
        _existing: Option<ObjId>,
    ) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id(s: &str) -> CanonicalId {
        CanonicalId::new(s).unwrap()
    }
    #[test]
    fn signed_crew_pack_has_distinct_persistent_people_and_command_scope() {
        let pack = packs().unwrap().remove(0);
        assert_eq!(pack.manifest().id().as_str(), PACK);
        let operator = id("crew.ilya_mendes");
        let commander = id("crew.sana_okonkwo");
        let worker = definition(&pack, &operator, pack.entry(&operator).unwrap()).unwrap();
        let leader = definition(&pack, &commander, pack.entry(&commander).unwrap()).unwrap();
        assert_ne!(worker.npc, leader.npc);
        assert_eq!(
            worker.terms.mission_scope,
            super::super::NpcMissionScope::SubordinateOperator
        );
        assert_eq!(
            leader.terms.mission_scope,
            super::super::NpcMissionScope::DetachmentCommander
        );
    }
    #[test]
    fn malformed_or_duplicate_consent_refuses_before_content_creates_people() {
        for change in 0..3 {
            let mut value: serde_json::Value = serde_json::from_str(JSON).unwrap();
            match change {
                0 => value["entries"]["crew.ilya_mendes"]["def"]["npc"] = "npc.missing".into(),
                1 => {
                    value["entries"]["crew.another"] = value["entries"]["crew.ilya_mendes"].clone()
                }
                _ => {
                    value["entries"]["crew.ilya_mendes"]["def"]["terms"]["duties"] =
                        serde_json::json!([])
                }
            }
            let pack = mush_content::parse_pack(&value.to_string()).unwrap();
            let role = id("crew.ilya_mendes");
            assert!(definition(&pack, &role, pack.entry(&role).unwrap()).is_err());
        }
    }
    #[test]
    fn content_update_preserves_person_location_and_memories_and_refuses_rebinding() {
        let pack = packs().unwrap().remove(0);
        let (mut world, reg) = mush_testkit::boot_world();
        let home = world.create(ObjKind::Room, "Port", GOD, None).unwrap();
        let away = world.create(ObjKind::Room, "Aboard", GOD, None).unwrap();
        let npc = world.create(ObjKind::Npc, "Ilya", GOD, Some(home)).unwrap();
        let mut out = mush_core::Outbox::new();
        let mut ctx = Ctx {
            w: &mut world,
            actor: GOD,
            switches: Default::default(),
            args: Default::default(),
            out: &mut out,
            reg: &reg,
            module: Some(mush_content::NS.into()),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        let role = id("crew.ilya_mendes");
        let placed = BTreeMap::from([(id("npc.ilya_mendes"), npc)]);
        let applier = CrewApplier {
            kind: id("crew_offer"),
        };
        let entry = pack.entry(&role).unwrap();
        assert_eq!(
            applier
                .create(&mut ctx, &pack, &role, entry, &placed)
                .unwrap(),
            Some(npc)
        );
        ctx.w.move_to(npc, away).unwrap();
        ctx.w
            .set_attr(
                npc,
                mush_npc::SUMMARY_ATTR,
                Attr::text("Hired for the Ahuna run.", GOD),
            )
            .unwrap();
        assert_eq!(
            applier
                .update(&mut ctx, &pack, &role, entry, Some(npc), &placed)
                .unwrap(),
            Some(npc)
        );
        assert_eq!(ctx.w.get(npc).unwrap().location, Some(away));
        assert_eq!(
            ctx.w.attr(npc, mush_npc::SUMMARY_ATTR).unwrap().as_text(),
            Some("Hired for the Ahuna run.")
        );
        let stranger = ctx
            .w
            .create(ObjKind::Npc, "Another person", GOD, Some(home))
            .unwrap();
        let changed = BTreeMap::from([(id("npc.ilya_mendes"), stranger)]);
        assert!(
            applier
                .update(&mut ctx, &pack, &role, entry, Some(npc), &changed)
                .is_err()
        );
        assert!(ctx.w.attr(stranger, OFFER_ATTR).is_none());
    }
}
