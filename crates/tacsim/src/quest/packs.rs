//! Independent starter invitations, a Ceres traveller and Leila's later choice.

use mush_content::Pack;

pub const PASSENGER_PACK: &str = "tacsim.passengers.ceres";
pub const PACK: &str = "tacsim.quests.first_steps";
pub const PASSENGER_JSON: &str = include_str!("../../content/passengers/ceres.json");
pub const JSON: &str = include_str!("../../content/quests/first_steps.json");
pub const LEILA_PACK: &str = "tacsim.quests.leila_followup";
pub const LEILA_JSON: &str = include_str!("../../content/quests/leila_followup.json");
pub const CONTACT_PACK: &str = "tacsim.quests.port_connections";
pub const CONTACT_JSON: &str = include_str!("../../content/quests/port_connections.json");
pub const SAFE_PASSAGE_PACK: &str = "tacsim.quests.safe_passage";
pub const SAFE_PASSAGE_JSON: &str = include_str!("../../content/quests/safe_passage.json");

/// Dependency order: the real traveller exists before a quest refers to her.
pub fn packs() -> Result<Vec<Pack>, String> {
    [
        PASSENGER_JSON,
        JSON,
        LEILA_JSON,
        CONTACT_JSON,
        SAFE_PASSAGE_JSON,
    ]
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
        assert_eq!(packs.len(), 5);
        assert_eq!(packs[0].manifest().id().as_str(), PASSENGER_PACK);
        assert_eq!(packs[1].manifest().id().as_str(), PACK);
        assert_eq!(packs[2].manifest().id().as_str(), LEILA_PACK);
        assert_eq!(packs[3].manifest().id().as_str(), CONTACT_PACK);
        assert_eq!(packs[4].manifest().id().as_str(), SAFE_PASSAGE_PACK);
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
        assert_eq!(
            packs[2].manifest().digest().to_hex(),
            "8d9cdd3fe91f97ca6871bbbe9556e1c2b47a707ae7ae7f6c1396d502355f97e7"
        );
        assert_eq!(
            packs[3].manifest().digest().to_hex(),
            "6d38a545d42a91333c9a3f2a85716d8c999f4e87686f257070106d495741f933"
        );
        assert_eq!(
            packs[4].manifest().digest().to_hex(),
            "03c64ad7904943334144cacf4facaa7e48dea7715beab650b920c95510d2db8f"
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

    #[test]
    fn leilas_choice_extends_the_existing_person_after_the_exact_first_quest() {
        let pack = mush_content::load_pack(LEILA_JSON).unwrap();
        assert_eq!(pack.entries().len(), 2);
        assert!(
            pack.entries()
                .values()
                .all(|entry| entry.kind().as_str() == "quest")
        );
        assert!(
            pack.manifest()
                .requires()
                .iter()
                .any(
                    |dependency| dependency.id.as_str() == "tacsim.passengers.stickney"
                        && dependency.version == 2
                )
        );
        for (entry, path, room) in [
            ("quest.leila_trade", "trade", "room.c08_chandlery"),
            ("quest.leila_survey", "survey", "room.c07_exchange"),
        ] {
            let value = pack
                .entry(&CanonicalId::new(entry).unwrap())
                .unwrap()
                .def_value();
            assert_eq!(value["npc_pack"], "tacsim.passengers.stickney");
            assert_eq!(value["npc"], "npc.leila_mensah_kovacs");
            assert_eq!(
                value["prerequisite"]["completed"]["quest"],
                "tacsim.passengers.stickney/quest.leila_second_reading"
            );
            assert_eq!(value["choice"]["group"], "tacsim.leila_next_work");
            assert_eq!(value["choice"]["path"], path);
            assert!(
                value["checkpoints"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|checkpoint| checkpoint["room_entry"] == room)
            );
        }
    }

    #[test]
    fn port_introductions_bind_existing_people_without_other_gameplay_requirements() {
        let pack = mush_content::load_pack(CONTACT_JSON).unwrap();
        assert_eq!(pack.entries().len(), 1);
        let dependencies = pack.manifest().requires();
        assert_eq!(dependencies.len(), 1);
        assert_eq!(dependencies[0].id.as_str(), "neighborhood.ceres_freeport");
        assert_eq!(dependencies[0].version, 4);
        let def = pack
            .entry(&CanonicalId::new("quest.ma_port_connections").unwrap())
            .unwrap()
            .def_value();
        assert_eq!(def["npc"], "npc.petrossian");
        assert_eq!(def["prerequisite"], "open");
        assert!(def.get("choice").is_none());
        let checkpoints = def["checkpoints"].as_array().unwrap();
        assert_eq!(checkpoints.len(), 2);
        for (checkpoint, target) in checkpoints.iter().zip(["npc.akintola", "npc.qureshi"]) {
            assert_eq!(checkpoint["objective"]["kind"], "contact");
            assert_eq!(checkpoint["objective"]["npc"], target);
            assert_eq!(
                checkpoint["objective"]["npc_pack"],
                "neighborhood.ceres_freeport"
            );
        }
    }

    #[test]
    fn safe_passage_is_independent_and_uses_the_existing_finite_traveller() {
        let pack = mush_content::load_pack(SAFE_PASSAGE_JSON).unwrap();
        assert_eq!(pack.entries().len(), 1);
        assert_eq!(
            pack.manifest()
                .requires()
                .iter()
                .map(|dependency| { (dependency.id.as_str(), dependency.version) })
                .collect::<Vec<_>>(),
            vec![("neighborhood.ceres_freeport", 4), (PASSENGER_PACK, 1)]
        );
        let def = pack
            .entry(&CanonicalId::new("quest.ma_safe_passage").unwrap())
            .unwrap()
            .def_value();
        assert_eq!(def["npc"], "npc.petrossian");
        assert_eq!(def["prerequisite"], "open");
        assert!(def.get("choice").is_none());
        let checkpoints = def["checkpoints"].as_array().unwrap();
        assert_eq!(checkpoints.len(), 1);
        assert_eq!(checkpoints[0]["room_entry"], "room.c10_slack_line");
        let objective = &checkpoints[0]["objective"];
        assert_eq!(objective["kind"], "paid_passage");
        assert_eq!(objective["npc_pack"], PASSENGER_PACK);
        assert_eq!(objective["npc"], "npc.nadia_okafor_sato");
        assert_eq!(objective["origin"], "ceres");
        assert_eq!(objective["destination"], "stickney");
    }
}
