// Collector Drone — Orbital Cluster Mining Sortie Doctrine (SF.R14, D-35-13, D-35-17)
// name: Collector Drone Doctrine; role: mining; api: 9; stock: 3; rated: class I (1,500 ops a think)
fn think(v, c) {
    if !("stage" in this) {
        this.stage = 0;
    }
    if this.stage == 0 {
        this.stage = 1;
        speed(500);
        trace("DRONE_MINING_OUTBOUND");
        return;
    }
    if this.stage == 1 {
        this.stage = 2;
        hold();
        trace("DRONE_MINING_COLLECT");
        return;
    }
    hold();
    trace("DRONE_MINING_RETURN");
}
