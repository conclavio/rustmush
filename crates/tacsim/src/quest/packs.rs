//! Independent starter invitations and a reusable Ceres traveller.

use mush_content::Pack;

pub const PASSENGER_PACK: &str = "tacsim.passengers.ceres";
pub const PACK: &str = "tacsim.quests.first_steps";
pub const PASSENGER_JSON: &str = include_str!("../../content/passengers/ceres.json");
pub const JSON: &str = include_str!("../../content/quests/first_steps.json");

/// Dependency order: the real traveller exists before a quest refers to her.
pub fn packs() -> Result<Vec<Pack>, String> {
    [PASSENGER_JSON, JSON]
        .into_iter()
        .map(|json| {
            mush_content::load_pack(json).map_err(|error| format!("starter quest pack: {error}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::CanonicalId;

    #[test]
    fn signed_starter_packs_keep_the_passenger_independent_and_order_dependencies() {
        let packs = packs().unwrap();
        assert_eq!(packs.len(), 2);
        assert_eq!(packs[0].manifest().id().as_str(), PASSENGER_PACK);
        assert_eq!(packs[1].manifest().id().as_str(), PACK);
        for pack in &packs {
            assert_eq!(pack.manifest().version(), 1);
        }
        assert_eq!(
            packs[0].manifest().digest().to_hex(),
            "8e32dd2b54e360de9e3b49cb2d26ab29b04e8e7092af53f9f5d57b9ae9f26863"
        );
        assert_eq!(
            packs[1].manifest().digest().to_hex(),
            "c982d6ce6378044f8a8f3e682c89ee7ebc324d192615e54439746d80f0611ee2"
        );
        assert!(
            packs[1]
                .manifest()
                .requires()
                .iter()
                .any(|r| r.id.as_str() == PASSENGER_PACK && r.version == 1)
        );
        assert!(
            !packs[0]
                .manifest()
                .requires()
                .iter()
                .any(|r| r.id.as_str() == PACK)
        );
        assert_eq!(
            packs[0]
                .entries()
                .values()
                .filter(|e| e.kind().as_str() == "npc")
                .count(),
            1
        );
        assert!(
            packs[1]
                .entries()
                .values()
                .all(|e| e.kind().as_str() == "quest")
        );
    }

    #[test]
    fn four_optional_jobs_bind_existing_givers_without_rewriting_profiles() {
        let document: serde_json::Value = serde_json::from_str(JSON).unwrap();
        let entries = document["entries"].as_object().unwrap();
        assert_eq!(entries.len(), 4);
        for (key, npc) in [
            ("quest.oduya_first_hull", "npc.oduya"),
            ("quest.dov_fittings", "npc.akintola"),
            ("quest.ma_connections", "npc.petrossian"),
            ("quest.qureshi_first_readings", "npc.qureshi"),
        ] {
            let def = &entries[key]["def"];
            assert_eq!(def["npc"], npc);
            assert_eq!(def["npc_pack"], "neighborhood.ceres_freeport");
            assert_eq!(def["prerequisite"], "open");
            assert!(def.get("profile").is_none());
        }
        let meet = &entries["quest.ma_connections"]["def"]["checkpoints"][0]["objective"];
        assert_eq!(meet["kind"], "passenger");
        assert_eq!(meet["npc_pack"], PASSENGER_PACK);
        assert_eq!(meet["npc"], "npc.nadia_okafor_sato");
        let survey = entries["quest.qureshi_first_readings"]["def"]["checkpoints"]
            .as_array()
            .unwrap();
        assert_eq!(survey.len(), 2);
        assert_eq!(survey[0]["objective"]["mode"], "scan");
        assert_eq!(survey[1]["objective"]["mode"], "sound");
    }

    #[test]
    fn ceres_traveller_uses_the_existing_finite_passenger_template() {
        let document: serde_json::Value = serde_json::from_str(PASSENGER_JSON).unwrap();
        let source = &document["entries"]["passenger.nadia"]["def"];
        let def = crate::port::passenger::TravellerDef {
            schema: 1,
            key: format!("{PASSENGER_PACK}/passenger.nadia"),
            origin: source["origin"].as_str().unwrap().into(),
            destination: source["destination"].as_str().unwrap().into(),
            opening_fare_minor: source["opening_fare_minor"].as_i64().unwrap(),
            max_fare_minor: source["max_fare_minor"].as_i64().unwrap(),
            funding_minor: source["funding_minor"].as_i64().unwrap(),
        };
        def.check().unwrap();
        assert_eq!(def.origin, "ceres");
        assert_eq!(def.destination, "stickney");
        let pack = mush_content::load_pack(PASSENGER_JSON).unwrap();
        let npc = CanonicalId::new(source["npc"].as_str().unwrap()).unwrap();
        assert_eq!(pack.entry(&npc).unwrap().kind().as_str(), "npc");
        let npc: mush_npc::content::NpcDef =
            serde_json::from_value(document["entries"]["npc.nadia_okafor_sato"]["def"].clone())
                .unwrap();
        assert_eq!(npc.room, "room.c10_slack_line");
        assert_eq!(
            npc.room_pack.as_deref(),
            Some("neighborhood.ceres_freeport")
        );
        let profile = npc.profile;
        assert!(
            profile
                .interfaces
                .iter()
                .any(|name| name.as_str() == "passenger")
        );
        assert!(
            !profile
                .interfaces
                .iter()
                .any(|name| name.as_str() == "quest")
        );
        assert!(!profile.fallback.is_empty());
        assert!(!profile.does_not_know.is_empty());
    }
}
