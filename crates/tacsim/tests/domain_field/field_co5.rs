//! **Doc 47 unit CO.5 — hired crew in armed rigs** (R-47-6: hired crew may be seated in armed
//! rigs), over the composed world (`driving`'s keyed rig), every step the production command
//! (`roster_rig`, MW.15's rig).
//!
//! 1. **A hire under a combat contract flies and fires the escort tank**
//!    ([`co5_a_combat_hire_is_seated_in_the_escort_tank_and_fires_under_its_contract`]). A captain
//!    leads a combat roster from a prospector, and Rook Adeyemi, hired under a combat contract,
//!    flies Occator's leased escort line tank (`+tv/lease escort go`). `+sortie/seat` puts her
//!    at the tank's one console, which carries the helm, the sensors and the fire channel, and
//!    Sim's hand-off leases her all three. The roster launches and stands in a field of three
//!    parties (the roster, another captain's escort, a third captain's intruder). The captain
//!    marks the intruder's party Hostile and orders `2 ENGAGE`: the tank presses the order and
//!    its rounds go at the intruder.
//! 2. **A hire not on a combat duty is refused the console that fires**
//!    ([`co5_a_hire_on_a_mining_duty_is_refused_the_escort_tanks_console`]). Ilya Mendes, under a
//!    mining contract, is planned into the escort tank of a mining roster. `+sortie/seat` refuses
//!    (`NPC_SEAT_NO_FIRE`); she holds no billet and no lease on the tank.
//! 3. **A prospector's seat is the seat it was** ([`co5_a_prospector_seat_leases_the_helm_alone`]).
//!    Mendes in a prospector under the same mining roster is seated at its helm console, which
//!    carries the helm alone, and holds the helm's lease and no other.
//!
//! What the stories stand in for: the escort and the intruder of the first are fixture hulls with
//! players seated, and the roster's rigs are stood on their cells by a staff hand before the field
//! opens (both `field_ap8::opened`'s, which this story shares).

use mush_core::{ChannelId, ObjId, PrincipalId, World};
use tacsim::arena::perception::MindObjective;
use tacsim::detachment::sortie::NPC_SEAT_NO_FIRE;

use crate::driving_rig::{cid, run, stocked_tuned};
use crate::field_ap8::{fired_at, hostiles, opened, standing};
use crate::grounds_rig::AHUNA_FLANK;
use crate::roster_rig::{Billet, Plan, beat, commit, fence, seat, staged};

const ADEYEMI: &str = "Rook Adeyemi";
const MENDES: &str = "Ilya Mendes";

/// The one console's three channels on an armed rig.
const COCKPIT: [&str; 3] = ["fire", "helm", "sensors"];

/// Who holds `channel`'s K6 lease on `unit`.
fn holder(w: &World, unit: ObjId, channel: &str) -> Option<PrincipalId> {
    mush_sim::channel_lease_holder(w, unit, &ChannelId::new(channel).expect("a channel"))
}

/// The channels `unit`'s pilot console carries, and who holds its billet.
fn console(w: &World, unit: ObjId) -> (Vec<String>, Option<ObjId>) {
    let state = w
        .module_state::<mush_sim::data::SpatialState>()
        .expect("spatial state");
    let pilot = mush_sim::StationId::new("pilot").expect("the pilot station");
    let channels = state
        .stations
        .get(&unit)
        .and_then(|stations| stations.stations.get(&pilot))
        .map(|spec| {
            spec.channels
                .iter()
                .map(|c| c.as_str().to_owned())
                .collect()
        })
        .unwrap_or_default();
    let seated = state
        .assignments
        .get(&unit)
        .and_then(|seats| seats.seats.get(&pilot))
        .map(|seat| seat.actor);
    (channels, seated)
}

