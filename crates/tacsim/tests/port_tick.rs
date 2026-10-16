//! **Doc 18 cut IC5: the port tick and tacsim's authoritative plumbing**
//! (§4.3, §4.4 "The queued-orders model" and "Offline execution", §4.8; D-18-32,
//! D-18-33).
//!
//! Every later port arm — IC8's stock clerk, IC9's launch and dock, IC11's order
//! queue — acts from one Coarse tick, `tacsim.port`, as a principal that is not
//! GOD. This file proves the machinery they stand on, on the production roster
//! composed through the composition root (`server_modules_with_tacsim` and the
//! wired layer), with only the test world's probe action standing in for an arm:
//!
//! 1. an agent's submission from the tick **lands**, as its principal — a player
//!    or (IC8's clerk shape) a Thing;
//! 2. two agents in one fire never void each other (D-18-32: at most one
//!    submission per actor per fire), agents sharing a principal take turns, and
//!    no agent starves when a budget bites;
//! 3. **GOD never submits**: a GOD or gone principal is refused and stages
//!    nothing, and no tacsim-owned submission in any test carries actor `#1`;
//! 4. the tick reads an inventory pool through its own `Ctx::action_state`
//!    because `mush-inventory` names `tacsim` its third quantity delegate
//!    (D-18-33), and an unwired layer's read refuses rather than reading zero;
//! 5. the budgets bound a fire, a disabled layer fires nothing, and production
//!    registers no probe;
//! 6. reconnect notices survive a kept-world reboot and render exactly once.

use std::collections::HashMap;
use std::sync::Arc;

use mush_core::{
    Audience, CommitToken, GOD, IdempotencyKey, IdempotencyScope, LIMBO, ModuleOp, ModuleWrite,
    ObjId, ObjKind, OfferExecutionContext, OfferSubmission, RequestUid, ScopedIdempotencyKey,
};
use mush_inventory::{EndpointRef, Grade, InventoryHandles, LotId};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, fold_module_ops, text_of};
use smol_str::SmolStr;
use tacsim::TacsimConfig;
use tacsim::port::PortHandles;
use tacsim::port_tick::{
    NOTICE_HEADING, PROBE_ACTION, ProbeAgent, ProbeKey, ProbeMark, enlist_probe, last_fire,
    notice_key, pending_notices,
};

// ---------------------------------------------------------------------------
// The rig
// ---------------------------------------------------------------------------

const FAN_OUT: &str = "tried to submit more than one";

fn config() -> TacsimConfig {
    TacsimConfig {
        enabled: true,
        ..TacsimConfig::default()
    }
}

/// The production roster (`mush_server::boot::game_modules`' shape: the
/// built-ins, worldgen, npc, the sheet, the layer, the content module last),
/// composed through `server_modules_with_tacsim` so `tacsim` is the third
/// quantity delegate and the layer holds the inventory cell. Production couples
/// `game_content` and the content lane to `[tacsim].enabled`; so does the rig.
fn wired(test_world: bool, cfg: TacsimConfig) -> Vec<Box<dyn mush_core::GameModule>> {
    use tacsim::composition as comp;

    let llm = Arc::new(
        mush_llm::LlmService::from_config(&mush_llm::LlmConfig::default())
            .expect("a disabled LLM service composes"),
    );
    let onboarding = mush_core::OnboardingConfig::default();
    let game = tacsim::GameEconomyConfig::default();
    let (mut mods, cell) = comp::server_modules_with_tacsim(
        mush_weather::WeatherConfig::default(),
        mush_economy::EconomyConfig::default(),
        mush_atlas::AtlasConfig::default(),
        &onboarding,
        &game,
        comp::recovery_policy(&cfg.recovery),
    );
    mods.push(comp::worldgen_module_with(
        Arc::clone(&llm),
        mush_worldgen::WorldgenConfig::default(),
    ));
    mods.push(comp::npc_module_with(
        comp::npc_agent_adapter(),
        &mush_config::config::NpcCfg::default(),
    ));
    mods.extend(comp::sheet_modules(&onboarding, &game));
    let enabled = cfg.enabled;
    if test_world {
        mods.push(comp::tacsim_test_world_module_wired(cfg, enabled, cell));
    } else {
        mods.push(comp::tacsim_module_wired(cfg, enabled, cell));
    }
    mods.push(comp::content_module_with(true, enabled).expect("the content module composes"));
    mods
}

