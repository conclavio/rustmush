//! Authenticated, bounded filed-holding reads for the OOC simulator.
//!
//! There is no World cache of commercial authority. Every instance borrows the
//! dispatch's actual immutable owner-read lens. Final checkout creates a fresh
//! instance and derives admission subjects from the current slot/lead roster.
//! The explicit personal-paid composition admits only the current original
//! Sealed Pin product. Neither mode installs an IC claim or exposes source.

use mush_core::read_service::{OwnerReadError, OwnerReadHandle};
use mush_core::{Ctx, DefinitionRef, HookReadContext, ModError, ObjId, Registrar, World};
use mush_roll::RollError;
use mush_roll::commerce::LicenceId;
use mush_roll::commerce::holding_reader::{
    OriginalOwnerReader, OriginalReadRequest, OriginalReadResult,
};

use super::filed_holdings::FiledExecutable;
use super::holdings::{self, HeldMind};
use super::lobby::{MindChoice, SimError};
use mush_roll::commerce::personal_filed_reader::{
    PersonalFiledHolding, PersonalFiledReader, PersonalFiledRequest,
};

/// Only actual composition installs the reader handle; it stores no projection.
#[derive(Clone)]
struct OriginalAccessSeam {
    reader: OwnerReadHandle<OriginalOwnerReader>,
    personal: Option<OwnerReadHandle<PersonalFiledReader>>,
    entitlement: Option<
        OwnerReadHandle<mush_roll::commerce::entitlement::customer::EntitlementHoldingReader>,
    >,
}

/// Doc 17h §3 M8h: the original-holdings grant every composition of the original holdings
/// requested (whichever arm), for the House attestation tick's GOD-only Roll read; `None` where
/// no original holdings are composed.
pub(crate) fn reader(world: &World) -> Option<OwnerReadHandle<OriginalOwnerReader>> {
    world
        .module_state::<OriginalAccessSeam>()
        .map(|seam| seam.reader.clone())
}

pub(crate) fn register(reg: &mut Registrar<'_>, manifest: &DefinitionRef) -> Result<(), ModError> {
    if reg.module() != crate::NS {
        return Err(ModError::msg("original holdings belong to Tacsim"));
    }
    let reader = OriginalOwnerReader::request(reg, manifest)?;
    reg.state(OriginalAccessSeam {
        reader,
        personal: None,
        entitlement: None,
    });
    Ok(())
}

pub(crate) fn register_personal(
    reg: &mut Registrar<'_>,
    manifest: &DefinitionRef,
) -> Result<OwnerReadHandle<PersonalFiledReader>, ModError> {
    if reg.module() != crate::NS {
        return Err(ModError::msg("filed holdings belong to Tacsim"));
    }
    register_personal_protocol(reg, manifest, false, false)
}
pub(crate) fn register_personal_source_forms(
    reg: &mut Registrar<'_>,
    manifest: &DefinitionRef,
) -> Result<OwnerReadHandle<PersonalFiledReader>, ModError> {
    register_personal_protocol(reg, manifest, true, false)
}
pub(crate) fn register_dependent_owners(
    reg: &mut Registrar<'_>,
    manifest: &DefinitionRef,
) -> Result<OwnerReadHandle<PersonalFiledReader>, ModError> {
    register_personal_protocol(reg, manifest, true, true)
}
fn register_personal_protocol(
    reg: &mut Registrar<'_>,
    manifest: &DefinitionRef,
    source_forms: bool,
    dependent_owners: bool,
) -> Result<OwnerReadHandle<PersonalFiledReader>, ModError> {
    if reg.module() != crate::NS {
        return Err(ModError::msg("filed holdings belong to Tacsim"));
    }
    let reader = OriginalOwnerReader::request(reg, manifest)?;
    let personal = if dependent_owners {
        PersonalFiledReader::request_dependent_owners(reg, manifest)?
    } else if source_forms {
        PersonalFiledReader::request_source_forms(reg, manifest)?
    } else {
        PersonalFiledReader::request(reg, manifest)?
    };
    reg.state(OriginalAccessSeam {
        reader,
        personal: Some(personal.clone()),
        entitlement: None,
    });
    Ok(personal)
}

