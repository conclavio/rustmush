//! R9a: finite lunar fabrication, conserved output conversion and physical deployment.
use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn,
    AuthoritativeActionOutput, CanonicalId, Ctx, DefinitionRef, ObjId, PerceivedRef,
    ProjectedParameter, ProjectedParameterKind, ProjectedParameterSchema, ProjectedTargetSchema,
    ProviderOfferDraft, RevisionSet, RulesValue, StateFamilyHandle, SubjectKey, World,
};
use mush_economy::{EconomyStore, VaultId};
use mush_inventory::record::Lineage;
use mush_inventory::{EndpointRef, InventoryFamilies, LotId};
use mush_org::{OrgId, PrincipalId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const ACTION: &str = "tacsim_relay_fabricate";
pub const OUTCOME: &str = "tacsim_relay_fabricate_result";
pub const FAMILY: &str = "tacsim.relay_fabrications";
pub const KEY: &str = "tacsim:relay_foundry";
pub const CARGO_BEHAVIOR: &str = "tacsim.relay_cargo";
pub const PACK: &str = "tacsim.relay.manufacture";
pub const RECIPE: &str = "private_pair";
pub const KNOWHOW: &str = "relay.entanglement";
pub const BLANK: &str = "relay_blank";
pub const OUTPUT: &str = "relay_pair";
pub const MAX_PAIRS: usize = 8;
pub const HALF_KG: u64 = 5_000;

/// Exact inventory output consumed once. This is the provenance of two machines,
/// never a second writable quantity balance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub order: u64,
    pub lot: LotId,
    pub owner: PrincipalId,
    pub actor: ObjId,
}
/// One bounded world book, keyed by the actual foundry object.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fabrications {
    pub facility: ObjId,
    pub stocked: bool,
    pub receipts: BTreeMap<u64, Receipt>,
}
impl Fabrications {
    pub fn check(&self) -> Result<(), String> {
        if self.facility.0 == 0
            || self.receipts.len() > MAX_PAIRS
            || self
                .receipts
                .iter()
                .any(|(key, r)| *key == 0 || *key != r.order)
            || (!self.stocked && !self.receipts.is_empty())
        {
            return Err("invalid relay fabrication census".into());
        }
        let lots: std::collections::BTreeSet<_> = self.receipts.values().map(|r| r.lot).collect();
        if lots.len() != self.receipts.len() {
            return Err("duplicate fabricated lot".into());
        }
        Ok(())
    }
}
/// Physical identities are recorded with their creation/title post-images.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Machines {
    pub halves: [ObjId; 2],
    pub titled: [bool; 2],
    pub installed: [Option<ObjId>; 2],
    /// A removed Fleet pair retains its original cradles for checked remounting.
    #[serde(default)]
    pub dismounted: Option<[ObjId; 2]>,
    pub operator: Option<OrgId>,
    pub discovery: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Foundry {
    pub facility: ObjId,
    pub place: ObjId,
    pub vault: VaultId,
    pub owner: OrgId,
    pub machines: BTreeMap<u64, Machines>,
}
impl Foundry {
    pub fn check(&self) -> Result<(), String> {
        if self.facility.0 == 0 || self.place.0 == 0 || self.machines.len() > MAX_PAIRS {
            return Err("invalid relay foundry".into());
        }
        let ids: std::collections::BTreeSet<_> =
            self.machines.values().flat_map(|m| m.halves).collect();
        if ids.len() != self.machines.len() * 2 || ids.contains(&ObjId(0)) {
            return Err("duplicate relay cargo identity".into());
        }
        let nodes: Vec<_> = self
            .machines
            .values()
            .flat_map(|m| {
                m.installed
                    .into_iter()
                    .flatten()
                    .chain(m.dismounted.into_iter().flatten())
            })
            .collect();
        if nodes
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != nodes.len()
            || nodes.iter().any(|n| ids.contains(n))
            || self.machines.values().any(|m| {
                m.installed.iter().any(Option::is_some) != m.operator.is_some()
                    || m.installed
                        .iter()
                        .enumerate()
                        .any(|(i, n)| n.is_some() && !m.titled[i])
                    || m.dismounted.is_some() && m.installed.iter().any(Option::is_some)
                    || m.discovery.is_some()
                        && m.dismounted.is_none()
                        && m.installed.iter().any(Option::is_none)
            })
        {
            return Err("Invalid relay installation census".into());
        }
        Ok(())
    }
}
/// A command-created offer proposal, checked again by projection. RAM only;
/// losing it loses an offer, never inputs or a completed fabrication.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Proposal {
    pub actor: ObjId,
    pub facility: ObjId,
    pub vault: VaultId,
    pub order: u64,
    pub lot: LotId,
    pub owner: PrincipalId,
}
pub fn current(w: &World) -> Option<&Foundry> {
    w.module_state::<crate::TacsimStore>()?
        .relay_foundry
        .as_ref()
}
fn err(code: ActionErrorCode, message: &'static str) -> ActionError {
    crate::port_tick::port_error(code, message)
}
fn invalid() -> ActionError {
    err(
        ActionErrorCode::InvalidRequest,
        "No usable relay fabrication offer.",
    )
}
fn ready(w: &World, f: &Foundry) -> bool {
    w.get(f.facility).is_some_and(|o| {
        o.kind == mush_core::ObjKind::Thing
            && !o.flags.contains(mush_core::Flags::GOING)
            && o.location == Some(f.place)
    }) && w
        .module_state::<mush_industry::IndustryStore>()
        .and_then(|s| s.facility(f.facility))
        .is_some_and(|r| {
            r.owner() == &PrincipalId::Organization(f.owner)
                && r.capabilities().iter().any(|c| c.as_str() == KNOWHOW)
        })
        && w.module_state::<EconomyStore>()
            .and_then(|s| s.snapshot().vault(f.vault))
            .is_some_and(|v| {
                v.owner == PrincipalId::Organization(f.owner)
                    && v.backing == mush_economy::CustodyRef::Container(f.facility)
                    && v.status == mush_economy::VaultStatus::Active
            })
}
fn authorized(w: &World, actor: ObjId, f: &Foundry) -> bool {
    ready(w, f)
        && w.get(actor).is_some_and(|o| o.location == Some(f.place))
        && mush_economy::acts_for(w, actor, &PrincipalId::Organization(f.owner))
        && mush_economy::may_take_from_vault(w, actor, f.vault)
        && w.module_state::<mush_industry::IndustryStore>()
            .and_then(|s| s.facility(f.facility))
            .is_some_and(|r| r.owner() == &PrincipalId::Organization(f.owner))
}
fn save(ctx: &mut Ctx<'_>, f: Foundry) -> Result<(), String> {
    f.check()?;
    let value = serde_json::to_value(&f).map_err(|e| e.to_string())?;
    ctx.w
        .module_state_mut::<crate::TacsimStore>()
        .ok_or("TacSim missing")?
        .relay_foundry = Some(f);
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
fn paid(w: &World, wo: &mush_industry::WorkOrder) -> bool {
    mush_industry::money::live_hold_in(w, wo.id()).is_none()
        && w.module_state::<EconomyStore>().is_some_and(|s| s.snapshot().holds().any(|h|
            h.purpose == mush_industry::money::purpose(wo.id())
                && h.account == wo.budget_account()
                && matches!(h.status, mush_economy::HoldStatus::Consumed { amount, .. } if amount == mush_industry::money::order_price(wo))))
}
fn proposal_current(e: &mush_core::Engine, p: &Proposal) -> bool {
    let Some(h) = e.world.module_state::<mush_industry::IndustryHandles>() else {
        return false;
    };
    let Ok(Some(wo)) = e
        .action_state()
        .get(&h.0.work_orders, &mush_industry::WorkOrderId(p.order))
    else {
        return false;
    };
    paid(&e.world, wo)
        && matches!(wo.state(), mush_industry::WorkOrderState::Completed { .. })
        && wo.facility() == p.facility
        && wo.recipe().pack().as_str() == PACK
        && wo.recipe().id().as_str() == RECIPE
        && wo.recipe().version() == 1
        && wo.quantity() == 1
        && wo.destination().endpoint() == EndpointRef(p.vault)
        && wo.beneficiary() == &p.owner
        && !e
            .world
            .module_state::<mush_market::MarketStore>()
            .is_some_and(|s| {
                id(OUTPUT)
                    .ok()
                    .is_some_and(|good| s.lot_listing_of(EndpointRef(p.vault), &good).is_some())
            })
}
pub struct Provider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub inventory: InventoryFamilies,
    pub book: StateFamilyHandle<ObjId, Fabrications>,
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, i: &ActionInvocation) -> Result<(), ActionError> {
        if i.subject() != &PerceivedRef::SelfActor
            || !i.targets().is_empty()
            || !matches!(
                i.parameters().get("op").and_then(Value::as_str),
                Some("stock" | "claim")
            )
        {
            return Err(invalid());
        }
        Ok(())
    }
    fn project_offer(
        &self,
        action: &DefinitionRef,
        q: &mush_core::LegalActionQuery,
        e: &mush_core::Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        if action != &self.action
            || q.truth_subject() != &(SubjectKey::Object { id: q.actor() })
            || q.projected_subject() != &PerceivedRef::SelfActor
        {
            return Ok(None);
        }
        let Some(f) = current(&e.world).filter(|f| authorized(&e.world, q.actor(), f)) else {
            return Ok(None);
        };
        let proposal = e
            .world
            .module_state::<crate::TacsimStore>()
            .and_then(|s| s.relay_proposals.get(&q.actor()))
            .filter(|p| {
                p.facility == f.facility
                    && p.vault == f.vault
                    && mush_economy::acts_for(&e.world, q.actor(), &p.owner)
                    && proposal_current(e, p)
            })
            .cloned();
        let mut guards = e
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| invalid())?;
        guards
            .add_definition(
                action.clone(),
                e.registry.definitions().ok_or_else(invalid)?,
            )
            .map_err(|_| invalid())?;
        Ok(Some(ProviderOfferDraft {
            fixed_parameters: json!({"facility":f.facility,"vault":f.vault,"proposal":proposal}),
            parameters: ProjectedParameterSchema::new(vec![ProjectedParameter {
                id: CanonicalId::new("op").map_err(|_| invalid())?,
                required: true,
                kind: ProjectedParameterKind::Choice {
                    choices: vec!["stock".into(), "claim".into()],
                },
            }])
            .map_err(|_| invalid())?,
            targets: ProjectedTargetSchema::None,
            required_channels: vec![],
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "relay-fabricate",
                "Fabricate relay machinery",
                "Issue finite foundry inputs or consume completed industrial output.",
            )
            .map_err(|_| invalid())?,
            guards: guards.finish().map_err(|_| invalid())?,
        }))
    }
    fn prepare_authoritative(
        &self,
        i: &ActionInvocation,
        t: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.validate_invocation(i)?;
        let p = i.parameters();
        let facility: ObjId =
            serde_json::from_value(p["facility"].clone()).map_err(|_| invalid())?;
        let vault: VaultId = serde_json::from_value(p["vault"].clone()).map_err(|_| invalid())?;
        let before = t
            .get(&self.book, &facility)
            .map_err(|_| invalid())?
            .cloned();
        let mut book = before.clone().unwrap_or(Fabrications {
            facility,
            stocked: false,
            receipts: BTreeMap::new(),
        });
        if book.facility != facility {
            return Err(invalid());
        }
        let mut order = 0;
        if p["op"] == "stock" {
            if book.stocked {
                return Err(err(
                    ActionErrorCode::Conflict,
                    "The finite foundry stock was already issued.",
                ));
            }
            let pool = mush_inventory::pool_key(
                EndpointRef(vault),
                &CanonicalId::new(BLANK).map_err(|_| invalid())?,
            )
            .map_err(|_| invalid())?;
            if t.get(&self.inventory.pools, &pool)
                .map_err(|_| invalid())?
                .is_some()
            {
                return Err(err(
                    ActionErrorCode::Integrity,
                    "Foundry stock receipt missing; repair required.",
                ));
            }
            {
                let (good, qty) = (BLANK, 8);
                self.inventory
                    .credit(
                        t,
                        EndpointRef(vault),
                        &CanonicalId::new(good).map_err(|_| invalid())?,
                        qty,
                    )
                    .map_err(|_| {
                        err(
                            ActionErrorCode::ProviderRejected,
                            "Foundry stock could not be issued.",
                        )
                    })?;
            }
            book.stocked = true;
        } else {
            let proposal: Proposal =
                serde_json::from_value(p["proposal"].clone()).map_err(|_| invalid())?;
            order = proposal.order;
            if proposal.actor != i.actor()
                || proposal.facility != facility
                || proposal.vault != vault
                || !book.stocked
                || book.receipts.len() >= MAX_PAIRS
                || book.receipts.contains_key(&order)
            {
                return Err(invalid());
            }
            let lot = t
                .get(&self.inventory.lots, &proposal.lot)
                .map_err(|_| invalid())?
                .cloned()
                .ok_or_else(invalid)?;
            let source = format!("wo/{order:016x}");
            if lot.endpoint() != EndpointRef(vault)
                || lot.commodity().as_str() != OUTPUT
                || lot.quantity() != 1
                || !matches!(lot.lineage(),Lineage::Origin{source:s} if s.provider=="industry" && s.kind=="workorder.output" && s.id==source && s.revision==Some(1))
            {
                return Err(invalid());
            }
            self.inventory
                .consume_product(t, &lot)
                .map_err(|_| invalid())?;
            book.receipts.insert(
                order,
                Receipt {
                    order,
                    lot: proposal.lot,
                    owner: proposal.owner,
                    actor: i.actor(),
                },
            );
        }
        book.check().map_err(|_| invalid())?;
        if before.is_some() {
            t.replace(&self.book, &facility, book)
        } else {
            t.insert(&self.book, &facility, book)
        }
        .map_err(|_| invalid())?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({"actor":i.actor().0,"order":order,"op":p["op"]}),
        )
        .map_err(|_| invalid())?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![i.principal().clone()]))
    }
}

