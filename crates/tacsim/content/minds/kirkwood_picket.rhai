// Kirkwood Customs Picket — Republic Customs Corvette Doctrine (SF.R14, D-35-1, D-35-11, D-35-17; doc 44 MW.9)
// name: Kirkwood Picket Doctrine; role: combat; api: 8; stock: 3; rated: class I (1,500 ops a think)
//
// What she reads: each contact's relation, and the stop it is under
// (`space_contact(v, id).stop`), which the picket's host keeps. Nothing writes her memory;
// she keeps one word of her own in it, that she has hailed.
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
        for w in v.weapons {
            if w.ready_in_ms == 0 && w.id == "laser_1" {
                fire(w.id, 1);
                break;
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