/// The same roster through `server_modules_with` and the unwired test-world
/// layer: no delegation to `tacsim` and no cell.
fn unwired(cfg: TacsimConfig) -> Vec<Box<dyn mush_core::GameModule>> {
    use tacsim::composition as comp;

    let llm = Arc::new(
        mush_llm::LlmService::from_config(&mush_llm::LlmConfig::default())
            .expect("a disabled LLM service composes"),
    );
    let onboarding = mush_core::OnboardingConfig::default();
    let game = tacsim::GameEconomyConfig::default();
    let mut mods = comp::server_modules_with(
        mush_weather::WeatherConfig::default(),
        mush_economy::EconomyConfig::default(),
        mush_atlas::AtlasConfig::default(),
        &onboarding,
        &game,
        comp::recovery_policy(&cfg.recovery),
    );
    mods.push(comp::worldgen_module_with(
        Arc::clone(&llm),
        mush_worldgen::WorldgenConfig::default(),
    ));
    mods.push(comp::npc_module_with(
        comp::npc_agent_adapter(),
        &mush_config::config::NpcCfg::default(),
    ));
    mods.extend(comp::sheet_modules(&onboarding, &game));
    let enabled = cfg.enabled;
    mods.push(comp::tacsim_test_world_module(cfg, enabled));
    mods.push(comp::content_module_with(true, enabled).expect("the content module composes"));
    mods
}

fn boot(modules: Vec<Box<dyn mush_core::GameModule>>) -> TestWorld {
    with_offer_keys(TestWorld::with_modules(modules))
}

/// The durable offer-key ring a legal-action offer is signed with — the world
/// task installs it at boot; a rig that lands submissions needs one too
/// (`ic0.rs`'s rig).
fn with_offer_keys(mut tw: TestWorld) -> TestWorld {
    tw.engine_mut().set_offer_keys(
        mush_core::OfferKeyRing::new(
            mush_core::OfferSecret::new(1, [0x41; 32]).expect("an offer secret"),
            [],
        )
        .expect("an offer key ring"),
    );
    tw
}

/// A player made in Limbo and **never connected** (`spawn_player` connects).
fn offline_player(tw: &mut TestWorld, name: &str) -> ObjId {
    tw.world_mut()
        .create(ObjKind::Player, name, GOD, Some(LIMBO))
        .expect("an offline player")
}

fn thing(tw: &mut TestWorld, name: &str) -> ObjId {
    tw.world_mut()
        .create(ObjKind::Thing, name, GOD, Some(LIMBO))
        .expect("a thing")
}

fn enlist(tw: &mut TestWorld, agent: ObjId, principal: ObjId, remaining: u32) {
    enlist_probe(
        tw.world_mut(),
        agent,
        ProbeAgent {
            principal,
            pool: None,
            remaining,
            next_issue: 0,
        },
    )
    .expect("a test world enlists the probe");
}

/// One port-tick fire: the clock moves past a whole 5 s period (the campaign
/// `common/mod.rs` helper's shape).
fn fire(tw: &mut TestWorld) -> Vec<(Audience, OutputEvent)> {
    let t = tw.world().now() + 6;
    tw.set_now(t);
    tw.fire_due(t.saturating_mul(1_000))
}

/// Every queued submission, keeping tacsim's and dropping the rest (other
/// modules' ticks may queue their own); asserts none kept is GOD's.
fn drain_tacsim(tw: &mut TestWorld) -> Vec<mush_core::AuthoritativeSubmit> {
    let mut kept = Vec::new();
    while let Some(submit) = tw.engine_mut().take_authoritative_submit() {
        if submit.action.key().owner.as_str() == tacsim::NS {
            assert_ne!(submit.actor, GOD, "GOD never submits from the port tick");
            kept.push(*submit);
        }
    }
    kept
}

fn fanned_out(events: &[(Audience, OutputEvent)]) -> bool {
    events.iter().any(|(_, ev)| text_of(ev).contains(FAN_OUT))
}

/// A fresh commit token per landing.
struct Tokens(u8);

impl Tokens {
    fn next(&mut self) -> u8 {
        self.0 = self
            .0
            .checked_add(1)
            .expect("fewer than 256 landings per test");
        self.0
    }
}

