//! E7 cut 11 **independent verify fold** — §14's L3 writer census
//! (10b §14.2).

// ---- E7 cut 11 independent verify fold ----

/// **G4. §14.2's L3 writer set is materially incomplete, and this pins the real
/// one.**
///
/// L3 says *"Its writers today are `module.rs`'s boot hydration and whatever the
/// damage model eventually writes — there is **no production cell writer at all**
/// yet … which is exactly why this lane is cheap **now**"*. The narrow claim
/// about *cells* survives; the writer enumeration does not. `UnitStore` has four
/// synchronous production writers beyond boot, and one of them —
/// `on_obj_destroyed` — runs in a `WorldEvent` hook with `&mut Ctx` and **no
/// `ActionTxn`**, which is the same obstacle §15.5 calls fatal for the market
/// lift. §4.1's own cut-8 row already says the store is "written synchronously
/// by three legacy commands", so §14 contradicted §4.1 as well.
///
/// A new writer breaks this pin and forces §14.2's L3 paragraph to be edited in
/// the same commit — which is what §14.3 criterion 2 ("the lane moves whole")
/// needs in order to be checkable at all.
#[test]
fn vf_g4_the_unit_store_writer_census_is_pinned() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/module.rs");
    let text = std::fs::read_to_string(path).expect("mush-tactics/src/module.rs");
    let boundary = text.find("#[cfg(test)]").unwrap_or(text.len());
    let lines: Vec<&str> = text[..boundary].lines().collect();

    let mut sites: Vec<String> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let code = line.trim_start();
        if code.starts_with("//") || !code.contains("module_state_mut::<UnitStore>") {
            continue;
        }
        let mut enclosing = String::from("<none>");
        for candidate in lines[..=i].iter().rev() {
            let trimmed = candidate.trim_start();
            let rest = trimmed
                .strip_prefix("pub(crate) ")
                .or_else(|| trimmed.strip_prefix("pub "))
                .unwrap_or(trimmed);
            if let Some(tail) = rest.strip_prefix("fn ") {
                enclosing = tail
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                break;
            }
        }
        sites.push(enclosing);
    }
    assert_eq!(
        sites.len(),
        7,
        "seven `module_state_mut::<UnitStore>` sites"
    );
    sites.sort();
    sites.dedup();
    assert_eq!(
        sites,
        vec![
            // 16b cut F.1: `@unit/damage`'s body became the public `apply_plan`,
            // for the same reason cut C.2 hoisted `create_unit` — a caller with
            // no command line (the game crate's stock-tank reset, doc 16 §3.3)
            // goes through the *same* writer rather than a second one. The site
            // moved out of `unit_damage` and the census did not grow: still four
            // production writers beyond boot, and still no second lane.
            "apply_plan".to_owned(),
            "boot_units".to_owned(),
            // 16b cut C.2: `@unit/create`'s body became the public
            // `create_unit`, so a caller with no command line — the arena
            // pack's applier, making doc 16 §3.3's eight stock tanks at a
            // first boot — goes through the *same* writer rather than a second
            // one. The site moved out of `unit_create` and the census did not
            // grow: still four production writers beyond boot.
            "create_unit".to_owned(),
            // The `WorldEvent::ObjDestroyed` hook: `hold_active` / `remove_active`
            // / `update_active`, with `&mut Ctx` and NO `ActionTxn`.
            "on_obj_destroyed".to_owned(),
            // §4.1's "three legacy commands" (`unit_damage`'s site is
            // `apply_plan` above since 16b cut F.1).
            "unit_remove".to_owned(),
        ],
        "L3's synchronous writers — four production writers beyond boot, one of \
         them an event hook with no txn"
    );
    // The narrow claim §14 makes that DOES survive: no production CELL writer.
    // The store's binding-repair writer is declared once and called twice, both
    // inside boot, so nothing outside `boot_units` touches a cell.
    assert_eq!(
        text[..boundary].matches("push_binding_repair(").count(),
        3,
        "one declaration plus two boot-only call sites"
    );
}
