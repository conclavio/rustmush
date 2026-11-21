use super::LaunchVenue;
use mush_core::{CanonicalId, Flags, ObjId, ObjKind, World};
use mush_inventory::EndpointRef;
use mush_market::{MarketId, Presence, VenueGate, VenueRefusal};
use mush_org::PrincipalId;

/// A real configured counter, layered over the complete ordinary port gate.
#[derive(Debug)]
pub(super) struct Gate(pub(super) LaunchVenue);
impl VenueGate for Gate {
    fn venue_of_room(&self, w: &World, room: ObjId) -> Option<MarketId> {
        if room == self.0.room
            && w.get(room)
                .is_some_and(|o| o.kind == ObjKind::Room && !o.flags.contains(Flags::GOING))
        {
            Some(self.0.name.clone())
        } else {
            crate::port::market::PortVenueGate.venue_of_room(w, room)
        }
    }
    fn endpoint_for(
        &self,
        w: &World,
        actor: ObjId,
        venue: &MarketId,
        presence: Presence,
    ) -> Option<EndpointRef> {
        crate::port::market::PortVenueGate.endpoint_for(w, actor, venue, presence)
    }
    fn admit(
        &self,
        w: &World,
        actor: ObjId,
        venue: &MarketId,
        endpoint: Option<EndpointRef>,
        presence: Presence,
    ) -> Result<(), VenueRefusal> {
        // Existing governed endpoints always retain the native port predicate.
        crate::port::market::PortVenueGate.admit(w, actor, venue, endpoint, presence)?;
        if venue == &self.0.name
            && (presence != Presence::Here
                || w.get(actor).and_then(|o| o.location) != Some(self.0.room)
                || self.venue_of_room(w, self.0.room).as_ref() != Some(venue))
        {
            return Err(VenueRefusal {
                line: "Visit the configured mind catalog counter.".into(),
            });
        }
        Ok(())
    }
    fn governed(&self, w: &World, endpoint: EndpointRef) -> bool {
        crate::port::market::PortVenueGate.governed(w, endpoint)
    }
    fn receives_at(
        &self,
        w: &World,
        venue: &MarketId,
        endpoint: EndpointRef,
    ) -> Result<(), VenueRefusal> {
        crate::port::market::PortVenueGate.receives_at(w, venue, endpoint)
    }
    fn port_seller(&self, w: &World, venue: &MarketId, seller: &PrincipalId) -> bool {
        crate::port::market::PortVenueGate.port_seller(w, venue, seller)
    }
    fn signatory(&self, w: &World, venue: &MarketId, seller: &PrincipalId) -> Option<String> {
        crate::port::market::PortVenueGate.signatory(w, venue, seller)
    }
    fn indivisible(&self, commodity: &CanonicalId) -> bool {
        crate::port::market::PortVenueGate.indivisible(commodity)
    }
    fn house_takes(&self, commodity: &CanonicalId, grade: u8) -> bool {
        crate::port::market::PortVenueGate.house_takes(commodity, grade)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_economy::managed_entitlement::Recipient;
    use mush_economy::{AccountId, Vault, VaultId, VaultStatus};
    use mush_org::SystemPrincipalId;

    fn test_gate(room: ObjId) -> Gate {
        Gate(LaunchVenue {
            name: MarketId::new("launch.counter").expect("venue"),
            room,
            recipient: Recipient {
                account: AccountId(1),
                owner: PrincipalId::System(SystemPrincipalId::new("launch.venue").expect("sys")),
            },
        })
    }

    #[test]
    fn ext_p2_10_launch_gate_forwards_indivisible_and_house_takes() {
        let gate = test_gate(ObjId(10));
        let part = CanonicalId::new("tacsim.part.sensor.visual_mk1").expect("part");
        let bulk = CanonicalId::new("metals").expect("bulk");

        assert!(gate.indivisible(&part));
        assert!(!gate.indivisible(&bulk));

        assert!(
            !gate.house_takes(&part, crate::hardware::bay::WORN_GRADE),
            "launch gate must forward house_takes and refuse worn parts"
        );
        assert!(gate.house_takes(&part, crate::hardware::bay::INTACT_GRADE));
        assert!(gate.house_takes(&bulk, crate::hardware::bay::WORN_GRADE));
    }

    #[test]
    fn ext_p2_10_launch_gate_forwards_receives_at_for_scrap_escrow_and_lockers() {
        struct Seed {
            tacsim: Option<crate::store::TacsimStore>,
        }
        impl mush_core::GameModule for Seed {
            fn name(&self) -> &'static str {
                "seed"
            }
            fn register(
                &mut self,
                reg: &mut mush_core::Registrar<'_>,
            ) -> Result<(), mush_core::ModError> {
                if let Some(tacsim) = self.tacsim.take() {
                    reg.state(tacsim);
                }
                Ok(())
            }
        }

        let mut world = World::new_minimal();
        let room = world
            .create(ObjKind::Room, "Counter", mush_core::GOD, None)
            .expect("room");
        let player = world
            .create(ObjKind::Player, "Buyer", mush_core::GOD, Some(room))
            .expect("player");
        let container = world
            .create(ObjKind::Thing, "Box", player, Some(player))
            .expect("container");

        let mut tacsim = crate::store::TacsimStore::default();
        let escrow_vault = VaultId(60);
        tacsim
            .works
            .insert(escrow_vault.0, crate::port::works::WorksState::Unreadable);
        let locker_vault = VaultId(70);
        tacsim.lockers.insert(
            (player, smol_str::SmolStr::new("ceres")),
            crate::port::locker::Locker {
                schema: crate::port::locker::LOCKER_SCHEMA,
                player,
                venue: smol_str::SmolStr::new("ceres"),
                vault: locker_vault.0,
                container,
                opened_ic: 0,
                accrued_to_ic: 0,
                owed_minor: 0,
            },
        );

        let mut engine = mush_core::Engine::with_modules(
            world,
            mush_core::Registry::new(),
            vec![
                Box::new(mush_economy::EconomyModule::new()),
                Box::new(Seed {
                    tacsim: Some(tacsim),
                }),
            ],
        )
        .expect("engine");
        let scrap_vault = VaultId(50);
        {
            let mut out = mush_core::Outbox::new();
            let mut ctx = mush_core::Ctx {
                w: &mut engine.world,
                actor: mush_core::GOD,
                switches: Default::default(),
                args: Default::default(),
                out: &mut out,
                reg: &engine.registry,
                module: Some(crate::NS.into()),
                typed_state: mush_core::ModuleActionState::unavailable(),
            };
            mush_economy::create_vault(
                &mut ctx,
                Vault {
                    id: scrap_vault,
                    owner: PrincipalId::Object(player),
                    scope: None,
                    name: smol_str::SmolStr::new("scrap at ceres"),
                    backing: mush_economy::CustodyRef::Container(container),
                    status: VaultStatus::Active,
                    revision: 1,
                },
            );
        }
        let gate = test_gate(room);
        let launch_venue = MarketId::new("launch.counter").expect("venue");
        let ceres = MarketId::new("ceres").expect("ceres");

        // Scrap vault cannot receive delivery at either venue.
        let err = gate
            .receives_at(&engine.world, &ceres, EndpointRef(scrap_vault))
            .expect_err("scrap vault refused");
        assert_eq!(err.line, crate::port::market::MOVE_INTO_SCRAP);

        // Works escrow cannot receive delivery outside allow_escrow_buy.
        let err = gate
            .receives_at(&engine.world, &launch_venue, EndpointRef(escrow_vault))
            .expect_err("works escrow refused");
        assert_eq!(err.line, crate::port::market::MOVE_WORKS_ESCROW);

        // Ceres locker cannot receive delivery at launch.counter, but can at ceres.
        let err = gate
            .receives_at(&engine.world, &launch_venue, EndpointRef(locker_vault))
            .expect_err("foreign-venue locker refused");
        assert_eq!(
            err.line,
            crate::port::market::LOCKER_ELSEWHERE.replace("{port}", "launch.counter")
        );
        assert!(
            gate.receives_at(&engine.world, &ceres, EndpointRef(locker_vault))
                .is_ok()
        );
    }

