//! **The one read an inspection makes**, and the receipt line it prints (doc 17h cut M8a,
//! A4; the fix round's F2, F3, F5).
//!
//! `read` is pure over `&World` and a typed reader ([`Reads`]: the command's `Ctx`, the
//! provider's `ActionTxn`, a test's store), and it owns the known-hull gate ([`known`]), so the
//! line command and the typed action inspect the same set of hulls; `hull_facts` is its pure
//! core over a unit. The seal check is the IC audit's own (`crate::minds::ic_sortie::audit`:
//! the sealed core component, its class and the governor against what the hull reads now),
//! as a read. [`receipt`] renders the facts the provider records as the one line the driver
//! prints in place of `Done.`.

#[cfg(test)]
use mush_core::ActionStateStore;
use mush_core::{ActionTxn, Ctx, ObjId, StateFamilyHandle, World};
use mush_org::OrgStore;
use mush_tactics::TacticalUnit;

use super::{HullClass, InspectionFacts, Provenance, SealCheck, UngovernedNote};
use crate::arena::core::{CoreClass, CoreHardware, read_core};
use crate::port::families::PortHandles;
use crate::surface::black_box::SealFacts;

/// A typed-state read through whichever context makes it.
pub trait Reads {
    /// The record under `key`, cloned; `None` when absent or unreadable.
    fn typed<K, V>(&mut self, handle: &StateFamilyHandle<K, V>, key: &K) -> Option<V>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static;
}

impl Reads for ActionTxn<'_> {
    fn typed<K, V>(&mut self, handle: &StateFamilyHandle<K, V>, key: &K) -> Option<V>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        self.get(handle, key).ok().flatten().cloned()
    }
}

/// A command's context as a reader (`Ctx::action_state`), borrowed shared beside its world.
pub struct CtxRead<'c, 'a>(pub &'c Ctx<'a>);

impl Reads for CtxRead<'_, '_> {
    fn typed<K, V>(&mut self, handle: &StateFamilyHandle<K, V>, key: &K) -> Option<V>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        self.0.action_state(handle, key).ok().flatten().cloned()
    }
}

/// The engine's typed store as a reader (`Engine::action_state`), for a read beside its
/// world outside any command or action: a test's.
#[cfg(test)]
pub(crate) struct StoreRead<'s>(pub &'s ActionStateStore);

#[cfg(test)]
impl Reads for StoreRead<'_> {
    fn typed<K, V>(&mut self, handle: &StateFamilyHandle<K, V>, key: &K) -> Option<V>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        self.0.get(handle, key).ok().flatten().cloned()
    }
}

/// **What the read found**: the record's facts and the words around them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HullFacts {
    /// The record's half.
    pub facts: InspectionFacts,
    /// The hull's name.
    pub name: String,
    /// The open trip's seal facts, when a trip is open.
    pub seal: Option<SealFacts>,
    /// The IC instant the open trip opened.
    pub opened_ic: Option<i64>,
}

/// The record's class word for the arena's.
const fn hull_class(class: CoreClass) -> HullClass {
    match class {
        CoreClass::Zero => HullClass::Zero,
        CoreClass::I => HullClass::I,
        CoreClass::II => HullClass::II,
        CoreClass::III => HullClass::III,
    }
}

/// The arena's class for the record's word.
const fn core_class(class: HullClass) -> CoreClass {
    match class {
        HullClass::Zero => CoreClass::Zero,
        HullClass::I => CoreClass::I,
        HullClass::II => CoreClass::II,
        HullClass::III => CoreClass::III,
    }
}

/// The core component's asset object: the economy's titled core, when the component carries
/// one (the refit's `SetComponentAsset`).
fn core_asset(unit: &TacticalUnit, hardware: Option<&CoreHardware>) -> Option<ObjId> {
    let component = mush_tactics::ComponentId::new(hardware?.component.as_str()).ok()?;
    unit.component(&component)?.asset
}

/// **The seal checked against the hardware now** (the IC audit's check,
/// `ic_sortie::audit`; the M8a fix round's F5): the sealed core component, that core's class
/// and the governor against the fitted core read now; a hull with no core is the ROM — no
/// component, class 0, governed. `NoSeal` with no open trip.
#[must_use]
pub fn seal_check(seal: Option<&SealFacts>, hardware: Option<&CoreHardware>) -> SealCheck {
    let Some(seal) = seal else {
        return SealCheck::NoSeal;
    };
    // A seal with no `core` reads as the ROM's (no component, class 0, governed). Against a
    // cored hull that differs, which is right: such a seal was sealed on a hull with no core.
    // No older seal can stand on a cored hull and read `Differs` for want of the component —
    // pre-launch, no IC hull carried a core before M8b, so every seal of a cored hull is
    // written with its component.
    let sealed = (
        seal.core.as_deref(),
        seal.core_class.unwrap_or(CoreClass::Zero),
        seal.governed,
    );
    let now = hardware.map_or((None, CoreClass::Zero, true), |hw| {
        (Some(hw.component.as_str()), hw.class, hw.governed)
    });
    if sealed == now {
        SealCheck::Matches
    } else {
        SealCheck::Differs
    }
}

