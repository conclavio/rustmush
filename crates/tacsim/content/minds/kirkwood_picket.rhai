// Kirkwood Customs Picket — Republic Customs Corvette Doctrine (SF.R14, D-35-1, D-35-11, D-35-17; doc 44 MW.9; doc 49 CC.8)
// name: Kirkwood Picket Doctrine; role: combat; api: 10; stock: 3; rated: class I (1,500 ops a think)
//
// What she reads: each contact's relation, and the stop it is under
// (`space_contact(v, id).stop`), which the picket's host keeps. Nothing writes her memory;
// she keeps in it that she has hailed, and the hostile she fights and its velocity when she
// last looked.
//
// Against a hostile she fires what can hurt it (doc 49 unit CC.8): a salvo of two torpedoes
// at each reload; a spinal round only at a hull that has held its course since she last
// looked, since a hull that burns is not where the round was aimed; and her laser only
// within its reach. Her second laser stands in defence, as her fit starts it.
fn think(v, c) {
    let hostile = ();
    let hostile_range = 0;
    let hostile_ran = false;
    let stopped = ();
    let stopped_range = 0;
    let stopped_hove_to = false;
    for row in v.contacts {
        if row.reads_dead == true {
            continue;
        }
        let stop = "none";
        let more = space_contact(v, row.id);
        if more != () {
            stop = more.stop;
        }
        if row.relation == "hostile" {
            if hostile == () || row.range_m < hostile_range {
                hostile = row.id;
                hostile_range = row.range_m;
                hostile_ran = stop == "ran";
            }
        } else if stop == "hailed" || stop == "hove_to" {
            if stopped == () || row.range_m < stopped_range {
                stopped = row.id;
                stopped_range = row.range_m;
                stopped_hove_to = stop == "hove_to";
            }
        }
    }

    if hostile != () {
        emcon(true);
        lock(hostile);
        let steady = false;
        let more = space_contact(v, hostile);
        if more != () {
            if "foe" in this && this.foe == hostile {
                steady = more.vx_cms == this.vx && more.vy_cms == this.vy && more.vz_cms == this.vz;
            }
            this.foe = hostile;
            this.vx = more.vx_cms;
            this.vy = more.vy_cms;
            this.vz = more.vz_cms;
        }
        for w in v.weapons {
            let m = space_mount(v, w.id);
            if m == () || m.state != "ready" {
                continue;
            }
            if m.family == "tubes" {
                fire(w.id, 1);
                fire(w.id, 1);
            } else if m.family == "spinal" {
                if steady && m.lock_flight_ms != () {
                    fire(w.id, 1);
                }
            } else if m.mode == "offence" && m.lock_damage != () {
                fire(w.id, 1);
            }
        }
        if hostile_ran {
            trace("PICKET_RAN_THE_PICKET");
        } else {
            trace("PICKET_SCREEN_ENGAGE_RAIDER");
        }
        return;
    }

    if stopped != () {
        lock(stopped);
        hold();
        if stopped_hove_to {
            trace("PICKET_CARRIER_HOVE_TO_READY");
        } else if "hailed" in this {
            trace("PICKET_AWAIT_HEAVE_TO");
        } else {
            this.hailed = true;
            trace("PICKET_HAIL_HEAVE_TO");
        }
    }
}
