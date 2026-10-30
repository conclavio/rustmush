#![cfg(test)]

//! Genuine paid/owner history and the actual production four-owner IC union.
//! Only the absent region start/terminal event is a private fixture boundary.
//! It supplies no receipt, seal, grant, readiness or physical owner image.
use super::*;
use crate::minds::ic_sortie::{self, IcHost, IcHostTicket};
use mush_core::action::{PreparedAuthoritativeAction, PreparedCausalRef};
use mush_core::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft};
use mush_core::{Inbound, RevisionSet};
use mush_roll::commerce::LicenceId;
use mush_sim::{DomainId, InteriorSpaceRecord, SpaceAnchor, ic_exclusion::ConfiguredIcBase};
use std::collections::BTreeSet;

#[path = "conduct_integration_tests.rs"]
mod conduct;

const CONN: u64 = 931;
const BUYER_CONN: u64 = 932;
type SelectedTicket = Arc<Mutex<Option<Arc<IcHostTicket>>>>;
struct HostIngress {
    host: IcHost,
    selected: SelectedTicket,
}
impl GameModule for HostIngress {
    fn name(&self) -> &'static str {
        "ic_fixture"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::Authoritative);
        let host = self.host.clone();
        let selected = self.selected.clone();
        reg.command(
            mush_core::CommandSpec {
                name: "@ic-host-fixture".into(),
                shape: mush_core::ArgShape::Raw,
                ..Default::default()
            },
            Arc::new(move |ctx: &mut mush_core::Ctx<'_>| {
                let selected = selected
                    .lock()
                    .map_err(|_| mush_core::CmdError::msg("fixture selection"))?;
                let ticket = selected
                    .as_ref()
                    .ok_or_else(|| mush_core::CmdError::msg("fixture absent"))?;
                host.submit(ctx, ticket)
                    .map_err(|_| mush_core::CmdError::msg("actual host submission refused"))?;
                Ok(())
            }),
        )?;
        Ok(())
    }
}
struct RuntimeProof(Arc<std::sync::atomic::AtomicBool>);
impl mush_core::submission_observation::SubmissionDispositionReader for RuntimeProof {
    fn no_intent(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Acquire)
    }
}
struct Runtime;
impl mush_core::submission_observation::SubmissionRuntime for Runtime {
    fn observe(
        &self,
        _: &mush_core::AuthoritativeSubmit,
    ) -> mush_core::submission_observation::SubmissionRuntimeState {
        let proof = Arc::new(std::sync::atomic::AtomicBool::new(false));
        mush_core::submission_observation::SubmissionRuntimeState::new(
            Arc::new(RuntimeProof(proof.clone())),
            Box::new(proof),
        )
    }
}
struct IcWorld {
    engine: Box<Engine>,
    disk: Disk,
    host: IcHost,
    selected: SelectedTicket,
    base: ObjId,
    field: ObjId,
    hulls: [ObjId; 2],
    pending_charters: bool,
    paid_resales: bool,
    registry_revocation: Option<mush_roll::commerce::registry_revocation::RegistryRevocationPolicy>,
    tasks: crate::arena_bridge::ArenaTasks,
    coverage_products: Option<Vec<crate::underwriting::CoverageProductV1>>,
    coverage_host: Option<crate::underwriting::CoverageHost>,
    coverage_calls: coverage::Calls,
    transfer_facts: Option<resale::transfer_facts::Calls>,
    conduct: bool,
    qualified_coverage: Option<coverage::qualified::Configuration>,
}
impl Drop for IcWorld {
    fn drop(&mut self) {
        self.tasks.signal_stop_all();
    }
}
/// The IC world's venue gate: the authored bay is the `fixture` venue every policy
/// here names (the world has no port to give it one), and admission there needs real
/// presence in the bay. Every other room and venue is the production port gate's.
#[derive(Debug)]
struct IcVenue {
    base: ObjId,
}
impl mush_market::VenueGate for IcVenue {
    fn venue_of_room(&self, w: &mush_core::World, room: ObjId) -> Option<MarketId> {
        if room == self.base {
            return MarketId::new("fixture");
        }
        crate::port::market::PortVenueGate.venue_of_room(w, room)
    }
    fn endpoint_for(
        &self,
        w: &mush_core::World,
        actor: ObjId,
        venue: &MarketId,
        presence: mush_market::Presence,
    ) -> Option<mush_inventory::EndpointRef> {
        if venue.as_str() == "fixture" {
            return None;
        }
        crate::port::market::PortVenueGate.endpoint_for(w, actor, venue, presence)
    }
    fn admit(
        &self,
        w: &mush_core::World,
        actor: ObjId,
        venue: &MarketId,
        endpoint: Option<mush_inventory::EndpointRef>,
        presence: mush_market::Presence,
    ) -> Result<(), mush_market::VenueRefusal> {
        if venue.as_str() != "fixture" {
            return crate::port::market::PortVenueGate.admit(w, actor, venue, endpoint, presence);
        }
        if w.containing_room(actor) == Some(self.base) {
            Ok(())
        } else {
            Err(mush_market::VenueRefusal {
                line: "The fixture venue requires actual presence in the IC bay.".into(),
            })
        }
    }
    fn governed(&self, w: &mush_core::World, endpoint: mush_inventory::EndpointRef) -> bool {
        crate::port::market::PortVenueGate.governed(w, endpoint)
    }
    fn port_seller(
        &self,
        w: &mush_core::World,
        venue: &MarketId,
        seller: &mush_org::PrincipalId,
    ) -> bool {
        crate::port::market::PortVenueGate.port_seller(w, venue, seller)
    }
}
fn space(base: ObjId, field: ObjId) -> InteriorSpaceRecord {
    InteriorSpaceRecord {
        object: base,
        anchor: SpaceAnchor::FixedDomain {
            domain: DomainId(u64::from(field.0)),
            anchor_pose: None,
        },
        compartments: BTreeMap::new(),
        rooms: BTreeSet::from([base]),
        egresses: BTreeMap::new(),
    }
}
fn selected_base(base: ObjId, field: ObjId) -> TestResult<ConfiguredIcBase> {
    let space = space(base, field);
    let descriptor =
        ConfiguredIcBase::descriptor(DefinitionId::new("genuine_ic_fixture_base")?, &space)?;
    let reference =
        DefinitionRegistryBuilder::new().register(OwnerId::new("tacsim")?, descriptor)?;
    Ok(ConfiguredIcBase::new(reference, space)?)
}
fn policy() -> TestResult<OriginalQuotePolicy> {
    Ok(OriginalQuotePolicy::new(
        OriginalPrice::new(
            Amount::from_minor(100),
            InstrumentId::new("cr").ok_or("currency")?,
            issuer()?,
            WalletId(1),
            100,
        )?,
        QuoteVenue::new(
            MarketId::new("fixture").ok_or("venue")?,
            EconomicOwner::System(issuer()?),
            WalletId(1),
            0,
        )?,
        OriginalDuration::new(true, TERM_SECONDS, GRACE_SECONDS, QUOTE_SECONDS)?,
    )?)
}
// Borrow the actual registered fixture inputs; this carries no owner evidence.
#[derive(Clone, Copy)]
struct IcFixture<'a> {
    resources: &'a Resources,
    accounts: &'a Arc<Accounts>,
    engine: &'a Engine,
}
impl<'a> From<&'a PurchaseFixture> for IcFixture<'a> {
    fn from(f: &'a PurchaseFixture) -> Self {
        Self {
            resources: &f.resources,
            accounts: &f.accounts,
            engine: &f.engine,
        }
    }
}
impl<'a> From<&'a Fixture> for IcFixture<'a> {
    fn from(f: &'a Fixture) -> Self {
        Self {
            resources: &f.resources,
            accounts: &f.accounts,
            engine: &f.engine,
        }
    }
}
fn house_pin_policy() -> TestResult<mush_market::mind_house::HousePolicy> {
    use mush_economy::managed_entitlement::{GrossRecipientCut, Recipient};
    Ok(mush_market::mind_house::HousePolicy {
        price: Amount::from_minor(400),
        instrument: InstrumentId::new("cr").ok_or("currency")?,
        registry: GrossRecipientCut {
            permille: 100,
            recipient: Some(Recipient {
                account: WalletId(1),
                owner: EconomicOwner::System(issuer()?),
            }),
        },
        venue_name: MarketId::new("fixture").ok_or("venue")?,
        venue: GrossRecipientCut {
            permille: 0,
            recipient: None,
        },
        transferable: true,
        term_seconds: TERM_SECONDS,
        grace_seconds: GRACE_SECONDS,
        quote_seconds: QUOTE_SECONDS,
        renewal_price: Amount::from_minor(300),
        renewal_term_seconds: TERM_SECONDS - 60,
        renewal_grace_seconds: GRACE_SECONDS - 60,
    })
}
impl IcWorld {
    fn boot(
        f: &PurchaseFixture,
        saved: &Durable,
        base: ObjId,
        field: ObjId,
        hulls: [ObjId; 2],
        enabled: bool,
    ) -> TestResult<Self> {
        Self::boot_options(f, saved, base, field, hulls, enabled, false, None)
    }
    #[allow(clippy::too_many_arguments)]
    fn boot_options(
        f: &PurchaseFixture,
        saved: &Durable,
        base: ObjId,
        field: ObjId,
        hulls: [ObjId; 2],
        enabled: bool,
        pending_charters: bool,
        registry_revocation: Option<
            mush_roll::commerce::registry_revocation::RegistryRevocationPolicy,
        >,
    ) -> TestResult<Self> {
        Self::boot_all(
            f,
            saved,
            base,
            field,
            hulls,
            enabled,
            pending_charters,
            false,
            registry_revocation,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn boot_all(
        f: &PurchaseFixture,
        saved: &Durable,
        base: ObjId,
        field: ObjId,
        hulls: [ObjId; 2],
        enabled: bool,
        pending_charters: bool,
        paid_resales: bool,
        registry_revocation: Option<
            mush_roll::commerce::registry_revocation::RegistryRevocationPolicy,
        >,
    ) -> TestResult<Self> {
        Self::boot_all_coverage(
            f,
            saved,
            base,
            field,
            hulls,
            enabled,
            pending_charters,
            paid_resales,
            registry_revocation,
            None,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn boot_all_coverage(
        f: &PurchaseFixture,
        saved: &Durable,
        base: ObjId,
        field: ObjId,
        hulls: [ObjId; 2],
        enabled: bool,
        pending_charters: bool,
        paid_resales: bool,
        registry_revocation: Option<
            mush_roll::commerce::registry_revocation::RegistryRevocationPolicy,
        >,
        coverage_products: Option<Vec<crate::underwriting::CoverageProductV1>>,
    ) -> TestResult<Self> {
        Self::boot_source(
            f.into(),
            saved,
            base,
            field,
            hulls,
            enabled,
            pending_charters,
            paid_resales,
            registry_revocation,
            coverage_products,
            false,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn boot_source(
        f: IcFixture<'_>,
        saved: &Durable,
        base: ObjId,
        field: ObjId,
        hulls: [ObjId; 2],
        enabled: bool,
        pending_charters: bool,
        paid_resales: bool,
        registry_revocation: Option<
            mush_roll::commerce::registry_revocation::RegistryRevocationPolicy,
        >,
        coverage_products: Option<Vec<crate::underwriting::CoverageProductV1>>,
        two_seat_house: bool,
    ) -> TestResult<Self> {
        Self::boot_source_facts(
            f,
            saved,
            base,
            field,
            hulls,
            enabled,
            pending_charters,
            paid_resales,
            registry_revocation,
            coverage_products,
            two_seat_house,
            None,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn boot_source_facts(
        f: IcFixture<'_>,
        saved: &Durable,
        base: ObjId,
        field: ObjId,
        hulls: [ObjId; 2],
        enabled: bool,
        pending_charters: bool,
        paid_resales: bool,
        registry_revocation: Option<
            mush_roll::commerce::registry_revocation::RegistryRevocationPolicy,
        >,
        coverage_products: Option<Vec<crate::underwriting::CoverageProductV1>>,
        two_seat_house: bool,
        transfer_facts: Option<resale::transfer_facts::Calls>,
    ) -> TestResult<Self> {
        Self::boot_source_conduct(
            f,
            saved,
            base,
            field,
            hulls,
            enabled,
            pending_charters,
            paid_resales,
            registry_revocation,
            coverage_products,
            two_seat_house,
            transfer_facts,
            false,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn boot_source_conduct(
        f: IcFixture<'_>,
        saved: &Durable,
        base: ObjId,
        field: ObjId,
        hulls: [ObjId; 2],
        enabled: bool,
        pending_charters: bool,
        paid_resales: bool,
        registry_revocation: Option<
            mush_roll::commerce::registry_revocation::RegistryRevocationPolicy,
        >,
        coverage_products: Option<Vec<crate::underwriting::CoverageProductV1>>,
        two_seat_house: bool,
        transfer_facts: Option<resale::transfer_facts::Calls>,
        conduct: bool,
    ) -> TestResult<Self> {
        Self::boot_source_qualified(
            f,
            saved,
            base,
            field,
            hulls,
            enabled,
            pending_charters,
            paid_resales,
            registry_revocation,
            coverage_products,
            two_seat_house,
            transfer_facts,
            conduct,
            None,
        )
    }
    #[inline(never)]
    fn prepare_commerce(
        layer: &crate::TacsimRuleset,
        two_seat_house: bool,
        registry_revocation: &Option<
            mush_roll::commerce::registry_revocation::RegistryRevocationPolicy,
        >,
        pending_charters: bool,
        paid_resales: bool,
        transfer_facts: bool,
        conduct: bool,
    ) -> TestResult<crate::composition::mind_commerce::MindCommerce> {
        let composition = crate::composition::mind_commerce::MindCommerce::prepare(
            layer,
            CommerceMode::Active,
            InitialFilingTerms::new(TERM_SECONDS, GRACE_SECONDS)?,
            filing_policy()?,
        )?
        .with_original_purchases(policy()?)?
        .with_charter_transfers()?;
        let composition = if two_seat_house {
            let policy = house_pin_policy()?;
            let renewal = mush_market::mind_house_renewal::RenewalPolicy {
                registry: policy.registry.clone(),
                venue: Some(policy.venue.clone()),
                offer_ic_seconds: QUOTE_SECONDS,
            };
            composition
                .with_house_filings()?
                .with_house_minds(policy.clone())?
                .with_two_seat_house_pins(policy)?
                .with_house_mind_renewals(renewal)?
        } else {
            composition
        };
        let composition = if let Some(policy) = registry_revocation {
            composition.with_registry_revocation(policy.clone())?
        } else {
            composition
        };
        let composition = if pending_charters {
            composition.with_pending_charter_transfers(
                mush_roll::commerce::PendingCharterTransferConfig::new(GOD, 60)?,
            )?
        } else {
            composition
        };
        let composition = if paid_resales {
            composition.with_paid_mind_resales(GOD, 60)?
        } else {
            composition
        };
        let composition = if transfer_facts {
            composition.with_minds_facts()?
        } else {
            composition
        };
        let composition = if conduct {
            composition.with_conduct_trials(layer)?
        } else {
            composition
        };
        Ok(composition)
    }

    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    fn boot_source_qualified(
        f: IcFixture<'_>,
        saved: &Durable,
        base: ObjId,
        field: ObjId,
        hulls: [ObjId; 2],
        enabled: bool,
        pending_charters: bool,
        paid_resales: bool,
        registry_revocation: Option<
            mush_roll::commerce::registry_revocation::RegistryRevocationPolicy,
        >,
        coverage_products: Option<Vec<crate::underwriting::CoverageProductV1>>,
        two_seat_house: bool,
        transfer_facts: Option<resale::transfer_facts::Calls>,
        conduct: bool,
        qualified_coverage: Option<coverage::qualified::Configuration>,
    ) -> TestResult<Self> {
        let mut cfg = f.resources.config();
        cfg.enabled = true;
        let layer = crate::TacsimRuleset::with_config(cfg);
        let layer = if conduct {
            layer.with_conduct_carrier()?
        } else {
            layer
        };
        let composition = Self::prepare_commerce(
            &layer,
            two_seat_house,
            &registry_revocation,
            pending_charters,
            paid_resales,
            transfer_facts.is_some(),
            conduct,
        )?;
        // This is the production preparation path, never cfg(test) Resources::register.
        let old_manifest = f
            .engine
            .registry
            .definitions()
            .ok_or("definitions")?
            .iter()
            .map(|d| d.reference())
            .find(|r| {
                r.key().kind.as_str() == "registry_manifest" && r.key().id.as_str() == "combat"
            })
            .ok_or("genuine measured manifest")?;
        check_eq!(composition.manifest(), old_manifest);
        let composition = if enabled {
            composition.with_ic_minds(vec![selected_base(base, field)?])?
        } else {
            composition
        };
        // Doc 17h cut M8d fix round K9 (D5 composed): a product excluding an ungoverned core
        // is decided over the Registry inspection's records, so the inspection composes with it.
        let excluding = coverage_products.as_ref().is_some_and(|products| {
            products
                .iter()
                .any(|p| p.excludes(crate::underwriting::CoverageExclusion::UngovernedCore))
        });
        let composition = if excluding {
            composition.with_registry_inspection(coverage::void_policy()?)?
        } else {
            composition
        };
        let composition = if let Some(qualified) = &qualified_coverage {
            if !conduct {
                return Err("qualified fixture requires actual Conduct".into());
            }
            composition
                .with_registry_recall(qualified.recall.clone())?
                .with_qualified_hull_coverage(
                    GOD,
                    coverage_products.clone().ok_or("qualified products")?,
                    qualified.manifest.clone(),
                    qualified.selections.clone(),
                )?
        } else if let Some(products) = &coverage_products {
            composition.with_hull_coverage(GOD, products.clone())?
        } else {
            composition
        };
        let (tactics, sim) = composition.configure_ic_physical(
            mush_tactics::TacticsModule::with_catalog(layer.definition_catalog()?.units),
            mush_sim::SimulationModule::new().with_accounts(f.accounts.clone()),
        );
        let layer = composition.configure_tacsim(layer)?;
        let host = layer.ic_host();
        let coverage_host = layer.coverage_host();
        let coverage_calls = coverage::calls();
        let layer: Box<dyn GameModule> = if coverage_products.is_some() || conduct {
            Box::new(coverage::Ingress {
                inner: layer,
                calls: coverage_calls.clone(),
            })
        } else {
            Box::new(layer)
        };
        let selected: SelectedTicket = Arc::new(Mutex::new(None));
        let mut modules: Vec<Box<dyn GameModule>> = vec![
            Box::new(mush_org::OrgModule::new()),
            Box::new(mush_comsys::ComsysModule::new()),
            Box::new(sim),
            Box::new(mush_atlas::AtlasModule::new()),
            Box::new(tactics),
            Box::new(mush_npc::NpcModule::new()),
            layer,
            // Match production owner order: Roll's complete final audit opens
            // the readiness gate needed by Market's paired recovery audits.
            resale::transfer_facts::economy(
                composition.configure_economy(
                    EconomyModule::with_custody(vec![Arc::new(mush_sim::InteriorCustodyProvider)])
                        .with_title_issuers(vec![issuer()?]),
                )?,
                transfer_facts.clone(),
            ),
            Box::new(composition.configure_roll(RollModule::with_writers(["tacsim"]))),
            Box::new(
                composition.configure_market(
                    MarketModule::new().with_venue_gate(Arc::new(IcVenue { base })),
                ),
            ),
            paid_checkout::content()?,
        ];
        if let Some(host) = &host {
            modules.push(Box::new(HostIngress {
                host: host.clone(),
                selected: selected.clone(),
            }));
        }
        let mut engine = boxed_fixture_engine(saved.world()?, Registry::new(), modules)?;
        engine.set_account_identity(f.accounts.clone());
        engine.set_offer_keys(mush_core::OfferKeyRing::new(
            mush_core::OfferSecret::new(1, [0x49; 32])?,
            [],
        )?);
        let loaded = load(&mut engine, &saved.disk);
        if !enabled {
            check!(
                loaded.is_err(),
                "disabled production roster silently accepted IC history"
            );
            return Err("confirmed disabled retained-history refusal".into());
        }
        loaded?;
        let tasks = engine
            .world
            .module_state::<crate::arena_bridge::ArenaRegistry>()
            .ok_or("arena registry")?
            .tasks()
            .clone();
        let mut result = Self {
            engine,
            disk: saved.disk.clone(),
            host: host.ok_or("IC opt-in")?,
            selected,
            base,
            field,
            hulls,
            pending_charters,
            paid_resales,
            registry_revocation,
            tasks,
            coverage_products,
            coverage_host,
            coverage_calls,
            transfer_facts,
            conduct,
            qualified_coverage,
        };
        result.persist_setup()?;
        result.engine.step(
            Inbound::ConnUp {
                conn: CONN,
                actor: GOD,
                caps_structured: false,
                from_linkdead: false,
            },
            &mut Outbox::new(),
            &mut mush_core::Effects::new(),
        );
        result.engine.step(
            Inbound::ConnUp {
                conn: BUYER_CONN,
                actor: BUYER,
                caps_structured: false,
                from_linkdead: false,
            },
            &mut Outbox::new(),
            &mut mush_core::Effects::new(),
        );
        result.persist_setup()?;
        Ok(result)
    }
    fn new(f: &PurchaseFixture, saved: &Durable) -> TestResult<Self> {
        Self::new_source(f.into(), saved, false)
    }
    fn new_house(f: &Fixture, saved: &Durable) -> TestResult<Self> {
        Self::new_source(f.into(), saved, true)
    }
    fn new_source(f: IcFixture<'_>, saved: &Durable, two_seat_house: bool) -> TestResult<Self> {
        let mut world = saved.world()?;
        let field = world.create(ObjKind::Room, "IC authored ground", GOD, None)?;
        let base = world.create(ObjKind::Room, "IC authored bay", GOD, None)?;
        let first = world.create(ObjKind::Thing, "IC first hull", GOD, Some(base))?;
        let second = world.create(ObjKind::Thing, "IC second hull", GOD, Some(base))?;
        for (object, behavior) in [
            (field, "sim.domain"),
            (base, "sim.interior"),
            (first, "sim.participant"),
            (second, "sim.participant"),
        ] {
            world.attach_behavior(
                object,
                mush_core::BehaviorId::new(behavior)?,
                mush_core::BehaviorBinding {
                    schema_version: 1,
                    attached_by: GOD,
                    attached_at: 0,
                },
            )?;
        }
        // An authored test-world base, not evidence that an IC journey started.
        let mut image = saved.clone();
        image.objects = world.iter().cloned().collect();
        image.next_id = world.next_id();
        image.disk.insert(
            ("sim".into(), format!("domain:{}", field.0)),
            json!(mush_sim::DomainRecord {
                id: DomainId(u64::from(field.0)),
                host: field,
                driver: mush_sim::passive_definition_ref(),
                status: mush_sim::DomainStatus::Active,
                clock: mush_sim::DomainClock::Passive,
                presentation_location: field,
                revision: 1,
                window: None,
            }),
        );
        image.disk.insert(
            ("sim".into(), format!("space:{}", base.0)),
            json!(space(base, field)),
        );
        let mut actual = Self::boot_source(
            f,
            &image,
            base,
            field,
            [first, second],
            true,
            false,
            two_seat_house,
            None,
            None,
            two_seat_house,
        )?;
        for hull in [first, second] {
            actual.command(&format!("@unit/create #{}=tacsim.unit.heavy_mk1", hull.0))?;
            actual.command(&format!("@asset/title #{}=#{}", hull.0, BUYER.0))?;
            actual.command(&format!("@spatial/place/interior #{}=#{}", hull.0, base.0))?;
            check!(mush_tactics::module::unit_of(&actual.engine.world, hull).is_some());
            check_eq!(
                actual
                    .engine
                    .world
                    .module_state::<mush_sim::SpatialState>()
                    .ok_or("spatial")?
                    .parent_of(hull),
                mush_sim::SpatialParent::Interior {
                    space: base,
                    slot: None
                }
            );
        }
        Ok(actual)
    }
    fn persist_setup(&mut self) -> TestResult {
        module_ops(&mut self.disk, &self.engine.take_module_ops())?;
        // Ordinary boot/staff setup is persisted before the composed lane.
        // Never clear mutation bookkeeping between prepare, stage and take.
        let _ = self.engine.world.take_dirty();
        Ok(())
    }
    fn command(&mut self, line: &str) -> TestResult {
        let mut out = Outbox::new();
        self.engine.step(
            Inbound::Cmd {
                conn: CONN,
                actor: GOD,
                line: line.into(),
            },
            &mut out,
            &mut mush_core::Effects::new(),
        );
        for (_, event) in out.events() {
            let text = serde_json::to_string(event)?;
            check!(!text.contains(SOURCE), "private source in setup response");
            if matches!(event,mush_proto::OutputEvent::Sys(t) if t.spans().iter().any(|s|s.tag.as_deref()==Some("error")))
            {
                return Err("actual IC fixture setup refused".into());
            }
        }
        self.persist_setup()
    }
    fn snapshot(&self) -> Durable {
        Durable::capture(&self.engine, &self.disk)
    }
    fn reboot(&self, f: &PurchaseFixture, saved: &Durable) -> TestResult<Self> {
        Self::boot_source_qualified(
            f.into(),
            saved,
            self.base,
            self.field,
            self.hulls,
            true,
            self.pending_charters,
            self.paid_resales,
            self.registry_revocation.clone(),
            self.coverage_products.clone(),
            false,
            self.transfer_facts.clone(),
            self.conduct,
            self.qualified_coverage.clone(),
        )
    }
    fn reboot_house(&self, f: &Fixture, saved: &Durable) -> TestResult<Self> {
        Self::boot_source(
            f.into(),
            saved,
            self.base,
            self.field,
            self.hulls,
            true,
            false,
            true,
            None,
            None,
            true,
        )
    }
    fn ticket(&self, actor: ObjId, hull: ObjId, licence: u64) -> TestResult<IcHostTicket> {
        Ok(self.host.enqueue_start(
            &ic_sortie::fixture_start(&self.engine.world, actor, hull)?,
            LicenceId::new(licence)?,
            vec![],
        )?)
    }
    fn prepare(
        &mut self,
        ticket: &IcHostTicket,
        serial: u128,
    ) -> TestResult<(
        PreparedComposedTxn,
        PreparedAuthoritativeAction,
        ActionInvocation,
    )> {
        self.prepare_key(ticket, serial, None)
    }
    fn prepare_key(
        &mut self,
        ticket: &IcHostTicket,
        serial: u128,
        submitted_key: Option<mush_core::IdempotencyKeyText>,
    ) -> TestResult<(
        PreparedComposedTxn,
        PreparedAuthoritativeAction,
        ActionInvocation,
    )> {
        let principal = PrincipalId::Object { id: ticket.actor() };
        let query = self.engine.legal_action_query(
            principal.clone(),
            ticket.actor(),
            PerceivedRef::SelfActor,
        )?;
        let offers = self.engine.project_legal_actions(&query)?;
        let projected = offers
            .projected_for_action(ticket.action())
            .ok_or("real signed IC offer")?;
        let resolved = self.engine.resolve_legal_action(
            &offers,
            mush_core::OfferExecutionContext {
                request: RequestUid::from_bytes(serial.to_be_bytes()),
                principal: principal.clone(),
                actor: ticket.actor(),
                idempotency: ScopedIdempotencyKey::new(
                    IdempotencyScope {
                        principal,
                        owner: OwnerId::new("tacsim")?,
                        namespace: CanonicalId::new("ic_fixture")?,
                    },
                    1,
                    submitted_key
                        .map(IdempotencyKey::Caller)
                        .unwrap_or_else(|| {
                            IdempotencyKey::Request(RequestUid::from_bytes(serial.to_be_bytes()))
                        }),
                ),
                viewer_scope: query.viewer_scope(),
                now: None,
            },
            mush_core::OfferSubmission {
                offer: projected.offer(),
                subject: PerceivedRef::SelfActor,
                targets: vec![],
                parameters: ticket.parameters(),
            },
        )?;
        let (action, resolved, _) = resolved.into_authoritative_parts();
        let invocation = resolved.invocation().clone();
        let (p, o) = self
            .engine
            .prepare_registered_composed_resolved_authoritative_action(
                &action,
                &resolved,
                token(serial),
            )?;
        Ok((p, o, invocation))
    }
    fn stage(
        &mut self,
        ticket: &IcHostTicket,
        serial: u128,
        p: PreparedComposedTxn,
        o: PreparedAuthoritativeAction,
        inv: &ActionInvocation,
    ) -> TestResult {
        self.stage_action(ticket.action(), serial, p, o, inv)
    }
    fn stage_action(
        &mut self,
        action: &DefinitionRef,
        serial: u128,
        p: PreparedComposedTxn,
        o: PreparedAuthoritativeAction,
        inv: &ActionInvocation,
    ) -> TestResult {
        let (
            outcome,
            resolution,
            mut definitions,
            mut witnesses,
            mut durable_outbox,
            semantic_events,
        ) = o.into_parts();
        definitions.extend([action.clone(), outcome.schema().clone()]);
        definitions.sort();
        definitions.dedup();
        if witnesses.is_empty() {
            witnesses.push(inv.principal().clone());
        }
        witnesses.sort();
        witnesses.dedup();
        durable_outbox.sort_by_key(|o| o.ordinal());
        let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
            token: token(serial),
            request: inv.request(),
            idempotency: inv.idempotency().clone(),
            ingress_hash: CanonicalHash::from_bytes([3; 32]),
            request_hash: inv.request_hash(),
            plan_hash: CanonicalHash::from_bytes([4; 32]),
            action: action.clone(),
            provider: p.owner().clone(),
            cause: PreparedCausalRef::new(inv.request(), None, 0)?,
            definitions,
            writes: p.authoritative_replay_writes(),
            outcome,
            resolution: resolution.map(|r| r.bind(token(serial))),
            semantic_events,
            eligible_witnesses: witnesses,
            durable_outbox,
        })?;
        let guards = self
            .engine
            .registry
            .server_guard_set_builder(RevisionSet::default())?
            .finish()?;
        self.engine.stage_authoritative_action(p, guards, bundle)?;
        Ok(())
    }
    fn held(&self, hull: ObjId) -> TestResult<bool> {
        let t = self
            .engine
            .world
            .module_state::<mush_tactics::UnitStore>()
            .ok_or("Tactics")?
            .is_ic_custodied(hull);
        let s = mush_sim::ic_exclusion::identity_of(&self.engine.world, hull).is_some();
        check_eq!(t, s, "physical owner agreement");
        check_eq!(
            s,
            mush_sim::deployed::is_custodied(&self.engine.world, hull)
        );
        check!(
            mush_sim::deployed::mark_of(&self.engine.world, hull).is_none(),
            "no OOC marker"
        );
        Ok(s)
    }
    fn land(
        &mut self,
        ticket: &IcHostTicket,
        serial: u128,
        hull: ObjId,
        was_held: bool,
    ) -> TestResult {
        self.land_key(ticket, serial, hull, was_held, None)
    }
    fn land_key(
        &mut self,
        ticket: &IcHostTicket,
        serial: u128,
        hull: ObjId,
        was_held: bool,
        key: Option<mush_core::IdempotencyKeyText>,
    ) -> TestResult {
        self.land_check(ticket, serial, hull, was_held, key, |_, _| Ok(()))
    }
    fn land_check(
        &mut self,
        ticket: &IcHostTicket,
        serial: u128,
        hull: ObjId,
        was_held: bool,
        key: Option<mush_core::IdempotencyKeyText>,
        before_ack: impl FnOnce(&Self, &IcHostTicket) -> TestResult,
    ) -> TestResult {
        let before = self.snapshot();
        let physical = mush_tactics::module::unit_of(&self.engine.world, hull)
            .ok_or("unit")?
            .clone();
        let (p, o, inv) = self.prepare_key(ticket, serial, key)?;
        check!(
            self.disk == before.disk,
            "prepare wrote private persistent state"
        );
        check_eq!(self.held(hull)?, was_held);
        check!(self.host.acknowledge(&self.engine.world, ticket)?.is_none());
        self.stage(ticket, serial, p, o, &inv)?;
        check_eq!(self.held(hull)?, was_held);
        let staged = self
            .engine
            .take_staged_action()?
            .ok_or("actual four-owner custody")?;
        check_eq!(self.held(hull)?, was_held);
        check_eq!(
            mush_tactics::module::unit_of(&self.engine.world, hull),
            Some(&physical)
        );
        let (bundle, installation) = staged.into_parts();
        retain_writes(&mut self.disk, bundle.writes())?;
        check!(self.host.acknowledge(&self.engine.world, ticket)?.is_none());
        self.engine.install_composed_action(installation)?;
        before_ack(self, ticket)?;
        let ack = self
            .host
            .acknowledge(&self.engine.world, ticket)?
            .ok_or("actual installed readback")?;
        check_eq!(ack.closed().is_some(), was_held);
        check_ne!(self.held(hull)?, was_held);
        Ok(())
    }
}

pub(super) fn exercise(f: &PurchaseFixture, granted: &Durable) -> TestResult {
    eprintln!("M7_STORY phase=ic_composed event=begin");
    let mut actual = IcWorld::new(f, granted)?;
    let first = *actual.hulls.first().ok_or("first hull")?;
    let second = *actual.hulls.get(1).ok_or("second hull")?;
    let seeded = actual.snapshot();
    coverage::exercise(f, &actual, &seeded)?;
    pending::exercise(f, &actual, &seeded)?;
    resale::exercise(f, &actual, &seeded)?;
    expiry_fences(f, &actual, &seeded)?;
    observed_submission(f, &actual, &seeded)?;
    let start = actual.ticket(BUYER, first, 2)?;
    actual.land(&start, 12_000_001, first, false)?;
    let opened = actual.snapshot();
    let full = actual
        .host
        .resume(&actual.engine.world, token(12_000_001))?;
    let seal = full.seal().clone();
    let mut running = arena_for(&actual, &seal)?;
    full.install(&actual.engine.world, &mut running)?;
    running.resume_at(0);
    for wall in [250, 500, 750, 1000] {
        check_eq!(running.step_settled(wall, &[], &[]).motor.mind_faults, 0);
    }
    check!(
        running
            .minds
            .get(&first)
            .ok_or("actual sealed mind")?
            .think_seq
            > 0
    );
    check_eq!(seal.unit, first);
    check!(seal.requested.origin.certificate().is_some());
    check!(mush_sim::deployed::excluded(
        &actual.engine.world,
        mush_sim::deployed::Shown::Unit(first)
    ));
    check!(!mush_sim::deployed::excluded(
        &actual.engine.world,
        mush_sim::deployed::Shown::Unit(actual.base)
    ));
    // Same paid licence cannot claim another physical hull while its seat is held.
    let overflow = actual.ticket(BUYER, second, 2)?;
    check!(actual.prepare(&overflow, 12_000_002).is_err());
    check!(!actual.held(second)?);
    // Restart retains the actual seal and exclusion; no current rights query is used.
    let mut cold = actual.reboot(f, &opened)?;
    check_eq!(
        cold.host
            .resume(&cold.engine.world, token(12_000_001))?
            .seal(),
        &seal
    );
    let term: PaidTermRecord =
        serde_json::from_value(one(rows(&cold.disk, "roll", "roll_mind_paid_term"))?)?;
    cold.engine.world.set_now(term.window().grace_end().get());
    check_eq!(
        cold.host
            .resume(&cold.engine.world, token(12_000_001))?
            .seal(),
        &seal
    );
    let resumed = cold.host.resume(&cold.engine.world, token(12_000_001))?;
    let mut restarted = arena_for(&cold, &seal)?;
    resumed.install(&cold.engine.world, &mut restarted)?;
    restarted.resume_at(0);
    for wall in [250, 500, 750, 1000] {
        check_eq!(restarted.step_settled(wall, &[], &[]).motor.mind_faults, 0);
    }
    check!(
        restarted
            .minds
            .get(&first)
            .ok_or("post-expiry retained mind")?
            .think_seq
            > 0
    );
    check_eq!(
        restarted.unit(first).ok_or("runtime hull")?.seal,
        seal.seal_ref()
    );
    let expired = cold.ticket(BUYER, second, 2)?;
    check!(cold.prepare(&expired, 12_000_003).is_err());
    // Genuine host provenance is deliberately not claimed: this private fixture
    // event drives the real condition codec, four prepared owners and close receipt.
    let event = ic_sortie::fixture_close(&cold.engine.world, BUYER, token(12_000_001), cold.base)?;
    let closing = cold.host.enqueue_close(&event)?;
    cold.land(&closing, 12_000_004, first, true)?;
    check!(crate::test_checks::refused(full.install(
        &cold.engine.world,
        arena_for(&cold, &seal)?.as_mut()
    ))?);
    let closed = cold.snapshot();
    let rebooted = cold.reboot(f, &closed)?;
    check!(!rebooted.held(first)?);
    check!(
        rebooted
            .host
            .resume(&rebooted.engine.world, token(12_000_001))
            .is_err()
    );
    cold_corruption(f, &cold, &opened, &closed)?;
    // OwnedDispatch is an actual owner-licence claim, never a paid-seat alias.
    let mut owned = actual.reboot(f, &seeded)?;
    for hull in owned.hulls {
        owned.command(&format!("@asset/transfer #{}=#{}", hull.0, GOD.0))?;
    }
    let owner = owned.ticket(GOD, first, 1)?;
    owned.land(&owner, 12_000_011, first, false)?;
    let claim = one(rows(&owned.disk, "roll", "roll_mind_ic_claim"))?;
    check_eq!(
        claim.get("open").and_then(|o| o.get("capacity")),
        Some(&json!({"kind":"owned_dispatch"}))
    );
    chartered_owner(f, &mut owned, first)?;
    eprintln!("M7_STORY phase=ic_composed event=complete");
    Ok(())
}

fn chartered_owner(f: &PurchaseFixture, owned: &mut IcWorld, hull: ObjId) -> TestResult {
    const SALE: u128 = 48_300_000;
    let birth = one(rows(&owned.disk, "roll", "roll_mind_initial_filing"))?;
    let charter: mush_roll::commerce::CharterId =
        serde_json::from_value(birth.get("charter").cloned().ok_or("actual charter")?)?;
    // Actual staff command cannot bypass the permanent managed title history.
    check!(
        crate::test_checks::refused(owned.command(&format!(
            "@asset/transfer #{}=#{}",
            charter.get(),
            BUYER.0
        )))?,
        "staff transfer bypassed the managed title"
    );
    check_eq!(
        owned
            .engine
            .world
            .module_state::<EconomyStore>()
            .ok_or("actual Economy")?
            .snapshot()
            .asset(charter.object())
            .ok_or("actual managed title")?
            .title_owner,
        EconomicOwner::Object(GOD)
    );
    let money = owned
        .engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("actual Economy")?
        .snapshot()
        .account(WalletId(0))
        .ok_or("seller wallet")?
        .posted_balance;
    let (engine, disk) = (&mut owned.engine, &mut owned.disk);
    check!(
        charter_transfer_fixture::sale(engine, charter.object(), BUYER, SALE, |writes| {
            retain_writes(disk, writes)
        })
        .is_err(),
        "open OwnedDispatch admitted immediate transfer"
    );
    check_eq!(
        owned
            .engine
            .world
            .module_state::<EconomyStore>()
            .ok_or("actual Economy")?
            .snapshot()
            .account(WalletId(0))
            .ok_or("seller wallet")?
            .posted_balance,
        money
    );
    check!(owned.held(hull)?);
    let event = ic_sortie::fixture_close(&owned.engine.world, GOD, token(12_000_011), owned.base)?;
    let close = owned.host.enqueue_close(&event)?;
    owned.land(&close, SALE + 2, hull, true)?;
    let old_claim = one(rows(&owned.disk, "roll", "roll_mind_ic_claim"))?;
    check_eq!(old_claim.get("format"), Some(&json!(1)));
    check!(old_claim.pointer("/open/charter_ownership").is_none());
    let (engine, disk) = (&mut owned.engine, &mut owned.disk);
    charter_transfer_fixture::sale(engine, charter.object(), BUYER, SALE + 3, |writes| {
        retain_writes(disk, writes)
    })?;
    owned.command(&format!("@asset/transfer #{}=#{}", hull.0, BUYER.0))?;
    let open = owned.ticket(BUYER, hull, 1)?;
    owned.land(&open, SALE + 5, hull, false)?;
    let captured = rows(&owned.disk, "roll", "roll_mind_ic_claim")
        .into_iter()
        .find(|v| v.pointer("/open/operation") == Some(&json!(token(SALE + 5))))
        .ok_or("actual transferred-owner claim")?;
    check_eq!(captured.get("format"), Some(&json!(2)));
    check_eq!(captured.pointer("/open/dispatcher"), Some(&json!(BUYER)));
    check_eq!(
        captured.pointer("/open/charter_ownership/generation"),
        Some(&json!(2))
    );
    let opened = owned.snapshot();
    let mut cold = owned.reboot(f, &opened)?;
    let resumed = cold.host.resume(&cold.engine.world, token(SALE + 5))?;
    let seal = resumed.seal().clone();
    let term: mush_roll::commerce::records::OwnerTerm =
        serde_json::from_value(one(rows(&cold.disk, "roll", "roll_mind_owner_term"))?)?;
    cold.engine.world.set_now(term.window().grace_end().get());
    check_eq!(
        cold.host
            .resume(&cold.engine.world, token(SALE + 5))?
            .seal(),
        &seal
    );
    let event = ic_sortie::fixture_close(&cold.engine.world, BUYER, token(SALE + 5), cold.base)?;
    let close = cold.host.enqueue_close(&event)?;
    cold.land(&close, SALE + 6, hull, true)?;
    let (engine, disk) = (&mut cold.engine, &mut cold.disk);
    charter_transfer_fixture::sale(engine, charter.object(), GOD, SALE + 7, |writes| {
        retain_writes(disk, writes)
    })?;
    let final_world = cold.reboot(f, &cold.snapshot())?;
    check!(!final_world.held(hull)?);
    let claims = rows(&final_world.disk, "roll", "roll_mind_ic_claim");
    check!(claims.contains(&old_claim));
    let final_claim = claims
        .iter()
        .find(|v| v.pointer("/open/operation") == Some(&json!(token(SALE + 5))))
        .ok_or("retained actual transferred claim")?;
    check_eq!(final_claim.get("open"), captured.get("open"));
    check_eq!(final_claim.get("format"), Some(&json!(2)));
    Ok(())
}

fn arena_for(
    actual: &IcWorld,
    seal: &crate::arena::MindSeal,
) -> TestResult<Box<crate::arena::Arena>> {
    use crate::arena::{Arena, Controller, CustodyOut, CustodyUnit, MatchMode, PartyId, test_support};
    let unit = mush_tactics::module::unit_of(&actual.engine.world, seal.unit)
        .ok_or("physical unit")?
        .clone();
    let store = actual
        .engine
        .world
        .module_state::<TacsimStore>()
        .ok_or("catalog")?;
    let catalog = store.definitions();
    let custody = CustodyUnit {
        id: seal.unit,
        team: PartyId::RED,
        pose: mush_sim::Pose::at(2000, 2000, 0, test_support::rules()),
        mobility: crate::match_::tracked_rules(&unit),
        sensor_height_cm: crate::match_::sensor_height_cm(catalog, &unit),
        sensor: crate::match_::sensor_profile(catalog, &unit),
        weapons: crate::match_::weapon_summaries(catalog, &unit),
        profile: crate::match_::unit_profile(catalog, &unit),
        gunnery: 1,
        controller: Controller::Mind,
        slot: 0,
        seal: seal.clone(),
        unit,
    };
    Ok(Box::new(Arena::new(
        CustodyOut {
            definition: test_support::definition(),
            units: vec![
                custody,
                test_support::tank(
                    ObjId(u32::MAX - 1),
                    PartyId::BLUE,
                    mush_sim::Pose::at(6000, 6000, 0, test_support::rules()),
                ),
            ],
            pilots: vec![],
            resolution: test_support::resolution(),
            seed: 9,
            mode: MatchMode::Skirmish,
            host: crate::arena::FieldHost::all_hostile(),
        },
        &crate::config::ArenaConfig::default(),
        &crate::config::MatchConfig::default(),
        &crate::config::SensorsConfig::default(),
    )?))
}
fn cold_corruption(
    f: &PurchaseFixture,
    actual: &IcWorld,
    opened: &Durable,
    closed: &Durable,
) -> TestResult {
    for owner in ["roll", "tacsim", "tactics", "sim"] {
        let mut missing = opened.clone();
        missing.disk.retain(|(ns, key), value| {
            !(ns == owner
                && (key.starts_with("ic_sortie")
                    || key.starts_with("ic_custody")
                    || key.starts_with("ic_exclusion")
                    || value
                        .get("family")
                        .and_then(|v| v.get("id"))
                        .and_then(Value::as_str)
                        == Some("roll_mind_ic_claim")))
        });
        check!(
            crate::test_checks::refused(actual.reboot(f, &missing))?,
            "missing actual owner inventory must refuse"
        );
    }
    check!(crate::test_checks::refused(IcWorld::boot(
        f,
        opened,
        actual.base,
        actual.field,
        actual.hulls,
        false
    ))?);
    let mut crossed = closed.clone();
    let row = crossed
        .disk
        .iter_mut()
        .find(|((ns, key), _)| ns == "tactics" && key.starts_with("ic_custody:"))
        .ok_or("actual Tactics closed history")?
        .1;
    let value = row
        .get_mut("closed")
        .and_then(Value::as_object_mut)
        .ok_or("actual terminal")?;
    *value
        .get_mut("unit_digest")
        .ok_or("actual terminal unit digest")? =
        json!(mush_core::DefinitionDigest::from_bytes([8; 32]));
    check!(
        crate::test_checks::refused(actual.reboot(f, &crossed))?,
        "one-owner final unit digest corruption must refuse"
    );
    Ok(())
}

fn expiry_fences(f: &PurchaseFixture, actual: &IcWorld, seeded: &Durable) -> TestResult {
    let term: PaidTermRecord =
        serde_json::from_value(one(rows(&seeded.disk, "roll", "roll_mind_paid_term"))?)?;
    for after_stage in [false, true] {
        let mut branch = actual.reboot(f, seeded)?;
        let hull = *branch.hulls.first().ok_or("hull")?;
        let ticket = branch.ticket(BUYER, hull, 2)?;
        let before = branch.disk.clone();
        let (p, o, inv) = branch.prepare(&ticket, 12_000_020)?;
        if after_stage {
            branch.stage(&ticket, 12_000_020, p, o, &inv)?;
            branch.engine.world.set_now(term.window().grace_end().get());
            check!(
                branch.engine.take_staged_action().is_err(),
                "actual pre-custody expiry fence"
            );
        } else {
            branch.engine.world.set_now(term.window().grace_end().get());
            check!(
                branch.stage(&ticket, 12_000_020, p, o, &inv).is_err(),
                "actual prepare-to-stage expiry fence"
            );
        }
        check!(!branch.held(hull)?);
        check!(
            branch.disk == before,
            "expiry refusal changed private durable state"
        );
        check!(
            branch
                .host
                .acknowledge(&branch.engine.world, &ticket)?
                .is_none()
        );
    }
    Ok(())
}

fn submit_pulse(branch: &mut IcWorld, actor: ObjId) {
    branch.engine.step(
        Inbound::Cmd {
            conn: if actor == BUYER { BUYER_CONN } else { CONN },
            actor,
            line: "@ic-host-fixture".into(),
        },
        &mut Outbox::new(),
        &mut mush_core::Effects::new(),
    );
}

fn observed_submission(f: &PurchaseFixture, actual: &IcWorld, seeded: &Durable) -> TestResult {
    use crate::IcSubmissionState;
    let mut absent = actual.reboot(f, seeded)?;
    let hull = *absent.hulls.first().ok_or("hull")?;
    let permit = ic_sortie::fixture_start(&absent.engine.world, BUYER, hull)?;
    let ticket = Arc::new(
        absent
            .host
            .enqueue_start(&permit, LicenceId::new(2)?, vec![])?,
    );
    *absent.selected.lock().map_err(module_error)? = Some(ticket.clone());
    submit_pulse(&mut absent, GOD);
    check_eq!(
        absent.host.submission_state(&ticket)?,
        IcSubmissionState::Unsubmitted
    );
    check!(
        absent
            .host
            .discard_unadmitted_open(&absent.engine.world, &ticket)
            .is_err()
    );
    // The production server installs its submission runtime before any
    // authoritative ingress, so a queued observed attempt is always driven; a
    // definitive disposition only ever comes from that runtime.
    absent.engine.set_submission_runtime(Arc::new(Runtime))?;
    submit_pulse(&mut absent, BUYER);
    let driven = absent
        .engine
        .take_observed_authoritative_submit()
        .ok_or("actual host selector")?;
    let (request, driver) = driven.into_parts();
    check_eq!(request.actor, BUYER);
    check_eq!(&request.action, ticket.action());
    check_eq!(request.parameters, ticket.parameters());
    check_eq!(request.subject, PerceivedRef::SelfActor);
    check!(request.targets.is_empty());
    let proof = *driver
        .ok_or("runtime observation")?
        .downcast::<Arc<std::sync::atomic::AtomicBool>>()
        .map_err(|_| "proof type")?;
    for _ in 0..3 {
        submit_pulse(&mut absent, BUYER);
        check!(absent.engine.take_observed_authoritative_submit().is_none());
    }
    check_eq!(
        absent.host.submission_state(&ticket)?,
        IcSubmissionState::AwaitingAcknowledgement
    );
    check!(
        absent
            .host
            .acknowledge(&absent.engine.world, &ticket)?
            .is_none()
    );
    check!(!absent.held(hull)?);
    check!(
        absent
            .host
            .discard_unadmitted_open(&absent.engine.world, &ticket)
            .is_err()
    );
    // Consumer-only disposition fixture. The server's actual absence proof is
    // tested separately; this cell cannot create any IC owner history.
    proof.store(true, std::sync::atomic::Ordering::Release);
    check_eq!(
        absent.host.submission_state(&ticket)?,
        IcSubmissionState::Retryable
    );
    // Missing actual boot/audit never proves a row absent or releases a permit.
    let unaudited = World::new_minimal();
    check!(
        absent
            .host
            .discard_unadmitted_open(&unaudited, &ticket)
            .is_err()
    );
    let mut released = absent
        .host
        .discard_unadmitted_open(&absent.engine.world, &ticket)?;
    check!(Arc::ptr_eq(released.permit(), &permit));
    check_eq!(released.ticket().parameters(), ticket.parameters());
    check!(absent.host.submission_state(&ticket).is_err());
    check!(
        absent
            .host
            .restore_unadmitted_open(&unaudited, &mut released)
            .is_err()
    );
    let foreign = actual.reboot(f, seeded)?;
    check!(
        foreign
            .host
            .restore_unadmitted_open(&foreign.engine.world, &mut released)
            .is_err()
    );
    let restored = absent
        .host
        .restore_unadmitted_open(&absent.engine.world, &mut released)?;
    check_eq!(restored.actor(), ticket.actor());
    check_eq!(restored.action(), ticket.action());
    check_eq!(restored.parameters(), request.parameters);
    check!(
        absent
            .host
            .restore_unadmitted_open(&absent.engine.world, &mut released)
            .is_err()
    );
    submit_pulse(&mut absent, BUYER);
    let next = absent
        .engine
        .take_observed_authoritative_submit()
        .ok_or("same exact host retry")?;
    let (retry, driver) = next.into_parts();
    check_eq!(retry.actor, request.actor);
    check_eq!(retry.action, request.action);
    check_eq!(retry.parameters, request.parameters);
    check_eq!(retry.key, request.key);
    let unknown = *driver
        .ok_or("retry observation")?
        .downcast::<Arc<std::sync::atomic::AtomicBool>>()
        .map_err(|_| "retry proof type")?;
    check!(!unknown.load(std::sync::atomic::Ordering::Acquire));
    // Real signed offer, full paired prepare/stage/take/install, then readback.
    // No runtime success bit is minted to complete this retained intent.
    absent.land_check(
        &ticket,
        12_000_030,
        hull,
        false,
        Some(retry.key.clone()),
        |branch, ticket| {
            // Deliberately contradictory consumer proof cannot defeat actual
            // installed readback; the original capability remains for normal ACK.
            unknown.store(true, std::sync::atomic::Ordering::Release);
            check_eq!(
                branch.host.submission_state(ticket)?,
                IcSubmissionState::Retryable
            );
            check!(
                branch
                    .host
                    .discard_unadmitted_open(&branch.engine.world, ticket)
                    .is_err()
            );
            check_eq!(
                branch.host.submission_state(ticket)?,
                IcSubmissionState::Retryable
            );
            Ok(())
        },
    )?;
    check!(absent.host.submission_state(&ticket).is_err());
    // Even definitive non-dispatch never discards an actual Close obligation.
    let event =
        ic_sortie::fixture_close(&absent.engine.world, BUYER, token(12_000_030), absent.base)?;
    let close = Arc::new(absent.host.enqueue_close(&event)?);
    *absent.selected.lock().map_err(module_error)? = Some(close.clone());
    submit_pulse(&mut absent, BUYER);
    let pending = absent
        .engine
        .take_observed_authoritative_submit()
        .ok_or("actual Close selector")?;
    let (close_request, driver) = pending.into_parts();
    let close_proof = *driver
        .ok_or("Close observation")?
        .downcast::<Arc<std::sync::atomic::AtomicBool>>()
        .map_err(|_| "Close proof type")?;
    close_proof.store(true, std::sync::atomic::Ordering::Release);
    check_eq!(
        absent.host.submission_state(&close)?,
        IcSubmissionState::Retryable
    );
    check!(
        absent
            .host
            .discard_unadmitted_open(&absent.engine.world, &close)
            .is_err()
    );
    check!(absent.held(hull)?);
    absent.land_key(&close, 12_000_031, hull, true, Some(close_request.key))?;
    Ok(())
}

#[path = "charter_pending_integration_tests.rs"]
mod pending;

#[path = "underwriting_integration_tests.rs"]
mod coverage;

#[path = "paid_resale_integration_tests.rs"]
mod resale;

pub(super) fn exercise_house_two_seats(f: &Fixture, saved: &Durable) -> TestResult {
    resale::two_seat::exercise(f, saved)
}

#[inline(never)]
fn boxed_fixture_engine(
    world: mush_core::World,
    registry: mush_core::Registry,
    modules: Vec<Box<dyn mush_core::GameModule>>,
) -> Result<Box<mush_core::Engine>, mush_core::ModError> {
    mush_core::Engine::with_modules(world, registry, modules).map(Box::new)
}
