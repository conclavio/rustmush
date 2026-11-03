// Kirkwood Customs Picket — Republic Customs Corvette Doctrine (SF.R14, D-35-1, D-35-11, D-35-17)
// name: Kirkwood Picket Doctrine; role: combat; api: 7; stock: 3; rated: class I (1,500 ops a think)
fn think(v, c) {
    if !("stage" in this) {
        this.stage = 0;
    }
    let carrier_id = ();
    let carrier_range = 0;
    let carrier_moving = true;
    let hostile_id = ();
    let hostile_range = 0;
    for row in v.contacts {
        if row.relation == "hostile" && row.reads_dead != true {
            if hostile_id == () || row.range_m < hostile_range {
                hostile_id = row.id;
                hostile_range = row.range_m;
            }
        } else if row.relation == "neutral" && row.reads_dead != true {
            if carrier_id == () || row.range_m < carrier_range {
                carrier_id = row.id;
                carrier_range = row.range_m;
                carrier_moving = row.moving;
            }
        }
    }

    if hostile_id != () {
        this.stage = 3;
        emcon(true);
        lock(hostile_id);
        for w in v.weapons {
            if w.ready_in_ms == 0 && w.id == "laser_1" {
                fire(w.id, 1);
                break;
            }
        }
        trace("PICKET_SCREEN_ENGAGE_RAIDER");
        return;
    }

    if carrier_id != () {
        if carrier_range > 35000 && this.stage != 5 {
            this.stage = 4;
            emcon(true);
            lock(carrier_id);
            for w in v.weapons {
                if w.ready_in_ms == 0 {
                    fire(w.id, 1);
                    break;
                }
            }
            trace("PICKET_RAN_THE_PICKET");
            return;
        }
        if this.stage == 0 {
            lock(carrier_id);
            hold();
            if !carrier_moving {
                this.stage = 2;
                trace("PICKET_CARRIER_HOVE_TO_READY");
            } else {
                this.stage = 1;
                trace("PICKET_HAIL_HEAVE_TO");
            }
            return;
        }
        if !carrier_moving {
            hold();
            this.stage = 2;
            trace("PICKET_CARRIER_HOVE_TO_READY");
            return;
        }
        lock(carrier_id);
        hold();
        trace("PICKET_AWAIT_HEAVE_TO");
    }
}
