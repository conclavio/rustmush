// Safe Conduct v1 — authored ground-role content, not an issued certificate.
// Only this hull's published view, current commander order and station API.
// Escort never chases away from its earned charge plot. Transit never fires.

fn cease(view) {
    for weapon in view.weapons {
        if weapon.armed != () { disarm(()); return; }
    }
}

fn fresh(row, age) {
    row != () && row.identified && row.age_ms >= 0 && row.age_ms <= age && row.cell != () && row.reads_dead != true
}

fn charge(view, of, age) {
    if of == () { return (); }
    for row in view.contacts {
        if row.id == of && row.friendly && fresh(row, age) { return row; }
    }
    ()
}

fn threat(view, age, maximum) {
    let picked = ();
    for row in view.contacts {
        if !row.friendly && fresh(row, age) && row.range_m <= maximum {
            if picked == () || row.range_m < picked.range_m { picked = row; }
        }
    }
    picked
}

fn route(view, goal, retry) {
    if goal == () || view.immobile { hold(); return; }
    if !("goal" in this) || this.goal != goal {
        this.goal = goal;
        this.blocked = false;
        this.retry_at = 0;
    }
    let motor = view.motor;
    if motor != () && motor.goal == goal {
        if motor.outcome == "arrived" { hold(); return; }
        if motor.outcome == "no_route" {
            if !this.blocked {
                this.blocked = true;
                this.retry_at = min(t_ms(), 9223372036854770807) + retry;
            }
        } else { this.blocked = false; }
    }
    // HOLD may remove the motor row; the retry clock remains our own memory.
    if this.blocked && t_ms() < this.retry_at { hold(); return; }
    // A retry is a new attempt; a later NoRoute starts its own delay.
    this.blocked = false;
    goto (goal);
}

fn think(view, card) {
    if view.wreck { return; }
    if view.power != "online" { cease(view); startup(); return; }
    let order = view.order;
    if order == () || order.expires_ms <= t_ms() {
        cease(view); hold(); return;
    }
    let order_key = if order.identity == () { order.expires_ms } else { order.identity };
    if !("order_until" in this) || this.order_until != order_key {
        this.order_until = order_key;
        this.goal = ();
        this.blocked = false;
        this.retry_at = 0;
    }
    let retry = card.get("retry_ms");
    if order.objective == "withdraw" || health_permille(view) < card.get("withdraw_permille") {
        cease(view);
        this.route(view, entry_interior_cell(view), retry);
        return;
    }
    let age = card.get("fresh_ms");
    let danger = threat(view, age, card.get("threat_m"));
    if card.get("role") == "transit" {
        cease(view);
        if order.objective != "advance" || danger != () { hold(); return; }
        this.route(view, order.to, retry);
        return;
    }
    if order.objective != "support" { cease(view); hold(); return; }
    let held = charge(view, order.of, age);
    if held == () { cease(view); hold(); return; }
    let station = card.get("station_m");
    if held.range_m > card.get("leash_m") {
        cease(view);
        this.route(view, stand_off_cell(view, held.id, station * 100), retry);
        return;
    }
    if held.range_m > station {
        this.route(view, stand_off_cell(view, held.id, station * 100), retry);
    } else { hold(); }
    if order.hold_fire || danger == () { cease(view); return; }
    // One selected visible hostile; no target pursuit, hidden charge target or
    // native full-think helper. The native slots decide each actual legal shot.
    let count = 0;
    let floor = card.get("fire_min_permille");
    let ceiling = min(card.get("threat_m"), ranges(view, 1000, true).release_cm / 100);
    if ceiling <= 0 || danger.range_m > ceiling { cease(view); return; }
    for weapon in view.weapons {
        if count >= slots() { break; }
        if weapon.rounds > 0 {
            arm(weapon.id, danger.id, floor, 15, 0, 0, ceiling);
            count += 1;
        }
    }
    if count > 0 { lock(danger.id); }
}