/// **Story 1** (this module's head).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn co5_a_combat_hire_is_seated_in_the_escort_tank_and_fires_under_its_contract() {
    let mut field = opened(
        &Plan {
            who: ("Pilot", 1, "Salt Moth"),
            duty: "combat",
            billets: vec![Billet::Leader, Billet::HiredArmed(ADEYEMI)],
            commander: None,
        },
        false,
    )
    .await;
    let (captain, adeyemi) = (field.flown.organiser, field.flown.operator(2));
    let tank = field.flown.unit(2);
    let intruder = field.intruder;

    // The tank is the armed rig as leased, its console as authored, and the hand-off leased the
    // hire every channel on it.
    let (channels, seated) = console(field.rig.tw.world(), tank);
    assert_eq!(
        channels, COCKPIT,
        "the escort's one console carries three channels"
    );
    assert_eq!(seated, Some(adeyemi), "+sortie/seat put the hire at it");
    for channel in COCKPIT {
        assert_eq!(
            holder(field.rig.tw.world(), tank, channel),
            Some(PrincipalId::Object { id: adeyemi }),
            "the hire holds the {channel} lease"
        );
    }

    // The captain marks the intruder's party Hostile and orders the tank onto it.
    let letter = tacsim::field::live_on(field.rig.tw.world(), &cid(AHUNA_FLANK.entry))
        .and_then(|live| live.party_of(intruder))
        .expect("the intruder's party")
        .letter();
    let marked = run(
        &mut field.rig.tw,
        captain,
        &format!("+field/mark {letter}=hostile"),
    );
    assert!(
        field.until(30, |rig, _| hostiles(rig, adeyemi) > 0).await,
        "the tank's own frame holds the intruder Hostile: {marked}"
    );
    let fenced = fence(&mut field.rig, captain, &field.flown);
    let _ = commit(&mut field.rig, captain, &format!("ORDER {fenced} 2 ENGAGE"));
    let pressing = field
        .until(90, |rig, lines| {
            matches!(standing(rig, adeyemi), Some(MindObjective::Engage { .. }))
                && fired_at(lines, tank, intruder) > 0
        })
        .await;
    assert!(
        pressing,
        "the hire presses its leader's ENGAGE from the escort tank: {:?}, {} rounds",
        standing(&field.rig, adeyemi),
        fired_at(&field.lines, tank, intruder),
    );
}

/// **Story 2** (this module's head).
#[test]
fn co5_a_hire_on_a_mining_duty_is_refused_the_escort_tanks_console() {
    let mut rig = stocked_tuned(crate::field_host_rig::tune);
    let plan = Plan {
        who: ("Pilot", 1, "Salt Moth"),
        duty: "mining",
        billets: vec![Billet::Leader, Billet::HiredArmed(MENDES)],
        commander: None,
    };
    let staged = staged(&mut rig, &AHUNA_FLANK, &plan);
    let (tank, mendes) = (staged.units[1], staged.hired[1].expect("the hire"));
    let (channels, _) = console(rig.tw.world(), tank);
    assert_eq!(channels, COCKPIT, "the tank is the armed rig");
    let (refused, _) =
        crate::roster_rig::commit_counted(&mut rig, staged.organiser, "+sortie/seat 1:1=2");
    assert!(
        refused.contains(NPC_SEAT_NO_FIRE),
        "a mining hire is refused the console that fires: {refused}"
    );
    let (_, seated) = console(rig.tw.world(), tank);
    assert_eq!(seated, None, "she holds no billet on the tank");
    for channel in COCKPIT {
        assert_eq!(
            holder(rig.tw.world(), tank, channel),
            None,
            "and no {channel} lease"
        );
    }
    assert_ne!(
        rig.tw.world().get(mendes).and_then(|o| o.location),
        None,
        "she is still aboard"
    );
}

/// **Story 3** (this module's head).
#[test]
fn co5_a_prospector_seat_leases_the_helm_alone() {
    let mut rig = stocked_tuned(crate::field_host_rig::tune);
    let plan = Plan {
        who: ("Pilot", 1, "Salt Moth"),
        duty: "mining",
        billets: vec![Billet::Leader, Billet::Hired(MENDES)],
        commander: None,
    };
    let staged = staged(&mut rig, &AHUNA_FLANK, &plan);
    let (rig2, mendes) = (staged.units[1], staged.hired[1].expect("the hire"));
    seat(&mut rig, staged.organiser, 2);
    // The lease mirror is the world's 1 Hz projection of Sim's leases.
    for _ in 0..4 {
        let _ = beat(&mut rig);
    }
    let (channels, seated) = console(rig.tw.world(), rig2);
    assert_eq!(
        channels,
        ["helm"],
        "a prospector's console carries the helm alone"
    );
    assert_eq!(seated, Some(mendes), "the hire is at it");
    assert_eq!(
        holder(rig.tw.world(), rig2, "helm"),
        Some(PrincipalId::Object { id: mendes }),
        "and holds its lease"
    );
    for channel in ["fire", "sensors"] {
        assert_eq!(
            holder(rig.tw.world(), rig2, channel),
            None,
            "and no {channel} lease"
        );
    }
}