/// **The record's facts over a unit** (the brief's §2.3): the fitted core, its class and
/// governor, the provenance handed in, and the open trip's seal checked against the hardware.
#[must_use]
pub fn hull_facts(
    hull: ObjId,
    unit: &TacticalUnit,
    provenance: Provenance,
    seal: Option<&SealFacts>,
) -> InspectionFacts {
    let hardware = read_core(unit);
    InspectionFacts {
        hull,
        core: core_asset(unit, hardware.as_ref()),
        class: hull_class(hardware.as_ref().map_or(CoreClass::Zero, |hw| hw.class)),
        governed: hardware.as_ref().map(|hw| hw.governed),
        provenance,
        seal_digest: seal.map(|facts| facts.seal_digest.clone()),
        seal_check: seal_check(seal, hardware.as_ref()),
        licence_state: seal.and_then(|facts| facts.licence_state),
        ungoverned_note: None,
        holder: None,
    }
}

/// **The ungoverning refit's note on `core`'s title** (doc 17h cut M8b C2), read from the
/// economy (`EconomyStore::core_provenance_note`): a record field, printed by no reply line.
/// `Err` when the economy's refit history cannot be read: the note fails closed (G9), and the
/// inspection refuses rather than record "never ungoverned". A world with no economy has no
/// managed core to note.
fn ungoverned_note(w: &World, core: ObjId) -> Result<Option<UngovernedNote>, ()> {
    if w.module_state_owner(std::any::TypeId::of::<mush_economy::EconomyStore>())
        .is_none_or(|owner| owner.as_str() != "economy")
    {
        return Ok(None);
    }
    let Some(economy) = w.module_state::<mush_economy::EconomyStore>() else {
        return Ok(None);
    };
    Ok(economy
        .core_provenance_note(core)
        .map_err(|_| ())?
        .map(|note| match note {
            mush_economy::CoreProvenanceNote::Ungoverned { facility, at_ic } => {
                UngovernedNote { facility, at_ic }
            }
        }))
}

/// **A hull the programme knows** (the fix round's F2: the provider's gate, so the line
/// command and the typed action inspect one set):
/// `crate::surface::recovery::world_hulls` (the port maps' hull rows and the yards' hulls)
/// plus every port map's rig rows — the lease pool's (`map.vehicles.rigs`) and the yard's —
/// the terrain vehicles the hull-row set leaves out.
#[must_use]
pub fn known(w: &World, hull: ObjId) -> bool {
    use crate::port::vehicle::{RigRows, rig_rows};
    crate::surface::recovery::world_hulls(w).contains(&hull)
        || w.module_state::<crate::store::TacsimStore>()
            .is_some_and(|store| {
                store.port_maps().any(|map| {
                    [RigRows::Pool, RigRows::Yard]
                        .into_iter()
                        .flat_map(|which| rig_rows(map, which))
                        .any(|rig| rig.tv == hull.0)
                })
            })
}

/// **Every hull [`known`] admits** (doc 17h cut M8d): the hull rows, the yards' hulls and every
/// port map's rig rows, for the boot passes that re-read each hull's rings.
#[must_use]
pub fn known_hulls(w: &World) -> std::collections::BTreeSet<ObjId> {
    use crate::port::vehicle::{RigRows, rig_rows};
    let mut hulls = crate::surface::recovery::world_hulls(w);
    if let Some(store) = w.module_state::<crate::store::TacsimStore>() {
        for map in store.port_maps() {
            for which in [RigRows::Pool, RigRows::Yard] {
                hulls.extend(rig_rows(map, which).iter().map(|rig| ObjId(rig.tv)));
            }
        }
    }
    hulls
}

/// **Read `hull`** (the brief's §2.3): `None` when the programme does not know the hull
/// ([`known`]) or it has no tactical unit (nothing to inspect; the command says
/// `No such hull.`, the provider refuses with its one sentence).
pub fn read<R: Reads>(w: &World, reader: &mut R, hull: ObjId) -> Option<HullFacts> {
    if !known(w, hull) {
        return None;
    }
    read_unit(w, reader, hull)
}

