//! **The closed physical image of a governed core resold to a player** (doc 48 unit EX.2b;
//! Q-48-47, the core leg of Q-48-52). The second closed plan beside CO.13's buy-back
//! ([`super`]): a seller's private offer names one buyer and a price, and the buyer's acceptance
//! is one fixed operation, in the same coordinator union as the economy's closed resale leg (the
//! buyer's money, the cuts and both titles, `mush_economy::registry_filing::core_sale::resale`).
//! Core owns the object images; the trusted tacsim policy only names which core, its seller,
//! its buyer (the acceptance's own actor) and, for a fitted core, the hull it is fitted in, and
//! repeats that read before custody.
//!
//! Two shapes, one record family:
//!
//! * **A stored core** leaves its seller's hands for the buyer's: the buy-back's image with the
//!   one rule it refuses, a **player** destination (the buyer's hands), selected only here.
//!   The carry hooks are the buyer's, as a completed core's delivery into a requester's hands.
//! * **A core fitted in a hull** that is sold with her does not move: the hull and the core keep
//!   their places, and the image is a fence only (both exact pre-images and the policy's read,
//!   kept through custody), writing no object. The titles move in the economy's leg.
//!
//! One record per resale, keyed by the operation, in the tacsim-owned `core_resales` family;
//! the producer's boot audit pairs it with the economy's receipt.
use super::{Destination, Fence, SaleFacts, SaleObjects, bounded_with, state_error};
use crate::action::{ActionInvocation, PerceivedRef, PrincipalId};
use crate::action_state::{ActionTxn, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId};
use crate::core_delivery::CarryCheck;
use crate::mechanics::{CanonicalId, DefinitionRef, canonical_json_bytes_with_limit};
use crate::module::HookReg;
use crate::participant::ParticipantError;
use crate::reserved_shell::ShellObjectStorage;
use crate::{CommitToken, Flags, ModError, ModuleCtx, Obj, ObjId, ObjKind, Registrar, World};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

/// The resale action's id (tacsim-owned: the buyer's acceptance).
pub const ACTION: &str = "accept_core_resale";
const FAMILY: &str = "core_resales";
const RECORD_BYTES: usize = 4_096;

/// The exact resale action: tacsim's [`ACTION`] at version 1.
#[must_use]
pub fn fixed_action(def: &DefinitionRef) -> bool {
    def.key().owner.as_str() == super::OWNER
        && def.key().kind.as_str() == "action"
        && def.key().id.as_str() == ACTION
        && def.version() == 1
}

/// What the buyer asks: accept the standing offer of this core, from this seller, with this
/// hull if it is fitted in one, on the state the offer was made on. The producer builds it from
/// the offer it reads (and removes) in the same union's transaction; `pin` is the producer's
/// opaque digest of that state, which its policy reads again. Not a wire format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResaleRequest {
    core: ObjId,
    seller: ObjId,
    hull: Option<ObjId>,
    pin: u64,
}
impl ResaleRequest {
    #[must_use]
    pub const fn new(core: ObjId, seller: ObjId, hull: Option<ObjId>, pin: u64) -> Self {
        Self {
            core,
            seller,
            hull,
            pin,
        }
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
    pub const fn hull(&self) -> Option<ObjId> {
        self.hull
    }
    #[must_use]
    pub const fn pin(&self) -> u64 {
        self.pin
    }
}

/// **The trusted policy's read**: the core, the seller whose offer stands, the buyer it names
/// and, for a core fitted in a hull sold with it, that hull. Produced only by the registered
/// policy, read again before custody; confers no authority outside this service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResaleFacts {
    core: ObjId,
    seller: ObjId,
    buyer: ObjId,
    hull: Option<ObjId>,
}
impl ResaleFacts {
    #[must_use]
    pub const fn new(core: ObjId, seller: ObjId, buyer: ObjId, hull: Option<ObjId>) -> Self {
        Self {
            core,
            seller,
            buyer,
            hull,
        }
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
    pub const fn buyer(&self) -> ObjId {
        self.buyer
    }
    #[must_use]
    pub const fn hull(&self) -> Option<ObjId> {
        self.hull
    }
}

/// **The trusted resale policy** (tacsim's): whether `request`'s core may pass from its seller
/// to `buyer` now. A pure read of the world: the service calls it at prepare and again before
/// custody, and refuses unless both reads agree exactly. A concrete policy proves the core is
/// still where the offer found it, on the state it pinned: in the seller's own hands, or fitted
/// in the hull sold with it, every title the seller's alone. The offer itself (that it names
/// this buyer and has not lapsed) is the producer's, read and removed in the same transaction.
pub trait CoreResalePolicy: Send + Sync + 'static {
    /// # Errors
    /// Any refusal; the resale does not happen.
    fn read(
        &self,
        world: &World,
        buyer: ObjId,
        request: &ResaleRequest,
    ) -> Result<ResaleFacts, ParticipantError>;
}

