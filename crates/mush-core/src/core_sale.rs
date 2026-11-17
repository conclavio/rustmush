//! **The closed physical image of a stored core's sale** (doc 47 unit CO.13; R-47-5's "or
//! sold"). One fixed operation: a core in its seller's own hands goes into the buyer's shop, in
//! the same coordinator union as the economy's closed sale leg (its money and its title,
//! `mush_economy::registry_filing::core_sale`). Core owns the object images; the trusted tacsim
//! policy only names which core, whose hands and which shop, and repeats that read before
//! custody.
//!
//! The shape is the completed-core delivery's (`core_delivery`) run the other way: three exact
//! object pre-images (the core, its seller, the shop), the real registered carry hooks
//! evaluated against committed pre-images and retained through custody, and one record per
//! sale in an owner-scoped state family the producer's boot audit pairs with the economy's
//! receipts. No provider supplies a World postimage; no caller supplies a source or a
//! destination.
//!
//! **The resale** (doc 48 unit EX.2b, Q-48-47) is the second closed plan beside this buy-back,
//! in [`resale`]: one player sells a stored core to another player named in a private offer. Its
//! image is this one with the one rule the buy-back refuses, a **player** destination (the
//! buyer's own hands), admitted for the resale's fixed action only (the private destination
//! rule).
use crate::action::{ActionInvocation, PerceivedRef, PrincipalId};
use crate::action_state::{
    ActionTxn, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId, TypedStateLens,
};
use crate::authoritative::ReplayWrite;
use crate::core_delivery::CarryCheck;
use crate::mechanics::{CanonicalId, DefinitionRef, canonical_json_bytes_with_limit};
use crate::module::HookReg;
use crate::participant::ParticipantError;
use crate::reserved_shell::ShellObjectStorage;
use crate::{CommitToken, Flags, ModError, ModuleCtx, Obj, ObjId, ObjKind, Registrar, World};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::Arc;

/// The sale action's id (tacsim-owned).
pub const ACTION: &str = "sell_stored_core";
/// The only module that may register the sale service.
pub const OWNER: &str = "tacsim";
const FAMILY: &str = "core_sales";

pub mod resale;
const RECORD_BYTES: usize = 4_096;
// A private local ceiling, as the delivery's: three objects, never the bundle cap.
const IMAGE_BYTES: usize = 1_048_576;

/// The exact sale action: tacsim's [`ACTION`] at version 1.
#[must_use]
pub fn fixed_action(def: &DefinitionRef) -> bool {
    def.key().owner.as_str() == OWNER
        && def.key().kind.as_str() == "action"
        && def.key().id.as_str() == ACTION
        && def.version() == 1
}

/// What the producer asks: sell this core. Not a wire format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaleRequest {
    core: ObjId,
}
impl SaleRequest {
    #[must_use]
    pub const fn new(core: ObjId) -> Self {
        Self { core }
    }
    #[must_use]
    pub const fn core(&self) -> ObjId {
        self.core
    }
}

/// **The trusted policy's read**: the core, the seller holding it and the shop it goes into.
/// Produced only by the registered policy, read again before custody; confers no authority
/// outside this service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaleFacts {
    core: ObjId,
    seller: ObjId,
    shop: ObjId,
}
impl SaleFacts {
    #[must_use]
    pub const fn new(core: ObjId, seller: ObjId, shop: ObjId) -> Self {
        Self { core, seller, shop }
    }
    #[must_use]
    pub const fn core(&self) -> ObjId {
        self.core
    }
    #[must_use]
    pub const fn seller(&self) -> ObjId {
        self.seller
    }
    #[must_use]
    pub const fn shop(&self) -> ObjId {
        self.shop
    }
}

/// **The trusted sale policy** (tacsim's): whether `seller` may sell `request`'s core now, and
/// into which shop. A pure read of the world: the service calls it at prepare and again before
/// custody, and refuses unless both reads agree exactly. A concrete policy proves the core is a
/// stored managed core in the seller's own hands (not fitted, not destroyed) and names the
/// buy-back vendor's shop.
pub trait CoreSalePolicy: Send + Sync + 'static {
    /// # Errors
    /// Any refusal; the sale does not happen.
    fn read(
        &self,
        world: &World,
        seller: ObjId,
        request: &SaleRequest,
    ) -> Result<SaleFacts, ParticipantError>;
}

