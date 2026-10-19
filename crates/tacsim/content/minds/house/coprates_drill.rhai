// Coprates Drill v1. Provisional authored content; no Republic entitlement.
// Marching and firing are distinct drills, commanded only by the public order.

fn clear_fire(view) {
    for weapon in view.weapons {
        if weapon.armed != () { disarm(()); return; }
    }
}

fn fresh(row, age) {
    row.identified && row.age_ms >= 0 && row.age_ms <= age && row.cell != () && row.reads_dead != true
}

fn ordered_row(view, id, friendly, age) {
    if id == () { return (); }
    for row in view.contacts {
        if row.id == id && row.friendly == friendly && fresh(row, age) { return row; }
    }
    ()
}

fn screen_row(view, age, ceiling, facing, half) {
    let selected = ();
    for row in view.contacts {
        if !row.friendly && fresh(row, age) && row.range_m <= ceiling && arc(row.bearing_deg * 1000, facing) <= half {
            if selected == () || row.range_m < selected.range_m || (row.range_m == selected.range_m && row.id < selected.id) {
                selected = row;
            }
        }
    }
    selected
}

fn march(view, goal, retry) {
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
    if this.blocked && t_ms() < this.retry_at { hold(); return; }
    this.blocked = false;
    goto (goal);
}

fn fire_drill(view, card, target, ceiling) {
    // A requested hold or lock is not evidence that it already happened.
    if target == () || target.range_m > ceiling || ceiling <= 0 || view.weapons.len() == 0 { return; }
    if view.lock != target.id { lock(target.id); return; }
    if view.speed_cms != 0 { return; }
    let count = 0;
    let kept = card.get("keep_rounds");
    let floor = max(card.get("fire_min_permille"), view.odds_floor_permille);
    for weapon in view.weapons {
        if count >= slots() { break; }
        if weapon.rounds > kept {
            arm(weapon.id, target.id, floor, 15, kept, 0, ceiling);
            count += 1;
        }
    }
}

fn think(view, card) {
    if view.wreck { return; }
    clear_fire(view);
    let order = view.order;
    if order == () || order.expires_ms <= t_ms() { hold(); return; }
    if view.power != "online" { startup(); return; }
    let retry = card.get("retry_ms");
    if order.objective == "withdraw" || health_permille(view) < card.get("withdraw_permille") {
        this.march(view, entry_cell(view), retry);
        return;
    }
    let age = card.get("fresh_ms");
    if order.objective == "advance" {
        this.march(view, order.to, retry);
        return;
    }
    if order.objective == "support" {
        let charge = ordered_row(view, order.of, true, age);
        if charge == () { hold(); return; }
        if charge.range_m > card.get("station_m") {
            this.march(view, stand_off_cell(view, charge.id, card.get("station_m") * 100), retry);
        } else { hold(); }
        return;
    }
    if order.objective == "hold" { hold(); return; }
    let ranges_now = ranges(view, card.get("band_permille"), true);
    let ceiling = min(card.get("max_target_m"), ranges_now.release_cm / 100);
    let target = ();
    if order.objective == "engage" {
        target = ordered_row(view, order.target, false, age);
        if target == () || target.range_m > card.get("max_target_m") || view.weapons.len() == 0 { hold(); return; }
        if target.range_m * 100 > ranges_now.engage_cm {
            this.march(view, stand_off_cell(view, target.id, ranges_now.engage_cm), retry);
            return;
        }
        hold();
    } else if order.objective == "screen" {
        hold();
        helm(order.facing_mdeg);
        target = screen_row(view, age, ceiling, order.facing_mdeg, card.get("screen_half_deg") * 1000);
    } else { hold(); return; }
    if !order.hold_fire { fire_drill(view, card, target, ceiling); }
}
