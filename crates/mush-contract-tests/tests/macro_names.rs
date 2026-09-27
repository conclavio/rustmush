//! **16b cut AD.1 (16c N14): what a player's macro may be named, asserted
//! against the production composition.**
//!
//! A macro is expanded by the kernel's alias stage before `home`, exits and
//! the command table, so a macro named after a word some other surface trusts
//! would change what that word does for its owner. Two such surfaces trust a
//! typed head without ever seeing the macro: the TUI's and the web's bare
//! keys (`web/src/lib/run-allowlist.json`, the one list both clients read) and
//! the store-read-only gate (`mush_server::world_task::STORE_READ_ONLY_ALLOWED`,
//! which lets a typed `look` through while the database is down). The first
//! case proves `@macro` refuses every word of both on the real registry.
//!
//! The second case is the observer rule on a refusal (doc 16 §5). `@macro`
//! checks a name's CHARACTERS before it consults anything, and `@` and `+` are
//! not in its set, so every `@`/`+` word is `BAD_NAME` whether a verb of that
//! name exists or not. That is only a guarantee while every gated verb — a
//! staff role, a power, a privilege or a lock — is `@`/`+`-prefixed. A bare
//! gated verb would be refused as `TAKEN`, which is still observer-safe (typing
//! it already answers `Permission denied.` rather than `Huh?`), but it would be
//! the first word through which `@macro` could tell a mortal a staff verb
//! exists. This gate makes a person rule on it, the way
//! `exit_command_words.rs` does for an exit that takes a verb.

use mush_core::commands::macros::{BAD_NAME, name_refusal};
use mush_core::{GOD, Role, World};

mod production_registry;

use production_registry::production_composition;

#[test]
fn no_bare_key_or_read_only_word_can_be_a_macro_name() {
    let composition = production_composition();
    let reg = composition.registry();
    let w = World::new_minimal();

    let run_allowlist: Vec<String> =
        serde_json::from_str(include_str!("../../../web/src/lib/run-allowlist.json"))
            .expect("web/src/lib/run-allowlist.json is a JSON array of verbs");
    assert!(!run_allowlist.is_empty());
    let read_only = mush_server::world_task::STORE_READ_ONLY_ALLOWED;
    assert!(!read_only.is_empty());

    for word in run_allowlist
        .iter()
        .map(String::as_str)
        .chain(read_only.iter().copied())
    {
        assert!(
            name_refusal(reg, &w, GOD, word).is_some(),
            "{word:?} is trusted as a typed head by a bare key or the read-only gate, \
             and @macro would let a player name a macro after it"
        );
    }
}

#[test]
fn every_gated_verb_in_production_is_unnameable() {
    let composition = production_composition();
    let reg = composition.registry();
    let w = World::new_minimal();

    let gated: Vec<String> = reg
        .specs()
        .filter(|s| {
            s.min_role != Role::Player
                || s.power.is_some()
                || s.privilege.is_some()
                || s.lock.is_some()
        })
        .flat_map(|s| {
            std::iter::once(s.name.to_string()).chain(s.aliases.iter().map(ToString::to_string))
        })
        .collect();
    assert!(
        !gated.is_empty(),
        "the production registry has no gated verb; this gate proves nothing"
    );
    for word in &gated {
        assert_eq!(
            name_refusal(reg, &w, GOD, word),
            Some(BAD_NAME),
            "{word:?} is a gated verb that @macro would refuse as TAKEN rather than \
             on its characters: a bare staff verb needs a person's ruling"
        );
    }
}
