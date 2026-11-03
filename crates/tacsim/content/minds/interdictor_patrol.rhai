// Interdictor Patrol — Waypoint Interdictor Picket Doctrine (SF.R14, D-35-1, D-35-17)
// name: Interdictor Patrol Doctrine; role: combat; api: 7; stock: 3; rated: class I (1,500 ops a think)
fn think(v, c) {
    if !("stage" in this) {
        this.stage = 0;
    }
    let target_id = ();
    let target_moving = true;
    for row in v.contacts {
        if row.relation == "hostile" && row.reads_dead != true {
            target_id = row.id;
            target_moving = row.moving;
            break;
        }
    }
    if target_id == () {
        hold();
        return;
    }
    if !target_moving {
        hold();
        this.stage = 2;
        trace("INTERDICTOR_STAND_DOWN_CLEARED");
        return;
    }
    this.stage = 1;
    emcon(true);
    lock(target_id);
    speed(1000);
    for w in v.weapons {
        if w.ready_in_ms == 0 {
            fire(w.id, 1);
        }
    }
    trace("INTERDICTOR_ENGAGE_HOSTILE");
}
