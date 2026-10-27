//! Reusable authored traveller roles, separate from the NPC's live passage.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_content::{Appliers, EntryApplier, Pack, PackError, RawEntry};
use mush_core::{Attr, AttrFlags, CanonicalId, Ctx, Flags, GOD, ObjId, ObjKind};
use serde::Deserialize;

use super::TravellerDef;

pub const PACK: &str = "tacsim.passengers.stickney";
pub const JSON: &str = include_str!("../../../content/passengers/stickney.json");
const ATTR: &str = "passage:traveller";

pub fn packs() -> Result<Vec<Pack>, String> {
    mush_content::load_pack(JSON)
        .map(|pack| vec![pack])
        .map_err(|error| format!("passenger pack: {error}"))
}

pub fn register_appliers(appliers: &mut Appliers) -> Result<(), PackError> {
    let kind = CanonicalId::new("passenger").map_err(|error| PackError::EntryDecode {
        kind: "passenger".into(),
        detail: error.to_string(),
    })?;
    appliers.register(crate::NS, Arc::new(PassengerApplier { kind }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PassengerDef {
    npc: CanonicalId,
    origin: String,
    destination: String,
    opening_fare_minor: i64,
    max_fare_minor: i64,
    funding_minor: i64,
}

fn definition(
    pack: &Pack,
    id: &CanonicalId,
    entry: &RawEntry,
) -> Result<(CanonicalId, TravellerDef), String> {
    let def: PassengerDef = entry.decode().map_err(|error| format!("{id}: {error}"))?;
    if !pack
        .entry(&def.npc)
        .is_some_and(|entry| entry.kind().as_str() == "npc")
    {
        return Err(format!("{id}: npc must name an NPC entry in this pack"));
    }
    for (other_id, other) in pack
        .entries()
        .iter()
        .filter(|(_, entry)| entry.kind().as_str() == "passenger")
    {
        let other: PassengerDef = other
            .decode()
            .map_err(|error| format!("{other_id}: {error}"))?;
        if other_id != id && other.npc == def.npc {
            return Err(format!("{id}: NPC already has passenger role {other_id}"));
        }
    }
    let traveller = TravellerDef {
        schema: 1,
        key: format!("{}/{id}", pack.manifest().id()),
        origin: def.origin,
        destination: def.destination,
        opening_fare_minor: def.opening_fare_minor,
        max_fare_minor: def.max_fare_minor,
        funding_minor: def.funding_minor,
    };
    traveller.check()?;
    Ok((def.npc, traveller))
}

struct PassengerApplier {
    kind: CanonicalId,
}

impl PassengerApplier {
    fn bind(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        placed: &BTreeMap<CanonicalId, ObjId>,
        existing: Option<ObjId>,
    ) -> Result<Option<ObjId>, String> {
        let (npc_entry, def) = definition(pack, id, entry)?;
        let npc = placed
            .get(&npc_entry)
            .copied()
            .filter(|npc| {
                ctx.w.get(*npc).is_some_and(|object| {
                    object.kind == ObjKind::Npc && !object.flags.contains(Flags::GOING)
                })
            })
            .ok_or_else(|| format!("{id}: NPC {npc_entry} has not been created"))?;
        if existing.is_some_and(|old| old != npc) {
            return Err(format!("{id}: a traveller role cannot move to another NPC"));
        }
        if let Some(old) = ctx.w.attr(npc, ATTR) {
            let old: TravellerDef = old
                .as_json()
                .cloned()
                .ok_or("existing traveller role is not JSON")
                .and_then(|value| {
                    serde_json::from_value(value).map_err(|_| "existing traveller role is invalid")
                })?;
            old.check()?;
            if old.key != def.key {
                return Err(format!(
                    "{id}: NPC already belongs to another traveller role"
                ));
            }
        }
        let mut attr = Attr::json(
            serde_json::to_value(def).map_err(|error| error.to_string())?,
            GOD,
        );
        attr.flags = AttrFlags::WIZARD_ONLY | AttrFlags::HIDDEN | AttrFlags::NO_INHERIT;
        ctx.w
            .set_attr(npc, ATTR, attr)
            .map_err(|error| error.to_string())?;
        // The runtime's funded account and booking live elsewhere. Reapplying an
        // authored role neither refills its purse nor moves or rebooks its NPC.
        Ok(Some(npc))
    }
}

impl EntryApplier for PassengerApplier {
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
                "{id}: a live traveller role cannot be removed through a content pack"
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

    fn id(text: &str) -> CanonicalId {
        CanonicalId::new(text).unwrap()
    }

    #[test]
    fn shipped_traveller_pack_is_signed_and_uses_the_shared_template() {
        let pack = packs().unwrap().remove(0);
        assert_eq!(pack.manifest().id().as_str(), PACK);
        assert_eq!(pack.manifest().version(), 1);
        assert_eq!(
            pack.manifest().digest().to_hex(),
            "02d9fdcfae813fd566ed64a7d30b15104b9c847ac3193fe001e1614708b824c4"
        );
        let (npc, def) = definition(
            &pack,
            &id("passenger.leila"),
            pack.entry(&id("passenger.leila")).unwrap(),
        )
        .unwrap();
        assert_eq!(npc.as_str(), "npc.leila_mensah_kovacs");
        assert_eq!(def.key, "tacsim.passengers.stickney/passenger.leila");
        assert_eq!(
            (def.origin.as_str(), def.destination.as_str()),
            ("stickney", "ceres")
        );
        assert_eq!(
            (
                def.opening_fare_minor,
                def.max_fare_minor,
                def.funding_minor
            ),
            (30_000, 50_000, 50_000)
        );
    }

    #[test]
    fn invalid_or_duplicate_roles_refuse_before_creating_people() {
        #[derive(Debug)]
        enum Change {
            MissingNpc,
            Duplicate,
            Unfunded,
            SamePort,
        }
        for change in [
            Change::MissingNpc,
            Change::Duplicate,
            Change::Unfunded,
            Change::SamePort,
        ] {
            let mut value: serde_json::Value = serde_json::from_str(JSON).unwrap();
            match change {
                Change::MissingNpc => {
                    value["entries"]["passenger.leila"]["def"]["npc"] = "npc.missing".into()
                }
                Change::Duplicate => {
                    value["entries"]["passenger.another"] =
                        value["entries"]["passenger.leila"].clone()
                }
                Change::Unfunded => {
                    value["entries"]["passenger.leila"]["def"]["funding_minor"] = 1.into()
                }
                Change::SamePort => {
                    value["entries"]["passenger.leila"]["def"]["destination"] = "stickney".into()
                }
            }
            let pack = mush_content::parse_pack(&value.to_string()).unwrap();
            assert!(
                definition(
                    &pack,
                    &id("passenger.leila"),
                    pack.entry(&id("passenger.leila")).unwrap()
                )
                .is_err(),
                "{change:?}"
            );
        }
    }

    #[test]
    fn reapplying_role_keeps_live_state_and_hides_bargaining_limits() {
        let pack = packs().unwrap().remove(0);
        let (mut w, reg) = mush_testkit::boot_world();
        let npc = w
            .create(ObjKind::Npc, "Leila", GOD, Some(mush_core::LIMBO))
            .unwrap();
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
        let applier = PassengerApplier {
            kind: id("passenger"),
        };
        let role = id("passenger.leila");
        let placed = BTreeMap::from([(id("npc.leila_mensah_kovacs"), npc)]);
        let entry = pack.entry(&role).unwrap();
        assert_eq!(
            applier
                .create(&mut ctx, &pack, &role, entry, &placed)
                .unwrap(),
            Some(npc)
        );
        let attr = ctx.w.attr(npc, ATTR).unwrap();
        assert_eq!(attr.owner, GOD);
        assert!(
            attr.flags
                .contains(AttrFlags::WIZARD_ONLY | AttrFlags::HIDDEN | AttrFlags::NO_INHERIT)
        );
        let destination = ctx.w.create(ObjKind::Room, "Ceres", GOD, None).unwrap();
        ctx.w.move_to(npc, destination).unwrap();
        ctx.w
            .set_attr(
                npc,
                mush_npc::SUMMARY_ATTR,
                Attr::text("The journey was kind.", GOD),
            )
            .unwrap();
        assert_eq!(
            applier
                .update(&mut ctx, &pack, &role, entry, Some(npc), &placed)
                .unwrap(),
            Some(npc)
        );
        assert_eq!(ctx.w.get(npc).unwrap().location, Some(destination));
        assert_eq!(
            ctx.w.attr(npc, mush_npc::SUMMARY_ATTR).unwrap().as_text(),
            Some("The journey was kind.")
        );
        assert!(
            applier
                .retire_check(&ctx, pack.manifest().id(), &role, Some(npc), &placed)
                .is_err()
        );
        let stranger = ctx
            .w
            .create(ObjKind::Npc, "Different person", GOD, Some(destination))
            .unwrap();
        let rebound = BTreeMap::from([(id("npc.leila_mensah_kovacs"), stranger)]);
        assert!(
            applier
                .update(&mut ctx, &pack, &role, entry, Some(npc), &rebound)
                .is_err()
        );
        assert!(ctx.w.attr(stranger, ATTR).is_none());
    }
}