/// One resale's record: the operation, the action, the core, its seller and buyer, the hull it
/// went with (if fitted) and the UTC second. Kept in the tacsim-owned `core_resales` family,
/// keyed by the operation's hex.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResaleRecord {
    format: u16,
    operation: CommitToken,
    action: DefinitionRef,
    core: ObjId,
    seller: ObjId,
    buyer: ObjId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hull: Option<ObjId>,
    at_utc: i64,
}
impl ResaleRecord {
    fn validate(&self) -> Result<(), ParticipantError> {
        let ids = [
            Some(self.core),
            Some(self.seller),
            Some(self.buyer),
            self.hull,
        ];
        let distinct = ids
            .iter()
            .flatten()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == ids.iter().flatten().count();
        if self.format != 1
            || !fixed_action(&self.action)
            || ids.iter().flatten().any(|id| id.0 == 0)
            || !distinct
        {
            return Err(ParticipantError::Authority);
        }
        bounded_with(self, RECORD_BYTES)?;
        Ok(())
    }
}

/// **A prepared or replayed resale**, minted only by this service: the producer pairs it with
/// the economy's receipt of the same operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResoldCore {
    row: ResaleRecord,
}
impl ResoldCore {
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
    pub const fn buyer(&self) -> ObjId {
        self.row.buyer
    }
    /// The hull the core was fitted in and sold with, `None` for a stored core.
    #[must_use]
    pub const fn hull(&self) -> Option<ObjId> {
        self.row.hull
    }
    #[must_use]
    pub const fn sold_at(&self) -> i64 {
        self.row.at_utc
    }
}

/// The fence the image keeps through custody: the policy's read, repeated.
struct ResaleFence<P: CoreResalePolicy> {
    policy: Arc<P>,
    request: ResaleRequest,
    facts: ResaleFacts,
}
impl<P: CoreResalePolicy> Fence for ResaleFence<P> {
    fn recheck(&self, world: &World) -> Result<(), ParticipantError> {
        if self.policy.read(world, self.facts.buyer, &self.request)? != self.facts {
            return Err(ParticipantError::Authority);
        }
        Ok(())
    }
}

/// **A fitted core sold with its hull**: nothing moves. The exact pre-images of the core and
/// the hull it is fitted in, and the policy's read, are kept through custody.
pub(crate) struct FittedObjects {
    before: [Obj; 2],
    fence: Box<dyn Fence>,
}
impl FittedObjects {
    fn prepare(
        world: &World,
        facts: &ResaleFacts,
        fence: Box<dyn Fence>,
    ) -> Result<Self, ParticipantError> {
        let hull = facts.hull.ok_or(ParticipantError::Authority)?;
        let get = |id| world.get(id).cloned().ok_or(ParticipantError::Authority);
        let core = get(facts.core)?;
        let vessel = get(hull)?;
        if core.kind != ObjKind::Thing
            || core.location != Some(hull)
            || vessel
                .contents
                .iter()
                .filter(|id| **id == facts.core)
                .count()
                != 1
            || [&core, &vessel]
                .into_iter()
                .any(|o| o.flags.contains(Flags::GOING))
            || [facts.seller, facts.buyer].into_iter().any(|id| {
                world
                    .get(id)
                    .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
            })
        {
            return Err(ParticipantError::Authority);
        }
        let result = Self {
            before: [core, vessel],
            fence,
        };
        result.recheck(world)?;
        Ok(result)
    }
    fn recheck(&self, world: &World) -> Result<(), ParticipantError> {
        self.fence.recheck(world)?;
        self.local(world)
    }
    fn local(&self, world: &World) -> Result<(), ParticipantError> {
        if self.before.iter().any(|o| world.get(o.id) != Some(o)) {
            return Err(ParticipantError::Authority);
        }
        Ok(())
    }
}