/// Returns the original-holdings grant so the line-mode command slot
/// (`minds::market_commands`, 17g C2a) reads its folded kinds through the
/// same grant: a requester holds one grant per definition.
pub(crate) fn register_entitlements(
    reg: &mut Registrar<'_>,
    manifest: &DefinitionRef,
) -> Result<OwnerReadHandle<OriginalOwnerReader>, ModError> {
    if reg.module() != crate::NS {
        return Err(ModError::msg("entitlement holdings belong to Tacsim"));
    }
    let reader = OriginalOwnerReader::request(reg, manifest)?;
    let entitlement =
        mush_roll::commerce::entitlement::customer::EntitlementHoldingReader::request(
            reg, manifest,
        )?;
    reg.state(OriginalAccessSeam {
        reader: reader.clone(),
        personal: None,
        entitlement: Some(entitlement),
    });
    Ok(reader)
}

#[derive(Clone)]
struct HouseAccessSeam {
    reader: OwnerReadHandle<mush_roll::commerce::house_purchase::customer::HouseHoldingReader>,
}
pub(crate) fn register_house(
    reg: &mut Registrar<'_>,
    manifest: &DefinitionRef,
) -> Result<(), ModError> {
    if reg.module() != crate::NS {
        return Err(ModError::msg("house holdings belong to Tacsim"));
    }
    let reader =
        mush_roll::commerce::house_purchase::customer::HouseHoldingReader::request(reg, manifest)?;
    reg.state(HouseAccessSeam { reader });
    Ok(())
}

