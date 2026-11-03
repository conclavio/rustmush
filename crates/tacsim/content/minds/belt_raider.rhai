// Belt Raider — Ambush & Torpedo/Railgun Strike Doctrine (SF.R14, D-35-1, D-35-11, D-35-17)
// name: Belt Raider Doctrine; role: combat; api: 7; stock: 3; rated: class I (1,500 ops a think)
fn think(v, c) {
    if !("stage" in this) {
        this.stage = 0;
    }
    let target_id = ();
    let target_range = 0;
    for row in v.contacts {
        if row.relation == "hostile" && row.reads_dead != true {
            target_id = row.id;
            target_range = row.range_m;
            break;
        }
    }
    if target_id == () {
        return;
    }
    if this.stage == 0 {
        this.stage = 1;
        emcon(true);
        lock(target_id);
        speed(2500);
        for w in v.weapons {
            if w.ready_in_ms == 0 {
                fire(w.id, 1);
            }
        }
        trace("RAIDER_TORCH_AND_SALVO");
        return;
    }
    this.stage = 2;
    lock(target_id);
    trace("RAIDER_PRESS_ATTACK");
}
