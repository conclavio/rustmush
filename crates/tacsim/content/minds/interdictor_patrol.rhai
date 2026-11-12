// Interdictor Patrol — Waypoint Interdictor Picket Doctrine (SF.R14, D-35-1, D-35-17; doc 44 MW.9, MW.11)
// name: Interdictor Patrol Doctrine; role: combat; api: 9; stock: 3; rated: class I (1,500 ops a think)
//
// She takes the nearest hull that is still hostile to her. The first think she holds it, she
// lights up and hails, unless it is already radiating for a fight. What she asks lands a step
// after she thinks it, and so does the hull's answer: on her next think the hull has only
// just been able to hear her, so she waits that one out. After that she reads its
// velocity: a hull whose change of velocity since her last think is against its drift past
// her is slowing to heave to, and she holds her fire and her drive for it. Any other hull she
// closes on and fires at. A hull that comes under the heave-to speed is stood down by the
// bubble's host, hull by hull, and reads neutral from then on; with nobody left hostile she
// cuts her drive and holds station, showing herself at STANDARD.
fn think(v, c) {
    let target = ();
    let target_range = 0;
    for row in v.contacts {
        if row.relation == "hostile" && row.reads_dead != true {
            if target == () || row.range_m < target_range {
                target = row.id;
                target_range = row.range_m;
            }
        }
    }
    if target == () {
        // On station with nobody hostile in sight she shows herself, at her quietest posture
        // that is not dark: a hull she cannot see can still see her.
        emcon(false);
        hold();
        this.target = 0;
        this.closing = false;
        this.heard = true;
        trace("INTERDICTOR_HOLD_STATION");
        return;
    }

    let s = space(v);
    let more = space_contact(v, target);
    let known = "target" in this && this.target == target;
    let slowing = false;
    let combat = false;
    if s != () && more != () {
        combat = more.emcon == "combat";
        if known {
            // Her change of velocity, against her drift past this hull.
            let turn = (more.vx_cms - this.vx) * (more.vx_cms - s.vx_cms)
                + (more.vy_cms - this.vy) * (more.vy_cms - s.vy_cms)
                + (more.vz_cms - this.vz) * (more.vz_cms - s.vz_cms);
            slowing = turn < 0;
        }
        this.vx = more.vx_cms;
        this.vy = more.vy_cms;
        this.vz = more.vz_cms;
    }
    this.target = target;

    emcon(true);
    lock(target);
    if !known && !combat {
        hold();
        this.closing = false;
        this.heard = false;
        trace("INTERDICTOR_HAIL_HEAVE_TO");
        return;
    }
    if !known {
        // A hull she takes without hailing it, because it is radiating for a fight, owes
        // her no answer: a wait left over from another hull's hail is dropped.
        this.heard = true;
    }
    if !combat && "heard" in this && this.heard == false {
        // The think after her hail: the hull heard it on this snapshot and no sooner, and
        // what it does about it shows on the next.
        hold();
        this.closing = false;
        this.heard = true;
        trace("INTERDICTOR_AWAIT_ANSWER");
        return;
    }
    if slowing {
        hold();
        this.closing = false;
        trace("INTERDICTOR_AWAIT_HEAVE_TO");
        return;
    }

    if !("closing" in this) || this.closing != true {
        this.closing = true;
        speed(1000);
    }
    for w in v.weapons {
        if w.ready_in_ms != 0 {
            continue;
        }
        if w.id == "laser_1" {
            fire(w.id, 1);
        } else if w.id == "torp_bow" && combat && w.rounds == w.rounds_max {
            fire(w.id, 1);
        }
    }
    trace("INTERDICTOR_ENGAGE_HOSTILE");
}