pub(super) struct HoldingAccess<'a> {
    context: HookReadContext<'a>,
}
impl<'a> HoldingAccess<'a> {
    pub(super) fn from_ctx(ctx: &'a Ctx<'_>) -> Self {
        Self {
            context: ctx.hook_read_context(crate::NS),
        }
    }
    /// Existing World-only diagnostic helpers retain their M6 behavior. A
    /// detached context cannot authorize a filed choice, even with a real handle.
    pub(super) fn legacy(world: &'a World) -> Self {
        Self {
            context: HookReadContext::without_typed_state(world, crate::NS),
        }
    }
    pub(super) fn world(&self) -> &'a World {
        self.context.world()
    }
    pub(super) fn enabled(&self) -> bool {
        self.world().module_state::<OriginalAccessSeam>().is_some()
            || self.world().module_state::<HouseAccessSeam>().is_some()
    }
    fn read(&self, request: OriginalReadRequest) -> Result<OriginalReadResult, OwnerReadError> {
        let seam = self
            .world()
            .module_state::<OriginalAccessSeam>()
            .ok_or(OwnerReadError::Unavailable)?;
        self.context.read_owner(&seam.reader, request)
    }
    fn read_holding(
        &self,
        holder: Option<ObjId>,
        licence: LicenceId,
    ) -> Result<PersonalFiledHolding, OwnerReadError> {
        if let Some(house) = self.world().module_state::<HouseAccessSeam>() {
            use mush_roll::commerce::house_purchase::customer::{
                HouseHoldingRequest, HouseHoldingResult,
            };
            let request = match holder {
                Some(holder) => HouseHoldingRequest::admission(holder, licence),
                None => HouseHoldingRequest::own(licence),
            };
            match self.context.read_owner(&house.reader, request)? {
                HouseHoldingResult::Holding(holding) => return Ok(*holding),
                HouseHoldingResult::Other => {}
            }
        }
        let seam = self
            .world()
            .module_state::<OriginalAccessSeam>()
            .ok_or(OwnerReadError::Unavailable)?;
        if let Some(reader) = &seam.entitlement {
            use mush_roll::commerce::entitlement::customer::EntitlementHoldingRequest;
            return self.context.read_owner(
                reader,
                match holder {
                    Some(holder) => EntitlementHoldingRequest::admission(holder, licence),
                    None => EntitlementHoldingRequest::own(licence),
                },
            );
        }
        if let Some(reader) = &seam.personal {
            return self.context.read_owner(
                reader,
                match holder {
                    Some(holder) => PersonalFiledRequest::admission(holder, licence),
                    None => PersonalFiledRequest::own(licence),
                },
            );
        }
        self.read(match holder {
            Some(holder) => OriginalReadRequest::admission(holder, licence),
            None => OriginalReadRequest::own(licence),
        })?
        .into_holding()
        .map(PersonalFiledHolding::original)
        .ok_or(OwnerReadError::Refused)
    }
    fn executable(&self, holding: PersonalFiledHolding, holder: ObjId) -> Option<HeldMind> {
        if holding.actor() != holder {
            return None;
        }
        let sources = self.world().module_state::<crate::TacsimStore>()?;
        FiledExecutable::resolve(holding, sources)
            .ok()
            .map(FiledExecutable::into_held)
    }
    /// Called only after lobby derives `pilot` from the caller's claimed slot
    /// or the side they lead. Own reads still use core's captured actor; an alt
    /// sharing that account cannot select another character's original licence.
    pub(super) fn choose(
        &self,
        pilot: ObjId,
        word: &str,
        revision: Option<&str>,
    ) -> Result<HeldMind, RollError> {
        if let Some(id) = filed_word(word) {
            let licence = id.ok_or(RollError::Missing)?;
            if revision.is_some() {
                return Err(RollError::Missing);
            }
            let result = self
                .read_holding(None, licence)
                .map_err(|_| RollError::Missing)?;
            return self.executable(result, pilot).ok_or(RollError::Missing);
        }
        holdings::choose(self.world(), pilot, word, revision)
    }
    /// The holder is derived exclusively from the real slot/lead by this
    /// module's caller. It is a subject, never a replacement requester identity.
    pub(super) fn resolve(&self, holder: ObjId, choice: &MindChoice) -> Option<HeldMind> {
        match (choice.filed, choice.saved.as_ref()) {
            (Some(_), Some(_)) => None,
            (Some(licence), None) => {
                let result = self.read_holding(Some(holder), licence).ok()?;
                self.executable(result, holder)
            }
            (None, saved) => holdings::resolve_choice(self.world(), holder, &choice.mind, saved),
        }
    }
    /// One owner-bounded page per command. Never render the internal projection,
    /// source text, certificate, account, revision key or another holder's data.
    pub(super) fn list(&self, after: Option<ObjId>) -> Result<Vec<String>, SimError> {
        let page = self
            .read(OriginalReadRequest::own_page(after))
            .map_err(|_| unavailable())?;
        let mut lines = Vec::new();
        match page.holding() {
            Some(holding) => {
                let profiles = holding
                    .revision()
                    .envelope()
                    .profiles
                    .as_object()
                    .ok_or_else(unavailable)?;
                lines.push(format!(
                    "filed:{}: {} (certified Class {}).",
                    holding.licence().get(),
                    holding.name(),
                    class_word(holding.minimum())
                ));
                lines.push(format!(
                    "Profiles: {}.",
                    profiles
                        .keys()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                lines.push(format!(
                    "Choose with SIM MIND filed:{} [PROFILE <name>].",
                    holding.licence().get()
                ));
            }
            None => lines.push("No eligible original-owner holding on this page.".into()),
        }
        if let Some(next) = page.next_after() {
            lines.push(format!("Continue with SIM MIND /filed AFTER {}.", next.0));
        }
        Ok(lines)
    }
}

/// **The holding reads an IC dispatch makes inside its preparation** (doc 17h cut M8e E4; doc 23
/// S.4a): the composed seams' own grants, read through `PrepareCtx::read_owner_preimage` as
/// the action's actor — the lobby's [`HoldingAccess`] reads, at an authoritative act's prepare
/// instead of a command's. `None` when no holdings seam is composed (a Standard world).
#[derive(Clone)]
pub(crate) struct DispatchReads {
    original: OriginalAccessSeam,
    house: Option<HouseAccessSeam>,
}

impl DispatchReads {
    /// The composed seams, if any.
    pub(crate) fn of(world: &World) -> Option<DispatchReads> {
        Some(DispatchReads {
            original: world.module_state::<OriginalAccessSeam>()?.clone(),
            house: world.module_state::<HouseAccessSeam>().cloned(),
        })
    }

    /// **The actor's paid licence as `+minds/holdings` reads it** (the line-mode `OwnPaid`):
    /// its phase, revoked bit and selling bit; `org` the organisation it is held by and read
    /// for (M8c B3). `None` for a licence the actor does not hold, cannot see or cannot read.
    pub(crate) fn paid(
        &self,
        ctx: &mut mush_core::PrepareCtx<'_>,
        txn: &mut mush_core::ActionTxn<'_>,
        licence: LicenceId,
        org: Option<mush_org::OrgId>,
    ) -> Option<mush_roll::commerce::command_reader::PaidHoldingView> {
        use mush_roll::commerce::command_reader::{RollCommandReadRequest, RollCommandReadResult};
        ctx.read_owner_preimage(
            txn,
            &self.original.reader,
            OriginalReadRequest::command(RollCommandReadRequest::own_paid_for(licence, org)),
        )
        .ok()
        .and_then(OriginalReadResult::into_command)
        .and_then(RollCommandReadResult::into_own_paid)
    }

    /// **The actor's owner licence as `+minds/holdings` reads it** (doc 17h cut M8e fix round
    /// 1, J2; the line-mode `OwnHoldings { section: Owner }`, the owner projection's rows): the
    /// row naming `licence` as its owner licence — its owner phase, selling bit and name — over
    /// at most [`MAX_OWNER_PAGES`] pages; `org` the organisation read for (M8c B3). `None` for
    /// a licence no owner row names.
    pub(crate) fn owner(
        &self,
        ctx: &mut mush_core::PrepareCtx<'_>,
        txn: &mut mush_core::ActionTxn<'_>,
        licence: LicenceId,
        org: Option<mush_org::OrgId>,
    ) -> Option<mush_roll::commerce::command_reader::OwnerHoldingView> {
        use mush_roll::commerce::command_reader::{
            HoldingSection, RollCommandReadRequest, RollCommandReadResult,
        };
        let mut after = None;
        for _ in 0..MAX_OWNER_PAGES {
            let page = ctx
                .read_owner_preimage(
                    txn,
                    &self.original.reader,
                    OriginalReadRequest::command(RollCommandReadRequest::own_holdings_for(
                        HoldingSection::Owner,
                        after,
                        org,
                    )),
                )
                .ok()
                .and_then(OriginalReadResult::into_command)
                .and_then(RollCommandReadResult::into_owner_holdings)?;
            if let Some(row) = page
                .rows()
                .iter()
                .find(|row| row.owner_licence() == licence)
            {
                return Some(row.clone());
            }
            after = Some(page.next()?);
        }
        None
    }

    /// **The executable of `licence` admitted for `holder`**, as the lobby's
    /// `HoldingAccess::resolve` reads it for a filed choice (the House, entitlement, personal
    /// and original readers, in that order), resolved against the store's sources. `None`
    /// when no reader admits it.
    pub(crate) fn executable(
        &self,
        ctx: &mut mush_core::PrepareCtx<'_>,
        txn: &mut mush_core::ActionTxn<'_>,
        holder: ObjId,
        licence: LicenceId,
    ) -> Option<HeldMind> {
        let holding = self.holding(ctx, txn, holder, licence)?;
        if holding.actor() != holder {
            return None;
        }
        let sources = ctx.world().ok()?.module_state::<crate::TacsimStore>()?;
        FiledExecutable::resolve(holding, sources)
            .ok()
            .map(FiledExecutable::into_held)
    }

    fn holding(
        &self,
        ctx: &mut mush_core::PrepareCtx<'_>,
        txn: &mut mush_core::ActionTxn<'_>,
        holder: ObjId,
        licence: LicenceId,
    ) -> Option<PersonalFiledHolding> {
        if let Some(house) = &self.house {
            use mush_roll::commerce::house_purchase::customer::{
                HouseHoldingRequest, HouseHoldingResult,
            };
            if let Ok(HouseHoldingResult::Holding(holding)) = ctx.read_owner_preimage(
                txn,
                &house.reader,
                HouseHoldingRequest::admission(holder, licence),
            ) {
                return Some(*holding);
            }
        }
        if let Some(reader) = &self.original.entitlement {
            use mush_roll::commerce::entitlement::customer::EntitlementHoldingRequest;
            return ctx
                .read_owner_preimage(
                    txn,
                    reader,
                    EntitlementHoldingRequest::admission(holder, licence),
                )
                .ok();
        }
        if let Some(reader) = &self.original.personal {
            return ctx
                .read_owner_preimage(
                    txn,
                    reader,
                    PersonalFiledRequest::admission(holder, licence),
                )
                .ok();
        }
        ctx.read_owner_preimage(
            txn,
            &self.original.reader,
            OriginalReadRequest::admission(holder, licence),
        )
        .ok()?
        .into_holding()
        .map(PersonalFiledHolding::original)
    }
}

/// The most owner-holdings pages one dispatch reads looking for its licence (J2).
pub(crate) const MAX_OWNER_PAGES: usize = 32;

/// `filed:<n>`'s licence, as the lobby reads the word (doc 17h cut M8e E4: `mind=filed:<n>`).
pub(crate) fn filed_licence(word: &str) -> Option<Option<LicenceId>> {
    filed_word(word)
}

fn class_word(class: mush_roll::commerce::rating::RatingClass) -> &'static str {
    use mush_roll::commerce::rating::RatingClass;
    match class {
        RatingClass::I => "I",
        RatingClass::II => "II",
        RatingClass::III => "III",
    }
}
fn unavailable() -> SimError {
    SimError::MindAdmission {
        reason: "Original-owner holdings are unavailable.".into(),
    }
}
fn number(text: &str) -> Option<u64> {
    if text.is_empty() || text.len() > 20 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let value = text.parse::<u64>().ok()?;
    (value.to_string() == text).then_some(value)
}
fn filed_word(word: &str) -> Option<Option<LicenceId>> {
    let (kind, value) = word.trim().split_once(':')?;
    kind.eq_ignore_ascii_case("filed")
        .then(|| number(value).and_then(|n| LicenceId::new(n).ok()))
}
pub(super) fn parse_cursor(text: &str) -> Option<ObjId> {
    number(text).and_then(|n| u32::try_from(n).ok()).map(ObjId)
}
pub(super) fn filed_selection(held: &HeldMind) -> Result<Option<LicenceId>, SimError> {
    let crate::arena::profile_seal::MindOrigin::Filed { licence, .. } = &held.identity.origin
    else {
        return Ok(None);
    };
    if held.saved.is_some() {
        return Err(unavailable());
    }
    number(licence)
        .and_then(|n| LicenceId::new(n).ok())
        .map(Some)
        .ok_or_else(unavailable)
}

