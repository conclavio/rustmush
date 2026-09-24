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

/// **The recorded edge set** (16b cut U.3; 16c `T-42`, its last item).
///
/// The test above pins the set only in the negative: five host crates the game
/// crate may never name. What it *does* name is open on purpose: 16c `T-42`'s
/// fix column asks for "the edge set recorded as deliberately open in the R-a
/// ledger row" (16b §9; R-a itself only dropped the dead `mush-tactics` to
/// `mush-dice` edge), since the composition root boots every library crate it
/// composes and that list grows with the game. Open is not unrecorded, though:
/// an edge nobody reviewed would build and pass the negative test above. So
/// this pins the set as it stands, and a new `mush-*` dependency is a diff to
/// this list somebody reads.
///
/// Every `mush-*` key under `[dependencies]` in either form cargo accepts — the
/// `name.workspace = true` / `name = { .. }` line form, and a
/// `[dependencies.name]` table — sorted. Dev-dependencies are not edges of the
/// game crate and are not read.
#[test]
fn the_game_crates_workspace_edges_are_the_recorded_set() {
    const RECORDED: [&str; 31] = [
        "mush-atlas",
        "mush-campaign",
        "mush-canon",
        "mush-comsys",
        "mush-config",
        "mush-content",
        "mush-core",
        "mush-dice",
        "mush-economy",
        "mush-faction",
        "mush-housing",
        "mush-industry",
        "mush-inventory",
        "mush-jobs",
        "mush-llm",
        "mush-mail",
        "mush-market",
        "mush-npc",
        "mush-org",
        "mush-party",
        "mush-privileges",
        "mush-propagation",
        "mush-proto",
        "mush-ruleset",
        "mush-scenes",
        "mush-script",
        "mush-sim",
        "mush-store",
        "mush-tactics",
        "mush-weather",
        "mush-worldgen",
    ];
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest).unwrap_or_else(|e| panic!("{manifest:?}: {e}"));
    let mut edges: Vec<String> = Vec::new();
    let mut in_dependencies = false;
    for line in text.lines() {
        let line = line.trim();
        if let Some(header) = line.strip_prefix('[') {
            let header = header.trim_end_matches(']').trim();
            in_dependencies = header == "dependencies";
            // The table form: `[dependencies.mush-foo]` is one edge.
            if let Some(name) = header.strip_prefix("dependencies.")
                && name.starts_with("mush-")
            {
                edges.push(name.to_owned());
            }
            continue;
        }
        if !in_dependencies || line.starts_with('#') {
            continue;
        }
        // The line form: the key is everything before `.workspace` or `=`.
        let key = line
            .split(['=', '.'])
            .next()
            .map(str::trim)
            .unwrap_or_default();
        if key.starts_with("mush-") {
            edges.push(key.to_owned());
        }
    }
    edges.sort();
    assert_eq!(
        edges, RECORDED,
        "crates/tacsim/Cargo.toml's `mush-*` dependencies are not the recorded set: an \
         edge was added or removed. If it is meant, change the list in this test in the \
         same diff, so the edge is reviewed"
    );
}
