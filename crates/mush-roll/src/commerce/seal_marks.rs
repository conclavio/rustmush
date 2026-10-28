//! Doc 17h §3 M8c (the lead's ruling B5; doc 17h §2.6, D-UM-42): the
//! per-licence ring of seal marks, the one channel through which the Registry
//! learns that a licence flew on an ungoverned core.
//!
//! A seal mark says that the hull `hull` (an object number) flew under this
//! licence at the IC second `at_ic`, and whether the core was governed
//! (`governed`); `sortie` is the canonical string form of the flight's
//! `SortieId` (what the black-box line carries), opaque to the Roll. The
//! ring keeps [`MAX_MARKS`] marks per licence in one row keyed by the
//! licence (`licence/{hex16}`), so a read is one get and a write is one
//! replace. Past `MAX_MARKS` the oldest GOVERNED mark is dropped first (M8c
//! F6): an ungoverned mark is evidence and is dropped only when every mark
//! in the ring is ungoverned, so a run of governed flights cannot wash one
//! out. Nothing else is derived from a mark: detection is
//! evidence only (D-UM-42), and the only consumers are
//! `owner_attestation`'s refusal (a licence that flew ungoverned during the
//! term being renewed is not attested) and the scoped fact
//! `minds.licence.unattestable` (`facts::reader`), which an E4 contract
//! condition may read exactly as it reads `licence.lapsed`.
//!
//! **Writer.** The one writer is the action `record_licence_seal_mark`,
//! never wrapped by a player command. It is gated on the Registry's own
//! capability word [`SEAL_MARK_CAPABILITY`] (M8c F7): GOD passes, as
//! everywhere, and so does a live character granted the word on the root
//! scope of an organisation that granted it (a LIVE `mush_org` ask, no
//! amount). M8c ships no production writer: M8e's black-box participant
//! records marks on the surface finish (submitted as the operator); until
//! then the ring is written only by tests. The refusal that quotes a mark
//! resolves the hull's name when it is written ("a hull" when the object is
//! gone), never a sixteen-hex-digit word, so it can reach a player surface.
use super::families::CommerceFamilies;
use super::read::CommerceRead;
use super::records::{PublicationError, Record};
use super::{LicenceId, WorldIcSeconds};
use mush_core::action::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, AuthoritativeActionOutput,
    PerceivedRef, PrincipalId,
};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::{
    ActionTxn, DefinitionRef, Flags, GOD, ModError, ObjId, PrepareCtx, Registrar, RulesValue, World,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// The family name.
pub const FAMILY: &str = "roll_licence_seal_marks";
/// Marks one licence's ring keeps: the newest 64.
pub const MAX_MARKS: usize = 64;
/// A sortie's longest canonical string form (`SortieId`'s `<hull>@<at_ic>`
/// or the arena's bare number is at most 32 bytes).
pub const MAX_SORTIE_BYTES: usize = 64;
/// The Registry's capability word for the writer (M8c F7); declared in the
/// game's capability vocabulary so `@org/grant` accepts it.
pub const SEAL_MARK_CAPABILITY: &str = "roll.registry.seal_mark";
/// The writer action and its outcome, version 1 each.
pub const ACTION: &str = "record_licence_seal_mark";
pub const OUTCOME: &str = "licence_seal_mark_recorded";
/// The refusal's first sentence (B5), verbatim; the line names the hull and
/// the IC date after it (`unattestable_line`).
pub const UNATTESTABLE: &str =
    "The Registry cannot attest a licence that flew on an ungoverned core.";

fn invalid<T>(_: T) -> PublicationError {
    PublicationError::Invalid
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}

/// One flight under a licence: which hull (its object number), when (world
/// IC seconds, and the world UTC second the mark was written), whether the
/// core was governed, and the flight's sortie (`SortieId`'s canonical string
/// form; internal, never shown).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SealMark {
    pub hull: ObjId,
    pub at_ic: WorldIcSeconds,
    pub at_utc: i64,
    pub governed: bool,
    pub sortie: String,
}
impl SealMark {
    fn validate(&self) -> Result<(), PublicationError> {
        if self.hull.0 == 0
            || self.sortie.is_empty()
            || self.sortie.len() > MAX_SORTIE_BYTES
            || self.sortie.chars().any(char::is_control)
            || self.sortie != self.sortie.trim()
        {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
    /// The refusal line: `UNATTESTABLE`, then the hull's current name
    /// ("a hull" when its object is gone) and the IC date of the flight
    /// (`"August 28, 2075, 02:57"`).
    #[must_use]
    pub fn unattestable_line(&self, world: &World) -> String {
        let name = world
            .get(self.hull)
            .map_or_else(|| "a hull".to_owned(), |o| o.name.to_string());
        unattestable_line(&name, self.at_ic)
    }
}
/// M8c F10: the ONE window the attestation's refusal and the
/// `licence.unattestable` fact both read: `[issued_at, min(now, grace_end)]`.
#[must_use]
pub fn attestation_window(
    window: super::TermWindow,
    now: WorldIcSeconds,
) -> (WorldIcSeconds, WorldIcSeconds) {
    let end = if now < window.grace_end() {
        now
    } else {
        window.grace_end()
    };
    (window.issued_at(), end)
}
/// `UNATTESTABLE`, then `{hull} flew ungoverned on {IC date}.`
#[must_use]
pub fn unattestable_line(hull: &str, at_ic: WorldIcSeconds) -> String {
    let date = mush_core::clock::CivilDate::from_ic_seconds(at_ic.get());
    format!("{UNATTESTABLE} {hull} flew ungoverned on {date}.")
}

/// One licence's ring: its newest `MAX_MARKS` marks, in the order written.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RingWire")]
pub struct SealMarkRing {
    format: u16,
    licence: LicenceId,
    marks: Vec<SealMark>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RingWire {
    format: u16,
    licence: LicenceId,
    marks: Vec<SealMark>,
}
impl TryFrom<RingWire> for SealMarkRing {
    type Error = PublicationError;
    fn try_from(w: RingWire) -> Result<Self, Self::Error> {
        let v = Self {
            format: w.format,
            licence: w.licence,
            marks: w.marks,
        };
        v.validate()?;
        Ok(v)
    }
}
impl Record for SealMarkRing {
    const FAMILY: &'static str = FAMILY;
    const MAX_BYTES: usize = 16_384;
    fn key(&self) -> String {
        Self::key_for(self.licence)
    }
    fn validate(&self) -> Result<(), PublicationError> {
        if self.format != 1 || self.marks.len() > MAX_MARKS {
            return Err(PublicationError::Invalid);
        }
        for mark in &self.marks {
            mark.validate()?;
        }
        mush_core::canonical_json_bytes_with_limit(
            &serde_json::to_value(self).map_err(invalid)?,
            Self::MAX_BYTES,
        )
        .map_err(|_| PublicationError::Capacity)?;
        Ok(())
    }
}
impl SealMarkRing {
    /// The row key: the licence's own key (`licence/{hex16}`).
    #[must_use]
    pub fn key_for(licence: LicenceId) -> String {
        licence.key()
    }
    #[must_use]
    pub const fn licence(&self) -> LicenceId {
        self.licence
    }
    #[must_use]
    pub fn marks(&self) -> &[SealMark] {
        &self.marks
    }
    fn empty(licence: LicenceId) -> Self {
        Self {
            format: 1,
            licence,
            marks: Vec::new(),
        }
    }
    /// The ring with `mark` appended; past `MAX_MARKS` the oldest GOVERNED
    /// mark is dropped, the oldest mark only when every mark is ungoverned
    /// (M8c F6: evidence is kept over routine flights).
    fn pushed(mut self, mark: SealMark) -> Result<Self, PublicationError> {
        mark.validate()?;
        self.marks.push(mark);
        while self.marks.len() > MAX_MARKS {
            let drop = self.marks.iter().position(|m| m.governed).unwrap_or(0);
            self.marks.remove(drop);
        }
        self.validate()?;
        Ok(self)
    }
    /// The marks at IC seconds in `from..=to`, in the order written.
    pub fn marks_in(
        &self,
        from: WorldIcSeconds,
        to: WorldIcSeconds,
    ) -> impl Iterator<Item = &SealMark> {
        self.marks
            .iter()
            .filter(move |m| from <= m.at_ic && m.at_ic <= to)
    }
    /// The latest ungoverned flight in `from..=to`, if any: the one the
    /// attestation refusal and the `licence.unattestable` fact name.
    #[must_use]
    pub fn ungoverned_in(&self, from: WorldIcSeconds, to: WorldIcSeconds) -> Option<&SealMark> {
        self.marks_in(from, to)
            .filter(|m| !m.governed)
            .max_by_key(|m| m.at_ic)
    }
}

/// `marks_in(licence, from_ic, to_ic)`: the ring's marks in the window, or
/// none when the licence has no ring. A row that fails its own checks is
/// `Mismatch`.
pub(crate) fn marks_in(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    licence: LicenceId,
    from: WorldIcSeconds,
    to: WorldIcSeconds,
) -> Result<Vec<SealMark>, PublicationError> {
    let Some(ring) = ctx
        .get(&f.seal_marks, &SealMarkRing::key_for(licence))
        .map_err(|_| PublicationError::Unavailable)?
    else {
        return Ok(Vec::new());
    };
    ring.validate()?;
    if ring.licence != licence {
        return Err(PublicationError::Mismatch);
    }
    Ok(ring.marks_in(from, to).cloned().collect())
}
/// `marks_in` inside an action transaction.
pub(crate) fn marks_in_txn(
    txn: &mut ActionTxn<'_>,
    f: &CommerceFamilies,
    licence: LicenceId,
    from: WorldIcSeconds,
    to: WorldIcSeconds,
) -> Result<Vec<SealMark>, PublicationError> {
    let Some(ring) = txn
        .get(&f.seal_marks, &SealMarkRing::key_for(licence))
        .map_err(|_| PublicationError::Unavailable)?
        .cloned()
    else {
        return Ok(Vec::new());
    };
    ring.validate()?;
    if ring.licence != licence {
        return Err(PublicationError::Mismatch);
    }
    Ok(ring.marks_in(from, to).cloned().collect())
}
/// The latest ungoverned flight under `licence` in the window, read through
/// the owner-read context (`facts::reader`).
pub(crate) fn ungoverned_in(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    licence: LicenceId,
    from: WorldIcSeconds,
    to: WorldIcSeconds,
) -> Result<Option<SealMark>, PublicationError> {
    Ok(marks_in(ctx, f, licence, from, to)?
        .into_iter()
        .filter(|m| !m.governed)
        .max_by_key(|m| m.at_ic))
}

// ---- the writer ----

pub(crate) fn definitions() -> Result<[DefinitionDescriptor; 2], ModError> {
    Ok([
        DefinitionDescriptor::new(
            DefinitionKind::new("action").map_err(error)?,
            DefinitionId::new(ACTION).map_err(error)?,
            1,
            json!({"format":1,"protocol":"licence_seal_mark_v1","authority":"capability","capability":SEAL_MARK_CAPABILITY,
                "subject":"licence_flight","ring":MAX_MARKS,"consumers":["owner_attestation","licence.unattestable"]}),
        )
        .map_err(error)?,
        DefinitionDescriptor::new(
            DefinitionKind::new("outcome").map_err(error)?,
            DefinitionId::new(OUTCOME).map_err(error)?,
            1,
            json!({"format":1}),
        )
        .map_err(error)?,
    ])
}
pub(crate) fn action() -> Result<DefinitionRef, ModError> {
    let [action, _] = definitions()?;
    DefinitionRegistryBuilder::new()
        .register(OwnerId::new(crate::NS).map_err(error)?, action)
        .map_err(error)
}

/// The writer's wire: the licence (decimal string, `LicenceId`'s encoding),
/// the hull's object number, the flight's IC second, whether the core was
/// governed, and the sortie's canonical string form.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    licence: LicenceId,
    hull: ObjId,
    at_ic: i64,
    governed: bool,
    sortie: String,
}
/// Whether `actor` may write seal marks (M8c F7): GOD, or a live character
/// granted [`SEAL_MARK_CAPABILITY`] on the root scope of an organisation
/// that granted it, asked LIVE with no amount.
pub(crate) fn authorized(world: &World, actor: ObjId) -> bool {
    if actor == GOD {
        return true;
    }
    if actor.0 == 0
        || world
            .get(actor)
            .is_none_or(|o| o.flags.contains(Flags::GOING))
    {
        return false;
    }
    let Some(capability) = mush_org::CapabilityId::new(SEAL_MARK_CAPABILITY) else {
        return false;
    };
    let Some(store) = world.module_state::<mush_org::OrgStore>() else {
        return false;
    };
    let state = store.state();
    let orgs: std::collections::BTreeSet<mush_org::OrgId> = state.grant_orgs(actor).collect();
    orgs.into_iter().any(|org| {
        state.org(org).is_some_and(|o| {
            matches!(
                mush_org::authorize_in_world(
                    world,
                    &mush_org::AuthRequest {
                        actor,
                        capability: capability.clone(),
                        scope: o.root_scope,
                        now: world.now(),
                        amount: None,
                    },
                ),
                mush_org::Decision::Allow(_)
            )
        })
    })
}
impl Request {
    /// The wire's own checks; the writer's authority (`authorized`) needs the
    /// world and is asked by `prepare` and `project_offer`.
    fn parse(inv: &ActionInvocation) -> Result<Self, PublicationError> {
        if inv.principal() != &(PrincipalId::Object { id: inv.actor() })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || inv.idempotency().scope().owner.as_str() != crate::NS
        {
            return Err(PublicationError::Invalid);
        }
        let r: Self = serde_json::from_value(inv.parameters().clone()).map_err(invalid)?;
        r.mark(0).validate()?;
        Ok(r)
    }
    fn mark(&self, at_utc: i64) -> SealMark {
        SealMark {
            hull: self.hull,
            at_ic: WorldIcSeconds(self.at_ic),
            at_utc,
            governed: self.governed,
            sortie: self.sortie.clone(),
        }
    }
}
struct Provider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    f: CommerceFamilies,
    refusal: ActionError,
}
impl Provider {
    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, PublicationError> {
        let request = Request::parse(inv)?;
        let world = ctx.world().map_err(invalid)?;
        if !authorized(world, inv.actor()) {
            return Err(PublicationError::Invalid);
        }
        world
            .module_state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?
            .commerce_ready()
            .map_err(|_| PublicationError::Unavailable)?;
        let licence = request.licence;
        let key = SealMarkRing::key_for(licence);
        let existing = txn.get(&self.f.seal_marks, &key).map_err(invalid)?.cloned();
        let ring = match existing {
            Some(ring) => {
                ring.validate()?;
                if ring.licence != licence {
                    return Err(PublicationError::Mismatch);
                }
                ring.pushed(request.mark(world.now()))?
            }
            None => SealMarkRing::empty(licence).pushed(request.mark(world.now()))?,
        };
        let present = txn
            .get(&self.f.seal_marks, &key)
            .map_err(invalid)?
            .is_some();
        if present {
            txn.replace(&self.f.seal_marks, &key, ring.clone())
                .map_err(invalid)?;
        } else {
            txn.insert(&self.f.seal_marks, &key, ring.clone())
                .map_err(invalid)?;
        }
        if txn.get(&self.f.seal_marks, &key).map_err(invalid)? != Some(&ring) {
            return Err(PublicationError::Mismatch);
        }
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(
                self.outcome.clone(),
                json!({"licence":licence,"marks":ring.marks.len(),"governed":request.governed}),
            )
            .map_err(invalid)?,
        )
        .with_definitions(vec![self.action.clone(), self.outcome.clone()])
        .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        Request::parse(inv)
            .map(|_| ())
            .map_err(|_| self.refusal.clone())
    }
    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.prepare(inv, txn, ctx)
            .map_err(|_| self.refusal.clone())
    }
    fn project_offer(
        &self,
        action: &DefinitionRef,
        q: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, ActionError> {
        if action != &self.action
            || !authorized(&engine.world, q.actor())
            || q.principal() != &(PrincipalId::Object { id: q.actor() })
            || q.projected_subject() != &PerceivedRef::SelfActor
            || q.truth_subject() != &(mush_core::SubjectKey::Object { id: q.actor() })
            || engine
                .world
                .module_state::<crate::RollStore>()
                .is_none_or(|r| r.commerce_ready().is_err())
        {
            return Ok(None);
        }
        let fail = || self.refusal.clone();
        let defs = engine.registry.definitions().ok_or_else(fail)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| fail())?;
        guards
            .add_definition(self.action.clone(), defs)
            .map_err(|_| fail())?;
        use mush_core::ProjectedParameterKind as K;
        let parameter = |id: &str, kind: K| -> Result<mush_core::ProjectedParameter, ActionError> {
            Ok(mush_core::ProjectedParameter {
                id: mush_core::CanonicalId::new(id).map_err(|_| fail())?,
                required: true,
                kind,
            })
        };
        let parameters = vec![
            parameter("licence", K::Text { maximum_bytes: 20 })?,
            parameter(
                "hull",
                K::Integer {
                    minimum: 1,
                    maximum: i64::from(u32::MAX),
                },
            )?,
            parameter(
                "at_ic",
                K::Integer {
                    minimum: i64::MIN,
                    maximum: i64::MAX,
                },
            )?,
            parameter("governed", K::Boolean)?,
            parameter(
                "sortie",
                K::Text {
                    maximum_bytes: MAX_SORTIE_BYTES,
                },
            )?,
        ];
        Ok(Some(mush_core::ProviderOfferDraft {
            fixed_parameters: json!({}),
            parameters: mush_core::ProjectedParameterSchema::new(parameters).map_err(|_| fail())?,
            targets: mush_core::ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "roll-licence-seal-mark",
                "Record a licence seal mark",
                "Record one flight under a licence and whether its core was governed.",
            )
            .map_err(|_| fail())?,
            guards: guards.finish().map_err(|_| fail())?,
        }))
    }
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    f: CommerceFamilies,
    definitions: [DefinitionRef; 2],
) -> Result<(), ModError> {
    let [action, outcome] = definitions;
    if reg.module() != crate::NS || action != self::action()? {
        return Err(error("licence seal mark registration differs"));
    }
    reg.action_provider(
        action.clone(),
        Provider {
            action,
            outcome,
            f,
            refusal: ActionError::new(
                ActionErrorCode::ProviderRejected,
                false,
                "The licence seal mark is unavailable.",
            )
            .map_err(error)?,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn mark(at: i64, governed: bool) -> SealMark {
        SealMark {
            hull: ObjId(u32::try_from(at).unwrap_or(1)),
            at_ic: WorldIcSeconds(at),
            at_utc: at,
            governed,
            sortie: format!("{}@{at}", 40 + at),
        }
    }
    #[test]
    fn ring_keeps_sixty_four_and_answers_the_window() {
        let licence = LicenceId::new(9).unwrap();
        let mut ring = SealMarkRing::empty(licence);
        for at in 1..=70 {
            ring = ring.pushed(mark(at, at % 7 != 0)).unwrap();
        }
        assert_eq!(ring.marks().len(), MAX_MARKS);
        // F6: the six dropped are the oldest GOVERNED marks (1..=6); 7 is
        // ungoverned and stays.
        assert_eq!(ring.marks()[0].at_ic, WorldIcSeconds(7));
        assert_eq!(ring.marks()[63].at_ic, WorldIcSeconds(70));
        assert_eq!(ring.key(), "licence/0000000000000009");
        let window: Vec<i64> = ring
            .marks_in(WorldIcSeconds(10), WorldIcSeconds(20))
            .map(|m| m.at_ic.get())
            .collect();
        assert_eq!(window, (10..=20).collect::<Vec<_>>());
        assert_eq!(
            ring.ungoverned_in(WorldIcSeconds(10), WorldIcSeconds(20))
                .map(|m| m.at_ic),
            Some(WorldIcSeconds(14))
        );
        assert!(
            ring.ungoverned_in(WorldIcSeconds(15), WorldIcSeconds(20))
                .is_none()
        );
        let json = serde_json::to_value(&ring).unwrap();
        let back: SealMarkRing = serde_json::from_value(json).unwrap();
        assert_eq!(back, ring);
    }
    /// M8c F6: one ungoverned mark then seventy governed ones: the ungoverned
    /// mark is kept, the oldest governed dropped.
    #[test]
    fn governed_flights_never_wash_out_an_ungoverned_one() {
        let licence = LicenceId::new(9).unwrap();
        let mut ring = SealMarkRing::empty(licence).pushed(mark(1, false)).unwrap();
        for at in 2..=71 {
            ring = ring.pushed(mark(at, true)).unwrap();
        }
        assert_eq!(ring.marks().len(), MAX_MARKS);
        assert_eq!(ring.marks()[0].at_ic, WorldIcSeconds(1));
        assert!(!ring.marks()[0].governed);
        assert_eq!(ring.marks()[1].at_ic, WorldIcSeconds(9));
        assert_eq!(ring.marks()[63].at_ic, WorldIcSeconds(71));
        assert_eq!(
            ring.ungoverned_in(WorldIcSeconds(0), WorldIcSeconds(100))
                .map(|m| m.at_ic),
            Some(WorldIcSeconds(1))
        );
        // Every mark ungoverned: the oldest goes.
        let mut all = SealMarkRing::empty(licence);
        for at in 1..=65 {
            all = all.pushed(mark(at, false)).unwrap();
        }
        assert_eq!(all.marks()[0].at_ic, WorldIcSeconds(2));
    }
    /// F7: the worst-case ring (64 marks, every field at its widest) fits
    /// `MAX_BYTES`.
    #[test]
    fn the_widest_ring_fits_its_record_bound() {
        let licence = LicenceId::new(u64::MAX).unwrap();
        let mut ring = SealMarkRing::empty(licence);
        for _ in 0..MAX_MARKS {
            ring = ring
                .pushed(SealMark {
                    hull: ObjId(u32::MAX),
                    at_ic: WorldIcSeconds(i64::MIN),
                    at_utc: i64::MIN,
                    governed: false,
                    sortie: "s".repeat(MAX_SORTIE_BYTES),
                })
                .unwrap();
        }
        let bytes = mush_core::canonical_json_bytes_with_limit(
            &serde_json::to_value(&ring).unwrap(),
            usize::MAX,
        )
        .unwrap()
        .len();
        assert!(bytes < SealMarkRing::MAX_BYTES, "{bytes}");
    }
    #[test]
    fn a_mark_names_a_hull_and_the_line_names_the_date() {
        let mut zero = mark(1, false);
        zero.hull = ObjId(0);
        assert!(zero.validate().is_err());
        for sortie in ["", " x", "a\nb", &"s".repeat(MAX_SORTIE_BYTES + 1)] {
            let mut m = mark(1, false);
            m.sortie = sortie.to_owned();
            assert!(m.validate().is_err(), "{sortie:?}");
        }
        let line = unattestable_line("Kestrel 4", WorldIcSeconds(0));
        assert!(line.starts_with(UNATTESTABLE), "{line}");
        assert!(line.contains("Kestrel 4 flew ungoverned on "), "{line}");
        // F12: no word of the line is sixteen hex digits.
        assert!(
            !line
                .split_whitespace()
                .any(|w| w.len() == 16 && w.bytes().all(|b| b.is_ascii_hexdigit())),
            "no sixteen-hex-digit word: {line}"
        );
        // A gone hull is "a hull".
        let world = World::new_minimal();
        let gone = mark(9_999, false).unattestable_line(&world);
        assert!(gone.contains(" a hull flew ungoverned on "), "{gone}");
    }
    /// F10: the one window, `[issued_at, min(now, grace_end)]`, at its edges.
    #[test]
    fn the_attestation_window_ends_at_now_or_the_grace_end() {
        let window = super::super::TermWindow::new(
            WorldIcSeconds(100),
            WorldIcSeconds(100),
            WorldIcSeconds(200),
            WorldIcSeconds(300),
        )
        .unwrap();
        let w = |now| attestation_window(window, WorldIcSeconds(now));
        assert_eq!(w(100), (WorldIcSeconds(100), WorldIcSeconds(100)));
        assert_eq!(w(250), (WorldIcSeconds(100), WorldIcSeconds(250)));
        assert_eq!(w(300), (WorldIcSeconds(100), WorldIcSeconds(300)));
        assert_eq!(w(400), (WorldIcSeconds(100), WorldIcSeconds(300)));
        let mut ring = SealMarkRing::empty(LicenceId::new(9).unwrap());
        for at in [99, 100, 300, 301] {
            ring = ring.pushed(mark(at, false)).unwrap();
        }
        let seen = |now| {
            let (from, to) = w(now);
            ring.marks_in(from, to)
                .map(|m| m.at_ic.get())
                .collect::<Vec<_>>()
        };
        // A flight at `issued_at` counts; one before it does not.
        assert_eq!(seen(100), vec![100]);
        // At `grace_end` its flight counts; after it nothing more does.
        assert_eq!(seen(300), vec![100, 300]);
        assert_eq!(seen(1_000), vec![100, 300]);
    }
}
