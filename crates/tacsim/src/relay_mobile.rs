//! R9b: one finite Concord relief programme, a real capital hull and a moving half.
use mush_core::{CanonicalId, Ctx, ObjId, World};
use mush_org::{OrgId, PrincipalId};
use serde::{Deserialize, Serialize};

pub const KEY: &str = "tacsim:relay_mobile";
pub const UNIT: &str = "tacsim.unit.halcyon";
pub const MONTH: i64 = 30 * 86_400;
pub const PAYMENT_MINOR: i64 = 1_000_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Programme {
    pub hull: ObjId,
    pub fleet: OrgId,
    pub front: mush_faction::FrontId,
    pub postmaster: ObjId,
    pub payments: Vec<u64>,
    pub next_payment: Option<i64>,
    pub consent: Option<u64>,
    pub refit: Option<u64>,
    pub launch: Option<u64>,
}
impl Programme {
    pub fn progress(&self) -> u8 {
        self.payments.len() as u8
            + u8::from(self.consent.is_some())
            + u8::from(self.refit.is_some())
    }
    pub fn check(&self) -> Result<(), String> {
        if self.hull.0 == 0
            || self.postmaster.0 == 0
            || self.payments.len() > 6
            || self
                .payments
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.payments.len()
            || self.launch.is_some() && self.progress() != 8
        {
            return Err("Invalid mobile programme receipt census".into());
        }
        Ok(())
    }
}
pub fn current(w: &World) -> Option<&Programme> {
    w.module_state::<crate::TacsimStore>()?
        .relay_mobile
        .as_ref()
}
fn id(s: &str) -> Result<CanonicalId, String> {
    CanonicalId::new(s).map_err(|e| e.to_string())
}
fn fleet_port(w: &World) -> Option<crate::port::coop::ProgrammePort<'_>> {
    crate::port::coop::programme_ports(w)
        .into_iter()
        .find(|p| p.programme.id.as_str() == "concord.relief")
}
pub fn fleet_hull(w: &World) -> Option<ObjId> {
    Some(ObjId(fleet_port(w)?.map.hulls.first()?.hull))
}
pub fn commander(w: &World, hull: ObjId) -> Option<ObjId> {
    let p = fleet_port(w)?;
    if p.map.hulls.first()?.hull != hull.0 {
        return None;
    }
    let a = w
        .module_state::<mush_economy::EconomyStore>()?
        .snapshot()
        .asset(hull)?;
    if a.lifecycle != mush_economy::AssetLifecycle::Active
        || a.frozen
        || a.stolen.is_some()
        || Some(a.title_owner.clone()) != p.trust()
    {
        return None;
    }
    let assignment = a.assignment.as_ref()?;
    if assignment.valid_until.is_some_and(|t| t <= w.now()) {
        return None;
    }
    let PrincipalId::Object(actor) = assignment.assigned_to else {
        return None;
    };
    w.get(actor)
        .filter(|o| o.kind.is_actor() && !o.flags.contains(mush_core::Flags::GOING))
        .map(|_| actor)
}
pub fn assigned_hull(w: &World, actor: ObjId) -> Option<ObjId> {
    let h = fleet_hull(w)?;
    (commander(w, h) == Some(actor)).then_some(h)
}
fn save(ctx: &mut Ctx<'_>, p: Programme) -> Result<(), String> {
    p.check()?;
    let value = serde_json::to_value(&p).map_err(|e| e.to_string())?;
    ctx.w
        .module_state_mut::<crate::TacsimStore>()
        .ok_or("TacSim absent")?
        .relay_mobile = Some(p);
    crate::store::push_op(
        ctx.out,
        mush_core::ModuleWrite::JsonSet {
            key: KEY.into(),
            value,
        },
    );
    ctx.out.pin_deferred();
    Ok(())
}
fn draft(
    ctx: &Ctx<'_>,
    kind: &str,
    summary: String,
    entities: Vec<ObjId>,
    detail: serde_json::Value,
) -> mush_canon::CanonDraft {
    mush_canon::CanonDraft {
        kind: kind.into(),
        summary,
        entities,
        scope: mush_canon::CanonScope::Public,
        ic_time: ctx.w.world_ic_now().at,
        source: "tacsim.relay_mobile".into(),
        detail,
    }
}
fn clock_ok(w: &World, p: &Programme) -> Result<(), String> {
    let c = w
        .module_state::<mush_faction::SimState>()
        .and_then(|s| s.clocks.values().find(|c| c.front == p.front))
        .ok_or("Fleet front missing")?;
    if c.segments != 8 || c.filled != p.progress() || c.completed != (p.progress() == 8) {
        return Err(
            "Fleet front differs from actual payment/decision/refit receipts; repair required"
                .into(),
        );
    }
    Ok(())
}
pub fn boot(ctx: &mut Ctx<'_>) -> Result<(), String> {
    let Some(port) = fleet_port(ctx.w) else {
        return Ok(());
    };
    let Some(row) = port.map.hulls.first() else {
        return Ok(());
    };
    // Old station packs can defer the Exchange seed while the lane is already
    // current. Wait before creating the NPC/front, but never hide lost authority
    // beside an existing programme or an applied/held relay pack.
    let exchange = match mush_propagation::routing::registry(ctx.w) {
        Some(registry) => registry.operator,
        None => {
            let relay_pack = id("tacsim.relay.luna_ceres")?;
            let unseeded = ctx
                .w
                .module_state::<mush_content::ContentState>()
                .is_some_and(|s| {
                    s.latches.applied(&relay_pack).is_none() && !s.latches.is_held(&relay_pack)
                });
            let receipt_absent = ctx
                .w
                .module_state::<crate::TacsimStore>()
                .is_some_and(|s| s.relay_mobile.is_none() && !s.relay_mobile_held);
            let orphan = ctx
                .w
                .iter()
                .any(|o| o.name == "Postmaster Solveig Adebayo-Krishnan")
                || ctx
                    .w
                    .module_state::<mush_faction::SimState>()
                    .is_some_and(|s| s.fronts.values().any(|f| f.title == "A Half Afloat"));
            if unseeded && receipt_absent && !orphan {
                tracing::info!("Fleet programme waits for the explicitly upgraded Exchange seed");
                return Ok(());
            }
            return Err("Exchange absent beside Fleet or relay authority; repair required".into());
        }
    };
    let hull = ObjId(row.hull);
    let org = port
        .map
        .orgs
        .get(&port.programme.org)
        .ok_or("Fleet title owner missing")?
        .clone();
    let fleet = OrgId(org.org);
    let gallery = ObjId(port.map.presentation.ok_or("Fleet gallery missing")?);
    if let Some(p) = current(ctx.w).cloned() {
        p.check()?;
        clock_ok(ctx.w, &p)?;
        if p.hull != hull
            || p.fleet != fleet
            || ctx
                .w
                .get(p.postmaster)
                .is_none_or(|o| o.kind != mush_core::ObjKind::Npc)
        {
            return Err("Fleet programme identity drift".into());
        }
        if let Some(order) = p.refit {
            let m = crate::relay_foundry::current(ctx.w)
                .and_then(|f| f.machines.get(&order))
                .ok_or("Fleet fabrication missing")?;
            let node = m.installed[1]
                .or_else(|| m.dismounted.map(|nodes| nodes[1]))
                .ok_or("Fleet mounting missing")?;
            let room = ctx
                .w
                .get(node)
                .and_then(|o| o.location)
                .ok_or("Fleet mounting unplaced")?;
            if crate::port::ship::hull_of_room(ctx.w, room).is_none_or(|(v, _)| v.hull() != hull) {
                return Err("Fleet installation left its hull".into());
            }
        }
        return Ok(());
    }
    if ctx
        .w
        .module_state::<crate::TacsimStore>()
        .is_some_and(|s| s.relay_mobile_held)
    {
        return Err("Fleet programme unreadable; refusing reseed".into());
    }
    if ctx
        .w
        .iter()
        .any(|o| o.name == "Postmaster Solveig Adebayo-Krishnan")
    {
        return Err("Fleet receipt missing beside an existing Postmaster; repair required".into());
    }
    let postmaster = ctx
        .w
        .create(
            mush_core::ObjKind::Npc,
            "Postmaster Solveig Adebayo-Krishnan",
            mush_core::GOD,
            Some(gallery),
        )
        .map_err(|e| e.to_string())?;
    let front=mush_faction::seed_mobile_relay_front(ctx,fleet,draft(ctx,"relay.mobile_programme","A Half Afloat: the Concord relief cruiser awaits funding, consent and a manufactured pair.".into(),vec![hull],serde_json::json!({"hull":hull})))?;
    mush_faction::register_programme_principal(ctx, exchange, postmaster)?;
    let view = crate::port::ship::hull_view(ctx.w, hull).ok_or("Fleet hull unreadable")?;
    let backing =
        mush_economy::CustodyRef::SpatialInterior(view.stowage().ok_or("Fleet hold absent")?);
    if ctx
        .w
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|s| s.snapshot().vault_by_backing(&backing))
        .is_none()
    {
        let vault = crate::port::apply::alloc_vault_id(ctx)?;
        mush_economy::create_vault(
            ctx,
            mush_economy::Vault {
                id: vault,
                owner: PrincipalId::Organization(fleet),
                scope: Some(mush_org::OrgScopeId(org.root_scope)),
                name: "Concord relief cruiser stores".into(),
                backing,
                status: mush_economy::VaultStatus::Active,
                revision: 1,
            },
        );
    }
    save(
        ctx,
        Programme {
            hull,
            fleet,
            front,
            postmaster,
            payments: Vec::new(),
            next_payment: None,
            consent: None,
            refit: None,
            launch: None,
        },
    )
}
fn launch(ctx: &mut Ctx<'_>, p: &mut Programme) -> Result<(), String> {
    if p.progress() != 8 || p.launch.is_some() {
        return Ok(());
    }
    // Funding commissions the installation. First launch is a real departure,
    // not a synonym for the last treasury payment while the cruiser is docked.
    if !ctx
        .w
        .module_state::<mush_sim::JourneyBook>()
        .is_some_and(|b| b.under_way(p.hull))
    {
        return Ok(());
    }
    clock_ok(ctx.w, p)?;
    let order = p.refit.ok_or("Refit missing")?;
    let node = crate::relay_foundry::current(ctx.w)
        .and_then(|f| f.machines.get(&order))
        .and_then(|m| m.installed[1])
        .ok_or("Mobile mount missing")?;
    if !operational(ctx.w, p.hull, node) {
        return Err("The completed programme's installation is not operational".into());
    }
    let canon=mush_canon::commit(ctx,draft(ctx,"relay.first_mobile_launch","A Half Afloat: the Concord Fleet's Long Vigil launches the first mobile relay, bound for Belt Squadron relief. Six funded months, the Postmaster's consent and the Lunar refit are complete.".into(),vec![p.hull,p.postmaster,node],serde_json::json!({"front":p.front,"order":order,"payments":p.payments}))).ok_or("Launch canon unavailable")?;
    p.launch = Some(canon.0);
    Ok(())
}
// Keep the authoritative segment receipts even if commissioning must wait for
// repairs. A later tick retries only commissioning, never an already paid month.
fn finish(ctx: &mut Ctx<'_>, mut p: Programme) -> Result<(), String> {
    let result = launch(ctx, &mut p);
    save(ctx, p)?;
    result
}
fn fund_tick(ctx: &mut Ctx<'_>, _: &str, _: Option<mush_core::SceneId>) {
    if let Err(e) = tick(ctx) {
        tracing::warn!(error=%e,"Fleet programme waits");
    }
}
fn tick(ctx: &mut Ctx<'_>) -> Result<(), String> {
    let Some(mut p) = current(ctx.w).cloned() else {
        return Ok(());
    };
    if p.launch.is_some()
        || p.progress() == 8
            && !ctx
                .w
                .module_state::<mush_sim::JourneyBook>()
                .is_some_and(|b| b.under_way(p.hull))
    {
        return Ok(());
    }
    clock_ok(ctx.w, &p)?;
    let now = ctx.w.world_ic_now().at;
    if p.progress() < 8 && (p.payments.len() == 6 || p.next_payment.is_none_or(|t| t > now)) {
        return Ok(());
    }
    if p.payments.len() < 6 && p.next_payment.is_some_and(|t| t <= now) {
        let port = fleet_port(ctx.w).ok_or("Fleet port missing")?;
        let payer = port.trust_account().ok_or("Fleet treasury absent")?;
        let payee = port
            .map
            .orgs
            .get(&id("orbital_access")?)
            .and_then(|r| r.account)
            .map(mush_economy::AccountId)
            .ok_or("Lunar refit yard account absent")?;
        let available = ctx
            .w
            .module_state::<mush_economy::EconomyStore>()
            .and_then(|s| s.snapshot().account(payer))
            .and_then(|a| a.available())
            .ok_or("Fleet treasury unreadable")?;
        if available.minor() < PAYMENT_MINOR {
            return Ok(());
        }
        let plan = mush_economy::EconomyPlan::new(
            mush_economy::IdempotencyKey::new(
                "relay.fleet.month",
                format!("{}:{}", p.hull.0, p.payments.len() + 1),
            ),
            PrincipalId::Object(ctx.actor),
            mush_economy::EconomicCause::Internal {
                kind: "relay.fleet.month".into(),
                id: p.hull.0.into(),
            },
            vec![
                mush_economy::RequestedLine {
                    account: payer,
                    delta: mush_economy::Amount::from_minor(-PAYMENT_MINOR),
                    memo: "Concord relief refit monthly appropriation".into(),
                },
                mush_economy::RequestedLine {
                    account: payee,
                    delta: mush_economy::Amount::from_minor(PAYMENT_MINOR),
                    memo: "Funded Lunar relay refit work".into(),
                },
            ],
        );
        let tx = match mush_economy::post(ctx, &plan).map_err(|e| e.to_string())? {
            mush_economy::PostOutcome::Posted(t) | mush_economy::PostOutcome::AlreadyPosted(t) => t,
            mush_economy::PostOutcome::NeedsApproval(_) => {
                return Err("Fleet appropriation requires account approval".into());
            }
        };
        mush_faction::advance_mobile_relay_front(ctx, p.front, p.progress())?;
        p.payments.push(tx.0);
        // A late or unfunded programme never catches up several months in one tick.
        p.next_payment = if p.payments.len() < 6 {
            Some(now.checked_add(MONTH).ok_or("IC time exhausted")?)
        } else {
            None
        };
    }
    finish(ctx, p)
}
fn command(ctx: &mut Ctx<'_>) -> Result<(), String> {
    let mut p = current(ctx.w)
        .cloned()
        .ok_or("Fleet programme not present")?;
    clock_ok(ctx.w, &p)?;
    if ctx.switches.has("fund") {
        if !mush_economy::acts_for(ctx.w, ctx.actor, &PrincipalId::Organization(p.fleet)) {
            return Err("Fleet treasury authority required".into());
        }
        if p.payments.len() == 6 || p.next_payment.is_some() {
            return Err("Monthly appropriation already authorized or complete".into());
        }
        p.next_payment = Some(ctx.w.world_ic_now().at);
        save(ctx, p)?;
        ctx.out.info(ctx.actor,"Monthly Fleet appropriation authorized: six payments of 10000 cr, at most one per 30 IC days.");
    } else if ctx.switches.has("consent") {
        let exchange = mush_propagation::routing::registry(ctx.w)
            .ok_or("Exchange absent")?
            .operator;
        if !mush_economy::acts_for(ctx.w, ctx.actor, &PrincipalId::Organization(exchange)) {
            return Err("The Postmaster's Exchange authority is required".into());
        }
        if p.consent.is_some() {
            return Err("The Postmaster's consent is already recorded".into());
        }
        if ctx.args.lhs().trim() != "yes" {
            return Err(
                "Use +relayfleet/consent yes to record the Postmaster's affirmative decision"
                    .into(),
            );
        }
        let canon=mush_canon::commit(ctx,draft(ctx,"relay.postmaster_consent","Postmaster Solveig Adebayo-Krishnan consents to the Concord's relief-hull installation.".into(),vec![p.postmaster,ctx.actor],serde_json::json!({"principal":p.postmaster,"authorized_by":ctx.actor}))).ok_or("Consent canon unavailable")?;
        mush_faction::advance_mobile_relay_front(ctx, p.front, p.progress())?;
        p.consent = Some(canon.0);
        finish(ctx, p)?;
        ctx.out.info(ctx.actor, "Postmaster consent recorded.");
    } else if ctx.switches.has("remove") {
        if commander(ctx.w, p.hull) != Some(ctx.actor)
            || !mush_economy::acts_for(ctx.w, ctx.actor, &PrincipalId::Organization(p.fleet))
        {
            return Err("Assigned Fleet command and organization authority are required".into());
        }
        crate::relay_foundry::remove_mobile(ctx, p.refit.ok_or("Fleet refit absent")?, p.hull)?;
        ctx.out.info(ctx.actor,"Fleet pair removed to the Lunar dock; its original halves and cradles are retained for refit.");
    } else if ctx.switches.has("refit") {
        let order = mush_industry::WorkOrderId::parse(ctx.args.lhs().trim())
            .ok_or("Name a completed fabricated pair work order (wo#<16 hex>)")?
            .0;
        let remount = p.refit == Some(order)
            && crate::relay_foundry::current(ctx.w)
                .and_then(|f| f.machines.get(&order))
                .is_some_and(|m| m.dismounted.is_some());
        if p.refit.is_some() && !remount {
            return Err("Fleet relay refit already completed".into());
        }
        if commander(ctx.w, p.hull) != Some(ctx.actor)
            || !mush_economy::acts_for(ctx.w, ctx.actor, &PrincipalId::Organization(p.fleet))
        {
            return Err("Assigned Fleet command and organization authority are required".into());
        }
        crate::relay_foundry::fit_mobile(ctx, order, p.hull, p.fleet)?;
        if remount {
            ctx.out.info(ctx.actor,"Fleet refit restored using the original halves and cradles; no new front segment or first-launch record.");
            return Ok(());
        }
        let canon = mush_canon::commit(
            ctx,
            draft(
                ctx,
                "relay.mobile_refit",
                "The Concord relief cruiser's manufactured relay pair is installed at Luna.".into(),
                vec![p.hull, ctx.actor],
                serde_json::json!({"order":order}),
            ),
        )
        .ok_or("Refit canon unavailable")?;
        mush_faction::advance_mobile_relay_front(ctx, p.front, p.progress())?;
        p.refit = Some(order);
        finish(ctx, p)?;
        ctx.out.info(ctx.actor,format!("Fleet refit complete; canon #{}. The mobile node waits for A Half Afloat to complete.",canon.0));
    } else {
        ctx.out.info(ctx.actor,format!("A Half Afloat: {}/8; hull #{}; {} of 6 treasury payments; Postmaster #{} consent {}; refit {}; launched {}; mounted {}. +relayfleet/fund; /consent yes; /refit wo#<16-hex>; /remove. Command is an @asset/assign relationship; stores use the Fleet's real vault.",p.progress(),p.hull.0,p.payments.len(),p.postmaster.0,p.consent.is_some(),p.refit.is_some(),p.launch.is_some(),mounted(ctx.w,p.hull).is_some()));
    }
    Ok(())
}
pub fn capability(w: &World, hull: ObjId) -> Option<(u32, ObjId)> {
    let view = crate::port::ship::hull_view(w, hull)?;
    if view.unit_held || view.unit.as_str() != UNIT {
        return None;
    }
    let spec = w
        .module_state::<crate::TacsimStore>()?
        .definitions()
        .specs
        .get(&view.unit)?;
    let (radius, room) = spec.components.iter().find_map(|c| match &c.role {
        crate::definitions::unit::ComponentRole::Relay { radius_km, room } => {
            Some((*radius_km, room))
        }
        _ => None,
    })?;
    let unit = mush_tactics::module::unit_of(w, hull)?;
    if !unit
        .capabilities()
        .any(|c| c.id.as_str() == "tacsim.relay.mobile")
    {
        return None;
    }
    Some((radius, view.room(room)?))
}
pub fn mounted(w: &World, hull: ObjId) -> Option<(ObjId, ObjId)> {
    let p = current(w).filter(|p| p.hull == hull)?;
    let m = crate::relay_foundry::current(w)?.machines.get(&p.refit?)?;
    Some((m.installed[1]?, m.halves[1]))
}
pub fn carrier_of_node(w: &World, node: ObjId) -> Option<ObjId> {
    let p = current(w)?;
    (mounted(w, p.hull)?.0 == node).then_some(p.hull)
}
pub fn operational(w: &World, hull: ObjId, node: ObjId) -> bool {
    capability(w, hull).is_some()
        && [hull, node].into_iter().all(|o| {
            mush_tactics::module::unit_of(w, o).is_some_and(|u| {
                let section = if o == hull { "hull" } else { "core" };
                [section, "power"]
                    .into_iter()
                    .all(|s| super::relay_loss::internals(u, s).is_some_and(|n| n > 0))
            })
        })
}
pub fn active(w: &World, node: ObjId) -> Option<(ObjId, u32)> {
    let p = current(w)?;
    let hull = carrier_of_node(w, node)?;
    if p.progress() != 8 || !operational(w, hull, node) {
        return None;
    }
    Some((hull, capability(w, hull)?.0))
}
pub fn register(reg: &mut mush_core::Registrar<'_>) -> Result<(), mush_core::ModError> {
    reg.tick(
        "relay.fleet",
        mush_core::TickCadence::Coarse(std::time::Duration::from_secs(5)),
        std::sync::Arc::new(fund_tick),
    )?;
    reg.classify_commands(mush_core::CommandCompatibility::Authoritative);
    let result = reg.command(
        mush_core::CommandSpec {
            name: "+relayfleet".into(),
            shape: mush_core::ArgShape::EqPair,
            switches: ["fund", "consent", "refit", "remove"]
                .into_iter()
                .map(Into::into)
                .collect(),
            help: "+relayfleet; /fund; /consent yes; /refit wo#<16-hex>; /remove".into(),
            ..Default::default()
        },
        std::sync::Arc::new(|ctx: &mut Ctx<'_>| command(ctx).map_err(mush_core::CmdError::msg)),
    );
    reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
    result
}
