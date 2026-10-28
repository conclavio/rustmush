//! **The one read an inspection makes**, and the lines it prints (doc 17h cut M8a, A4).
//!
//! `read` is pure over `&World` and a typed reader ([`Reads`]: the command's `Ctx`, the
//! provider's `ActionTxn`, a test's store), so the command's reply and the provider's record
//! are one code path over the same facts; `hull_facts` is its pure core over a unit. The seal
//! check is the IC audit's own (`crate::minds::ic_sortie::audit`: the seal's class and governor
//! against what the hull reads now), as a read.

use mush_core::{ActionStateStore, ActionTxn, Ctx, ObjId, StateFamilyHandle, World};
use mush_org::OrgStore;
use mush_tactics::TacticalUnit;
use smol_str::SmolStr;

use super::{HullClass, InspectionFacts, Provenance, SealCheck};
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
pub struct StoreRead<'s>(pub &'s ActionStateStore);

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
/// `ic_sortie::audit`): the seal's class and governor against the fitted core; a hull with no
/// core is the ROM, governed. `NoSeal` with no open trip.
#[must_use]
pub fn seal_check(seal: Option<&SealFacts>, hardware: Option<&CoreHardware>) -> SealCheck {
    let Some(seal) = seal else {
        return SealCheck::NoSeal;
    };
    let (class, governed) = hardware.map_or((CoreClass::Zero, true), |hw| (hw.class, hw.governed));
    if seal.class == class && seal.governed == governed {
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
    }
}

/// **Read `hull`** (the brief's §2.3): `None` when the hull has no tactical unit (nothing to
/// inspect; the command says `No such hull.`, the provider refuses).
pub fn read<R: Reads>(w: &World, reader: &mut R, hull: ObjId) -> Option<HullFacts> {
    let unit = mush_tactics::module::unit_of(w, hull)?;
    let provenance = core_asset(unit, read_core(unit).as_ref())
        .map_or(Provenance::Unknown, |core| {
            crate::minds::provenance::provenance_of(w, reader, core)
        });
    let (seal, opened_ic) = open_trip(w, reader, hull);
    Some(HullFacts {
        facts: hull_facts(hull, unit, provenance, seal.as_ref()),
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

/// **The reply, in words** (the brief's §2.3: A3's vocabulary, the provenance and the seal
/// check): the hull and its core; its provenance; its open trip's seal, or none. Three
/// lines; the command's staged line makes four.
#[must_use]
pub fn lines(w: &World, found: &HullFacts) -> Vec<String> {
    let f = &found.facts;
    let mut hull = format!(
        "Hull #{} {} · {}",
        f.hull.0,
        found.name,
        crate::surface::hull::class_word(core_class(f.class))
    );
    if let Some(governed) = f.governed {
        hull.push_str(" · ");
        hull.push_str(governed_word(governed));
    }
    let provenance = match &f.provenance {
        Provenance::Certified { importer, grade } => format!(
            "provenance certified by {}, grade {}",
            org_name(w, *importer),
            grade_word(*grade)
        ),
        Provenance::Heirloom => "provenance heirloom".to_owned(),
        Provenance::Grey => "provenance grey".to_owned(),
        Provenance::Unknown => "provenance unknown".to_owned(),
    };
    let trip = match (&found.seal, found.opened_ic) {
        (Some(seal), Some(opened_ic)) => {
            // A seal recorded before M8a carries no state: the licence label stands in, as
            // `+hull/box` prints it.
            let state: SmolStr = seal
                .licence_state
                .map_or_else(|| seal.licence.clone(), |state| state.as_str().into());
            let digest = seal.seal_digest.get(..8).unwrap_or(&seal.seal_digest);
            format!(
                "out since {} · {} · licence {} · seal {} · {}",
                crate::port::licence::ic_stamp(opened_ic),
                seal.mind,
                state,
                digest,
                match f.seal_check {
                    SealCheck::Matches => "seal matches the fitted core",
                    SealCheck::Differs => "seal differs from the fitted core",
                    SealCheck::NoSeal => "no seal",
                }
            )
        }
        _ => "no trip open".to_owned(),
    };
    vec![hull, provenance, trip]
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

    /// A seal's facts as a trip would record them, at `class` and `governed`, a draft licence.
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
}