/// [`read`] without the known-hull gate: any object with a tactical unit (a rig without port
/// maps reads its refitted hull here).
pub(crate) fn read_unit<R: Reads>(w: &World, reader: &mut R, hull: ObjId) -> Option<HullFacts> {
    let unit = mush_tactics::module::unit_of(w, hull)?;
    let provenance = core_asset(unit, read_core(unit).as_ref())
        .map_or(Provenance::Unknown, |core| {
            crate::minds::provenance::provenance_of(w, reader, core)
        });
    let (seal, opened_ic) = open_trip(w, reader, hull);
    let mut facts = hull_facts(hull, unit, provenance, seal.as_ref());
    // G9: a note that cannot be read refuses the read (the provider's uniform refusal).
    // Doc 17h cut M8d K1: the title holder now, recorded with the inspection.
    facts.holder = w
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|economy| {
            economy
                .snapshot()
                .asset(hull)
                .map(|a| a.title_owner.clone())
        });
    facts.ungoverned_note = match facts.core {
        Some(core) => ungoverned_note(w, core).ok()?,
        None => None,
    };
    Some(HullFacts {
        facts,
        name: crate::port::coop::name_of(w, hull),
        seal,
        opened_ic,
    })
}

/// The open trip's seal and opening instant: a rig's open sortie, else a ship's open epoch.
fn open_trip<R: Reads>(w: &World, reader: &mut R, hull: ObjId) -> (Option<SealFacts>, Option<i64>) {
    let Some(handles) = w.module_state::<PortHandles>() else {
        return (None, None);
    };
    if let Some(open) = reader
        .typed(
            &handles.families.excursions,
            &crate::surface::excursion::RigKey(hull),
        )
        .and_then(|record| record.sortie)
    {
        return (open.seal, Some(open.opened_ic));
    }
    if let Some(open) = reader
        .typed(
            &handles.families.epochs,
            &crate::port::epoch::EpochKey(hull),
        )
        .and_then(|record| record.open)
    {
        return (open.seal, Some(open.opened_ic));
    }
    (None, None)
}

/// `governed`, `ungoverned`.
const fn governed_word(governed: bool) -> &'static str {
    if governed { "governed" } else { "ungoverned" }
}

/// The most characters of an importer's name the receipt prints (an org's name has no cap;
/// the receipt holds the 120-column floor at worst-case words).
const RECEIPT_ORG_CHARS: usize = 24;

/// **The receipt** (the fix round's F3, the lead's ruling): one line over the facts the
/// provider records, which the driver prints in place of `Done.` — the driver's receipt is one
/// line (`ProjectedActionResult` refuses a newline), so the mind, the out-since stamp and the
/// licence state stay in the record and `+hull/box`. `Hull #<n>: <the ROM | Class X governed
/// | ungoverned> · provenance <certified (<importer>, <grade>) | heirloom | grey | unknown> ·
/// <no trip open | no seal | seal <8> matches | seal <8> differs>`; at most 120 columns.
#[must_use]
pub fn receipt(w: &World, facts: &InspectionFacts, trip_open: bool) -> String {
    let importer = match &facts.provenance {
        Provenance::Certified { importer, .. } => org_name(w, *importer),
        _ => String::new(),
    };
    receipt_words(facts, trip_open, &importer)
}

/// [`receipt`]'s words over the importer's name it looked up. Pure, so the width test can feed
/// worst-case names.
fn receipt_words(facts: &InspectionFacts, trip_open: bool, importer: &str) -> String {
    let mut hardware = crate::surface::hull::class_word(core_class(facts.class)).to_owned();
    if let Some(governed) = facts.governed {
        hardware.push(' ');
        hardware.push_str(governed_word(governed));
    }
    let provenance = match &facts.provenance {
        Provenance::Certified { grade, .. } => {
            let mut short: String = importer.chars().take(RECEIPT_ORG_CHARS).collect();
            if importer.chars().count() > RECEIPT_ORG_CHARS {
                short.pop();
                short.push('…');
            }
            format!("certified ({short}, {})", grade_word(*grade))
        }
        Provenance::Heirloom => "heirloom".to_owned(),
        Provenance::Grey => "grey".to_owned(),
        Provenance::Unknown => "unknown".to_owned(),
    };
    let digest = facts
        .seal_digest
        .as_deref()
        .map(|d| d.get(..8).unwrap_or(d));
    let trip = match (trip_open, facts.seal_check, digest) {
        (false, _, _) => "no trip open".to_owned(),
        (true, SealCheck::Matches, Some(d)) => format!("seal {d} matches"),
        (true, SealCheck::Differs, Some(d)) => format!("seal {d} differs"),
        (true, _, _) => "no seal".to_owned(),
    };
    format!(
        "Hull #{}: {hardware} · provenance {provenance} · {trip}",
        facts.hull.0
    )
}

