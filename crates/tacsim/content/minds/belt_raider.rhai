// Belt Raider — Ambush & Torpedo/Railgun Strike Doctrine (SF.R14, D-35-1, D-35-11, D-35-17; doc 44 MW.9; doc 49 CC.8)
// name: Belt Raider Doctrine; role: combat; api: 10; stock: 3; rated: class I (1,500 ops a think)
//
// She lurks dark until the picket's radio says customs have finished with a carrier
// (`space(v).heard`), then lights her torch. Her prey is a hostile customs have finished with,
// one radiating for a fight before one that is not (doc 49 unit CC.8), and the nearest of
// those. She opens with everything that is ready and two torpedoes, and after that presses
// only a prey that is radiating for a fight.
//
// How she presses (doc 49 unit CC.8): her torch dies after the opener and she presses from
// where she is: a salvo of two torpedoes at each reload, a spinal round at a prey that has
// held its course since her last think (a hull that burns is not where the round was aimed),
// and her laser within its reach. Her second laser stands in defence, as her fit starts it.
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
    let prey_vel = ();
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
        let better = prey == () ||
            (settled && !prey_settled) ||
            (settled == prey_settled && combat && !prey_combat) ||
            (settled == prey_settled && combat == prey_combat && row.range_m < prey_range);
        if better {
            prey = row.id;
            prey_range = row.range_m;
            prey_settled = settled;
            prey_combat = combat;
            prey_vel = more;
        }
    }
    if prey == () {
        return;
    }

    lock(prey);
    let steady = false;
    if prey_vel != () {
        if "prey" in this && this.prey == prey {
            steady = prey_vel.vx_cms == this.vx && prey_vel.vy_cms == this.vy && prey_vel.vz_cms == this.vz;
        }
        this.prey = prey;
        this.vx = prey_vel.vx_cms;
        this.vy = prey_vel.vy_cms;
        this.vz = prey_vel.vz_cms;
    }
    if this.stage == 1 {
        this.stage = 2;
        speed(2500);
        // Every mount that is ready and fires at a hull: a laser in offence, a pair from each
        // tube family by its family, never by a name (doc 48e kernel G9).
        for w in v.weapons {
            let m = space_mount(v, w.id);
            if m == () || m.state != "ready" {
                continue;
            }
            if m.family == "laser" && m.mode != "offence" {
                continue;
            }
            fire(w.id, 1);
            if m.family == "tubes" {
                fire(w.id, 1);
            }
        }
        trace("RAIDER_TORCH_AND_SALVO");
        return;
    }

    hold();
    if prey_settled && prey_combat {
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
    }
    trace("RAIDER_PRESS_ATTACK");
}
