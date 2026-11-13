// Collector Drone — Orbital Cluster Mining Sortie Doctrine (SF.R14, D-35-13, D-35-17; doc 44 MW.12)
// name: Collector Drone Doctrine; role: mining; api: 9; stock: 3; rated: class I (1,500 ops a think)
//
// A drone flies its own sortie: out to the rock, lift until its hold is full or the rock is
// done, and home. What it brings back is what it lifted and carried into its bay.
//
// What it reads. The rock is the contact that is not friendly: its range, its velocity
// through `space_contact`, and `reads_dead` once it bears no ore. The ship it left is the
// contact that is friendly. Its collector is its one mount row: `rounds` is the lifts its
// hold still has room for, `rounds_max` the lifts a whole hold takes, `ready` whether it has
// cycled and has room.
//
// What it asks. `lock(id)` on the rock or the ship; `speed(n)` to burn toward its lock;
// `helm(word)` with `speed(n)` to burn against its drift; `hold()` to cut the drive; and
// `fire(collector, 1)` to lift a tonne from the rock it has locked, when it is in reach and
// slow against it.
//
// It burns in pulses, one think on and the next off. A burn that is changed takes effect at
// once and again over the step it stands for, so a pulse of n for one step gives n for a
// second and then n for the step; and a pulse that follows a cut drive is always a changed
// one, so each pulse gives what it was sized for.
fn think(v, c) {
    let s = space(v);
    let rock = ();
    let ship = ();
    for row in v.contacts {
        if row.friendly {
            ship = row;
        } else {
            rock = row;
        }
    }
    let tool = ();
    if v.weapons.len() > 0 {
        tool = v.weapons[0];
    }
    if s == () || tool == () {
        hold();
        trace("DRONE_MINING_IDLE");
        return;
    }
    if !("home" in this) {
        this.home = false;
        this.pulse = false;
        this.range = 0;
        this.goal = 0;
        this.had = 0;
        this.tries = 0;
    }
    // The lifts it goes home with: a whole hold.
    let enough = tool.rounds_max;
    let lifted = tool.rounds_max - tool.rounds;
    if rock == () || rock.reads_dead == true || lifted >= enough {
        this.home = true;
    }
    let goal = if this.home { ship } else { rock };
    if goal == () {
        hold();
        trace("DRONE_MINING_ADRIFT");
        return;
    }
    let word = if this.home { "DRONE_MINING_RETURN" } else { "DRONE_MINING_OUTBOUND" };
    if v.lock != goal.id {
        lock(goal.id);
    }

    // Its drift against what it flies to, and whether the range has closed since it last
    // looked at the same goal.
    let rvx = 0;
    let rvy = 0;
    let rvz = 0;
    let more = space_contact(v, goal.id);
    if more != () {
        rvx = s.vx_cms - more.vx_cms;
        rvy = s.vy_cms - more.vy_cms;
        rvz = s.vz_cms - more.vz_cms;
    }
    let drift = isqrt(rvx * rvx + rvy * rvy + rvz * rvz);
    let closing = this.goal != goal.id || goal.range_m < this.range;
    this.goal = goal.id;
    this.range = goal.range_m;

    if this.pulse {
        // The think after a pulse cuts the drive, so the next burn is a changed one.
        this.pulse = false;
        hold();
        trace(word);
        return;
    }

    // How close to come: inside the collector's reach of a rock, inside the bay's reach of
    // the ship. And the speed it may carry at the distance left and still stop there with
    // half of what its drive gives.
    let accel = max(s.accel_cms2, 1);
    let near_m = if this.home { 40 } else { 100 };
    let left_cm = (goal.range_m - near_m) * 100;
    let carry = 0;
    if left_cm > 0 {
        carry = min(4000, isqrt(left_cm * accel));
    }
    // A pulse of n gives n for a second and n for the step.
    let span_ms = 1000 + max(s.step_ms, 1);

    if left_cm <= 0 && drift <= 300 {
        // There, and slow: work the rock, or wait for the bay to take it.
        hold();
        if this.home {
            trace(word);
        } else {
            if tool.ready {
                // A lift it asked for and did not get: after a few it goes home with what
                // it has.
                if lifted == this.had {
                    this.tries += 1;
                } else {
                    this.had = lifted;
                    this.tries = 0;
                }
                if this.tries > 5 {
                    this.home = true;
                    trace("DRONE_MINING_RETURN");
                    return;
                }
                fire(tool.id, 1);
            }
            trace("DRONE_MINING_COLLECT");
        }
        return;
    }
    if drift > carry + 100 {
        // Too fast for the distance left: a pulse against its drift.
        helm(course_of(-rvx, -rvy, -rvz));
        speed(clamp((drift - carry) * 1000 / span_ms, 1, accel));
        this.pulse = true;
    } else if drift > 200 && !closing {
        // Moving, and not toward it: a pulse that ends the drift.
        helm(course_of(-rvx, -rvy, -rvz));
        speed(clamp(drift * 1000 / span_ms, 1, accel));
        this.pulse = true;
    } else if left_cm > 0 && drift < carry - 100 {
        // Slower than it may be: a pulse toward its lock.
        speed(clamp((carry - drift) * 1000 / span_ms, 1, accel));
        this.pulse = true;
    } else {
        hold();
    }
    trace(word);
}
