// Meridiani Field Series v1. Provisional authored combat content, not a grant.
// Field and Reserve share this explicit order-bound policy with different cards.

fn cease(view) {
    for weapon in view.weapons {
        if weapon.armed != () { disarm(()); return; }
    }
}

fn fresh(row, age) {
    row.identified && row.age_ms >= 0 && row.age_ms <= age && row.cell != () && row.reads_dead != true
}

fn named(view, id, friendly, age) {
    if id == () { return (); }
    for row in view.contacts {
        if row.id == id && row.friendly == friendly && fresh(row, age) { return row; }
    }
    ()
}

fn closest(view, age, ceiling, facing, half) {
    let found = ();
    for row in view.contacts {
        if !row.friendly && fresh(row, age) && row.range_m <= ceiling {
            if facing == () || arc(row.bearing_deg * 1000, facing) <= half {
                if found == () || row.range_m < found.range_m || (row.range_m == found.range_m && row.id < found.id) {
                    found = row;
                }
            }
        }
    }
    found
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
    if this.blocked && t_ms() < this.retry_at { hold(); return; }
    // A new actual NoRoute after this retry must start a new delay, even when
    // HOLD removed the previous native motor row.
    this.blocked = false;
    goto (goal);
}

fn engage(view, card, target, ceiling) {
    if target == () || target.range_m > ceiling || ceiling <= 0 { return; }
    let count = 0;
    let kept = card.get("keep_rounds");
    for weapon in view.weapons {
        if count >= slots() { break; }
        if weapon.rounds > kept {
            arm(weapon.id, target.id, max(card.get("fire_min_permille"), view.odds_floor_permille), 15, kept, 0, ceiling);
            count += 1;
        }
    }
    if count > 0 { lock(target.id); }
}

// With no current order the Field Series keeps its standing doctrine: take
// the match's objective while nothing hostile is in reach, otherwise stand and
// fight the closest fresh hostile inside its own band and pursuit range on the
// card's odds and kept rounds. It never chases and never breaks off.
fn standing(view, card) {
    let range = ranges(view, card.get("band_permille"), false);
    let ceiling = min(card.get("pursuit_m"), range.release_cm / 100);
    let target = closest(view, card.get("fresh_ms"), ceiling, (), 0);
    let objective = view.objective;
    if target == () && objective != () {
        this.route(view, objective.cell, card.get("retry_ms"));
    } else { hold(); }
    engage(view, card, target, ceiling);
}

fn think(view, card) {
    if view.wreck { return; }
    // Old standing conditions never carry a target or permission into this
    // order's decision. On an unarmed hull this emits no fire-channel verb.
    cease(view);
    if view.power != "online" { startup(); return; }
    let order = view.order;
    if order == () || order.expires_ms <= t_ms() { this.standing(view, card); return; }
    let retry = card.get("retry_ms");
    if order.objective == "withdraw" || health_permille(view) < card.get("withdraw_permille") {
        this.route(view, entry_cell(view), retry);
        return;
    }
    let age = card.get("fresh_ms");
    let range = ranges(view, card.get("band_permille"), true);
    let ceiling = min(card.get("pursuit_m"), range.release_cm / 100);
    let target = ();
    if order.objective == "engage" {
        // A missing, stale, friendly or foreign ordered row is never silently
        // replaced by a different hostile that happened to be visible.
        target = named(view, order.target, false, age);
        if target == () || target.range_m > card.get("pursuit_m") { hold(); return; }
        if view.weapons.len() == 0 { hold(); return; }
        let wanted = range.engage_cm;
        if target.range_m * 100 > wanted {
            this.route(view, stand_off_cell(view, target.id, wanted), retry);
        } else { hold(); }
    } else if order.objective == "advance" {
        this.route(view, order.to, retry);
        target = closest(view, age, ceiling, (), 0);
    } else if order.objective == "support" {
        let charge = named(view, order.of, true, age);
        if charge == () { hold(); return; }
        if charge.range_m > card.get("station_m") {
            this.route(view, stand_off_cell(view, charge.id, card.get("station_m") * 100), retry);
        } else { hold(); }
        if charge.range_m > card.get("support_leash_m") { return; }
        target = closest(view, age, ceiling, (), 0);
    } else if order.objective == "screen" {
        hold();
        helm(order.facing_mdeg);
        target = closest(view, age, ceiling, order.facing_mdeg, card.get("screen_half_deg") * 1000);
    } else if order.objective == "hold" {
        hold();
        return;
    } else { hold(); return; }
    if !order.hold_fire { engage(view, card, target, ceiling); }
}