/// Drive one queued submission the way the world task does (`mush-market/tests/
/// ic0.rs`'s `land`, taking the submission instead of popping it).
fn land_one(
    tw: &mut TestWorld,
    submit: &mush_core::AuthoritativeSubmit,
    token: u8,
) -> Result<(), String> {
    let (action, resolved) = {
        let engine = tw.engine();
        let query = engine
            .legal_action_query(
                mush_core::PrincipalId::Object { id: submit.actor },
                submit.actor,
                submit.subject.clone(),
            )
            .map_err(|error| error.to_string())?;
        let offers = engine
            .project_legal_actions(&query)
            .map_err(|error| error.to_string())?;
        let projected = offers
            .projected_for_action(&submit.action)
            .ok_or_else(|| "that action is not currently offered".to_owned())?;
        let submission = OfferSubmission {
            offer: projected.offer(),
            subject: projected.subject().clone(),
            targets: Vec::new(),
            parameters: submit.parameters.clone(),
        };
        let execution = OfferExecutionContext {
            request: RequestUid::from_bytes([token; 16]),
            principal: mush_core::PrincipalId::Object { id: submit.actor },
            actor: submit.actor,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal: mush_core::PrincipalId::Object { id: submit.actor },
                    owner: submit.action.key().owner.clone(),
                    namespace: mush_core::CanonicalId::new(submit.action.key().id.as_str())
                        .map_err(|_| "namespace".to_owned())?,
                },
                1,
                IdempotencyKey::Request(RequestUid::from_bytes([token; 16])),
            ),
            viewer_scope: query.viewer_scope(),
            now: None,
        };
        let resolved = engine
            .resolve_legal_action(&offers, execution, submission)
            .map_err(|error| format!("{error:?}"))?;
        let (action, resolved, _guards) = resolved.into_authoritative_parts();
        (action, resolved)
    };
    let (prepared, _) = tw
        .engine_mut()
        .prepare_registered_resolved_authoritative_action(
            &action,
            &resolved,
            CommitToken::from_bytes([token; 16]),
        )
        .map_err(|error| format!("{error:?}"))?;
    tw.engine_mut()
        .install_prepared_action(prepared.into_parts().1)
        .map_err(|error| format!("{error:?}"))?;
    Ok(())
}

fn land_all(tw: &mut TestWorld, submits: &[mush_core::AuthoritativeSubmit], t: &mut Tokens) {
    for submit in submits {
        land_one(tw, submit, t.next()).expect("the probe lands");
    }
}

fn handles(tw: &TestWorld) -> PortHandles {
    tw.world()
        .module_state::<PortHandles>()
        .cloned()
        .expect("the port handles are published")
}

/// The probe mark at `(agent, issue)`, read through an owner-stamped
/// transaction (`ic0.rs`'s `read`).
fn mark(tw: &mut TestWorld, agent: ObjId, issue: u32) -> Option<ProbeMark> {
    let probe = handles(tw).probe.expect("a test world's probe family");
    let mut txn = tw
        .engine_mut()
        .action_transaction(&probe)
        .expect("a tacsim transaction");
    txn.get(&probe, &ProbeKey { agent, issue })
        .expect("the probe family reads")
        .cloned()
}

fn stats(tw: &TestWorld) -> tacsim::port_tick::PortFireStats {
    last_fire(tw.world()).expect("the tick fired")
}

fn sets_notice(ops: &[ModuleOp], who: ObjId) -> bool {
    ops.iter().any(|op| {
        op.ns == tacsim::NS
            && matches!(&op.op, ModuleWrite::JsonSet { key, .. } if *key == notice_key(who))
    })
}

fn deletes_notice(ops: &[ModuleOp], who: ObjId) -> bool {
    ops.iter().any(|op| {
        op.ns == tacsim::NS && matches!(&op.op, ModuleWrite::Del { key } if *key == notice_key(who))
    })
}

// ---------------------------------------------------------------------------
// 1-2. A submission from the tick lands, as its principal
// ---------------------------------------------------------------------------

#[test]
fn ic5_a_probe_submitted_from_the_tick_lands() {
    let mut tw = boot(wired(true, config()));
    let mut t = Tokens(0);
    let agent = thing(&mut tw, "probe one");
    let pilot = tw.spawn_player("Pilot");
    enlist(&mut tw, agent, pilot, 1);
    let events = fire(&mut tw);
    let queued = drain_tacsim(&mut tw);
    assert_eq!(queued.len(), 1, "one fire queues exactly one submission");
    assert_eq!(
        queued[0].actor, pilot,
        "its actor is the principal, not GOD"
    );
    assert_eq!(queued[0].action.key().id.as_str(), PROBE_ACTION);
    assert!(!fanned_out(&events));
    let s = stats(&tw);
    assert_eq!((s.visited, s.worked, s.submitted), (1, 1, 1));
    land_all(&mut tw, &queued, &mut t);
    assert_eq!(
        mark(&mut tw, agent, 0),
        Some(ProbeMark {
            agent,
            principal: pilot,
            issue: 0,
            seen: 0
        })
    );
    let _ = fire(&mut tw);
    assert!(drain_tacsim(&mut tw).is_empty(), "nothing more is due");
    assert_eq!(stats(&tw).submitted, 0);
}