#[cfg(test)]
mod tests {
    use super::super::lobby::MindRequest;
    use super::*;

    #[test]
    fn selectors_are_unambiguous_and_page_cursors_are_bounded() {
        assert_eq!(filed_word("filed:1"), Some(LicenceId::new(1).ok()));
        assert_eq!(
            filed_word("FILED:18446744073709551615"),
            Some(LicenceId::new(u64::MAX).ok())
        );
        for value in [
            "0",
            "01",
            "+1",
            "-1",
            "1@latest",
            "18446744073709551616",
            "",
        ] {
            assert_eq!(filed_word(&format!("filed:{value}")), Some(None), "{value}");
        }
        assert_eq!(filed_word("draft:1"), None);
        assert_eq!(
            MindRequest::parse("/filed"),
            Ok(MindRequest::FiledRead { after: None })
        );
        assert_eq!(
            MindRequest::parse("/FILED after 42"),
            Ok(MindRequest::FiledRead {
                after: Some(ObjId(42))
            })
        );
        for line in [
            "/filed AFTER",
            "/filed AFTER -1",
            "/filed AFTER 01",
            "/filed AFTER 4294967296",
            "/filed AFTER 1 extra",
            "/filed PROFILE x",
        ] {
            assert!(MindRequest::parse(line).is_err(), "{line}");
        }
        assert!(matches!(
            MindRequest::parse("filed:7 PROFILE Default"),
            Ok(MindRequest::Choose { .. })
        ));
        assert_eq!(
            MindRequest::parse("Filed PROFILE Line"),
            Ok(MindRequest::Choose {
                mind: "Filed".into(),
                revision: None,
                profiles: vec!["Line".into()],
            }),
            "a legal M6 draft name remains a normal selection"
        );
    }

