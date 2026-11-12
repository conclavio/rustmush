// Belt Raider — Ambush & Torpedo/Railgun Strike Doctrine (SF.R14, D-35-1, D-35-11, D-35-17; doc 44 MW.9)
// name: Belt Raider Doctrine; role: combat; api: 8; stock: 3; rated: class I (1,500 ops a think)
//
// She lurks dark until the picket's radio says customs have finished with a carrier
// (`space(v).heard`), then lights her torch. Her prey is the nearest hostile customs have
// finished with. She opens with everything that is ready and two torpedoes, and after that
// presses only a prey that is radiating for a fight.
fn think(v, c) {
    if !("stage" in this) {
        this.stage = 0;
    }
    if this.stage == 0 {
        let s = space(v);
        if s == () || (s.heard != "cleared" && s.heard != "ran") {
            return;
        }
        this.stage = 1;
        emcon(true);
        trace("RAIDER_LIGHT_TORCH");
        return;
    }

    let prey = ();
    let prey_range = 0;
    let prey_settled = false;
    let prey_combat = false;
    for row in v.contacts {
        if row.relation != "hostile" || row.reads_dead == true {
            continue;
        }
        let settled = false;
        let combat = false;
        let more = space_contact(v, row.id);
        if more != () {
            settled = more.stop == "cleared" || more.stop == "ran";
            combat = more.emcon == "combat";
        }
        if prey == () || (settled && !prey_settled) || (settled == prey_settled && row.range_m < prey_range) {
            prey = row.id;
            prey_range = row.range_m;
            prey_settled = settled;
            prey_combat = combat;
        }
    }
    if prey == () {
        return;
    }

    lock(prey);
    if this.stage == 1 {
        this.stage = 2;
        speed(2500);
        for w in v.weapons {
            if w.ready_in_ms == 0 {
                fire(w.id, 1);
                if w.id == "torp_bow" {
                    fire(w.id, 1);
                }
            }
        }
        trace("RAIDER_TORCH_AND_SALVO");
        return;
    }

    if prey_settled && prey_combat {
        for w in v.weapons {
            if w.id != "torp_bow" {
                fire(w.id, 1);
            }
        }
    }
    trace("RAIDER_PRESS_ATTACK");
}