/// The organization's name, or a plain word when the Org store cannot name it.
fn org_name(w: &World, org: mush_org::OrgId) -> String {
    w.module_state::<OrgStore>()
        .and_then(|store| store.state().org(org).map(|o| o.name.to_string()))
        .unwrap_or_else(|| "an organization the Org store does not know".to_owned())
}

/// `I`, `II`, `III`: the die's class, the economy's word.
const fn grade_word(class: mush_economy::core_integration::CoreClass) -> &'static str {
    match class {
        mush_economy::core_integration::CoreClass::I => "I",
        mush_economy::core_integration::CoreClass::II => "II",
        mush_economy::core_integration::CoreClass::III => "III",
    }
}

#[cfg(test)]
mod tests {
    use mush_tactics::{CellAddr, CellId, ComponentId, UnitMutationOp, UnitMutationPlan};

    use super::*;
    use crate::arena::core::tests::cored;
    use crate::definitions::unit::CORE_CLASS_CELL;
    use crate::minds::licence_state::SealLicenceState;

    /// A seal's facts as a trip would record them, at `class` and `governed`, a draft licence;
    /// a class above 0 sealed the fixture's `core` component at that class.
    fn sealed(class: CoreClass, governed: bool) -> SealFacts {
        SealFacts {
            mind: "Fixture Mind".into(),
            source: "fixture".into(),
            card_digest: "c".repeat(64).into(),
            class,
            governed,
            licence: "draft".into(),
            licence_state: Some(SealLicenceState::Draft),
            seal_digest: "d".repeat(64).into(),
            fallback: None,
            core: (class != CoreClass::Zero).then(|| "core".into()),
            core_class: (class != CoreClass::Zero).then_some(class),
            admissions: Vec::new(),
        }
    }

    /// **5. A cored hull** (the brief's test 5, A4, A6): over `arena/core.rs`'s `cored()`
    /// fixture (Class II in its one core component; ungoverned by its profile, the fixture's
    /// own setting, so the governor reads `false` where the brief's sketch said governed) the
    /// facts read class II, the fixture's governor and the provenance handed in; the seal check
    /// `Matches` a seal at the fixture's class and governor, `Differs` once the core class cell
    /// is moved after the seal, and is `NoSeal` with no trip open. The fixture's component
    /// carries no asset, so `core` is `None` and the provenance is whatever the economy said.
    #[test]
    fn m8a_a_cored_hull_reads_its_class_and_checks_its_seal() {
        let unit = cored(2, 3);
        let certified = Provenance::Certified {
            importer: mush_org::OrgId(7),
            grade: mush_economy::core_integration::CoreClass::II,
        };
        let seal = sealed(CoreClass::II, false);
        let facts = hull_facts(ObjId(4), &unit, certified.clone(), Some(&seal));
        assert_eq!(facts.hull, ObjId(4));
        assert_eq!(facts.core, None, "the fixture's component carries no asset");
        assert_eq!(facts.class, HullClass::II);
        assert_eq!(
            facts.governed,
            Some(false),
            "the fixture's profile is ungoverned"
        );
        assert_eq!(facts.provenance, certified);
        assert_eq!(facts.seal_digest.as_deref(), Some("d".repeat(64).as_str()));
        assert_eq!(facts.seal_check, SealCheck::Matches);
        assert_eq!(facts.licence_state, Some(SealLicenceState::Draft));
        // A seal at the other governor differs.
        assert_eq!(
            hull_facts(
                ObjId(4),
                &unit,
                Provenance::Unknown,
                Some(&sealed(CoreClass::II, true))
            )
            .seal_check,
            SealCheck::Differs
        );
        // The core class cell moved after the seal: the hull reads Class I now.
        let plan = UnitMutationPlan::new(
            vec![UnitMutationOp::SetCell {
                addr: CellAddr::Component {
                    component: ComponentId::new("core").expect("a component"),
                    cell: CellId::new(CORE_CLASS_CELL).expect("a cell"),
                },
                value: 1,
            }],
            None,
            None,
        )
        .expect("a plan");
        let (moved, _) = mush_tactics::apply(&unit, &plan, 0).expect("the plan applies");
        let facts = hull_facts(ObjId(4), &moved, Provenance::Unknown, Some(&seal));
        assert_eq!(facts.class, HullClass::I);
        assert_eq!(facts.seal_check, SealCheck::Differs);
        // No trip open: no seal, no digest, no state.
        let facts = hull_facts(ObjId(4), &unit, Provenance::Unknown, None);
        assert_eq!(
            (facts.seal_check, facts.seal_digest, facts.licence_state),
            (SealCheck::NoSeal, None, None)
        );
        // A bare hull is the ROM: class 0, no governor, and a ROM seal matches it.
        let bare = crate::arena::test_support::tactical_unit(ObjId(5));
        let facts = hull_facts(
            ObjId(5),
            &bare,
            Provenance::Unknown,
            Some(&sealed(CoreClass::Zero, true)),
        );
        assert_eq!(
            (facts.class, facts.governed, facts.core),
            (HullClass::Zero, None, None)
        );
        assert_eq!(facts.seal_check, SealCheck::Matches);
    }