#[test]
fn ic5_a_thing_principal_lands() {
    let mut tw = boot(wired(true, config()));
    let mut t = Tokens(0);
    // IC8's clerk acts as itself: the agent is its own Thing principal.
    let clerk = thing(&mut tw, "stock clerk");
    enlist(&mut tw, clerk, clerk, 1);
    let _ = tw.take_module_ops();
    let _ = fire(&mut tw);
    let ops = tw.take_module_ops();
    let queued = drain_tacsim(&mut tw);
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].actor, clerk);
    land_all(&mut tw, &queued, &mut t);
    assert_eq!(mark(&mut tw, clerk, 0).map(|m| m.principal), Some(clerk));
    assert!(
        !sets_notice(&ops, clerk),
        "a Thing principal never connects, so nothing is queued for it"
    );
    assert!(pending_notices(tw.world(), clerk).is_empty());
}

#[test]
fn ic5_two_agents_in_one_fire_do_not_void_each_other() {
    let mut tw = boot(wired(true, config()));
    let mut t = Tokens(0);
    let (a, b) = (thing(&mut tw, "probe a"), thing(&mut tw, "probe b"));
    let (pa, pb) = (tw.spawn_player("Alpha"), tw.spawn_player("Bravo"));
    enlist(&mut tw, a, pa, 1);
    enlist(&mut tw, b, pb, 1);
    let events = fire(&mut tw);
    assert!(!fanned_out(&events), "no fan-out line in the fire");
    let queued = drain_tacsim(&mut tw);
    assert_eq!(queued.len(), 2);
    land_all(&mut tw, &queued, &mut t);
    assert!(mark(&mut tw, a, 0).is_some());
    assert!(mark(&mut tw, b, 0).is_some());
}

#[test]
fn ic5_one_actor_acts_once_per_fire_and_agents_sharing_it_alternate() {
    let mut tw = boot(wired(true, config()));
    let mut t = Tokens(0);
    let lo = thing(&mut tw, "probe lo");
    let hi = thing(&mut tw, "probe hi");
    assert!(lo < hi);
    let pilot = tw.spawn_player("Pilot");
    enlist(&mut tw, lo, pilot, 3);
    enlist(&mut tw, hi, pilot, 3);
    let mut who = Vec::new();
    for _ in 0..3 {
        let events = fire(&mut tw);
        assert!(!fanned_out(&events), "no fan-out line in any fire");
        let queued = drain_tacsim(&mut tw);
        assert_eq!(queued.len(), 1, "one submission per principal per fire");
        assert_eq!(stats(&tw).deferred, 1);
        who.push(queued[0].parameters["agent"].as_u64());
        land_all(&mut tw, &queued, &mut t);
    }
    assert_eq!(
        who,
        vec![
            Some(u64::from(lo.0)),
            Some(u64::from(hi.0)),
            Some(u64::from(lo.0))
        ]
    );
    assert!(mark(&mut tw, lo, 0).is_some());
    assert!(mark(&mut tw, lo, 1).is_some());
    assert!(mark(&mut tw, hi, 0).is_some());
    assert!(mark(&mut tw, hi, 1).is_none());
}

// ---------------------------------------------------------------------------
// 3. GOD never submits
// ---------------------------------------------------------------------------

#[test]
fn ic5_god_never_submits() {
    let mut tw = boot(wired(true, config()));
    let agent = thing(&mut tw, "probe god");
    enlist(&mut tw, agent, GOD, 1);
    for _ in 0..2 {
        let _ = fire(&mut tw);
        assert!(drain_tacsim(&mut tw).is_empty());
        let s = stats(&tw);
        assert_eq!((s.refused, s.submitted), (1, 0));
    }
    // A destroyed principal is refused the same way.
    let mut tw = boot(wired(true, config()));
    let agent = thing(&mut tw, "probe gone");
    let principal = thing(&mut tw, "gone principal");
    tw.world_mut().destroy(principal).expect("destroyed");
    enlist(&mut tw, agent, principal, 1);
    for _ in 0..2 {
        let _ = fire(&mut tw);
        assert!(drain_tacsim(&mut tw).is_empty());
        let s = stats(&tw);
        assert_eq!((s.refused, s.submitted), (1, 0));
    }
}

// ---------------------------------------------------------------------------
// 4. The delegation: pools read through `action_state`
// ---------------------------------------------------------------------------