fn id(s: &str) -> Result<CanonicalId, String> {
    CanonicalId::new(s).map_err(|e| e.to_string())
}
fn book(ctx: &Ctx<'_>, f: &Foundry) -> Result<Option<Fabrications>, String> {
    let handles = ctx
        .w
        .module_state::<crate::port::families::PortHandles>()
        .ok_or("Foundry unwired")?;
    let found = ctx
        .action_state(&handles.families.relay_fabrications, &f.facility)
        .map_err(|e| e.to_string())?
        .cloned();
    if found.as_ref().is_some_and(|b| b.facility != f.facility) {
        return Err("Foundry receipt identity drift".into());
    }
    Ok(found)
}
/// Create the authored lunar facility once; no pair or finished product is seeded.
/// An absent/held lane defers initialization, preserving the existing repair path.
pub fn boot(ctx: &mut Ctx<'_>) -> Result<(), String> {
    // Reverse physical audit runs at boot, before any rematerialization. A lost
    // mapping cannot turn an existing half back into an uncollected receipt.
    for object in ctx
        .w
        .iter()
        .filter(|o| o.behaviors.keys().any(|b| b.as_str() == CARGO_BEHAVIOR))
    {
        if !current(ctx.w)
            .is_some_and(|f| f.machines.values().any(|m| m.halves.contains(&object.id)))
        {
            return Err("Relay cargo exists without its fabrication mapping".into());
        }
    }

    let Some(store) = ctx.w.module_state::<crate::TacsimStore>() else {
        return Ok(());
    };
    if store.relay_foundry_held && store.relay_foundry.is_none() {
        return Err("Relay foundry document held; repair required".into());
    }
    if let Some(f) = current(ctx.w).cloned() {
        f.check()?;
        let facility = ctx
            .w
            .module_state::<mush_industry::IndustryStore>()
            .and_then(|s| s.facility(f.facility))
            .ok_or("Foundry facility missing")?;
        if facility.owner() != &PrincipalId::Organization(f.owner)
            || !facility.capabilities().contains(&id(KNOWHOW)?)
            || ctx
                .w
                .get(f.facility)
                .is_none_or(|o| o.location != Some(f.place))
        {
            return Err("Foundry identity drift".into());
        }
        let vault = ctx
            .w
            .module_state::<EconomyStore>()
            .and_then(|s| s.snapshot().vault(f.vault))
            .ok_or("Foundry vault missing")?;
        if vault.owner != PrincipalId::Organization(f.owner)
            || vault.backing != mush_economy::CustodyRef::Container(f.facility)
        {
            return Err("Foundry vault drift".into());
        }
        let b = book(ctx, &f)?;
        let handles = ctx
            .w
            .module_state::<crate::port::families::PortHandles>()
            .ok_or("Foundry unwired")?;
        let inventory = handles
            .inventory
            .as_ref()
            .ok_or("Foundry inventory unavailable")?;
        let key = mush_inventory::pool_key(EndpointRef(f.vault), &id(BLANK)?)
            .map_err(|e| e.to_string())?;
        if b.is_none()
            && ctx
                .action_state(&inventory.pools, &key)
                .map_err(|e| e.to_string())?
                .is_some()
        {
            return Err("Foundry stock receipt missing".into());
        }
        for (order, machines) in &f.machines {
            let receipt = b
                .as_ref()
                .and_then(|b| b.receipts.get(order))
                .ok_or("Machine has no fabrication receipt")?;
            if ctx
                .action_state(&inventory.lots, &receipt.lot)
                .map_err(|e| e.to_string())?
                .is_some()
            {
                return Err("Fabricated lot still exists".into());
            }
            if machines.installed.iter().all(Option::is_some) {
                let reg = mush_propagation::routing::registry(ctx.w)
                    .ok_or("Installed private registry missing")?;
                let pair = reg
                    .pairs
                    .get(&id(&format!("private_{order:016x}"))?)
                    .ok_or("Installed private pair missing")?;
                for (n, half) in pair.halves.iter().enumerate() {
                    let node = reg
                        .nodes
                        .get(&half.node)
                        .ok_or("Private mounting node missing")?;
                    if half.object != machines.halves[n]
                        || Some(node.object) != machines.installed[n]
                        || Some(node.operator) != machines.operator
                        || pair.class_policy != mush_sim::relay::ClassPolicy::Private
                        || pair.capacity_units_per_ic_second != 1
                    {
                        return Err("Private installation identity drift".into());
                    }
                }
            }
            for (n, half) in machines.halves.into_iter().enumerate() {
                // Missing/destroyed machinery stays lost. Its receipt never permits reminting.
                if machines.titled[n]
                    && ctx
                        .w
                        .module_state::<EconomyStore>()
                        .and_then(|s| s.snapshot().asset(half))
                        .is_none()
                {
                    return Err("Fabricated half title missing".into());
                }
            }
        }
        return Ok(());
    }
    let lane = id(crate::definitions::content::LANE_PACK)?;
    let Some(map) = store
        .port_maps()
        .find(|m| m.pack == lane && m.entry.as_str() == "luna_port")
    else {
        return Ok(());
    };
    let (Some(place), Some(org)) = (map.presentation, map.orgs.get(&id("orbital_access")?)) else {
        return Ok(());
    };
    let owner = OrgId(org.org);
    let scope = org.root_scope;
    // A lost owning document is not permission to seed a second facility/vault.
    let knowhow = id(KNOWHOW)?;
    if ctx
        .w
        .module_state::<mush_industry::IndustryStore>()
        .is_some_and(|s| {
            s.facilities()
                .into_iter()
                .any(|f| f.capabilities().contains(&knowhow))
        })
    {
        return Err("Relay foundry exists without its owner record".into());
    }
    let facility = ctx
        .w
        .create(
            mush_core::ObjKind::Thing,
            "Lunar entanglement foundry",
            mush_core::GOD,
            Some(ObjId(place)),
        )
        .map_err(|e| e.to_string())?;
    let record = mush_industry::FacilityRecord::new(
        facility,
        PrincipalId::Organization(owner),
        [id(KNOWHOW)?].into_iter().collect(),
        1,
    )
    .map_err(|e| e.to_string())?;
    mush_industry::store::install_authored_facility(ctx, record)?;
    let vault = crate::port::apply::alloc_vault_id(ctx)?;
    mush_economy::create_vault(
        ctx,
        mush_economy::Vault {
            id: vault,
            owner: PrincipalId::Organization(owner),
            scope: Some(mush_org::OrgScopeId(scope)),
            name: "Lunar entanglement foundry output and inputs".into(),
            backing: mush_economy::CustodyRef::Container(facility),
            status: mush_economy::VaultStatus::Active,
            revision: 1,
        },
    );
    save(
        ctx,
        Foundry {
            facility,
            place: ObjId(place),
            vault,
            owner,
            machines: BTreeMap::new(),
        },
    )
}
fn object(text: &str) -> Result<ObjId, String> {
    text.trim()
        .trim_start_matches('#')
        .parse::<u32>()
        .map(ObjId)
        .map_err(|_| "Expected an object number".into())
}
fn submit(ctx: &mut Ctx<'_>, op: &str, order: u64) -> Result<(), String> {
    let action = ctx
        .w
        .module_state::<crate::port::families::PortHandles>()
        .and_then(|h| h.relay_fabricate.clone())
        .ok_or("Foundry unwired")?;
    let key = mush_core::IdempotencyKeyText::parse_canonical(format!(
        "relay.fabricate/{}/{op}/{order}",
        ctx.actor.0
    ))
    .map_err(|e| e.to_string())?;
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        vec![],
        json!({"op":op}),
        key,
    );
    ctx.out.info(
        ctx.actor,
        "Foundry action submitted. Use +relaypair to inspect the committed receipt.",
    );
    Ok(())
}
fn claim(ctx: &mut Ctx<'_>, f: &Foundry) -> Result<(), String> {
    if !authorized(ctx.w, ctx.actor, f) {
        return Err("Lunar foundry authority and presence required.".into());
    }
    let order = mush_industry::WorkOrderId::parse(ctx.args.lhs().trim())
        .ok_or("Expected a work-order id")?;
    let lot = LotId(
        ctx.args
            .rhs()
            .unwrap_or_default()
            .trim()
            .parse()
            .map_err(|_| "Expected the completed output lot number")?,
    );
    let handles = ctx
        .w
        .module_state::<mush_industry::IndustryHandles>()
        .ok_or("Industry unwired")?;
    let wo = ctx
        .action_state(&handles.0.work_orders, &order)
        .map_err(|e| e.to_string())?
        .ok_or("No such work order")?;
    if !matches!(wo.state(), mush_industry::WorkOrderState::Completed { .. })
        || wo.facility() != f.facility
        || wo.recipe().pack().as_str() != PACK
        || wo.recipe().id().as_str() != RECIPE
        || wo.recipe().version() != 1
        || wo.quantity() != 1
        || wo.destination().endpoint() != EndpointRef(f.vault)
        || !mush_economy::acts_for(ctx.w, ctx.actor, wo.beneficiary())
    {
        return Err(
            "Complete one authorized pair at the lunar foundry before collecting its output."
                .into(),
        );
    }
    if !paid(ctx.w, wo) {
        return Err(
            "The production budget is still settling; try again after the industry sweep.".into(),
        );
    }
    let output = id(OUTPUT)?;
    if ctx
        .w
        .module_state::<mush_market::MarketStore>()
        .is_some_and(|s| s.lot_listing_of(EndpointRef(f.vault), &output).is_some())
    {
        return Err("Withdraw the output sale before fabrication.".into());
    }
    let proposal = Proposal {
        actor: ctx.actor,
        facility: f.facility,
        vault: f.vault,
        order: order.0,
        lot,
        owner: wo.beneficiary().clone(),
    };
    let store = ctx
        .w
        .module_state_mut::<crate::TacsimStore>()
        .ok_or("TacSim missing")?;
    if store.relay_proposals.len() >= 64 && !store.relay_proposals.contains_key(&ctx.actor) {
        return Err("Foundry offer capacity reached".into());
    }
    store.relay_proposals.insert(ctx.actor, proposal);
    submit(ctx, "claim", order.0)
}
fn collect(ctx: &mut Ctx<'_>, mut f: Foundry) -> Result<(), String> {
    let order = mush_industry::WorkOrderId::parse(ctx.args.lhs().trim())
        .ok_or("Expected a work-order id")?
        .0;
    let receipt = book(ctx, &f)?
        .and_then(|b| b.receipts.get(&order).cloned())
        .ok_or("No committed fabrication receipt")?;
    if !mush_economy::acts_for(ctx.w, ctx.actor, &receipt.owner)
        || ctx.w.get(ctx.actor).and_then(|o| o.location) != Some(f.place)
    {
        return Err("Output owner must collect at the lunar foundry.".into());
    }
    let economy = ctx
        .w
        .module_state::<EconomyStore>()
        .ok_or("Economy missing")?;
    if economy.is_frozen() {
        return Err("Economy held for repair".into());
    }
    let issuer = mush_industry::industry_title_issuer();
    if !economy.is_title_issuer(&issuer) {
        return Err("Foundry title issuer unavailable".into());
    }
    if !f.machines.contains_key(&order) {
        ctx.w
            .next_id()
            .checked_add(2)
            .ok_or("Object ids exhausted")?;
        let mut halves = [ObjId(0); 2];
        for (n, half) in halves.iter_mut().enumerate() {
            *half = ctx
                .w
                .create(
                    mush_core::ObjKind::Thing,
                    &format!("Relay pair {order:016x} half {} (5 tonnes)", n + 1),
                    mush_core::GOD,
                    Some(f.place),
                )
                .map_err(|e| e.to_string())?;
        }
        for half in halves {
            bind(ctx, half, CARGO_BEHAVIOR)?;
        }
        f.machines.insert(
            order,
            Machines {
                halves,
                titled: [false, false],
                installed: [None, None],
                dismounted: None,
                operator: None,
                discovery: None,
            },
        );
        save(ctx, f.clone())?;
    }
    let mut machine = f
        .machines
        .get(&order)
        .cloned()
        .ok_or("Fabrication mapping missing")?;
    let halves = machine.halves;
    for (n, half) in halves.into_iter().enumerate() {
        if !machine.titled[n] {
            mush_economy::issue_title(ctx, half, receipt.owner.clone(), None, issuer.clone(), None)
                .map_err(|e| e.to_string())?;
            machine.titled[n] = true;
            f.machines.insert(order, machine.clone());
            save(ctx, f.clone())?;
        }
    }
    ctx.out.info(ctx.actor,format!("Pair {order:016x}: titled halves #{} and #{}. Load them into a checked cargo bay before shipping.",halves[0].0,halves[1].0));
    Ok(())
}
pub fn known_cargo(w: &World, half: ObjId) -> bool {
    if w.get(half)
        .is_some_and(|o| o.behaviors.keys().any(|b| b.as_str() == CARGO_BEHAVIOR))
    {
        return true;
    }
    current(w).is_some_and(|f| f.machines.values().any(|m| m.halves.contains(&half)))
}
pub fn cargo(w: &World, half: ObjId) -> bool {
    current(w).is_some_and(|f| {
        f.machines.values().any(|m| {
            m.halves
                .iter()
                .enumerate()
                .any(|(n, h)| *h == half && m.titled[n] && m.installed[n].is_none())
        })
    }) && w
        .get(half)
        .is_some_and(|o| !o.flags.contains(mush_core::Flags::GOING))
        && w.module_state::<EconomyStore>()
            .and_then(|s| s.snapshot().asset(half))
            .is_some_and(|a| a.lifecycle == mush_economy::AssetLifecycle::Active)
}
fn owns(w: &World, actor: ObjId, half: ObjId) -> bool {
    w.module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().asset(half))
        .is_some_and(|a| {
            !a.frozen
                && a.stolen.is_none()
                && a.lifecycle == mush_economy::AssetLifecycle::Active
                && mush_economy::acts_for(w, actor, &a.title_owner)
        })
}
fn freight(ctx: &mut Ctx<'_>, load: bool) -> Result<(), String> {
    let half = object(ctx.args.lhs())?;
    if !cargo(ctx.w, half) || !owns(ctx.w, ctx.actor, half) {
        return Err("No usable titled relay cargo.".into());
    }
    let helm =
        crate::port::coop::helm_of(ctx.w, ctx.actor).ok_or("A real carrier helm is required")?;
    let view = crate::port::ship::hull_view(ctx.w, helm.hull).ok_or("Carrier unavailable")?;
    if crate::port::voyage::voyage_blocks(ctx.w, helm.hull) {
        return Err("Cancel the filed clearance before moving cargo".into());
    }
    let crate::port::ship::Berth::Docked { map, .. } =
        crate::port::ship::berth_of(ctx.w, helm.hull)
    else {
        return Err("Cargo moves only at a real dock".into());
    };
    let gallery = ObjId(map.presentation.ok_or("Dock gallery missing")?);
    let (bay, room) = view.bay().ok_or("A checked cargo bay is required")?;
    let slot = mush_sim::SlotId::new(bay.slot.id.as_str()).ok_or("Invalid bay slot")?;
    if ctx.w.get(ctx.actor).and_then(|o| o.location) != Some(gallery) {
        return Err("Stand at the carrier's dock gallery".into());
    }
    if load {
        if ctx.w.get(half).and_then(|o| o.location) != Some(gallery) {
            return Err("Relay cargo is not at this dock".into());
        }
        mush_sim::place_interior(ctx, half, room, Some(slot)).map_err(|e| e.to_string())?;
    } else {
        if !crate::port::vehicle::rigs_in_bay(ctx.w, &view)
            .iter()
            .any(|r| r.tv == half)
        {
            return Err("Relay cargo is not aboard this carrier".into());
        }
        mush_sim::unplace(ctx, half, "Relay cargo discharged at dock")
            .map_err(|e| e.to_string())?;
        ctx.w.move_to(half, gallery).map_err(|e| e.to_string())?;
    }
    ctx.out.pin_deferred();
    ctx.out.info(
        ctx.actor,
        format!(
            "Relay cargo #{} {} (5 tonnes, one bay slot).",
            half.0,
            if load { "loaded" } else { "unloaded" }
        ),
    );
    Ok(())
}
fn command(ctx: &mut Ctx<'_>) -> Result<(), String> {
    if current(ctx.w).is_none() {
        boot(ctx)?;
    }
    let f = current(ctx.w)
        .cloned()
        .ok_or("The lunar relay foundry is unavailable")?;
    if !ready(ctx.w, &f) {
        return Err("The lunar relay foundry needs repair".into());
    }
    if ctx.switches.has("install") {
        return install_half(ctx, f);
    }
    if ctx.switches.has("discover") {
        return discover(ctx, f);
    }
    if ctx.switches.has("stock") {
        if !authorized(ctx.w, ctx.actor, &f) {
            return Err("Lunar foundry authority and presence required.".into());
        }
        return submit(ctx, "stock", 0);
    }
    if ctx.switches.has("claim") {
        return claim(ctx, &f);
    }
    if ctx.switches.has("collect") {
        return collect(ctx, f);
    }
    if ctx.switches.has("load") {
        return freight(ctx, true);
    }
    if ctx.switches.has("unload") {
        return freight(ctx, false);
    }
    ctx.out.info(ctx.actor,format!("Lunar relay foundry #{} at #{}; org:{}; vault {}. Recipe {PACK}/{RECIPE}/1 consumes one finite relay blank and two instrument boards. No installed link is produced.",f.facility.0,f.place.0,f.owner.0,f.vault.0));
    if authorized(ctx.w, ctx.actor, &f) {
        let b = book(ctx, &f)?;
        ctx.out.info(
            ctx.actor,
            format!(
                "Original blank stock issued: {}; fabrication receipts: {} / {MAX_PAIRS}.",
                b.as_ref().is_some_and(|b| b.stocked),
                b.as_ref().map_or(0, |b| b.receipts.len())
            ),
        );
        if let Some(b) = b {
            for r in b.receipts.values() {
                ctx.out.info(
                    ctx.actor,
                    format!(
                        "Work order {:016x}, consumed lot {}, collected: {}",
                        r.order,
                        r.lot.0,
                        f.machines.contains_key(&r.order)
                    ),
                );
            }
        }
    }
    Ok(())
}
pub fn register(reg: &mut mush_core::Registrar<'_>) -> Result<(), mush_core::ModError> {
    reg.persist_delegation(
        crate::NS,
        "faction",
        "R9 private relay discovery is consumed through the faction owner API",
    )?;
    reg.persist_delegation(
        crate::NS,
        "npc",
        "R9 physical relay inspection files an actual knowledge observation",
    )?;
    reg.persist_delegation(
        crate::NS,
        "industry",
        "R9 authored lunar foundry facility uses the industry's owner API",
    )?;
    let mut cargo = mush_core::BehaviorSpec::new(
        mush_core::BehaviorId::new(CARGO_BEHAVIOR)
            .map_err(|e| mush_core::ModError::msg(e.to_string()))?,
    );
    cargo.allowed_kinds = vec![mush_core::ObjKind::Thing];
    reg.behavior(cargo)?;
    reg.detach_gate(std::sync::Arc::new(CargoGate));
    reg.hook(
        mush_core::HookPoint::CarryAttempt,
        std::sync::Arc::new(CargoGate),
    );
    reg.hook(
        mush_core::HookPoint::MoveAttempt,
        std::sync::Arc::new(CargoGate),
    );
    reg.classify_commands(mush_core::CommandCompatibility::Authoritative);
    let result=reg.command(mush_core::CommandSpec{name:"+relaypair".into(),shape:mush_core::ArgShape::EqPair,
        switches:["stock","claim","collect","load","unload","install","discover"].into_iter().map(Into::into).collect(),
        help:"+relaypair; /stock; /claim <work-order>=<output-lot>; /collect <work-order>; /load #half; /unload #half; /install #half=#mount org:id; /discover #half".into(),..Default::default()},
        std::sync::Arc::new(|ctx: &mut Ctx<'_>|command(ctx).map_err(mush_core::CmdError::msg)));
    reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
    result
}

fn bind(ctx: &mut Ctx<'_>, object: ObjId, behavior: &str) -> Result<(), String> {
    ctx.w
        .attach_behavior(
            object,
            mush_core::BehaviorId::new(behavior).map_err(|e| e.to_string())?,
            mush_core::BehaviorBinding {
                schema_version: 1,
                attached_by: ctx.actor,
                attached_at: ctx.w.now(),
            },
        )
        .map_err(|e| e.to_string())
}
fn healthy(w: &World, object: ObjId) -> bool {
    mush_tactics::module::unit_of(w, object).is_some_and(|u| {
        ["core", "power"].iter().all(|s| {
            let (Ok(section), Ok(cell)) = (
                mush_tactics::SectionId::new(s),
                mush_tactics::CellId::new("internals"),
            ) else {
                return false;
            };
            u.cell(&mush_tactics::CellAddr::Section { section, cell })
                .is_some_and(|c| c.current > 0)
        })
    })
}
fn install_half(ctx: &mut Ctx<'_>, mut f: Foundry) -> Result<(), String> {
    let half = object(ctx.args.lhs())?;
    let mut args = ctx.args.rhs().unwrap_or_default().split_whitespace();
    let node = object(
        args.next()
            .ok_or("Name an owned mounting object and operator org")?,
    )?;
    let operator = OrgId(
        args.next()
            .and_then(|s| s.strip_prefix("org:"))
            .and_then(|s| s.parse().ok())
            .ok_or("Name the independent operator as org:id")?,
    );
    let (order, n) = f
        .machines
        .iter()
        .find_map(|(order, m)| {
            m.halves
                .iter()
                .position(|h| *h == half)
                .map(|n| (*order, n))
        })
        .ok_or("No such fabricated half")?;
    let mut m = f
        .machines
        .get(&order)
        .cloned()
        .ok_or("Fabrication missing")?;
    if m.dismounted.is_some() {
        return Err("A removed Fleet pair must use +relayfleet/refit".into());
    }
    if m.installed[n].is_some() {
        return Err("That half is already installed".into());
    }
    let here = ctx
        .w
        .get(ctx.actor)
        .and_then(|o| o.location)
        .filter(|r| {
            ctx.w.get(*r).is_some_and(|o| {
                o.kind == mush_core::ObjKind::Room && !o.flags.contains(mush_core::Flags::GOING)
            })
        })
        .ok_or("No physical room")?;
    let mount = ctx.w.get(node).ok_or("No mounting object")?;
    if !cargo(ctx.w, half)
        || !owns(ctx.w, ctx.actor, half)
        || ctx.w.get(half).and_then(|o| o.location) != Some(here)
        || mount.flags.contains(mush_core::Flags::GOING)
        || mount.kind != mush_core::ObjKind::Thing
        || mount.location != Some(here)
        || !mush_core::perm::controls(ctx.w, ctx.actor, node)
        || m.halves.contains(&node)
        || f.machines
            .values()
            .any(|m| m.installed.contains(&Some(node)))
        || mush_tactics::module::unit_of(ctx.w, node).is_some()
        || mount.behaviors.keys().any(|b| {
            [
                mush_sim::relay::NODE_BEHAVIOR,
                mush_sim::relay::HALF_BEHAVIOR,
            ]
            .contains(&b.as_str())
        })
        || !mush_economy::acts_for(ctx.w, ctx.actor, &PrincipalId::Organization(operator))
        || m.operator.is_some_and(|o| o != operator)
    {
        return Err(
            "Local cargo title, owned empty mount and operator authority are required.".into(),
        );
    }
    let mut registry = mush_propagation::routing::registry(ctx.w)
        .cloned()
        .ok_or("Relay registry missing")?;
    if operator == registry.operator {
        return Err("Use an independent private operator".into());
    }
    if registry.nodes.len() + 2 > mush_sim::relay::MAX_NODES
        || registry.pairs.len() + 1 > mush_sim::relay::MAX_PAIRS
    {
        return Err("Relay registry capacity reached".into());
    }
    if let Some(other) = m.installed[1 - n] {
        if ctx.w.get(other).and_then(|o| o.location) == Some(here) {
            return Err("The two halves need distinct places".into());
        }
        if !healthy(ctx.w, other)
            || !owns(ctx.w, ctx.actor, m.halves[1 - n])
            || ctx.w.get(m.halves[1 - n]).is_none_or(|h| {
                h.location != Some(other) || h.flags.contains(mush_core::Flags::GOING)
            })
        {
            return Err(
                "The other mounting station or its titled half is damaged, missing or unavailable"
                    .into(),
            );
        }
    }
    registry.reconcile(ctx.w)?;
    mush_sim::relay::check_installer(ctx)?;
    registry
        .revision
        .checked_add(1)
        .ok_or("Relay revisions exhausted")?;
    if registry
        .pairs
        .contains_key(&id(&format!("private_{order:016x}"))?)
    {
        return Err("Private pair identity already registered".into());
    }
    let catalog = crate::builtin_definitions()?;
    let spec = catalog
        .units
        .get(crate::relay_loss::STATION)
        .ok_or("Station schema unavailable")?;
    m.installed[n] = Some(node);
    m.operator = Some(operator);
    if let [Some(a), Some(b)] = m.installed {
        let pair_id = id(&format!("private_{order:016x}"))?;
        let mut halves = Vec::new();
        for (index, mount) in [a, b].into_iter().enumerate() {
            let node_id = id(&format!("private_{order:016x}_{index}"))?;
            if registry.nodes.contains_key(&node_id) {
                return Err("Private node identity already registered".into());
            }
            let place = ctx
                .w
                .get(mount)
                .and_then(|o| o.location)
                .filter(|p| {
                    ctx.w.get(*p).is_some_and(|o| {
                        o.kind == mush_core::ObjKind::Room
                            && !o.flags.contains(mush_core::Flags::GOING)
                    })
                })
                .ok_or("Mount location missing")?;
            if ctx.w.get(m.halves[index]).is_none_or(|o| {
                o.behaviors.keys().any(|b| {
                    [
                        mush_sim::relay::NODE_BEHAVIOR,
                        mush_sim::relay::HALF_BEHAVIOR,
                    ]
                    .contains(&b.as_str())
                })
            }) {
                return Err("Private half has a conflicting installation".into());
            }
            registry.nodes.insert(
                node_id.clone(),
                mush_sim::relay::RelayNode {
                    object: mount,
                    place,
                    label: format!("Private relay {order:016x}/{}", index + 1),
                    operator,
                    coverage: vec![place],
                },
            );
            halves.push(mush_sim::relay::RelayHalf {
                id: id(&format!("private_{order:016x}_half_{index}"))?,
                object: m.halves[index],
                node: node_id,
                enabled: true,
                powered: true,
                staffed: true,
                destroyed: false,
                damaged: false,
            });
        }
        registry.pairs.insert(
            pair_id,
            mush_sim::relay::RelayPair {
                halves: halves.try_into().map_err(|_| "Pair identity error")?,
                capacity_units_per_ic_second: 1,
                class_policy: mush_sim::relay::ClassPolicy::Private,
            },
        );
        registry.revision = registry
            .revision
            .checked_add(1)
            .ok_or("Relay revisions exhausted")?;
        registry.validate()?;
    }
    f.machines.insert(order, m.clone());
    f.check()?;
    // All topology, identity, permissions and bounds were checked before the
    // first physical mutation. These post-images share the durable legacy batch.
    mush_tactics::module::create_unit(ctx, node, spec)?;
    ctx.w.move_to(half, node).map_err(|e| e.to_string())?;
    if let [Some(a), Some(b)] = m.installed {
        for (index, mount) in [a, b].into_iter().enumerate() {
            bind(ctx, mount, mush_sim::relay::NODE_BEHAVIOR)?;
            bind(ctx, m.halves[index], mush_sim::relay::HALF_BEHAVIOR)?;
        }
        mush_sim::relay::install(ctx, registry)?;
    }
    save(ctx, f)?;
    ctx.out.info(ctx.actor,format!("Half #{} installed at #{}. Pair private_{order:016x} activates when both distinct stations are installed.",half.0,node.0));
    Ok(())
}
fn discover(ctx: &mut Ctx<'_>, mut f: Foundry) -> Result<(), String> {
    let half = object(ctx.args.lhs())?;
    let (order, m) = f
        .machines
        .iter()
        .find(|(_, m)| m.halves.contains(&half))
        .map(|(o, m)| (*o, m.clone()))
        .ok_or("No visible private relay")?;
    let here = ctx
        .w
        .get(ctx.actor)
        .and_then(|o| o.location)
        .ok_or("No physical location")?;
    if !m
        .installed
        .iter()
        .flatten()
        .any(|n| ctx.w.get(*n).and_then(|o| o.location) == Some(here))
        || m.installed.iter().any(Option::is_none)
    {
        return Err("Inspect an installed private relay in person.".into());
    }
    if let Some(front) = m.discovery {
        ctx.out.info(
            ctx.actor,
            format!("This discovery is already recorded in anti-proliferation front #{front}."),
        );
        return Ok(());
    }
    let source = format!("relay.private/{order:016x}");
    mush_npc::file_observation(ctx,mush_npc::IntelRecord{informant:ctx.actor,principal:mush_faction::Principal::Character(ctx.actor),subject:half,
        subject_name:format!("Private relay {order:016x}"),kind:mush_npc::IntelKind::ServiceEvidence,
        detail:"Physical inspection confirms a private entangled pair, independent of the Malapert Exchange.".into(),room_name:format!("Room #{}",here.0),
        provenance:source.clone(),ts:ctx.w.now()})?;
    let front = mush_faction::record_relay_discovery(
        ctx,
        f.owner,
        mush_canon::CanonDraft {
            kind: "relay.private_discovered".into(),
            summary: format!(
                "A private entangled relay pair {order:016x} has been physically discovered; the Concord's anti-proliferation campaign advances."
            ),
            entities: vec![ctx.actor, half],
            scope: mush_canon::CanonScope::Public,
            ic_time: ctx.w.world_ic_now().at,
            source,
            detail: json!({"order":order,"half":half.0}),
        },
    )?;
    if let Some(m) = f.machines.get_mut(&order) {
        m.discovery = Some(front.0);
    }
    save(ctx, f)?;
    ctx.out.info(
        ctx.actor,
        format!(
            "Discovery filed and consumed by anti-proliferation front #{}.",
            front.0
        ),
    );
    Ok(())
}
struct CargoGate;
fn protected(w: &World, o: ObjId) -> bool {
    if known_cargo(w, o) {
        return true;
    }
    current(w).is_some_and(|f| {
        f.machines.values().any(|m| {
            m.halves.contains(&o)
                || m.installed.contains(&Some(o))
                || m.dismounted.is_some_and(|nodes| nodes.contains(&o))
        })
    })
}
impl mush_core::EventHook for CargoGate {
    fn carry_attempt(
        &self,
        ctx: &mush_core::HookReadContext<'_>,
        _: ObjId,
        o: ObjId,
        _: ObjId,
    ) -> Result<(), mush_proto::RichText> {
        if protected(ctx.world(), o) {
            Err(mush_proto::RichText::tag(
                "error",
                "Relay machinery moves through its checked carrier bay workflow.",
            ))
        } else {
            Ok(())
        }
    }
    fn move_attempt(
        &self,
        ctx: &mush_core::HookReadContext<'_>,
        o: ObjId,
        _: Option<ObjId>,
        _: ObjId,
    ) -> Result<(), mush_proto::RichText> {
        if protected(ctx.world(), o) {
            Err(mush_proto::RichText::tag(
                "error",
                "Relay machinery moves through its checked carrier bay workflow.",
            ))
        } else {
            Ok(())
        }
    }
}

impl mush_core::DetachGate for CargoGate {
    fn check(
        &self,
        w: &World,
        _: &mush_core::Registry,
        _: ObjId,
        obj: ObjId,
        behavior: &mush_core::BehaviorId,
    ) -> Result<(), String> {
        if behavior.as_str() == CARGO_BEHAVIOR && known_cargo(w, obj) {
            Err("Relay cargo identity is still referenced by its fabrication receipt".into())
        } else {
            Ok(())
        }
    }
}

/// The Fleet yard fits both actually fabricated halves while the real hull is
/// docked at Luna. This moves local hardware into mounts; it never creates output.
pub(crate) fn fit_mobile(
    ctx: &mut Ctx<'_>,
    order: u64,
    hull: ObjId,
    operator: OrgId,
) -> Result<(), String> {
    let mut f = current(ctx.w).cloned().ok_or("Foundry missing")?;
    let mut m = f
        .machines
        .get(&order)
        .cloned()
        .ok_or("Fabricated pair missing")?;
    let (_, room) = crate::relay_mobile::capability(ctx.w, hull)
        .ok_or("A checked capital relay socket is required")?;
    let crate::port::ship::Berth::Docked { map, .. } = crate::port::ship::berth_of(ctx.w, hull)
    else {
        return Err("Refit requires the real Lunar berth".into());
    };
    if map.entry.as_str() != "luna_port"
        || map.presentation != Some(f.place.0)
        || ctx.w.get(ctx.actor).and_then(|o| o.location) != Some(f.place)
        || crate::port::voyage::voyage_blocks(ctx.w, hull)
        || m.installed.iter().any(Option::is_some)
        || m.halves.iter().any(|h| {
            !cargo(ctx.w, *h)
                || !owns(ctx.w, ctx.actor, *h)
                || ctx.w.get(*h).and_then(|o| o.location) != Some(f.place)
        })
        || !mush_economy::acts_for(ctx.w, ctx.actor, &PrincipalId::Organization(operator))
    {
        return Err("Refit needs both titled, usable halves on the Lunar dock, an idle cruiser and Fleet authority".into());
    }
    let unit = mush_tactics::module::unit_of(ctx.w, hull).ok_or("Capital structure missing")?;
    if ["hull", "power"]
        .into_iter()
        .any(|s| crate::relay_loss::internals(unit, s).is_none_or(|n| n == 0))
    {
        return Err("Repair the cruiser before refitting".into());
    }
    let mut r = mush_propagation::routing::registry(ctx.w)
        .cloned()
        .ok_or("Registry missing")?;
    r.reconcile(ctx.w)?;
    mush_sim::relay::check_installer(ctx)?;
    let pair = id(&format!("private_{order:016x}"))?;
    if r.pairs.contains_key(&pair)
        || r.pairs.len() + 1 > mush_sim::relay::MAX_PAIRS
        || r.nodes.len() + 2 > mush_sim::relay::MAX_NODES
        || operator == r.operator
    {
        return Err("No independent relay installation capacity".into());
    }
    let revision = r
        .revision
        .checked_add(1)
        .ok_or("Relay revisions exhausted")?;
    let catalog = crate::builtin_definitions()?;
    let spec = catalog
        .units
        .get(crate::relay_loss::STATION)
        .ok_or("Station schema missing")?;
    let node_ids = [
        id(&format!("private_{order:016x}_0"))?,
        id(&format!("private_{order:016x}_1"))?,
    ];
    if node_ids
        .iter()
        .any(|n| r.nodes.contains_key(n) || r.losses.contains_key(n))
    {
        return Err("Fleet node identity conflict or terminal loss".into());
    }
    let places = [f.place, room];
    if let Some(nodes) = m.dismounted {
        for (n, mount) in nodes.into_iter().enumerate() {
            if ctx.w.get(mount).and_then(|o| o.location) != Some(places[n])
                || !healthy(ctx.w, mount)
            {
                return Err("The original Fleet cradle needs repair".into());
            }
        }
    }
    let mut halves = Vec::new();
    for (n, place) in places.into_iter().enumerate() {
        let node_id = node_ids[n].clone();
        let mount = if let Some(nodes) = m.dismounted {
            nodes[n]
        } else {
            let mount = ctx
                .w
                .create(
                    mush_core::ObjKind::Thing,
                    if n == 0 {
                        "Fleet Lunar relay cradle"
                    } else {
                        "Fleet mobile relay cradle"
                    },
                    ctx.actor,
                    Some(place),
                )
                .map_err(|e| e.to_string())?;
            mush_tactics::module::create_unit(ctx, mount, spec)?;
            mount
        };
        ctx.w
            .move_to(m.halves[n], mount)
            .map_err(|e| e.to_string())?;
        bind(ctx, mount, mush_sim::relay::NODE_BEHAVIOR)?;
        bind(ctx, m.halves[n], mush_sim::relay::HALF_BEHAVIOR)?;
        m.installed[n] = Some(mount);
        r.nodes.insert(
            node_id.clone(),
            mush_sim::relay::RelayNode {
                object: mount,
                place,
                label: if n == 0 {
                    "Concord Fleet lunar anchor".into()
                } else {
                    "Concord Fleet mobile half".into()
                },
                operator,
                coverage: vec![place],
            },
        );
        halves.push(mush_sim::relay::RelayHalf {
            id: id(&format!("private_{order:016x}_half_{n}"))?,
            object: m.halves[n],
            node: node_id,
            enabled: true,
            powered: true,
            staffed: true,
            destroyed: false,
            damaged: false,
        });
    }
    r.pairs.insert(
        pair,
        mush_sim::relay::RelayPair {
            halves: halves
                .try_into()
                .map_err(|_| "Fleet pair identity failure")?,
            capacity_units_per_ic_second: 1,
            class_policy: mush_sim::relay::ClassPolicy::Private,
        },
    );
    r.revision = revision;
    m.operator = Some(operator);
    m.dismounted = None;
    f.machines.insert(order, m);
    mush_sim::relay::install(ctx, r)?;
    save(ctx, f)
}

/// Remove at the original yard, retaining the manufactured identities and cradles.
/// Lost hardware cannot be removed and remounted to erase a terminal loss.
pub(crate) fn remove_mobile(ctx: &mut Ctx<'_>, order: u64, hull: ObjId) -> Result<(), String> {
    let mut f = current(ctx.w).cloned().ok_or("Foundry absent")?;
    let mut m = f
        .machines
        .get(&order)
        .cloned()
        .ok_or("Fabrication absent")?;
    let [Some(a), Some(b)] = m.installed else {
        return Err("Fleet pair is already removed".into());
    };
    let crate::port::ship::Berth::Docked { map, .. } = crate::port::ship::berth_of(ctx.w, hull)
    else {
        return Err("Removal requires the real Lunar berth".into());
    };
    if map.entry.as_str() != "luna_port"
        || ctx.w.get(ctx.actor).and_then(|o| o.location) != Some(f.place)
        || crate::port::voyage::voyage_blocks(ctx.w, hull)
    {
        return Err("Removal requires an idle cruiser and its commander on the Lunar dock".into());
    }
    let mut r = mush_propagation::routing::registry(ctx.w)
        .cloned()
        .ok_or("Registry absent")?;
    r.reconcile(ctx.w)?;
    mush_sim::relay::check_installer(ctx)?;
    let pair_id = id(&format!("private_{order:016x}"))?;
    let pair = r.pairs.get(&pair_id).cloned().ok_or("Fleet pair absent")?;
    if pair.halves.iter().enumerate().any(|(n, h)| {
        h.object != m.halves[n]
            || r.nodes
                .get(&h.node)
                .is_none_or(|v| Some(v.object) != m.installed[n])
            || h.destroyed
            || r.losses.contains_key(&h.node)
    }) {
        return Err("Lost or inconsistent Fleet hardware cannot be removed for reuse".into());
    }
    r.revision = r
        .revision
        .checked_add(1)
        .ok_or("Relay revisions exhausted")?;
    let node_binding =
        mush_core::BehaviorId::new(mush_sim::relay::NODE_BEHAVIOR).map_err(|e| e.to_string())?;
    let half_binding =
        mush_core::BehaviorId::new(mush_sim::relay::HALF_BEHAVIOR).map_err(|e| e.to_string())?;
    for (n, mount) in [a, b].into_iter().enumerate() {
        ctx.w
            .detach_behavior(mount, &node_binding)
            .map_err(|e| e.to_string())?;
        ctx.w
            .detach_behavior(m.halves[n], &half_binding)
            .map_err(|e| e.to_string())?;
        ctx.w
            .move_to(m.halves[n], f.place)
            .map_err(|e| e.to_string())?;
        r.nodes.remove(&pair.halves[n].node);
    }
    r.pairs.remove(&pair_id);
    m.installed = [None, None];
    m.operator = None;
    m.dismounted = Some([a, b]);
    f.machines.insert(order, m);
    mush_sim::relay::install(ctx, r)?;
    save(ctx, f)
}