    #[test]
    fn ext_p2_10_placed_non_port_venue_enforces_escrow_and_locker_rules() {
        use super::super::{
            HardwareCoordinates, HouseDeployment, LaunchDeployment, LaunchMode, NamedHouses,
            Registration, admission::CopratesAdmission, registration::LaunchService,
        };
        use mush_economy::core_integration::ScopedCoreAccount;
        use mush_faction::FactionId;
        use mush_industry::core_production::OriginAgent;
        use mush_inventory::Grade;
        use mush_org::{OrgId, OrgScopeId};

        struct Seed {
            tacsim: Option<crate::store::TacsimStore>,
            launch: Option<LaunchService>,
        }
        impl mush_core::GameModule for Seed {
            fn name(&self) -> &'static str {
                "seed"
            }
            fn register(
                &mut self,
                reg: &mut mush_core::Registrar<'_>,
            ) -> Result<(), mush_core::ModError> {
                if let Some(tacsim) = self.tacsim.take() {
                    reg.state(tacsim);
                }
                if let Some(launch) = self.launch.take() {
                    reg.state(launch);
                }
                Ok(())
            }
        }

        let mut world = World::new_minimal();
        let counter_room = world
            .create(ObjKind::Room, "Launch Counter", mush_core::GOD, None)
            .expect("counter room");
        let elsewhere = world
            .create(ObjKind::Room, "Elsewhere", mush_core::GOD, None)
            .expect("elsewhere");
        let trader = world
            .create(
                ObjKind::Player,
                "Trader",
                mush_core::GOD,
                Some(counter_room),
            )
            .expect("trader");
        let away = world
            .create(ObjKind::Player, "Away", mush_core::GOD, Some(elsewhere))
            .expect("away");
        let container = world
            .create(ObjKind::Thing, "Box", trader, Some(trader))
            .expect("container");