#[test]
fn ic5_the_tick_reads_an_inventory_pool_through_action_state() {
    let mut tw = boot(wired(true, config()));
    let mut t = Tokens(0);
    assert!(
        handles(&tw).inventory.is_some(),
        "the wired layer holds the cell"
    );
    let pilot = tw.spawn_player("Pilot");
    let chest = thing(&mut tw, "strongbox");
    let line = format!(
        "@vault/create strongbox=container:#{} owner=#{}",
        chest.0, pilot.0
    );
    let said: Vec<String> = tw
        .run(GOD, &line)
        .iter()
        .map(|(_, ev)| text_of(ev))
        .collect();
    let vault = tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .expect("the economy")
        .snapshot()
        .vaults()
        .find(|v| v.owner == mush_org::PrincipalId::Object(pilot))
        .map(|v| v.id)
        .unwrap_or_else(|| panic!("the vault exists: {said:?}"));
    let endpoint = EndpointRef(vault);
    let iron = mush_core::CanonicalId::new("iron").expect("id");
    let families = tw
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone();
    // `ic0.rs`'s `seed_rooted_pool`: mint an origin lot, dissolve it into the pool.
    for step in 0..2 {
        let engine = tw.engine_mut();
        let prepared = {
            let mut txn = engine
                .action_transaction(&families.pools)
                .expect("inventory transaction");
            if step == 0 {
                families
                    .mint_origin(
                        &mut txn,
                        endpoint,
                        &iron,
                        42,
                        Grade::new(2).expect("grade"),
                        mush_economy::ExternalRef {
                            provider: "test".into(),
                            kind: "seed".into(),
                            id: "1".into(),
                            revision: None,
                        },
                        LotId(1),
                    )
                    .map(|_| ())
                    .expect("seed mints one origin lot");
            } else {
                families
                    .dissolve(&mut txn, endpoint, LotId(1))
                    .map(|_| ())
                    .expect("the seed lot dissolves");
            }
            txn.prepare().expect("prepares")
        };
        engine
            .install_prepared_action(prepared.into_parts().1)
            .expect("installs");
    }
    let agent = thing(&mut tw, "probe reader");
    enlist_probe(
        tw.world_mut(),
        agent,
        ProbeAgent {
            principal: pilot,
            pool: Some((endpoint, iron)),
            remaining: 1,
            next_issue: 0,
        },
    )
    .expect("enlisted");
    let _ = fire(&mut tw);
    let queued = drain_tacsim(&mut tw);
    assert_eq!(queued.len(), 1, "{:?}", last_fire(tw.world()));
    land_all(&mut tw, &queued, &mut t);
    assert_eq!(mark(&mut tw, agent, 0).map(|m| m.seen), Some(42));
}

#[test]
fn ic5_an_unwired_layer_refuses_the_read_and_submits_nothing() {
    let mut tw = boot(unwired(config()));
    assert!(
        handles(&tw).inventory.is_none(),
        "the unwired layer holds no cell"
    );
    let pilot = tw.spawn_player("Pilot");
    let agent = thing(&mut tw, "probe reader");
    enlist_probe(
        tw.world_mut(),
        agent,
        ProbeAgent {
            principal: pilot,
            pool: Some((
                EndpointRef(mush_economy::VaultId(1)),
                mush_core::CanonicalId::new("iron").expect("id"),
            )),
            remaining: 1,
            next_issue: 0,
        },
    )
    .expect("enlisted");
    let _ = fire(&mut tw);
    assert!(drain_tacsim(&mut tw).is_empty());
    let s = stats(&tw);
    assert_eq!((s.refused, s.submitted), (1, 0));
    assert!(mark(&mut tw, agent, 0).is_none());
}

// ---------------------------------------------------------------------------
// 5. Budgets, the disabled layer, production
// ---------------------------------------------------------------------------

#[test]
fn ic5_budgets_bound_a_fire() {
    // (a) 20 agents, each its own principal.
    let mut tw = boot(wired(true, config()));
    let mut t = Tokens(0);
    let agents: Vec<ObjId> = (0..20).map(|n| thing(&mut tw, &format!("a{n}"))).collect();
    for (n, agent) in agents.iter().enumerate() {
        let pilot = tw.spawn_player(&format!("Pilot{n}"));
        enlist(&mut tw, *agent, pilot, 1);
    }
    let _ = fire(&mut tw);
    let s = stats(&tw);
    assert_eq!((s.visited, s.worked, s.submitted), (16, 16, 16));
    let queued = drain_tacsim(&mut tw);
    assert_eq!(queued.len(), 16);
    land_all(&mut tw, &queued, &mut t);
    let _ = fire(&mut tw);
    let s = stats(&tw);
    assert_eq!((s.submitted, s.visited), (4, 20));
    assert_eq!(drain_tacsim(&mut tw).len(), 4);

    // (b) 70 agents: the first 64 idle under one principal, the last 6 due.
    let mut tw = boot(wired(true, config()));
    let agents: Vec<ObjId> = (0..70).map(|n| thing(&mut tw, &format!("b{n}"))).collect();
    let shared = tw.spawn_player("Shared");
    for (n, agent) in agents.iter().enumerate() {
        if n < 64 {
            enlist(&mut tw, *agent, shared, 0);
        } else {
            let pilot = tw.spawn_player(&format!("Due{n}"));
            enlist(&mut tw, *agent, pilot, 1);
        }
    }
    let _ = fire(&mut tw);
    let s = stats(&tw);
    assert_eq!((s.visited, s.worked), (64, 0));
    assert!(drain_tacsim(&mut tw).is_empty());
    let _ = fire(&mut tw);
    let s = stats(&tw);
    assert_eq!((s.submitted, s.visited), (6, 64));
    assert_eq!(drain_tacsim(&mut tw).len(), 6);
}