/// **The resale's object image**: a stored core moved into the buyer's hands, or a fitted core's
/// fence.
pub(crate) enum ResaleObjects {
    Stored(Box<SaleObjects>),
    Fitted(Box<FittedObjects>),
}
impl std::fmt::Debug for ResaleObjects {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stored(x) => f.debug_tuple("Stored").field(x).finish(),
            Self::Fitted(x) => f
                .debug_struct("Fitted")
                .field("objects", &x.before.each_ref().map(|o| o.id))
                .finish_non_exhaustive(),
        }
    }
}
impl ResaleObjects {
    pub(crate) fn writes(&self) -> &[crate::authoritative::ReplayWrite] {
        match self {
            Self::Stored(x) => x.writes(),
            Self::Fitted(_) => &[],
        }
    }
    pub(crate) fn recheck(
        &self,
        world: &World,
        lens: crate::action_state::TypedStateLens<'_>,
    ) -> Result<(), ParticipantError> {
        match self {
            Self::Stored(x) => x.recheck(world, lens),
            Self::Fitted(x) => x.recheck(world),
        }
    }
    pub(crate) fn local(&self, world: &World) -> Result<(), ParticipantError> {
        match self {
            Self::Stored(x) => x.local(world),
            Self::Fitted(x) => x.local(world),
        }
    }
    pub(crate) fn bind(
        self,
        storage: ShellObjectStorage<'_>,
    ) -> Result<BoundResaleObjects<'_>, ParticipantError> {
        match self {
            Self::Stored(x) => (*x).bind(storage).map(BoundResaleObjects::Stored),
            Self::Fitted(x) => {
                let ShellObjectStorage { objects, .. } = storage;
                if x.before
                    .iter()
                    .any(|o| objects.get(o.id.0 as usize).and_then(Option::as_ref) != Some(o))
                {
                    return Err(ParticipantError::Authority);
                }
                Ok(BoundResaleObjects::Fitted)
            }
        }
    }
}
pub(crate) enum BoundResaleObjects<'a> {
    Stored(super::BoundSaleObjects<'a>),
    Fitted,
}
impl BoundResaleObjects<'_> {
    pub(crate) fn apply(self) {
        match self {
            Self::Stored(x) => x.apply(),
            // A fitted core keeps its place: the fence held, and nothing is written.
            Self::Fitted => {}
        }
    }
}

