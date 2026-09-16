//! The composition root lives in the game crate (16b cut R-a, R2), and the game
//! crate is composed *by* the host, never the other way round: `tacsim` may
//! depend on every library crate it boots, but on neither host crate. Cargo
//! would refuse a cycle at build time; this test states the intent so a future
//! edge is a reviewed decision, not an accident that happens to build.

use std::path::Path;

#[test]
fn the_game_crate_depends_on_no_host_crate() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest).unwrap_or_else(|e| panic!("{manifest:?}: {e}"));
    for host in [
        "mush-server",
        "mushd",
        "mush-gateway",
        "mush-net-ssh",
        "mush-net-web",
    ] {
        let key = format!("{host}.workspace");
        let table = format!("{host} = ");
        assert!(
            !text.contains(&key) && !text.contains(&table),
            "crates/tacsim/Cargo.toml must not depend on the host crate `{host}`"
        );
    }
}