#[test]
fn ic5_a_disabled_layer_fires_nothing() {
    let mut tw = boot(wired(true, TacsimConfig::default()));
    let agent = thing(&mut tw, "probe idle");
    let pilot = tw.spawn_player("Pilot");
    // `enlist_probe` asks for a test world, not for the layer.
    enlist(&mut tw, agent, pilot, 1);
    let _ = fire(&mut tw);
    assert_eq!(last_fire(tw.world()), None, "the off layer visits nothing");
    assert!(drain_tacsim(&mut tw).is_empty());
}

#[test]
fn ic5_production_registers_no_probe() {
    let mut tw = boot(wired(false, config()));
    let agent = thing(&mut tw, "probe");
    let pilot = tw.spawn_player("Pilot");
    assert!(
        enlist_probe(
            tw.world_mut(),
            agent,
            ProbeAgent {
                principal: pilot,
                pool: None,
                remaining: 1,
                next_issue: 0
            }
        )
        .is_err(),
        "production enlists no probe"
    );
    let h = handles(&tw);
    assert!(h.probe.is_none(), "production registers no probe family");
    assert!(h.inventory.is_some(), "production holds the inventory cell");
    let definitions = tw.registry().definitions().expect("frozen definitions");
    let key = mush_core::DefinitionKey {
        owner: mush_core::OwnerId::new(tacsim::NS).expect("owner"),
        kind: mush_core::DefinitionKind::new("action").expect("kind"),
        id: mush_core::DefinitionId::new(PROBE_ACTION).expect("id"),
    };
    assert!(
        definitions.current(&key).is_none(),
        "no probe action definition"
    );
    // The two production families are registered under `tacsim`.
    let families = h.families;
    assert_eq!(families.port_stock.owner().as_str(), tacsim::NS);
    assert_eq!(
        families.port_stock.descriptor().id().as_str(),
        "tacsim.port_stock"
    );
    assert_eq!(families.tickets.owner().as_str(), tacsim::NS);
    assert_eq!(families.tickets.descriptor().id().as_str(), "tacsim.ticket");
    let registry = format!(
        "{:?}",
        tw.engine()
            .registry
            .state_families()
            .expect("the state-family registry")
    );
    assert!(registry.contains("tacsim.port_stock") && registry.contains("tacsim.ticket"));
    let mut txn = tw
        .engine_mut()
        .action_transaction(&families.tickets)
        .expect("a tacsim transaction");
    assert!(
        txn.get(
            &families.tickets,
            &tacsim::port::TicketId {
                hull: ObjId(1),
                seq: 0
            }
        )
        .expect("the ticket family is composed")
        .is_none()
    );
}

#[test]
fn ic5_a_receipt_style_replay_is_refused() {
    let mut tw = boot(wired(true, config()));
    let mut t = Tokens(0);
    let agent = thing(&mut tw, "probe replay");
    let pilot = tw.spawn_player("Pilot");
    enlist(&mut tw, agent, pilot, 1);
    let _ = fire(&mut tw);
    let queued = drain_tacsim(&mut tw);
    land_all(&mut tw, &queued, &mut t);
    // Re-enlisted at the same issue: the provider refuses the second mark.
    enlist(&mut tw, agent, pilot, 1);
    let _ = fire(&mut tw);
    let queued = drain_tacsim(&mut tw);
    assert_eq!(queued.len(), 1);
    // The refusal is the provider's own `Conflict` at prepare, not any earlier
    // failure of the land path: the offer still projects for the re-enlisted
    // principal and the resolve accepts it, so only the replay guard can say this.
    let error =
        land_one(&mut tw, &queued[0], t.next()).expect_err("the replay is refused at prepare");
    assert!(
        error.contains("Conflict") && error.contains("that probe issue has already landed"),
        "the replay guard refused it: {error}"
    );
    assert!(mark(&mut tw, agent, 0).is_some());
    assert!(mark(&mut tw, agent, 1).is_none());
}

// ---------------------------------------------------------------------------
// 6. Reconnect notices
// ---------------------------------------------------------------------------

fn seen(tw: &TestWorld, events: &[(Audience, OutputEvent)], who: ObjId) -> Vec<String> {
    tw.seen_by(events, who)
}

