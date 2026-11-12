// Link-Loss Return — the house backup mind every hull carries (doc 35 section 11, Q-35d-2; doc 44 MW.9)
// name: Link-Loss Return Doctrine; role: backup; api: 8; stock: 3; rated: class I (1,500 ops a think)
//
// What the backup seat reads: inside a bubble, `space(v)`: her velocity, what her drive
// gives, the stop a picket or an interdictor has on her and the contact that holds it;
// outside one, `v.motor.outcome == "no_route"` while no port is within her tank and stores.
// What it may ask: `helm(word)` with `speed(n)` (a burn; outside a bubble `speed(n)` above
// nought makes for port by the ruled order), `hold()` (cut the drive; hove to under a customs
// hail, submit to inspection; outside a bubble, hold where she is), `emcon(on)` and
// `trace(word)`. `lock`, `fire` and `arm` are refused by the seat.
fn think(v, c) {
    let s = space(v);
    if s != () && s.stop == "hailed" {
        // She is told to heave to: end her drift against the ship that told her, by a burn
        // along the way that ship is moving against her, sized to end it in the step ahead.
        let by = ();
        if s.stop_by != () {
            by = space_contact(v, s.stop_by);
        }
        if by != () {
            let dx = by.vx_cms - s.vx_cms;
            let dy = by.vy_cms - s.vy_cms;
            let dz = by.vz_cms - s.vz_cms;
            let drift = isqrt(dx * dx + dy * dy + dz * dz);
            if drift > 0 {
                helm(course_of(dx, dy, dz));
                speed(clamp(drift * 1000 / max(s.step_ms, 1), 1, max(s.accel_cms2, 1)));
                trace("BACKUP_HEAVE_TO");
                return;
            }
        }
        hold();
        trace("BACKUP_HEAVE_TO");
        return;
    }
    if v.held {
        // Hove to under a customs hail: cut the drive and stand for inspection.
        hold();
        trace("BACKUP_HEAVE_TO_FOR_CUSTOMS");
        return;
    }
    if s != () {
        // Inside a bubble with nobody stopping her: cut whatever burn stands, and coast.
        hold();
        trace("BACKUP_COAST");
        return;
    }
    let motor = v.motor;
    if motor != () && motor.outcome == "no_route" {
        hold();
        trace("BACKUP_HOLD_FOR_RETRIEVAL");
        return;
    }
    speed(1000);
    trace("BACKUP_MAKE_FOR_PORT");
}
