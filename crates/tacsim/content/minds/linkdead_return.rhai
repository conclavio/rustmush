// Link-Loss Return — the house backup mind every hull carries (doc 35 section 11, Q-35d-2)
// name: Link-Loss Return Doctrine; role: backup; api: 7; stock: 3; rated: class I (1,500 ops a think)
//
// What the backup seat reads: `v.held` while a customs picket holds her uncleared, and
// `v.motor.outcome == "no_route"` while no port is within her tank and stores.
// What it may ask: `hold()` (heave to, and submit to a customs hail; outside a bubble, hold
// where she is), `speed(n)` with n above nought (make for port by the ruled order),
// `emcon(on)` and `trace(word)`. `lock`, `fire` and `arm` are refused by the seat.
fn think(v, c) {
    if v.held {
        hold();
        trace("BACKUP_HEAVE_TO_FOR_CUSTOMS");
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