        let launch_venue = MarketId::new("launch.counter").expect("venue");
        let sys = |s: &str| PrincipalId::System(SystemPrincipalId::new(s).expect("sys"));
        let house = |id: u64, name: &str| HouseDeployment {
            organization: OrgId(id),
            scope: OrgScopeId(id),
            treasury: AccountId(id),
            publisher: trader,
            source_name: name.to_owned(),
            attestation_agent: None,
            stipend_minor_per_ic_day: None,
        };
        let venue_cfg = LaunchVenue {
            name: launch_venue.clone(),
            room: counter_room,
            recipient: Recipient {
                account: AccountId(21),
                owner: sys("launch.venue"),
            },
        };
        let deployment = LaunchDeployment {
            format: 1,
            mode: LaunchMode::Initialize,
            registry_issuer: SystemPrincipalId::new("hardware.factory").expect("issuer"),
            registry: Recipient {
                account: AccountId(20),
                owner: sys("launch.registry"),
            },
            venue: venue_cfg.clone(),
            houses: NamedHouses {
                meridiani: house(1, "Meridiani Field Series"),
                coprates: house(2, "Coprates Drill"),
                tycho: house(3, "Safe Conduct"),
            },
            republic: FactionId(1),
            hardware: HardwareCoordinates {
                importer: OrgId(4),
                stock_scope: OrgScopeId(4),
                depot: VaultId(1),
                agents: [
                    OriginAgent {
                        actor: mush_core::GOD,
                        stock: VaultId(2),
                    },
                    OriginAgent {
                        actor: trader,
                        stock: VaultId(3),
                    },
                ],
                grades: [
                    Grade::new(1).expect("g1"),
                    Grade::new(2).expect("g2"),
                    Grade::new(3).expect("g3"),
                ],
                supplier_account: AccountId(30),
                integrator: ScopedCoreAccount {
                    organization: OrgId(5),
                    scope: OrgScopeId(5),
                    account: AccountId(31),
                },
                factory: counter_room,
                output_container: container,
                ungovern: None,
                bay: None,
                buy_back_permille: None,
                depot_stock: Vec::new(),
            },
            market: None,
            procurement: Vec::new(),
        };
        let admission = CopratesAdmission::new(&deployment).expect("admission");
        let reg = Registration::new(deployment, admission).expect("registration");

        let escrow_vault = VaultId(60);
        let ceres_locker = VaultId(70);
        let local_locker = VaultId(71);
        let ordinary_vault = VaultId(72);