/// End every connection `who` holds the way the session layer does (the
/// transport drops, then the linkdead grace expires: `mush-campaign`'s
/// `common::disconnect`), so the next [`TestWorld::connect`] is a first
/// connection and fires `Connected`.
fn disconnect(tw: &mut TestWorld, who: ObjId) {
    let mut out = mush_core::Outbox::new();
    let mut fx = mush_core::Effects::new();
    let conns: Vec<u64> = (1..=64)
        .filter(|conn| tw.engine().actor_of(*conn) == Some(who))
        .collect();
    assert!(!conns.is_empty(), "{who} holds a connection");
    for conn in conns {
        tw.engine_mut().step(
            mush_core::Inbound::ConnDown {
                conn,
                actor: who,
                reason: mush_core::DisconnectKind::Lost,
            },
            &mut out,
            &mut fx,
        );
    }
    tw.engine_mut().step(
        mush_core::Inbound::SessionGone { actor: who },
        &mut out,
        &mut fx,
    );
    assert_eq!(tw.engine().connections_of(who), 0);
}

#[test]
fn ic5_queued_notices_render_once_on_reconnect() {
    let mut tw = boot(wired(true, config()));
    let mut t = Tokens(0);
    let agent = thing(&mut tw, "probe notice");
    let principal = offline_player(&mut tw, "Offline");
    enlist(&mut tw, agent, principal, 1);
    let mut ops = tw.take_module_ops();
    let _ = fire(&mut tw);
    let fire_ops = tw.take_module_ops();
    let line = format!("Probe 0 for #{} was staged.", agent.0);
    assert_eq!(pending_notices(tw.world(), principal), vec![line.clone()]);
    assert!(sets_notice(&fire_ops, principal));
    ops.extend(fire_ops);
    let queued = drain_tacsim(&mut tw);
    land_all(&mut tw, &queued, &mut t);
    ops.extend(tw.take_module_ops());
    let boot_data: HashMap<SmolStr, mush_core::ModuleBootData> = fold_module_ops(ops);
    let mut tw = tw
        .reboot_into(wired(true, config()), boot_data)
        .expect("the kept world boots");
    assert_eq!(
        pending_notices(tw.world(), principal),
        vec![line.clone()],
        "the queue survives the reboot"
    );
    let events = tw.connect(principal);
    let ops = tw.take_module_ops();
    let lines = seen(&tw, &events, principal);
    let heading_at = lines.iter().position(|l| l.contains(NOTICE_HEADING));
    let line_at = lines.iter().position(|l| l.contains(&line));
    assert!(
        heading_at.is_some() && line_at.is_some() && heading_at < line_at,
        "the heading, then the line: {lines:?}"
    );
    assert_eq!(
        lines.iter().filter(|l| l.contains(NOTICE_HEADING)).count(),
        1
    );
    assert_eq!(lines.iter().filter(|l| l.contains(&line)).count(), 1);
    assert!(
        deletes_notice(&ops, principal),
        "the render deletes the document"
    );
    assert!(pending_notices(tw.world(), principal).is_empty());
    // End the session fully, so the next connect is a real `Connected` transition
    // (the engine fires the hook only on a first connection of a disconnected
    // actor; a second open connection would not run it at all).
    disconnect(&mut tw, principal);
    assert!(!tw.world().is_connected(principal));
    let again = tw.connect(principal);
    assert!(
        again
            .iter()
            .any(|(_, ev)| text_of(ev).contains("has connected.")),
        "the second connect is a real Connected transition"
    );
    let lines = seen(&tw, &again, principal);
    assert!(
        !lines
            .iter()
            .any(|l| l.contains(NOTICE_HEADING) || l.contains(&line)),
        "a second connect renders nothing: {lines:?}"
    );
}

#[test]
fn ic5_a_connected_principal_is_told_live() {
    let mut tw = boot(wired(true, config()));
    let agent = thing(&mut tw, "probe live");
    let pilot = tw.spawn_player("Pilot");
    enlist(&mut tw, agent, pilot, 1);
    let _ = tw.take_module_ops();
    let events = fire(&mut tw);
    let ops = tw.take_module_ops();
    let queued = drain_tacsim(&mut tw);
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].actor, pilot);
    let line = format!("Probe 0 for #{} was staged.", agent.0);
    assert!(
        seen(&tw, &events, pilot).iter().any(|l| l.contains(&line)),
        "told at once"
    );
    assert!(!sets_notice(&ops, pilot), "no notice document is written");
    assert!(pending_notices(tw.world(), pilot).is_empty());
}