/// **The resale service**: trusted construction by the tacsim module selects the policy; no
/// caller supplies a source, a destination or an image.
pub struct CoreResales<P: CoreResalePolicy> {
    records: StateFamilyHandle<CommitToken, ResaleRecord>,
    policy: Arc<P>,
    action: DefinitionRef,
}
impl<P: CoreResalePolicy> Clone for CoreResales<P> {
    fn clone(&self) -> Self {
        Self {
            records: self.records.clone(),
            policy: Arc::clone(&self.policy),
            action: self.action.clone(),
        }
    }
}
impl<P: CoreResalePolicy> CoreResales<P> {
    /// # Errors
    /// A registrar that is not tacsim's, an action that is not the fixed one, or the family's
    /// own registration error.
    pub fn register(
        reg: &mut Registrar<'_>,
        action: DefinitionRef,
        policy: P,
    ) -> Result<Self, ModError> {
        if reg.module() != super::OWNER || !fixed_action(&action) {
            return Err(error("invalid fixed core resale binding"));
        }
        let key = |name: &str| {
            CanonicalId::new(format!("{}.{FAMILY}.{name}.v1", super::OWNER)).map_err(error)
        };
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
            |r: &ResaleRecord| r.validate().map_err(|e| e.to_string()),
            |r: &ResaleRecord| bounded_with(r, RECORD_BYTES).map_err(|e| e.to_string()),
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
        request: ResaleRequest,
        hooks: Arc<[HookReg]>,
        base: &crate::action_state::ActionReadBase,
        provider: &crate::action::ProviderRegistrationOrdinal,
    ) -> Result<(ResoldCore, ResaleObjects), ParticipantError> {
        let buyer = invocation.actor();
        if action != &self.action
            || txn.owner().as_str() != super::OWNER
            || invocation.principal() != &(PrincipalId::Object { id: buyer })
            || invocation.subject() != &PerceivedRef::SelfActor
            || !invocation.targets().is_empty()
        {
            return Err(ParticipantError::Authority);
        }
        let facts = self.policy.read(world, buyer, &request)?;
        if facts.core != request.core
            || facts.seller != request.seller
            || facts.hull != request.hull
            || facts.buyer != buyer
            || facts.seller == buyer
            || txn
                .get(&self.records, &commit)
                .map_err(state_error)?
                .is_some()
        {
            return Err(ParticipantError::Authority);
        }
        let fence = Box::new(ResaleFence {
            policy: Arc::clone(&self.policy),
            request: request.clone(),
            facts: facts.clone(),
        });
        let image = match facts.hull {
            None => {
                let lens = txn
                    .read_preimage(base, provider)
                    .map_err(|_| ParticipantError::Authority)?;
                // The buyer receives the core into their own hands: theirs are the carry hooks
                // asked, as a requester's are for a completed core's delivery.
                let carry = CarryCheck::capture(hooks, world, lens, buyer, facts.core, buyer)?;
                let sale = SaleFacts::new(facts.core, facts.seller, buyer);
                ResaleObjects::Stored(Box::new(SaleObjects::prepare(
                    world,
                    lens,
                    &sale,
                    carry,
                    fence,
                    Destination::Buyer,
                )?))
            }
            Some(_) => {
                ResaleObjects::Fitted(Box::new(FittedObjects::prepare(world, &facts, fence)?))
            }
        };
        let row = ResaleRecord {
            format: 1,
            operation: commit,
            action: action.clone(),
            core: facts.core,
            seller: facts.seller,
            buyer,
            hull: facts.hull,
            at_utc: world.now(),
        };
        row.validate()?;
        txn.insert(&self.records, &commit, row.clone())
            .map_err(state_error)?;
        Ok((ResoldCore { row }, image))
    }
    /// **Every resale this family holds**, for the producer's boot audit: each row validated
    /// and keyed by its own operation. Not a title certificate: the economy's replayed receipt
    /// is.
    ///
    /// # Errors
    /// A malformed or mis-keyed row, or more rows than a boot may scan.
    pub fn audit(
        &self,
        ctx: &ModuleCtx<'_>,
        mut observe: impl FnMut(ResoldCore) -> Result<(), ParticipantError>,
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
                observe(ResoldCore { row: row.clone() })?;
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
fn decode(v: &Value) -> Result<ResaleRecord, String> {
    canonical_json_bytes_with_limit(v, RECORD_BYTES).map_err(|e| e.to_string())?;
    let row: ResaleRecord = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
    if serde_json::to_value(&row).map_err(|e| e.to_string())? != *v {
        return Err("noncanonical core resale record".into());
    }
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(hull: Option<ObjId>) -> ResaleRecord {
        ResaleRecord {
            format: 1,
            operation: CommitToken::from_bytes([0xab; 16]),
            action: crate::mechanics::DefinitionRegistryBuilder::new()
                .register(
                    crate::mechanics::OwnerId::new(super::super::OWNER).expect("owner"),
                    crate::mechanics::DefinitionDescriptor::new(
                        crate::mechanics::DefinitionKind::new("action").expect("kind"),
                        crate::mechanics::DefinitionId::new(ACTION).expect("id"),
                        1,
                        serde_json::json!({"format": 1}),
                    )
                    .expect("descriptor")
                    .as_current(),
                )
                .expect("registers"),
            core: ObjId(10),
            seller: ObjId(11),
            buyer: ObjId(12),
            hull,
            at_utc: 1,
        }
    }

    /// The record is canonical both ways, keeps the hull only when fitted, and refuses a buyer
    /// who is the seller, a hull that is the core, and a zero id.
    #[test]
    fn ex2b_the_resale_record_is_closed_and_canonical() {
        for hull in [None, Some(ObjId(13))] {
            let row = record(hull);
            row.validate().expect("a well-formed record");
            let value = serde_json::to_value(&row).expect("encodes");
            assert_eq!(value.get("hull").is_some(), hull.is_some(), "{value}");
            assert_eq!(decode(&value).expect("decodes"), row);
        }
        let mut same = record(None);
        same.buyer = same.seller;
        assert!(same.validate().is_err());
        let mut hull_is_core = record(Some(ObjId(10)));
        hull_is_core.format = 1;
        assert!(hull_is_core.validate().is_err());
        let mut zero = record(Some(ObjId(0)));
        zero.format = 1;
        assert!(zero.validate().is_err());
        let mut format = record(None);
        format.format = 2;
        assert!(format.validate().is_err());
    }

    /// Only the resale's rule admits a player; the buy-back's admits a Thing or a Room.
    #[test]
    fn ex2b_only_the_resale_admits_a_player_destination() {
        assert!(Destination::Buyer.admits(ObjKind::Player));
        assert!(!Destination::Buyer.admits(ObjKind::Thing));
        assert!(!Destination::Buyer.admits(ObjKind::Room));
        assert!(!Destination::Shop.admits(ObjKind::Player));
        assert!(Destination::Shop.admits(ObjKind::Thing));
        assert!(Destination::Shop.admits(ObjKind::Room));
    }
}