/// One sale's record: the operation, the action, the core, its seller, the shop and the UTC
/// second. Kept in the tacsim-owned `core_sales` family, keyed by the operation's hex.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SaleRecord {
    format: u16,
    operation: CommitToken,
    action: DefinitionRef,
    core: ObjId,
    seller: ObjId,
    shop: ObjId,
    at_utc: i64,
}
impl SaleRecord {
    fn validate(&self) -> Result<(), ParticipantError> {
        if self.format != 1
            || !fixed_action(&self.action)
            || self.core.0 == 0
            || self.seller.0 == 0
            || self.shop.0 == 0
            || self.core == self.seller
            || self.core == self.shop
            || self.seller == self.shop
        {
            return Err(ParticipantError::Authority);
        }
        bounded(self)?;
        Ok(())
    }
}

/// **A prepared or replayed sale**, minted only by this service: the producer pairs it with the
/// economy's receipt of the same operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoldCore {
    row: SaleRecord,
}
impl SoldCore {
    #[must_use]
    pub const fn commit(&self) -> CommitToken {
        self.row.operation
    }
    #[must_use]
    pub fn action(&self) -> &DefinitionRef {
        &self.row.action
    }
    #[must_use]
    pub const fn core(&self) -> ObjId {
        self.row.core
    }
    #[must_use]
    pub const fn seller(&self) -> ObjId {
        self.row.seller
    }
    #[must_use]
    pub const fn shop(&self) -> ObjId {
        self.row.shop
    }
    #[must_use]
    pub const fn sold_at(&self) -> i64 {
        self.row.at_utc
    }
}

/// **Where the core may go** (doc 48 unit EX.2b): each closed plan's own rule, chosen by the
/// service, never by a caller. The buy-back's shop is a Thing or a Room; the resale's buyer is a
/// Player (their own hands), and only the resale's fixed action selects it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Destination {
    /// CO.13's buy-back: the shop's container (a Thing) or room.
    Shop,
    /// EX.2b's resale: the buyer's own hands.
    Buyer,
}
impl Destination {
    const fn admits(self, kind: ObjKind) -> bool {
        match self {
            Self::Shop => matches!(kind, ObjKind::Thing | ObjKind::Room),
            Self::Buyer => matches!(kind, ObjKind::Player),
        }
    }
}

/// The fence the image keeps through custody: the policy's read, repeated.
struct SaleFence<P: CoreSalePolicy> {
    policy: Arc<P>,
    request: SaleRequest,
    facts: SaleFacts,
}
pub(crate) trait Fence: Send + Sync {
    fn recheck(&self, world: &World) -> Result<(), ParticipantError>;
}
impl<P: CoreSalePolicy> Fence for SaleFence<P> {
    fn recheck(&self, world: &World) -> Result<(), ParticipantError> {
        if self.policy.read(world, self.facts.seller, &self.request)? != self.facts {
            return Err(ParticipantError::Authority);
        }
        Ok(())
    }
}