    /// **F4: a fitted core whose class cell reads 0 is still a core**: over `cored(0, 3)` the
    /// facts read class 0 with the governor read (the record's no-core rule keys on the
    /// governor, `mush_roll`'s `m8a_the_record_validates_its_shape_and_words`).
    #[test]
    fn m8a_a_class_zero_core_is_a_core() {
        let facts = hull_facts(ObjId(4), &cored(0, 3), Provenance::Unknown, None);
        assert_eq!(facts.class, HullClass::Zero);
        assert_eq!(
            facts.governed,
            Some(false),
            "the fixture's governor is read"
        );
    }

    /// **F5: the seal check compares the component**: a seal at the fixture's class and
    /// governor over another component differs; a seal that names no component (the ROM's)
    /// against a cored hull differs too.
    #[test]
    fn m8a_the_seal_check_compares_the_component() {
        let unit = cored(2, 3);
        let mut other = sealed(CoreClass::II, false);
        other.core = Some("spare".into());
        assert_eq!(
            hull_facts(ObjId(4), &unit, Provenance::Unknown, Some(&other)).seal_check,
            SealCheck::Differs
        );
        let mut romless = sealed(CoreClass::II, false);
        romless.core = None;
        romless.core_class = None;
        assert_eq!(
            hull_facts(ObjId(4), &unit, Provenance::Unknown, Some(&romless)).seal_check,
            SealCheck::Differs
        );
    }

    /// **F1/F3: the receipt is one line of at most 120 columns** at the worst-case words — the
    /// largest hull number, Class III ungoverned, a 40-character importer (clipped), a seal that
    /// differs — and reads as the ruling's shape for the ROM with no trip and a trip with no
    /// seal.
    #[test]
    fn m8a_the_receipt_fits_one_hundred_and_twenty_columns() {
        let worst = InspectionFacts {
            hull: ObjId(u32::MAX),
            core: Some(ObjId(u32::MAX)),
            class: HullClass::III,
            governed: Some(false),
            provenance: Provenance::Certified {
                importer: mush_org::OrgId(u64::MAX),
                grade: mush_economy::core_integration::CoreClass::III,
            },
            seal_digest: Some("d".repeat(64).into()),
            seal_check: SealCheck::Differs,
            licence_state: Some(SealLicenceState::Recalled),
            ungoverned_note: None,
            holder: None,
        };
        let line = receipt_words(&worst, true, &"W".repeat(40));
        assert!(
            line.chars().count() <= 120,
            "{} columns: {line}",
            line.chars().count()
        );
        assert!(!line.contains('\n') && !line.contains("sortie"), "{line}");
        assert_eq!(
            line,
            format!(
                "Hull #{}: Class III ungoverned · provenance certified ({}…, III) · seal \
                 dddddddd differs",
                u32::MAX,
                "W".repeat(23)
            )
        );
        let rom = InspectionFacts {
            hull: ObjId(546),
            core: None,
            class: HullClass::Zero,
            governed: None,
            provenance: Provenance::Unknown,
            seal_digest: None,
            seal_check: SealCheck::NoSeal,
            licence_state: None,
            ungoverned_note: None,
            holder: None,
        };
        assert_eq!(
            receipt_words(&rom, false, ""),
            "Hull #546: the ROM · provenance unknown · no trip open"
        );
        assert_eq!(
            receipt_words(&rom, true, ""),
            "Hull #546: the ROM · provenance unknown · no seal"
        );
    }
}