#[test]
fn ic5_a_corrupt_notice_document_is_dropped_for_that_character() {
    let mut tw = boot(wired(true, config()));
    let mut t = Tokens(0);
    let (a, b) = (thing(&mut tw, "probe a"), thing(&mut tw, "probe b"));
    let (pa, pb) = (
        offline_player(&mut tw, "OfflineA"),
        offline_player(&mut tw, "OfflineB"),
    );
    enlist(&mut tw, a, pa, 1);
    enlist(&mut tw, b, pb, 1);
    let _ = fire(&mut tw);
    let queued = drain_tacsim(&mut tw);
    land_all(&mut tw, &queued, &mut t);
    let mut boot_data = fold_module_ops(tw.take_module_ops());
    let ns = boot_data
        .get_mut(tacsim::NS)
        .expect("the tacsim namespace was written");
    let key = notice_key(pa);
    let slot = ns
        .kv
        .iter_mut()
        .find(|(k, _)| *k == key)
        .expect("pa's notice document");
    slot.1 = serde_json::json!({ "schema": 1, "who": pa.0, "lines": "x" });
    let mut tw = tw
        .reboot_into(wired(true, config()), boot_data)
        .expect("the reboot succeeds over a corrupt notice");
    let events = tw.connect(pa);
    assert!(
        !seen(&tw, &events, pa)
            .iter()
            .any(|l| l.contains(NOTICE_HEADING)),
        "the corrupt queue is dropped"
    );
    let events = tw.connect(pb);
    let lines = seen(&tw, &events, pb);
    assert!(
        lines.iter().any(|l| l.contains(NOTICE_HEADING)),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains(&format!("Probe 0 for #{} was staged.", b.0))),
        "{lines:?}"
    );
}

// ---------------------------------------------------------------------------
// 2 (cont.). A shared principal under a biting budget starves no one
// ---------------------------------------------------------------------------

#[test]
fn ic5_a_shared_principal_under_a_biting_budget_starves_no_one() {
    let mut tw = boot(wired(true, config()));
    let mut t = Tokens(0);
    let s1 = thing(&mut tw, "s1");
    let s2 = thing(&mut tw, "s2");
    let xs: Vec<ObjId> = (1..=20).map(|n| thing(&mut tw, &format!("x{n}"))).collect();
    assert!(s1 < s2 && xs.iter().all(|x| s2 < *x));
    let shared = tw.spawn_player("Shared");
    enlist(&mut tw, s1, shared, 3);
    enlist(&mut tw, s2, shared, 3);
    for (n, x) in xs.iter().enumerate() {
        let pilot = tw.spawn_player(&format!("X{n}"));
        enlist(&mut tw, *x, pilot, 3);
    }
    let agent_of = |submit: &mush_core::AuthoritativeSubmit| {
        ObjId(u32::try_from(submit.parameters["agent"].as_u64().expect("agent")).expect("u32"))
    };
    let mut fires: Vec<Vec<ObjId>> = Vec::new();
    for _ in 0..3 {
        let events = fire(&mut tw);
        assert!(!fanned_out(&events), "no fan-out line in any fire");
        let s = stats(&tw);
        assert_eq!((s.submitted, s.deferred), (16, 1));
        let queued = drain_tacsim(&mut tw);
        fires.push(queued.iter().map(agent_of).collect());
        land_all(&mut tw, &queued, &mut t);
    }
    let set = |agents: &[ObjId]| -> std::collections::BTreeSet<ObjId> {
        agents.iter().copied().collect()
    };
    let expect = |list: Vec<ObjId>| set(&list);
    let x = |range: std::ops::RangeInclusive<usize>| -> Vec<ObjId> {
        range.map(|n| xs[n - 1]).collect()
    };
    let mut f1 = vec![s1];
    f1.extend(x(1..=15));
    let mut f2 = x(16..=20);
    f2.push(s2);
    f2.extend(x(1..=10));
    let mut f3 = x(11..=20);
    f3.push(s1);
    f3.extend(x(1..=5));
    assert_eq!(set(&fires[0]), expect(f1), "fire 1");
    assert_eq!(set(&fires[1]), expect(f2), "fire 2");
    assert_eq!(set(&fires[2]), expect(f3), "fire 3");
    // Every agent acted by the end of fire 2 (within ceil(22/16) + 1 fires).
    let by_two: std::collections::BTreeSet<ObjId> =
        fires[0].iter().chain(&fires[1]).copied().collect();
    for agent in std::iter::once(s1)
        .chain(std::iter::once(s2))
        .chain(xs.iter().copied())
    {
        assert!(by_two.contains(&agent), "{agent:?} starved");
    }
    // The shared agents alternate.
    assert!(fires[0].contains(&s1) && !fires[0].contains(&s2));
    assert!(fires[1].contains(&s2) && !fires[1].contains(&s1));
    assert!(fires[2].contains(&s1) && !fires[2].contains(&s2));
}