/// **The three object images**: the core leaves its seller's contents and joins the shop's.
pub(crate) struct SaleObjects {
    before: [Obj; 3],
    after: Box<[Obj; 3]>,
    ancestors: Vec<Obj>,
    carry: CarryCheck,
    fence: Box<dyn Fence>,
    writes: Vec<ReplayWrite>,
}
impl std::fmt::Debug for SaleObjects {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaleObjects")
            .field("objects", &self.before.each_ref().map(|o| o.id))
            .finish_non_exhaustive()
    }
}
impl SaleObjects {
    fn prepare(
        world: &World,
        lens: TypedStateLens<'_>,
        facts: &SaleFacts,
        carry: CarryCheck,
        fence: Box<dyn Fence>,
        rule: Destination,
    ) -> Result<Self, ParticipantError> {
        let (object, seller, shop) = (facts.core, facts.seller, facts.shop);
        if object == seller || object == shop || seller == shop {
            return Err(ParticipantError::Authority);
        }
        let get = |id| world.get(id).cloned().ok_or(ParticipantError::Authority);
        let item = get(object)?;
        let source = get(seller)?;
        let destination = get(shop)?;
        if item.kind != ObjKind::Thing
            || item.location != Some(seller)
            || !item.contents.is_empty()
            || !item.exits.is_empty()
            || source.kind != ObjKind::Player
            || !rule.admits(destination.kind)
            || [&item, &source, &destination]
                .into_iter()
                .any(|o| o.flags.contains(Flags::GOING))
            || source.contents.iter().filter(|id| **id == object).count() != 1
            || destination.contents.contains(&object)
            || source.exits.contains(&object)
            || destination.exits.contains(&object)
        {
            return Err(ParticipantError::Authority);
        }
        world
            .can_move_to(object, shop)
            .map_err(|_| ParticipantError::Authority)?;
        // Complete ancestry pre-images of both holders, as the delivery keeps them.
        let mut ancestors = Vec::new();
        let mut retained = BTreeSet::new();
        for start in [seller, shop] {
            let mut seen = BTreeSet::new();
            let mut at = Some(start);
            while let Some(id) = at {
                if seen.len() >= crate::MAX_CONTAINMENT_DEPTH || !seen.insert(id) || id == object {
                    return Err(ParticipantError::Authority);
                }
                let parent = get(id)?;
                if parent.flags.contains(Flags::GOING) {
                    return Err(ParticipantError::Authority);
                }
                if retained.insert(id) {
                    ancestors.push(parent.clone());
                }
                at = parent.location;
            }
        }
        let mut depth = 1usize;
        let mut at = Some(shop);
        while let Some(id) = at {
            depth = depth.checked_add(1).ok_or(ParticipantError::ImageLimit)?;
            if depth > crate::MAX_CONTAINMENT_DEPTH {
                return Err(ParticipantError::Authority);
            }
            at = world.get(id).ok_or(ParticipantError::Authority)?.location;
        }
        let before = [item, source, destination];
        let mut after = Box::new(before.clone());
        after[0].location = Some(shop);
        after[0].modified = world.now();
        after[1].contents.retain(|id| *id != object);
        after[1].modified = world.now();
        after[2].contents.push(object);
        after[2].modified = world.now();
        let writes: Vec<_> = after
            .iter()
            .cloned()
            .map(|object| ReplayWrite::ObjectSet { object })
            .collect();
        let value = serde_json::to_value(&writes).map_err(|_| ParticipantError::Encoding)?;
        canonical_json_bytes_with_limit(&value, IMAGE_BYTES)
            .map_err(|_| ParticipantError::ImageLimit)?;
        let result = Self {
            before,
            after,
            ancestors,
            carry,
            fence,
            writes,
        };
        result.recheck(world, lens)?;
        Ok(result)
    }
    pub(crate) fn writes(&self) -> &[ReplayWrite] {
        &self.writes
    }
    pub(crate) fn recheck(
        &self,
        world: &World,
        lens: TypedStateLens<'_>,
    ) -> Result<(), ParticipantError> {
        self.fence.recheck(world)?;
        self.local(world)?;
        self.carry.recheck(world, lens)?;
        self.local(world)
    }
    pub(crate) fn local(&self, world: &World) -> Result<(), ParticipantError> {
        if self
            .before
            .iter()
            .chain(&self.ancestors)
            .any(|o| world.get(o.id) != Some(o))
        {
            return Err(ParticipantError::Authority);
        }
        Ok(())
    }
    pub(crate) fn bind(
        self,
        storage: ShellObjectStorage<'_>,
    ) -> Result<BoundSaleObjects<'_>, ParticipantError> {
        let ShellObjectStorage { objects, .. } = storage;
        if self
            .before
            .iter()
            .chain(&self.ancestors)
            .any(|o| objects.get(o.id.0 as usize).and_then(Option::as_ref) != Some(o))
        {
            return Err(ParticipantError::Authority);
        }
        Ok(BoundSaleObjects {
            objects,
            after: self.after,
        })
    }
}
pub(crate) struct BoundSaleObjects<'a> {
    objects: &'a mut Vec<Option<Obj>>,
    after: Box<[Obj; 3]>,
}
impl BoundSaleObjects<'_> {
    pub(crate) fn apply(self) {
        // All exact occupied indices are bound under the same exclusive borrow.
        for object in *self.after {
            let index = object.id.0 as usize;
            self.objects[index] = Some(object);
        }
    }
}