        let mut tacsim = crate::store::TacsimStore::default();
        tacsim
            .works
            .insert(escrow_vault.0, crate::port::works::WorksState::Unreadable);
        tacsim.lockers.insert(
            (trader, smol_str::SmolStr::new("ceres")),
            crate::port::locker::Locker {
                schema: crate::port::locker::LOCKER_SCHEMA,
                player: trader,
                venue: smol_str::SmolStr::new("ceres"),
                vault: ceres_locker.0,
                container,
                opened_ic: 0,
                accrued_to_ic: 0,
                owed_minor: 0,
            },
        );
        tacsim.lockers.insert(
            (trader, smol_str::SmolStr::new("launch.counter")),
            crate::port::locker::Locker {
                schema: crate::port::locker::LOCKER_SCHEMA,
                player: trader,
                venue: smol_str::SmolStr::new("launch.counter"),
                vault: local_locker.0,
                container,
                opened_ic: 0,
                accrued_to_ic: 0,
                owed_minor: 0,
            },
        );

        let engine = mush_core::Engine::with_modules(
            world,
            mush_core::Registry::new(),
            vec![Box::new(Seed {
                tacsim: Some(tacsim),
                launch: Some(LaunchService(reg)),
            })],
        )
        .expect("engine");
        let gate = Gate(venue_cfg);

        // Standing at the placed counter: ordinary ungoverned trade, no-endpoint trade, and own
        // local locker at the counter are admitted.
        assert!(
            gate.admit(&engine.world, trader, &launch_venue, None, Presence::Here)
                .is_ok()
        );
        assert!(
            gate.admit(
                &engine.world,
                trader,
                &launch_venue,
                Some(EndpointRef(ordinary_vault)),
                Presence::Here,
            )
            .is_ok()
        );
        assert!(
            gate.admit(
                &engine.world,
                trader,
                &launch_venue,
                Some(EndpointRef(local_locker)),
                Presence::Here,
            )
            .is_ok()
        );

        // Standing away from the counter is still refused.
        let err = gate
            .admit(&engine.world, away, &launch_venue, None, Presence::Here)
            .expect_err("away actor refused");
        assert_eq!(
            err.line,
            crate::port::market::NOT_AT_VENUE.replace("{venue}", "launch.counter")
        );

        // Works escrow vault at the placed counter MUST be refused (`MOVE_WORKS_ESCROW`).
        let err = gate
            .admit(
                &engine.world,
                trader,
                &launch_venue,
                Some(EndpointRef(escrow_vault)),
                Presence::Here,
            )
            .expect_err("works escrow refused at placed venue");
        assert_eq!(err.line, crate::port::market::MOVE_WORKS_ESCROW);