    #[test]
    fn world_only_access_preserves_doctrine_but_cannot_resolve_filed_choices() {
        let world = mush_testkit::TestWorld::new();
        let access = HoldingAccess::legacy(world.world());
        let actor = ObjId(7);
        let ordinary = MindChoice {
            filed: None,
            saved: None,
            mind: "Standard Doctrine".into(),
            profiles: vec![],
        };
        assert_eq!(access.resolve(actor, &ordinary), Some(HeldMind::doctrine()));
        assert_eq!(
            access.choose(actor, "Standard", None),
            Ok(HeldMind::doctrine())
        );
        let filed = MindChoice {
            filed: LicenceId::new(1).ok(),
            ..ordinary.clone()
        };
        assert!(access.resolve(actor, &filed).is_none());
        let mut crossed = filed.clone();
        crossed.saved = Some(super::super::draft_holdings::SavedChoice {
            character: actor,
            mind: 1,
            selection: mush_roll::Selection::Follow { mind: 1 },
        });
        assert!(
            access.resolve(actor, &crossed).is_none(),
            "two selection namespaces cannot supply fallback authority"
        );
        assert!(access.choose(actor, "filed:1", None).is_err());
        assert!(access.list(None).is_err());
        let encoded = serde_json::to_value(&ordinary).unwrap();
        assert!(encoded.get("filed").is_none());
        assert_eq!(
            serde_json::from_value::<MindChoice>(encoded).unwrap(),
            ordinary
        );
        let encoded = serde_json::to_value(&filed).unwrap();
        assert_eq!(
            serde_json::from_value::<MindChoice>(encoded).unwrap(),
            filed
        );
    }
}