/// **The sale service**: trusted construction by the tacsim module selects the policy; no
/// caller supplies a source, a destination or an image.
pub struct CoreSales<P: CoreSalePolicy> {
    records: StateFamilyHandle<CommitToken, SaleRecord>,
    policy: Arc<P>,
    action: DefinitionRef,
}
impl<P: CoreSalePolicy> Clone for CoreSales<P> {
    fn clone(&self) -> Self {
        Self {
            records: self.records.clone(),
            policy: Arc::clone(&self.policy),
            action: self.action.clone(),
        }
    }
}
impl<P: CoreSalePolicy> CoreSales<P> {
    /// # Errors
    /// A registrar that is not tacsim's, an action that is not the fixed one, or the family's
    /// own registration error.
    pub fn register(
        reg: &mut Registrar<'_>,
        action: DefinitionRef,
        policy: P,
    ) -> Result<Self, ModError> {
        if reg.module() != OWNER || !fixed_action(&action) {
            return Err(error("invalid fixed core sale binding"));
        }
        let key =
            |name: &str| CanonicalId::new(format!("{OWNER}.{FAMILY}.{name}.v1")).map_err(error);
        let descriptor = StateFamilyDescriptor::new(
            StateFamilyId::new(FAMILY).map_err(error)?,
            key("key")?,
            key("value")?,
            1,
            key("decode")?,
            key("validate")?,
            key("canonical")?,
        )
        .and_then(|d| d.with_limits(64, RECORD_BYTES))
        .map_err(error)?;
        let records = reg.state_family(
            descriptor,
            |commit: &CommitToken| Ok(commit.to_hex()),
            |v: &Value| decode(v),
            |r: &SaleRecord| r.validate().map_err(|e| e.to_string()),
            |r: &SaleRecord| bounded(r).map_err(|e| e.to_string()),
        )?;
        Ok(Self {
            records,
            policy: Arc::new(policy),
            action,
        })
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        &self,
        world: &World,
        invocation: &ActionInvocation,
        action: &DefinitionRef,
        commit: CommitToken,
        txn: &mut ActionTxn<'_>,
        request: SaleRequest,
        hooks: Arc<[HookReg]>,
        base: &crate::action_state::ActionReadBase,
        provider: &crate::action::ProviderRegistrationOrdinal,
    ) -> Result<(SoldCore, SaleObjects), ParticipantError> {
        let seller = invocation.actor();
        if action != &self.action
            || txn.owner().as_str() != OWNER
            || invocation.principal() != &(PrincipalId::Object { id: seller })
            || invocation.subject() != &PerceivedRef::SelfActor
            || !invocation.targets().is_empty()
        {
            return Err(ParticipantError::Authority);
        }
        let facts = self.policy.read(world, seller, &request)?;
        if facts.core != request.core
            || facts.seller != seller
            || txn
                .get(&self.records, &commit)
                .map_err(state_error)?
                .is_some()
        {
            return Err(ParticipantError::Authority);
        }
        let image = {
            let lens = txn
                .read_preimage(base, provider)
                .map_err(|_| ParticipantError::Authority)?;
            let carry = CarryCheck::capture(hooks, world, lens, seller, facts.core, facts.shop)?;
            let fence = Box::new(SaleFence {
                policy: Arc::clone(&self.policy),
                request: request.clone(),
                facts: facts.clone(),
            });
            SaleObjects::prepare(world, lens, &facts, carry, fence, Destination::Shop)?
        };
        let row = SaleRecord {
            format: 1,
            operation: commit,
            action: action.clone(),
            core: facts.core,
            seller,
            shop: facts.shop,
            at_utc: world.now(),
        };
        row.validate()?;
        txn.insert(&self.records, &commit, row.clone())
            .map_err(state_error)?;
        Ok((SoldCore { row }, image))
    }
    /// **Every sale this family holds**, for the producer's boot audit: each row validated and
    /// keyed by its own operation. Not a title certificate: the economy's replayed receipt is.
    ///
    /// # Errors
    /// A malformed or mis-keyed row, or more rows than a boot may scan.
    pub fn audit(
        &self,
        ctx: &ModuleCtx<'_>,
        mut observe: impl FnMut(SoldCore) -> Result<(), ParticipantError>,
    ) -> Result<(), ParticipantError> {
        let mut cursor = None;
        let mut count = 0usize;
        loop {
            let page = ctx
                .scan_action_state(&self.records, cursor.as_deref(), 128)
                .map_err(|_| ParticipantError::Authority)?;
            for entry in page.entries() {
                let row = entry.value();
                row.validate()?;
                count = count.checked_add(1).ok_or(ParticipantError::Authority)?;
                if count > crate::action_state::MAX_ACTION_STATE_BOOT_RECORDS
                    || entry.encoded_key() != row.operation.to_hex()
                {
                    return Err(ParticipantError::Authority);
                }
                observe(SoldCore { row: row.clone() })?;
            }
            cursor = page.next_after().map(ToString::to_string);
            if cursor.is_none() {
                break;
            }
        }
        Ok(())
    }
}

fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn state_error(_: crate::action_state::ActionStateError) -> ParticipantError {
    ParticipantError::Authority
}
fn bounded<T: Serialize>(row: &T) -> Result<Value, ParticipantError> {
    bounded_with(row, RECORD_BYTES)
}
fn bounded_with<T: Serialize>(row: &T, limit: usize) -> Result<Value, ParticipantError> {
    let v = serde_json::to_value(row).map_err(|_| ParticipantError::Encoding)?;
    canonical_json_bytes_with_limit(&v, limit).map_err(|_| ParticipantError::ImageLimit)?;
    Ok(v)
}
fn decode(v: &Value) -> Result<SaleRecord, String> {
    canonical_json_bytes_with_limit(v, RECORD_BYTES).map_err(|e| e.to_string())?;
    let row: SaleRecord = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
    if serde_json::to_value(&row).map_err(|e| e.to_string())? != *v {
        return Err("noncanonical core sale record".into());
    }
    Ok(row)
}