        // Another venue's locker at the placed counter MUST be refused (`LOCKER_ELSEWHERE`).
        let err = gate
            .admit(
                &engine.world,
                trader,
                &launch_venue,
                Some(EndpointRef(ceres_locker)),
                Presence::Here,
            )
            .expect_err("foreign locker refused at placed venue");
        assert_eq!(
            err.line,
            crate::port::market::LOCKER_ELSEWHERE.replace("{port}", "launch.counter")
        );
    }

    #[test]
    fn p2_tsw_2_launch_gate_rejects_foreign_scrap_facility_stores_and_unplaced_container() {
        use super::super::{
            HardwareCoordinates, HouseDeployment, LaunchDeployment, LaunchMode, NamedHouses,
            Registration, admission::CopratesAdmission, registration::LaunchService,
        };
        use mush_economy::core_integration::ScopedCoreAccount;
        use mush_faction::FactionId;
        use mush_industry::core_production::OriginAgent;
        use mush_inventory::Grade;
        use mush_org::{OrgId, OrgScopeId};

        struct Seed {
            tacsim: Option<crate::store::TacsimStore>,
            launch: Option<LaunchService>,
        }
        impl mush_core::GameModule for Seed {
            fn name(&self) -> &'static str {
                "seed"
            }
            fn register(
                &mut self,
                reg: &mut mush_core::Registrar<'_>,
            ) -> Result<(), mush_core::ModError> {
                if let Some(tacsim) = self.tacsim.take() {
                    reg.state(tacsim);
                }
                if let Some(launch) = self.launch.take() {
                    reg.state(launch);
                }
                Ok(())
            }
        }

        let mut world = World::new_minimal();
        let counter_room = world
            .create(ObjKind::Room, "Launch Counter", mush_core::GOD, None)
            .expect("counter room");
        let foreign_facility_room = world
            .create(ObjKind::Room, "Foreign Yard", mush_core::GOD, None)
            .expect("facility room");
        let trader = world
            .create(
                ObjKind::Player,
                "Trader",
                mush_core::GOD,
                Some(counter_room),
            )
            .expect("trader");
        let carried_box = world
            .create(ObjKind::Thing, "Carried Box", trader, Some(trader))
            .expect("carried box");
        let limbo_box = world
            .create(ObjKind::Thing, "Limbo Box", trader, Some(mush_core::LIMBO))
            .expect("limbo box");
        let scrap_box = world
            .create(
                ObjKind::Thing,
                "scrap at ceres",
                trader,
                Some(mush_core::LIMBO),
            )
            .expect("scrap box");

        let launch_venue = MarketId::new("launch.counter").expect("venue");
        let sys = |s: &str| PrincipalId::System(SystemPrincipalId::new(s).expect("sys"));
        let house = |id: u64, name: &str| HouseDeployment {
            organization: OrgId(id),
            scope: OrgScopeId(id),
            treasury: AccountId(id),
            publisher: trader,
            source_name: name.to_owned(),
            attestation_agent: None,
            stipend_minor_per_ic_day: None,
        };
        let venue_cfg = LaunchVenue {
            name: launch_venue.clone(),
            room: counter_room,
            recipient: Recipient {
                account: AccountId(21),
                owner: sys("launch.venue"),
            },
        };
        let deployment = LaunchDeployment {
            format: 1,
            mode: LaunchMode::Initialize,
            registry_issuer: SystemPrincipalId::new("hardware.factory").expect("issuer"),
            registry: Recipient {
                account: AccountId(20),
                owner: sys("launch.registry"),
            },
            venue: venue_cfg.clone(),
            houses: NamedHouses {
                meridiani: house(1, "Meridiani Field Series"),
                coprates: house(2, "Coprates Drill"),
                tycho: house(3, "Safe Conduct"),
            },
            republic: FactionId(1),
            hardware: HardwareCoordinates {
                importer: OrgId(4),
                stock_scope: OrgScopeId(4),
                depot: VaultId(1),
                agents: [
                    OriginAgent {
                        actor: mush_core::GOD,
                        stock: VaultId(2),
                    },
                    OriginAgent {
                        actor: trader,
                        stock: VaultId(3),
                    },
                ],
                grades: [
                    Grade::new(1).expect("g1"),
                    Grade::new(2).expect("g2"),
                    Grade::new(3).expect("g3"),
                ],
                supplier_account: AccountId(30),
                integrator: ScopedCoreAccount {
                    organization: OrgId(5),
                    scope: OrgScopeId(5),
                    account: AccountId(31),
                },
                factory: counter_room,
                output_container: carried_box,
                ungovern: None,
                bay: None,
                buy_back_permille: None,
                depot_stock: Vec::new(),
            },
            market: None,
            procurement: Vec::new(),
        };
        let admission = CopratesAdmission::new(&deployment).expect("admission");
        let reg = Registration::new(deployment, admission).expect("registration");

        let inv_cell: mush_inventory::InventoryHandleCell = std::sync::Arc::default();
        let mut engine = mush_core::Engine::with_modules(
            world,
            mush_core::Registry::new(),
            vec![
                Box::new(mush_org::OrgModule::new()),
                Box::new(mush_economy::EconomyModule::new()),
                Box::new(
                    mush_inventory::InventoryModule::new()
                        .with_quantity_delegates(["industry"])
                        .with_handle_cell(std::sync::Arc::clone(&inv_cell)),
                ),
                Box::new(mush_industry::IndustryModule::new().with_inventory(inv_cell)),
                Box::new(Seed {
                    tacsim: Some(crate::store::TacsimStore::default()),
                    launch: Some(LaunchService(reg)),
                }),
            ],
        )
        .expect("engine");

        let scrap_vault = VaultId(80);
        let facility_vault = VaultId(81);
        let limbo_vault = VaultId(82);
        let local_box_vault = VaultId(83);
        {
            let mut out = mush_core::Outbox::new();
            let mut ctx = mush_core::Ctx {
                w: &mut engine.world,
                actor: mush_core::GOD,
                switches: Default::default(),
                args: Default::default(),
                out: &mut out,
                reg: &engine.registry,
                module: Some(crate::NS.into()),
                typed_state: mush_core::ModuleActionState::unavailable(),
            };
            let caps = [CanonicalId::new("tacsim.yard").expect("cap")]
                .into_iter()
                .collect();
            let facility = mush_industry::FacilityRecord::new(
                foreign_facility_room,
                PrincipalId::Object(trader),
                caps,
                2,
            )
            .expect("facility");
            mush_industry::store::install_authored_facility(&mut ctx, facility)
                .expect("install facility");
            for (id, name, backing) in [
                (
                    scrap_vault,
                    "scrap at ceres",
                    mush_economy::CustodyRef::Container(scrap_box),
                ),
                (
                    facility_vault,
                    "Works stores",
                    mush_economy::CustodyRef::Container(foreign_facility_room),
                ),
                (
                    limbo_vault,
                    "Limbo stash",
                    mush_economy::CustodyRef::Container(limbo_box),
                ),
                (
                    local_box_vault,
                    "Counter satchel",
                    mush_economy::CustodyRef::Container(carried_box),
                ),
            ] {
                mush_economy::create_vault(
                    &mut ctx,
                    Vault {
                        id,
                        owner: PrincipalId::Object(trader),
                        scope: None,
                        name: smol_str::SmolStr::new(name),
                        backing,
                        status: VaultStatus::Active,
                        revision: 1,
                    },
                );
            }
        }

        let gate = Gate(venue_cfg);
        let w = &engine.world;

        // Foreign port scrap vault is refused at launch.counter in both admit and receives_at.
        let err = gate
            .admit(
                w,
                trader,
                &launch_venue,
                Some(EndpointRef(scrap_vault)),
                Presence::Here,
            )
            .expect_err("foreign scrap vault refused in admit");
        assert_eq!(
            err.line,
            crate::port::market::SCRAP_ELSEWHERE.replace("{port}", "launch.counter")
        );
        let err = gate
            .receives_at(w, &launch_venue, EndpointRef(scrap_vault))
            .expect_err("scrap vault refused in receives_at");
        assert_eq!(err.line, crate::port::market::MOVE_INTO_SCRAP);

        // Facility stores vault is refused at launch.counter in both admit and receives_at.
        let err = gate
            .admit(
                w,
                trader,
                &launch_venue,
                Some(EndpointRef(facility_vault)),
                Presence::Here,
            )
            .expect_err("facility stores refused in admit");
        assert_eq!(
            err.line,
            crate::port::market::STORES_ELSEWHERE.replace("{port}", "launch.counter")
        );
        let err = gate
            .receives_at(w, &launch_venue, EndpointRef(facility_vault))
            .expect_err("facility stores refused in receives_at");
        assert_eq!(
            err.line,
            crate::port::market::STORES_ELSEWHERE.replace("{port}", "launch.counter")
        );

        // Unplaced LIMBO container vault fails closed at launch.counter.
        let err = gate
            .admit(
                w,
                trader,
                &launch_venue,
                Some(EndpointRef(limbo_vault)),
                Presence::Here,
            )
            .expect_err("limbo container refused in admit");
        assert_eq!(
            err.line,
            crate::port::market::CONTAINER_ELSEWHERE.replace("{port}", "launch.counter")
        );
        let err = gate
            .receives_at(w, &launch_venue, EndpointRef(limbo_vault))
            .expect_err("limbo container refused in receives_at");
        assert_eq!(
            err.line,
            crate::port::market::CONTAINER_ELSEWHERE.replace("{port}", "launch.counter")
        );

        // A container physically carried by the trader at counter_room IS admitted.
        assert!(
            gate.admit(
                w,
                trader,
                &launch_venue,
                Some(EndpointRef(local_box_vault)),
                Presence::Here,
            )
            .is_ok()
        );
        assert!(
            gate.receives_at(w, &launch_venue, EndpointRef(local_box_vault))
                .is_ok()
        );
    }
}
